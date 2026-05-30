//! `org.freedesktop.IBus.Engine` 구현. presguel-core 의 조합 엔진을 감싼다.
//!
//! 키 이벤트(method)를 받아 조합하고, 결과를 CommitText / UpdatePreeditText
//! (signal)로 데몬에 돌려준다. 참고: `research/03-ibus-zbus.md` §2,§4.

use std::collections::{HashMap, HashSet};

use presguel_core::expr::{Ctx, Expr, Value as ExprValue};
use presguel_core::{Config, Engine as Core};
use zbus::object_server::SignalEmitter;
use zbus::{fdo, interface};
use zbus::zvariant::Value;

use crate::ibus_property::{make_input_mode_property, make_prop_list};
use crate::ibus_text::{make_ibus_text, make_preedit_text};

// 수식어/키 마스크 (research/03 §4, 실측).
const RELEASE_MASK: u32 = 1 << 30;
const LOCK_MASK: u32 = 1 << 1; // Caps Lock
const CONTROL_MASK: u32 = 1 << 2;
const MOD1_MASK: u32 = 1 << 3; // Alt
const SUPER_MASK: u32 = 1 << 26;
const META_MASK: u32 = 1 << 28;
const SPECIAL_MODS: u32 = CONTROL_MASK | MOD1_MASK | SUPER_MASK | META_MASK;

// 키심(keysym).
const KEY_BACKSPACE: u32 = 0xff08;
const KEY_HANGUL: u32 = 0xff31;

/// 수식어 키 자체(Shift/Ctrl/Caps/Meta/Alt/Super/Hyper, ISO_Level shifts, Mode_switch)인가.
/// 이런 키는 텍스트가 아니므로 조합에 영향을 주지 않고 그대로 통과시켜야 한다.
fn is_modifier_keysym(keyval: u32) -> bool {
    (0xffe1..=0xffee).contains(&keyval) // Shift_L..Hyper_R
        || (0xfe01..=0xfe0f).contains(&keyval) // ISO_Lock, ISO_LevelN_Shift 등
        || keyval == 0xff7e // Mode_switch (AltGr 류)
}

/// 날개셋 ShortcutTable 의 가상키(VK_*) 이름 → X11/ibus 키심.
fn vk_to_keysyms(vk: &str) -> &'static [u32] {
    match vk {
        "VK_HANGUL" => &[0xff31],        // Hangul (한/영)
        "VK_HANJA" => &[0xff34],         // Hangul_Hanja (한자)
        "VK_CAPITAL" => &[0xffe5],       // Caps_Lock
        "VK_SPACE" => &[0x20],
        "VK_RMENU" => &[0xffea],         // Alt_R (오른쪽 Alt, 한/영 대용)
        "VK_RCONTROL" => &[0xffe4],      // Control_R (한자 대용)
        _ => &[],
    }
}

/// 키 분류(순수 함수 결과). 라우팅 로직을 D-Bus 비동기와 분리해 단위 테스트한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyClass {
    Release,
    ImeSwitch,
    Modifier,
    ShortcutCombo,
    Backspace,
    Printable(u8),
    FunctionKey,
}

/// 한 입력 항목의 처리 방식.
enum Mode {
    /// 한글 조합 항목(KeyTable 에 H3| 낱자가 있는 항목).
    Hangul(Core),
    /// 로마자/직접 항목: KeyTable 로 문자만 내보내고(드보락 등), 키표가 없으면 패스스루.
    Latin { keys: HashMap<u32, Expr> },
}

impl Mode {
    fn is_hangul(&self) -> bool {
        matches!(self, Mode::Hangul(_))
    }
    /// 패널 심볼 접두(날개셋 방식: 한글=가, 로마자/직접=A).
    fn symbol_prefix(&self) -> &'static str {
        if self.is_hangul() {
            "가"
        } else {
            "A"
        }
    }
}

/// IBus 엔진 인스턴스 하나. 설정의 모든 입력 항목을 담고 IME_SWITCH 로 순환 전환한다.
/// 패널 표시기는 날개셋처럼 `접두+항목번호`(예: `가0`, `A1`)로 보인다.
pub struct IBusEngine {
    entries: Vec<Mode>,
    /// 현재 활성 입력 항목 인덱스.
    current: usize,
    /// IME_SWITCH 를 일으키는 키심들(설정 ShortcutTable 에서 해석).
    ime_switch: HashSet<u32>,
}

