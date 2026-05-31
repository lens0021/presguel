//! `org.freedesktop.IBus.Engine` 구현. presguel-core 의 조합 엔진을 감싼다.
//!
//! 키 이벤트(method)를 받아 조합하고, 결과를 CommitText / UpdatePreeditText
//! (signal)로 데몬에 돌려준다. 참고: `research/03-ibus-zbus.md` §2,§4.

use std::collections::HashMap;

use presguel_core::expr::{Ctx, Expr, Value as ExprValue};
use presguel_core::{us_qwerty_ascii, Config, Engine as Core};
use zbus::object_server::SignalEmitter;
use zbus::{fdo, interface};
use zbus::zvariant::Value;

use crate::ibus_property::{make_input_mode_property, make_prop_list};
use crate::ibus_text::{make_ibus_text, make_preedit_text};
use crate::settings::Settings;

// 수식어/키 마스크 (research/03 §4, 실측).
const RELEASE_MASK: u32 = 1 << 30;
const SHIFT_MASK: u32 = 1 << 0;
const LOCK_MASK: u32 = 1 << 1; // Caps Lock
const CONTROL_MASK: u32 = 1 << 2;
const MOD1_MASK: u32 = 1 << 3; // Alt
const SUPER_MASK: u32 = 1 << 26;
const META_MASK: u32 = 1 << 28;
const SPECIAL_MODS: u32 = CONTROL_MASK | MOD1_MASK | SUPER_MASK | META_MASK;

// 키심(keysym).
const KEY_BACKSPACE: u32 = 0xff08;
const KEY_HANGUL: u32 = 0xff31;