impl IBusEngine {
    pub fn new(config: &Config) -> Self {
        // 모든 입력 항목을 컴파일한다. KeyTable 에 H3| 낱자가 있으면 한글 조합 항목,
        // 아니면 로마자/직접(문자만 내보냄) 항목으로 본다.
        let mut entries = Vec::new();
        for i in 0..config.entries.len() {
            match config.compile(i) {
                Ok(layout) => {
                    let is_hangul = layout.keys.values().any(|e| e.contains_unit());
                    if is_hangul {
                        entries.push(Mode::Hangul(Core::new(layout)));
                    } else {
                        entries.push(Mode::Latin { keys: layout.keys });
                    }
                }
                Err(_) => entries.push(Mode::Latin { keys: HashMap::new() }),
            }
        }
        if entries.is_empty() {
            entries.push(Mode::Latin { keys: HashMap::new() });
        }
        let current = config.default_entry.min(entries.len() - 1);

        // usage=IME_SWITCH 단축글쇠의 키심. 한/영 키(0xff31)는 항상 포함.
        let mut ime_switch: HashSet<u32> = HashSet::new();
        ime_switch.insert(KEY_HANGUL);
        for sc in &config.editor.shortcuts {
            if sc.usage == "IME_SWITCH" {
                ime_switch.extend(vk_to_keysyms(&sc.key).iter().copied());
            }
        }
        Self { entries, current, ime_switch }
    }

    fn cur(&self) -> &Mode {
        &self.entries[self.current]
    }

    /// 확정 문자열과 preedit 를 신호로 내보낸다.
    async fn emit(se: &SignalEmitter<'_>, commit: &str, preedit: &str) {
        if !commit.is_empty() {
            let _ = Self::commit_text(se, make_ibus_text(commit.to_string())).await;
        }
        let cursor = preedit.chars().count() as u32;
        let _ = Self::update_preedit_text(
            se,
            make_preedit_text(preedit.to_string()),
            cursor,
            !preedit.is_empty(),
            0, // IBusPreeditFocusMode::CLEAR
        )
        .await;
    }

    /// 현재 항목이 한글 조합이면 조합을 확정해 내보낸다.
    async fn flush_current(&mut self, se: &SignalEmitter<'_>) {
        let i = self.current;
        if let Mode::Hangul(core) = &mut self.entries[i] {
            let commit = core.flush();
            if !commit.is_empty() {
                Self::emit(se, &commit, "").await;
            }
        }
    }

    /// 패널 심볼: 날개셋 방식의 `접두 + 항목번호`. 한글=가, 로마자/직접=A. 예: "가0", "A1".
    /// (항목을 추가하면 번호가 함께 늘어난다.)
    fn mode_symbol(&self) -> String {
        format!("{}{}", self.cur().symbol_prefix(), self.current)
    }

    /// 입력 모드 속성을 등록(패널이 심볼을 알도록). focus_in/enable 시 호출.
    async fn register_props(&self, se: &SignalEmitter<'_>) {
        let _ = Self::register_properties(se, make_prop_list(&self.mode_symbol(), "Presguel")).await;
    }

    /// 모드가 바뀌었을 때 패널 심볼을 갱신.
    async fn update_indicator(&self, se: &SignalEmitter<'_>) {
        let _ = Self::update_property(se, make_input_mode_property(&self.mode_symbol(), "Presguel")).await;
    }

    /// 키 이벤트를 분류한다(순수 함수). `process_key_event` 가 이 결과로 분기한다.
    /// IME_SWITCH 는 release/수식어보다 먼저 본다 — CapsLock 은 수식어 키심이기도 하므로.
    fn classify(&self, keyval: u32, state: u32) -> KeyClass {
        if self.ime_switch.contains(&keyval) {
            return KeyClass::ImeSwitch;
        }
        if state & RELEASE_MASK != 0 {
            return KeyClass::Release;
        }
        if is_modifier_keysym(keyval) {
            return KeyClass::Modifier;
        }
        if state & SPECIAL_MODS != 0 {
            return KeyClass::ShortcutCombo;
        }
        if keyval == KEY_BACKSPACE {
            return KeyClass::Backspace;
        }
        if (0x20..=0x7e).contains(&keyval) {
            return KeyClass::Printable(keyval as u8);
        }
        KeyClass::FunctionKey
    }
}

#[interface(name = "org.freedesktop.IBus.Engine")]
impl IBusEngine {
    async fn process_key_event(
        &mut self,
        #[zbus(signal_emitter)] se: SignalEmitter<'_>,
        keyval: u32,
        _keycode: u32,
        state: u32,
    ) -> fdo::Result<bool> {
        let class = self.classify(keyval, state);
        let release = state & RELEASE_MASK != 0;
        match class {
            // IME_SWITCH(한/영·CapsLock 등): 눌림/뗌 모두 소비, 눌림에서만 다음 항목으로 순환.
            KeyClass::ImeSwitch => {
                if !release {
                    self.flush_current(&se).await;
                    self.current = (self.current + 1) % self.entries.len();
                    self.update_indicator(&se).await; // 패널 심볼(가N/AN) 갱신
                }
                Ok(true)
            }
            // 뗌·수식어 키 자체: 조합에 영향 없이 통과.
            KeyClass::Release | KeyClass::Modifier => Ok(false),
            // Ctrl/Alt/Super/Meta 조합(단축키): 조합 확정 후 응용에 넘김.
            KeyClass::ShortcutCombo => {
                self.flush_current(&se).await;
                Ok(false)
            }
            // 나머지는 현재 항목의 방식에 따라 처리.
            KeyClass::Backspace | KeyClass::Printable(_) | KeyClass::FunctionKey => {
                let caps = state & LOCK_MASK != 0;
                let i = self.current;
                match &mut self.entries[i] {
                    // 한글 조합 항목.
                    Mode::Hangul(core) => match class {
                        KeyClass::Backspace => {
                            if core.is_empty() {
                                return Ok(false);
                            }
                            let out = core.backspace();
                            Self::emit(&se, &out.commit, &out.preedit).await;
                            Ok(out.consumed)
                        }
                        KeyClass::Printable(ascii) => {
                            let out = core.press(ascii, caps);
                            Self::emit(&se, &out.commit, &out.preedit).await;
                            Ok(out.consumed)
                        }
                        _ => {
                            // 기능키: 조합 확정 후 통과.
                            let commit = core.flush();
                            if !commit.is_empty() {
                                Self::emit(&se, &commit, "").await;
                            }
                            Ok(false)
                        }
                    },
                    // 로마자/직접 항목: KeyTable 로 문자만 내보내고, 매핑 없으면 패스스루.
                    Mode::Latin { keys } => {
                        if let KeyClass::Printable(ascii) = class {
                            if let Some(expr) = keys.get(&(ascii as u32)) {
                                let ctx = Ctx { p: caps as i64, ..Default::default() };
                                if let Ok(ExprValue::Int(n)) = expr.eval(&ctx) {
                                    if let Some(ch) = u32::try_from(n).ok().and_then(char::from_u32) {
                                        Self::emit(&se, &ch.to_string(), "").await;
                                        return Ok(true);
                                    }
                                }
                            }
                        }
                        Ok(false) // 매핑 없음 / 백스페이스 / 기능키 → 응용에 넘김
                    }
                }
            }
        }
    }

    async fn focus_in(&mut self, #[zbus(signal_emitter)] se: SignalEmitter<'_>) -> fdo::Result<()> {
        self.register_props(&se).await;
        Ok(())
    }

    async fn focus_out(&mut self, #[zbus(signal_emitter)] se: SignalEmitter<'_>) -> fdo::Result<()> {
        self.flush_current(&se).await;
        Ok(())
    }

    async fn reset(&mut self, #[zbus(signal_emitter)] se: SignalEmitter<'_>) -> fdo::Result<()> {
        let i = self.current;
        if let Mode::Hangul(core) = &mut self.entries[i] {
            core.reset();
        }
        Self::emit(&se, "", "").await;
        Ok(())
    }

    async fn enable(&mut self, #[zbus(signal_emitter)] se: SignalEmitter<'_>) -> fdo::Result<()> {
        self.register_props(&se).await;
        Ok(())
    }

    async fn disable(&mut self, #[zbus(signal_emitter)] se: SignalEmitter<'_>) -> fdo::Result<()> {
        self.flush_current(&se).await;
        Ok(())
    }

    fn set_capabilities(&mut self, _caps: u32) {}

    fn set_cursor_location(&mut self, _x: i32, _y: i32, _w: i32, _h: i32) {}

    fn property_activate(&mut self, _name: String, _state: u32) {}

    fn page_up(&mut self) {}
    fn page_down(&mut self) {}
    fn cursor_up(&mut self) {}
    fn cursor_down(&mut self) {}
    fn candidate_clicked(&mut self, _index: u32, _button: u32, _state: u32) {}

    // ── 신호(engine → daemon) ────────────────────────────────────────────────