/// `PRESGUEL_DEBUG_KEYS` 환경변수가 켜져 있으면 키 이벤트를 stderr 로 로깅한다.
fn debug_keys_enabled() -> bool {
    matches!(
        std::env::var("PRESGUEL_DEBUG_KEYS").ok().as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// 정수를 유니코드 아래첨자 숫자(U+2080..U+2089)로 만든다. 패널 심볼의 항목 번호용.
fn subscript_digits(n: usize) -> String {
    n.to_string()
        .chars()
        .map(|c| char::from_u32(0x2080 + (c as u32 - '0' as u32)).unwrap_or(c))
        .collect()
}

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
    /// 한글 조합 항목(KeyTable 에 H3| 낱자가 있는 항목). Core 가 커서 박싱.
    Hangul(Box<Core>),
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
    /// 전체 모드에서 기본으로 시작할 항목 인덱스(설정의 default).
    default_entry: usize,
    /// IME_SWITCH 키심 → 전환 대상 항목을 정하는 식. 식의 변수 `A` = 현재 항목 인덱스.
    /// 예: `!A` 는 0↔1 토글(0이면 1, 아니면 0). 설정 ShortcutTable 의 value 를 그대로 평가.
    ime_switch: HashMap<u32, Expr>,
    /// 마지막으로 반영한 사용자 설정(config.ini). focus_in/enable 마다 다시 읽어 즉시 반영.
    settings: Settings,
    /// 간단 모드에서 쓸 한글 항목 인덱스(settings 에서 파생, 항목 수로 클램프).
    hangul_idx: usize,
    /// 간단 모드에서 한/영 토글의 상대 항목 인덱스.
    latin_idx: usize,
}

impl IBusEngine {
    /// 설정 파일(config.ini)을 읽어 엔진을 만든다.
    pub fn new(config: &Config) -> Self {
        Self::with_settings(config, Settings::load())
    }

    /// 명시한 설정으로 엔진을 만든다(테스트는 이걸 써서 전역 config.ini 에 의존하지 않는다).
    pub fn with_settings(config: &Config, st: Settings) -> Self {
        // 모든 입력 항목을 컴파일한다. KeyTable 에 H3| 낱자가 있으면 한글 조합 항목,
        // 아니면 로마자/직접(문자만 내보냄) 항목으로 본다.
        let mut entries = Vec::new();
        for i in 0..config.entries.len() {
            match config.compile(i) {
                Ok(layout) => {
                    let is_hangul = layout.keys.values().any(|e| e.contains_unit());
                    if is_hangul {
                        entries.push(Mode::Hangul(Box::new(Core::new(layout))));
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
        let last = entries.len() - 1;

        // usage=IME_SWITCH 단축글쇠: value 식(예 "!A")을 키심에 매핑.
        let mut ime_switch: HashMap<u32, Expr> = HashMap::new();
        for sc in &config.editor.shortcuts {
            if sc.usage == "IME_SWITCH" {
                if let Ok(expr) = Expr::parse(&sc.value) {
                    for &ks in vk_to_keysyms(&sc.key) {
                        ime_switch.insert(ks, expr.clone());
                    }
                }
            }
        }
        // 한/영 키(0xff31)는 설정에 IME_SWITCH 가 없어도 기본 !A(0↔1) 로 동작.
        ime_switch
            .entry(KEY_HANGUL)
            .or_insert_with(|| Expr::parse("!A").expect("valid default switch expr"));

        let default_entry = config.default_entry.min(last);
        let mut engine = Self {
            entries,
            current: default_entry,
            default_entry,
            ime_switch,
            settings: Settings::default(), // apply_settings 가 곧 덮어쓴다
            hangul_idx: 0,
            latin_idx: 0,
        };
        engine.apply_settings(st);
        engine
    }

    /// 사용자 설정을 반영한다(파생 필드와 현재 항목 보정). 항상 적용한다.
    fn apply_settings(&mut self, st: Settings) {
        let last = self.entries.len() - 1;
        let was_simple = self.settings.simple_mode;
        let hangul_idx = st.hangul_entry.min(last);
        let latin_idx = st.latin_entry.min(last);
        self.hangul_idx = hangul_idx;
        self.latin_idx = latin_idx;
        if st.simple_mode {
            // 간단 모드 진입/항목 변경 시 현재 항목을 {한글, 영문} 안으로 보정.
            if !was_simple || (self.current != hangul_idx && self.current != latin_idx) {
                self.current = hangul_idx;
            }
        } else if was_simple {
            // 간단 → 전체 전환: 기본 항목으로.
            self.current = self.default_entry.min(last);
        }
        self.settings = st;
    }

    /// config.ini 를 다시 읽어 바뀌었으면 반영한다. 바뀜 여부를 돌려준다.
    /// focus_in/enable 에서 호출 → 설정창 변경이 입력창 클릭 시 즉시 반영(재시작 불필요).
    fn reload_settings(&mut self) -> bool {
        let st = Settings::load();
        if st == self.settings {
            return false;
        }
        self.apply_settings(st);
        true
    }

    /// IME_SWITCH 키를 눌렀을 때 전환할 대상 항목 인덱스.
    /// - 간단 모드: 한글 항목 ↔ 영문 항목만 오간다(설정에서 고른 둘).
    /// - 전체 모드: ShortcutTable value 식(예 `!A`, A=현재 항목)을 평가. `!A`→0이면 1, 아니면 0.
    fn switch_target(&self, keyval: u32) -> usize {
        if self.settings.simple_mode {
            return if self.current == self.hangul_idx { self.latin_idx } else { self.hangul_idx };
        }
        let len = self.entries.len() as i64;
        self.ime_switch
            .get(&keyval)
            .and_then(|e| e.eval(&Ctx { a: self.current as i64, ..Default::default() }).ok())
            .and_then(|v| match v {
                ExprValue::Int(t) => Some(t),
                _ => None,
            })
            .map(|t| t.clamp(0, len - 1) as usize)
            .unwrap_or(self.current)
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

    /// 패널 심볼: 접두(한글=가, 로마자/직접=A)에 항목 번호를 아래첨자로 붙인다(예: "가₀").
    /// 간단 모드에서는 한글/영문 둘뿐이라 번호 없이 접두만 보인다(예: "가", "A").
    fn mode_symbol(&self) -> String {
        let prefix = self.cur().symbol_prefix();
        if self.settings.simple_mode {
            prefix.to_string()
        } else {
            format!("{}{}", prefix, subscript_digits(self.current))
        }
    }

    /// 입력 모드 속성을 등록(패널이 심볼을 알도록). focus_in/enable 시 호출.
    /// 레이블("Presguel 설정")은 패널 컨텍스트 메뉴에 뜨며, 누르면 property_activate 가 설정창을 연다.
    async fn register_props(&self, se: &SignalEmitter<'_>) {
        let _ = Self::register_properties(se, make_prop_list(&self.mode_symbol(), "Presguel 설정")).await;
    }

    /// 모드가 바뀌었을 때 패널 심볼을 갱신.
    async fn update_indicator(&self, se: &SignalEmitter<'_>) {
        let _ = Self::update_property(se, make_input_mode_property(&self.mode_symbol(), "Presguel 설정")).await;
    }

    /// 키 이벤트를 분류한다(순수 함수). `process_key_event` 가 이 결과로 분기한다.
    /// IME_SWITCH 는 release/수식어보다 먼저 본다 — CapsLock 은 수식어 키심이기도 하므로.
    ///
    /// 인쇄 가능 키는 **물리 위치(keycode)** 를 US-QWERTY ASCII 로 환산해 KeyTable 을
    /// 조회한다(날개셋 모델). 그러면 사용자 XKB 가 드보락이어도 세벌식 자리가 고정된다.
    /// keycode 가 없거나(프로그램 주입) 매핑 밖이면 keyval 로 폴백한다.
    fn classify(&self, keyval: u32, keycode: u32, state: u32) -> KeyClass {
        if self.ime_switch.contains_key(&keyval) {
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
        let shift = state & SHIFT_MASK != 0;
        if let Some(ascii) = us_qwerty_ascii(keycode, shift) {
            return KeyClass::Printable(ascii);
        }
        if (0x20..=0x7e).contains(&keyval) {
            return KeyClass::Printable(keyval as u8); // keycode 없음 → keyval 폴백
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
        keycode: u32,
        state: u32,
    ) -> fdo::Result<bool> {
        // 진단: PRESGUEL_DEBUG_KEYS=1 이면 받은 keyval/keycode/state 를 stderr 로 찍는다.
        // presguel 활성 시 어떤 XKB 레이아웃 기준 keysym 이 오는지 확인용(드보락 vs us).
        if debug_keys_enabled() {
            let ch = char::from_u32(keyval).filter(|c| !c.is_control()).unwrap_or(' ');
            eprintln!(
                "presguel keyev: keyval=0x{keyval:04x} ({ch:?})  keycode=0x{keycode:x} ({keycode})  state=0x{state:x}"
            );
        }
        let class = self.classify(keyval, keycode, state);
        let release = state & RELEASE_MASK != 0;
        match class {
            // IME_SWITCH(한/영·CapsLock 등): 눌림/뗌 모두 소비, 눌림에서 전환식을 평가.
            // value 식(예 "!A")을 A=현재 항목으로 평가해 대상 항목을 얻는다. `!A` → 0이면 1, 아니면 0.
            KeyClass::ImeSwitch => {
                if !release {
                    let target = self.switch_target(keyval);
                    if target != self.current {
                        self.flush_current(&se).await;
                        self.current = target;
                        self.update_indicator(&se).await; // 패널 심볼(가N/AN) 갱신
                    }
                }
                Ok(true)
            }
            // 뗌·수식어 키 자체: 조합에 영향 없이 통과.
            KeyClass::Release | KeyClass::Modifier => Ok(false),
            // Ctrl/Alt/Super/Meta 조합(단축키): 조합만 확정하고 응용에 넘긴다.
            // 단축키 레이아웃은 IME 가 아니라 XKB(사용자 레이아웃)의 몫 — Wayland 에서
            // IME 의 ForwardKeyEvent 로 키 위치를 바꾸는 건 불가하므로 흉내 내지 않는다.
            // 사용자가 드보락 단축키를 원하면 자신의 키보드 레이아웃을 드보락으로 둔다.
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
        // 설정창 변경을 즉시 반영(재시작 불필요): 입력 컨텍스트가 잡힐 때마다 config.ini 재확인.
        self.reload_settings();
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
        self.reload_settings();
        self.register_props(&se).await;
        Ok(())
    }

    async fn disable(&mut self, #[zbus(signal_emitter)] se: SignalEmitter<'_>) -> fdo::Result<()> {
        self.flush_current(&se).await;
        Ok(())
    }

    fn set_capabilities(&mut self, _caps: u32) {}

    fn set_cursor_location(&mut self, _x: i32, _y: i32, _w: i32, _h: i32) {}

    fn property_activate(&mut self, name: String, _state: u32) {
        // 패널 컨텍스트 메뉴에서 InputMode 속성(설정 항목)을 누르면 설정창을 띄운다.
        if name == "InputMode" {
            let _ = std::process::Command::new("presguel-setup")
                .spawn()
                .or_else(|_| {
                    std::process::Command::new("/usr/local/bin/presguel-setup").spawn()
                });
        }
    }

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

    /// 전역 config.ini 에 의존하지 않도록 기본 설정(전체 모드)으로 만든다.
    fn engine() -> IBusEngine {
        let cfg = Config::parse(MINI).unwrap();
        IBusEngine::with_settings(&cfg, Settings::default())
    }

    #[test]
    fn subscript_digits_maps_to_unicode() {
        assert_eq!(subscript_digits(0), "₀");
        assert_eq!(subscript_digits(2), "₂");
        assert_eq!(subscript_digits(10), "₁₀");
    }

    #[test]
    fn mode_symbol_full_uses_subscript() {
        // 전체 모드(기본): 접두 + 아래첨자 항목번호. MINI 는 한글 항목 1개, current=0.
        let e = engine();
        assert_eq!(e.mode_symbol(), "가₀");
    }

    #[test]
    fn mode_symbol_simple_has_no_number() {
        // 간단 모드: 번호 없이 접두만.
        let cfg = Config::parse(MINI).unwrap();
        let e = IBusEngine::with_settings(
            &cfg,
            Settings { simple_mode: true, hangul_entry: 0, latin_entry: 0 },
        );
        assert_eq!(e.mode_symbol(), "가");
    }

    #[test]
    fn classify_uses_keycode_not_keyval() {
        // 핵심: 인쇄 키는 keycode(물리 위치)로 분류한다. keyval 이 드보락이어도(예: 'p'=0x70)
        // keycode 19(물리 R)면 US-QWERTY 'r' 로 본다 → 세벌식 자리 고정.
        let e = engine();
        // keyval=0x70('p', 드보락), keycode=19(물리 R), modifier 없음 → Printable('r')
        assert_eq!(e.classify(0x70, 19, 0), KeyClass::Printable(b'r'));
        // Shift+물리2(keycode 3) → '@' (세벌식 shifted 자모 인덱스)
        assert_eq!(e.classify(0x32, 3, SHIFT_MASK), KeyClass::Printable(b'@'));
        // keycode 0(프로그램 주입) → keyval 폴백
        assert_eq!(e.classify(b'k' as u32, 0, 0), KeyClass::Printable(b'k'));
    }

    #[test]
    fn default_settings_is_full_mode() {
        let e = engine();
        assert!(!e.settings.simple_mode);
    }

    #[test]
    fn apply_simple_mode_sets_current() {
        let cfg = Config::parse(MINI).unwrap();
        let st = Settings { simple_mode: true, hangul_entry: 0, latin_entry: 0 };
        let e = IBusEngine::with_settings(&cfg, st);
        assert!(e.settings.simple_mode);
        assert_eq!(e.current, 0); // 간단 모드 → 한글 항목에서 시작
    }

    #[test]
    fn reload_detects_change() {
        // 같은 설정이면 reload 가 false, 바뀌면 true 를 돌려준다(파일 IO 없이 내부 상태로).
        let mut e = engine();
        let before = e.settings;
        // settings 가 같으면(파일이 default 와 동일하거나 없으면) 변화 감지 안 함.
        // 여기선 apply_settings 로 직접 바꿔 동작만 확인.
        e.apply_settings(Settings { simple_mode: true, hangul_entry: 0, latin_entry: 0 });
        assert_ne!(before.simple_mode, e.settings.simple_mode);
        assert!(e.settings.simple_mode);
    }

    #[test]
    fn capslock_in_switch_set() {
        let e = engine();
        // 설정의 VK_CAPITAL → Caps_Lock(0xffe5), VK_HANGUL → 0xff31
        assert!(e.ime_switch.contains_key(&0xffe5));
        assert!(e.ime_switch.contains_key(&0xff31));
        // VK_HANJA 는 KEYCHAR 라 전환 집합에 없어야 한다.
        assert!(!e.ime_switch.contains_key(&0xff34));
    }

    #[test]
    fn switch_expr_is_not_a_toggle() {
        use presguel_core::expr::{Ctx, Value as EV};
        let e = engine();
        // ShortcutTable value="!A" → A=현재 항목. 0이면 1, 아니면 0.
        let expr = e.ime_switch.get(&0xffe5).expect("capslock switch expr");
        let f = |cur: i64| match expr.eval(&Ctx { a: cur, ..Default::default() }).unwrap() {
            EV::Int(t) => t,
            other => panic!("expected int, got {other:?}"),
        };
        assert_eq!(f(0), 1);
        assert_eq!(f(1), 0);
        assert_eq!(f(2), 0);
    }

    #[test]
    fn shift_is_modifier_not_function_key() {
        let e = engine();
        // 버그 재현 방지: Shift 는 Modifier(통과)여야지, FunctionKey(조합 확정)면 안 된다.
        assert_eq!(e.classify(0xffe1, 0, 0), KeyClass::Modifier); // Shift_L
        assert_eq!(e.classify(0xffe2, 0, 0), KeyClass::Modifier); // Shift_R
    }

    #[test]
    fn capslock_classifies_as_ime_switch_even_on_release() {
        let e = engine();
        assert_eq!(e.classify(0xffe5, 0, 0), KeyClass::ImeSwitch);
        assert_eq!(e.classify(0xffe5, 0, RELEASE_MASK), KeyClass::ImeSwitch);
    }

    #[test]
    fn hangul_key_is_ime_switch() {
        assert_eq!(engine().classify(0xff31, 0, 0), KeyClass::ImeSwitch);
    }

    #[test]
    fn at_key_with_shift_is_printable() {
        // 실키 Shift+물리2(keycode 3) → US-QWERTY '@' 로 분류 → 세벌식 ㄺ 조합 가능.
        assert_eq!(engine().classify(0x40, 3, SHIFT_MASK), KeyClass::Printable(b'@'));
    }

    #[test]
    fn ctrl_combo_is_shortcut() {
        // Ctrl+물리C(keycode 46) → 단축키(통과). keycode 있어도 ShortcutCombo 가 우선.
        assert_eq!(engine().classify(b'c' as u32, 46, CONTROL_MASK), KeyClass::ShortcutCombo);
    }

    #[test]
    fn release_of_normal_key_ignored() {
        assert_eq!(engine().classify(b'k' as u32, 37, RELEASE_MASK), KeyClass::Release);
    }

    #[test]
    fn backspace_classified() {
        assert_eq!(engine().classify(KEY_BACKSPACE, 14, 0), KeyClass::Backspace);
    }
}