    #[zbus(signal)]
    async fn commit_text(se: &SignalEmitter<'_>, text: Value<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn update_preedit_text(
        se: &SignalEmitter<'_>,
        text: Value<'_>,
        cursor_pos: u32,
        visible: bool,
        mode: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn forward_key_event(
        se: &SignalEmitter<'_>,
        keyval: u32,
        keycode: u32,
        state: u32,
    ) -> zbus::Result<()>;

    /// 패널에 속성(입력 모드 표시기) 목록을 등록.
    #[zbus(signal)]
    async fn register_properties(se: &SignalEmitter<'_>, props: Value<'_>) -> zbus::Result<()>;

    /// 모드 변경 시 패널 속성(심볼)을 갱신.
    #[zbus(signal)]
    async fn update_property(se: &SignalEmitter<'_>, prop: Value<'_>) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use presguel_core::Config;

    // VK_HANGUL 과 VK_CAPITAL 을 IME_SWITCH 로 둔 최소 설정.
    const MINI: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<EditContextSetting version="0x500">
  <EditorLayer flag="0">
    <ShortcutTable>
      <Shortcut key="VK_HANGUL" usage="IME_SWITCH" value="!A"/>
      <Shortcut key="VK_CAPITAL" modifier="DONT_EAT|KEEP_LAMP" usage="IME_SWITCH" value="!A"/>
      <Shortcut key="VK_HANJA" usage="KEYCHAR" value="C0|0x82"/>
    </ShortcutTable>
    <FinalConvTable><FinalConv from="0x1100" to="0x3131"/></FinalConvTable>
  </EditorLayer>
  <InputLayer default="0" current="0">
    <InputEntry>
      <InputSchemeSetting object="CBasicInputScheme">
        <KeyTable name="mini" flag="0" from="33" to="126">
          <Key at="0x6B" value="H3|G_"/>
          <Key at="0x40" value="T ? H3|_RG : 0x40"/>
        </KeyTable>
      </InputSchemeSetting>
      <GeneratorSetting object="CNgsImeEx">
        <UnitMixTable/><VirtualUnitTable/><AutomataTable default="0"/>
      </GeneratorSetting>
    </InputEntry>
  </InputLayer>
</EditContextSetting>"#;

    fn engine() -> IBusEngine {
        let cfg = Config::parse(MINI).unwrap();
        IBusEngine::new(&cfg)
    }

    #[test]
    fn capslock_in_switch_set() {
        let e = engine();
        // 설정의 VK_CAPITAL → Caps_Lock(0xffe5), VK_HANGUL → 0xff31
        assert!(e.ime_switch.contains(&0xffe5));
        assert!(e.ime_switch.contains(&0xff31));
        // VK_HANJA 는 KEYCHAR 라 전환 집합에 없어야 한다.
        assert!(!e.ime_switch.contains(&0xff34));
    }

    #[test]
    fn shift_is_modifier_not_function_key() {
        let e = engine();
        // 버그 재현 방지: Shift 는 Modifier(통과)여야지, FunctionKey(조합 확정)면 안 된다.
        assert_eq!(e.classify(0xffe1, 0), KeyClass::Modifier); // Shift_L
        assert_eq!(e.classify(0xffe2, 0), KeyClass::Modifier); // Shift_R
    }

    #[test]
    fn capslock_classifies_as_ime_switch_even_on_release() {
        let e = engine();
        assert_eq!(e.classify(0xffe5, 0), KeyClass::ImeSwitch);
        assert_eq!(e.classify(0xffe5, RELEASE_MASK), KeyClass::ImeSwitch);
    }

    #[test]
    fn hangul_key_is_ime_switch() {
        assert_eq!(engine().classify(0xff31, 0), KeyClass::ImeSwitch);
    }

    #[test]
    fn at_key_with_shift_is_printable() {
        // 실키 Shift+2 는 keyval 0x40('@') + SHIFT 상태로 도착 → 인쇄키로 분류되어 ㄺ 조합 가능.
        assert_eq!(engine().classify(0x40, 1 /*SHIFT*/), KeyClass::Printable(0x40));
    }

    #[test]
    fn ctrl_combo_is_shortcut() {
        assert_eq!(engine().classify(b'c' as u32, CONTROL_MASK), KeyClass::ShortcutCombo);
    }

    #[test]
    fn release_of_normal_key_ignored() {
        assert_eq!(engine().classify(b'k' as u32, RELEASE_MASK), KeyClass::Release);
    }

    #[test]
    fn backspace_classified() {
        assert_eq!(engine().classify(KEY_BACKSPACE, 0), KeyClass::Backspace);
    }
}
