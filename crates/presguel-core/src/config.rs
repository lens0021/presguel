//! `nalgaeset.xml`(날개셋 입력 설정) 파싱과 엔진용 컴파일.
//!
//! 문서 구조: `EditContextSetting > {EditorLayer, InputLayer > InputEntry*}`.
//! 각 InputEntry 는 `InputSchemeSetting`(KeyTable)과 `GeneratorSetting`
//! (UnitMix/VirtualUnit/Automata/Bksp)을 가진다.
//! 참고: `research/01-nalgaeset-format.md`, `research/02-config-decode.md`.

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use thiserror::Error;

use crate::expr::{Expr, ExprError};
use crate::unit::{self, Category, Jamo, Unit};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("XML 파싱 오류: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("XML 속성 오류: {0}")]
    Attr(#[from] quick_xml::events::attributes::AttrError),
    #[error("키 {at:?} 의 값-식 파싱 실패: {source}")]
    KeyExpr { at: String, source: ExprError },
    #[error("정수 파싱 실패: {0:?}")]
    BadInt(String),
    #[error("알 수 없는 낱자 갈래 {0:?} (CHO/JUNG/JONG 이어야 함)")]
    BadCategory(String),
    #[error("UnitMix/VirtualUnit 의 낱자 {0:?} 해석 실패")]
    BadUnit(String),
    #[error("InputEntry 인덱스 {0} 없음")]
    NoEntry(usize),
    #[error("오토마타 상태 {state} 의 식 파싱 실패: {source}")]
    AutomataExpr { state: i64, source: ExprError },
}

// ── 파싱된(raw) 모델 ─────────────────────────────────────────────────────────

/// 전체 설정.
#[derive(Debug, Clone)]
pub struct Config {
    pub version: String,
    pub editor: EditorLayer,
    pub default_entry: usize,
    pub current_entry: usize,
    pub entries: Vec<InputEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct EditorLayer {
    pub flags: Vec<String>,
    pub shortcuts: Vec<Shortcut>,
    /// 조합용/옛 자모 → 호환 자모 (홑낱자 출력용).
    pub final_conv: HashMap<u32, u32>,
}

#[derive(Debug, Clone)]
pub struct Shortcut {
    pub key: String,
    pub modifier: Vec<String>,
    pub usage: String,
    pub value: String,
}

#[derive(Debug, Clone, Default)]
pub struct InputEntry {
    pub scheme_object: String,
    pub generator_object: String,
    pub key_table: Option<KeyTable>,
    pub unit_mix: Vec<UnitMix>,
    pub virtual_units: Vec<VirtualUnit>,
    pub automata_default: String,
    pub automata: Vec<AutomataRow>,
    pub bksp: Vec<Bksp>,
}

#[derive(Debug, Clone)]
pub struct KeyTable {
    pub name: String,
    pub from: u32,
    pub to: u32,
    /// ASCII at(0x21..0x7E) → 파싱된 값-식.
    pub keys: HashMap<u32, KeyDef>,
}

#[derive(Debug, Clone)]
pub struct KeyDef {
    pub raw: String,
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub struct UnitMix {
    pub unit: Category,
    pub a: String,
    pub b: String,
    pub to: String,
}

#[derive(Debug, Clone)]
pub struct VirtualUnit {
    pub unit: Category,
    pub from: u32,
    pub to: String,
}

#[derive(Debug, Clone)]
pub struct AutomataRow {
    pub state: i64,
    pub value: String,
    pub default: String,
    pub remark: String,
}

#[derive(Debug, Clone)]
pub struct Bksp {
    pub key: u32,
    pub value1: String,
    pub value2: String,
    pub condition1: String,
    pub condition2: String,
}

// ── 컴파일된(engine-ready) 모델 ──────────────────────────────────────────────

/// 컴파일된 오토마타 상태 한 칸. H3| 낱자 입력 시 현재 상태의 `value` 식을 평가해
/// 다음 상태/동작 코드를 얻는다. 식이 적용되지 않는 상황이면 `default` 를 쓴다.
/// (변수·결과값 의미는 `research/ngs-automata-help.txt` 참고.)
#[derive(Debug, Clone)]
pub struct AutomataState {
    pub value: Expr,
    pub default: Expr,
}

/// 백스페이스 한 번에 지우는 단위(날개셋 Bksp 수식값 0~3).
/// 참고: `research/ngs-automata-help.txt` 의 Bksp 설명(cp_bkspset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BkspUnit {
    /// 0: 직전에 입력된 한 타만 취소(겹낱자/토글 한 단계). 기존 기본 동작.
    #[default]
    LastKey,
    /// 1: 최하위 낱자(종성→중성→초성 순)의 직전 한 타.
    LowestLastKey,
    /// 2: 최하위 낱자 전체(그 낱자를 몇 타에 넣었든 통째로).
    LowestWhole,
    /// 3: 글자 전체(한 타에 음절 통째).
    Syllable,
}

/// 컴파일된 백스페이스 동작(한 Bksp 슬롯). 조합 중일 때(제1동작)와 그렇지 않을 때
/// (제2동작, 앞의 완성 글자)에 각각 어느 단위로 지울지와, 글자를 다 지운 뒤 앞 한글에
/// 달라붙어 재조합할지(BkspAttach)를 담는다.
#[derive(Debug, Clone, Default)]
pub struct BkspBehavior {
    /// 조합 중 삭제 단위(제1동작, value1).
    pub composing: BkspUnit,
    /// 비조합 상태에서 앞 완성 글자 삭제 단위(제2동작, value2).
    pub idle: BkspUnit,
    /// 글자를 다 지운 뒤 앞 한글에 달라붙어 재조합(BkspAttach).
    pub attach: bool,
}

/// Bksp value 문자열(예 `"ByUnitStep|BkspAttach"`, `"BySyllable"`, `"0"`)을 (단위, attach)로.
/// 플래그명과 정수(0~3)를 인식한다. 알 수 없으면 기본(LastKey, attach 없음).
fn parse_bksp_value(s: &str) -> (BkspUnit, bool) {
    let mut unit = BkspUnit::LastKey;
    let mut attach = false;
    for tok in s.split('|') {
        match tok.trim() {
            "BkspAttach" => attach = true,
            "ByUnitStep" | "0" => unit = BkspUnit::LastKey,
            "1" => unit = BkspUnit::LowestLastKey,
            "2" => unit = BkspUnit::LowestWhole,
            "BySyllable" | "3" => unit = BkspUnit::Syllable,
            _ => {}
        }
    }
    (unit, attach)
}

/// 한 입력 항목을 엔진이 바로 쓰도록 컴파일한 배열.
#[derive(Debug, Clone)]
pub struct Layout {
    pub name: String,
    /// ASCII at → 값-식.
    pub keys: HashMap<u32, Expr>,
    /// (갈래, a 코드포인트, b 코드포인트 또는 TOGGLE) → 결과 코드포인트.
    pub combine: HashMap<(Category, u32, u32), u32>,
    /// 가상 단위 id → 자모.
    pub virtual_units: HashMap<u32, Jamo>,
    /// 조합용/옛 자모 → 호환 자모.
    pub final_conv: HashMap<u32, u32>,
    /// 에디터 레이어의 단축글쇠(한/영 전환·한자 등). 프런트엔드가 해석한다.
    pub shortcuts: Vec<Shortcut>,
    /// 오토마타: 상태 id → 컴파일된 전이 규칙. 비어 있으면 엔진이 기본 휴리스틱을 쓴다.
    pub automata: HashMap<i64, AutomataState>,
    /// 오토마타 시작 상태(AutomataTable 의 default 속성). 보통 0.
    pub automata_start: i64,
    /// 백스페이스 동작(`<Bksp key="1">` 슬롯). 물리 Backspace 하나에 대응.
    pub bksp: BkspBehavior,
}

impl Layout {
    /// 두 낱자(또는 a+토글)의 조합 결과를 찾는다.
    pub fn combine(&self, cat: Category, a_cp: u32, b_cp: u32) -> Option<u32> {
        self.combine.get(&(cat, a_cp, b_cp)).copied()
    }

    /// 홑낱자 출력용 호환 자모: 설정의 FinalConv 우선, 없으면 기본 호환표.
    pub fn standalone(&self, j: Jamo) -> Option<char> {
        let compat = self
            .final_conv
            .get(&j.cp)
            .copied()
            .or_else(|| j.default_compat());
        compat.and_then(char::from_u32)
    }
}

impl Config {
    /// `nalgaeset.xml` 문자열을 파싱한다.
    pub fn parse(xml: &str) -> Result<Config, ConfigError> {
        parse_config(xml)
    }

    /// 지정한 입력 항목을 엔진용 `Layout` 으로 컴파일한다.
    pub fn compile(&self, entry_idx: usize) -> Result<Layout, ConfigError> {
        let entry = self
            .entries
            .get(entry_idx)
            .ok_or(ConfigError::NoEntry(entry_idx))?;

        let keys = entry
            .key_table
            .as_ref()
            .map(|kt| {
                kt.keys
                    .iter()
                    .map(|(&at, kd)| (at, kd.expr.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let mut combine = HashMap::new();
        for m in &entry.unit_mix {
            let a = resolve_jamo(&m.a, m.unit)?;
            let to = resolve_jamo(&m.to, m.unit)?;
            let b_cp = match unit::resolve_operand(&m.b, Some(m.unit))
                .ok_or_else(|| ConfigError::BadUnit(m.b.clone()))?
            {
                Unit::Toggle => unit::TOGGLE,
                Unit::Jamo(j) => j.cp,
                Unit::Virtual(_) => return Err(ConfigError::BadUnit(m.b.clone())),
            };
            combine.insert((m.unit, a.cp, b_cp), to.cp);
        }

        let mut virtual_units = HashMap::new();
        for v in &entry.virtual_units {
            let j = resolve_jamo(&v.to, v.unit)?;
            virtual_units.insert(v.from, j);
        }

        // 오토마타 상태 식들을 컴파일한다(value=전이식, default=폴백식).
        let mut automata = HashMap::new();
        for row in &entry.automata {
            let value = Expr::parse(&row.value).map_err(|source| ConfigError::AutomataExpr {
                state: row.state,
                source,
            })?;
            let default =
                Expr::parse(&row.default).map_err(|source| ConfigError::AutomataExpr {
                    state: row.state,
                    source,
                })?;
            automata.insert(row.state, AutomataState { value, default });
        }
        let automata_start = entry.automata_default.trim().parse().unwrap_or(0);

        // 백스페이스: 물리 Backspace 하나이므로 key="1" 슬롯을 쓴다(없으면 기본).
        // value1=조합 중(제1동작), value2=비조합(제2동작). attach 는 둘 중 하나라도 켜지면.
        let bksp = entry
            .bksp
            .iter()
            .find(|b| b.key == 1)
            .map(|b| {
                let (composing, a1) = parse_bksp_value(&b.value1);
                let (idle, a2) = parse_bksp_value(&b.value2);
                BkspBehavior {
                    composing,
                    idle,
                    attach: a1 || a2,
                }
            })
            .unwrap_or_default();

        Ok(Layout {
            name: entry
                .key_table
                .as_ref()
                .map(|k| k.name.clone())
                .unwrap_or_default(),
            keys,
            combine,
            virtual_units,
            final_conv: self.editor.final_conv.clone(),
            shortcuts: self.editor.shortcuts.clone(),
            automata,
            automata_start,
            bksp,
        })
    }

    /// 한글 조합 항목(자동자/생성기가 한글)으로 보이는 첫 번째 인덱스.
    pub fn first_hangul_entry(&self) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.generator_object.starts_with("CNgsIme") && e.key_table.is_some())
    }
}

fn resolve_jamo(s: &str, cat: Category) -> Result<Jamo, ConfigError> {
    match unit::resolve_operand(s, Some(cat)) {
        Some(Unit::Jamo(j)) => Ok(j),
        _ => Err(ConfigError::BadUnit(s.to_string())),
    }
}

fn category_of(s: &str) -> Result<Category, ConfigError> {
    match s {
        "CHO" => Ok(Category::Cho),
        "JUNG" => Ok(Category::Jung),
        "JONG" => Ok(Category::Jong),
        other => Err(ConfigError::BadCategory(other.to_string())),
    }
}

fn parse_int(s: &str) -> Result<u32, ConfigError> {
    unit::parse_int(s).ok_or_else(|| ConfigError::BadInt(s.to_string()))
}

/// 시작/빈 태그의 속성을 (이름→값, unescape 적용) 맵으로 모은다.
fn attrs(e: &BytesStart) -> Result<HashMap<String, String>, ConfigError> {
    let mut m = HashMap::new();
    for a in e.attributes() {
        let a = a?;
        let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let val = a.unescape_value()?.into_owned();
        m.insert(key, val);
    }
    Ok(m)
}

fn get<'a>(m: &'a HashMap<String, String>, k: &str) -> &'a str {
    m.get(k).map(String::as_str).unwrap_or("")
}

fn parse_config(xml: &str) -> Result<Config, ConfigError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut cfg = Config {
        version: String::new(),
        editor: EditorLayer::default(),
        default_entry: 0,
        current_entry: 0,
        entries: Vec::new(),
    };

    // 현재 어느 InputEntry 의 어느 섹션을 채우는 중인지.
    #[derive(PartialEq)]
    enum Section {
        None,
        Scheme,
        Generator,
    }
    let mut section = Section::None;
    let mut in_input_layer = false;

    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name();
                let tag = name.as_ref();
                let a = attrs(&e)?;
                match tag {
                    b"EditContextSetting" => cfg.version = get(&a, "version").to_string(),
                    b"EditorLayer" => {
                        cfg.editor.flags = split_flags(get(&a, "flag"));
                    }
                    b"Shortcut" => cfg.editor.shortcuts.push(Shortcut {
                        key: get(&a, "key").to_string(),
                        modifier: split_flags(get(&a, "modifier")),
                        usage: get(&a, "usage").to_string(),
                        value: get(&a, "value").to_string(),
                    }),
                    b"FinalConv" => {
                        let from = parse_int(get(&a, "from"))?;
                        let to = parse_int(get(&a, "to"))?;
                        cfg.editor.final_conv.insert(from, to);
                    }
                    b"InputLayer" => {
                        in_input_layer = true;
                        cfg.default_entry = parse_int(get(&a, "default")).unwrap_or(0) as usize;
                        cfg.current_entry = parse_int(get(&a, "current")).unwrap_or(0) as usize;
                    }
                    b"InputEntry" => {
                        cfg.entries.push(InputEntry::default());
                    }
                    b"InputSchemeSetting" => {
                        section = Section::Scheme;
                        if let Some(en) = cfg.entries.last_mut() {
                            en.scheme_object = get(&a, "object").to_string();
                        }
                    }
                    b"GeneratorSetting" => {
                        section = Section::Generator;
                        if let Some(en) = cfg.entries.last_mut() {
                            en.generator_object = get(&a, "object").to_string();
                        }
                    }
                    b"KeyTable" => {
                        if let Some(en) = cfg.entries.last_mut() {
                            en.key_table = Some(KeyTable {
                                name: get(&a, "name").to_string(),
                                from: parse_int(get(&a, "from")).unwrap_or(33),
                                to: parse_int(get(&a, "to")).unwrap_or(126),
                                keys: HashMap::new(),
                            });
                        }
                    }
                    b"Key" if section == Section::Scheme => {
                        let at_s = get(&a, "at");
                        let val = get(&a, "value");
                        let at = parse_int(at_s)?;
                        let expr = Expr::parse(val).map_err(|source| ConfigError::KeyExpr {
                            at: at_s.to_string(),
                            source,
                        })?;
                        if let Some(en) = cfg.entries.last_mut() {
                            if let Some(kt) = en.key_table.as_mut() {
                                kt.keys.insert(
                                    at,
                                    KeyDef {
                                        raw: val.to_string(),
                                        expr,
                                    },
                                );
                            }
                        }
                    }
                    b"UnitMix" => {
                        let unit = category_of(get(&a, "unit"))?;
                        if let Some(en) = cfg.entries.last_mut() {
                            en.unit_mix.push(UnitMix {
                                unit,
                                a: get(&a, "a").to_string(),
                                b: get(&a, "b").to_string(),
                                to: get(&a, "to").to_string(),
                            });
                        }
                    }
                    b"VirtualUnit" => {
                        let unit = category_of(get(&a, "unit"))?;
                        if let Some(en) = cfg.entries.last_mut() {
                            en.virtual_units.push(VirtualUnit {
                                unit,
                                from: parse_int(get(&a, "from"))?,
                                to: get(&a, "to").to_string(),
                            });
                        }
                    }
                    b"AutomataTable" => {
                        if let Some(en) = cfg.entries.last_mut() {
                            en.automata_default = get(&a, "default").to_string();
                        }
                    }
                    b"Automata" => {
                        if let Some(en) = cfg.entries.last_mut() {
                            en.automata.push(AutomataRow {
                                state: parse_int(get(&a, "state")).unwrap_or(0) as i64,
                                value: get(&a, "value").to_string(),
                                default: get(&a, "default").to_string(),
                                remark: get(&a, "remark").to_string(),
                            });
                        }
                    }
                    b"Bksp" => {
                        if let Some(en) = cfg.entries.last_mut() {
                            en.bksp.push(Bksp {
                                key: parse_int(get(&a, "key")).unwrap_or(0),
                                value1: get(&a, "value1").to_string(),
                                value2: get(&a, "value2").to_string(),
                                condition1: get(&a, "condition1").to_string(),
                                condition2: get(&a, "condition2").to_string(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = e.name();
                match name.as_ref() {
                    b"InputSchemeSetting" | b"GeneratorSetting" => section = Section::None,
                    b"InputLayer" => in_input_layer = false,
                    _ => {}
                }
                let _ = in_input_layer; // (현재는 정보용)
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(cfg)
}

fn split_flags(s: &str) -> Vec<String> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split('|').map(|p| p.trim().to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 파서 단위 테스트용 작은 합성 설정.
    const MINI: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<EditContextSetting version="0x500">
  <EditorLayer flag="DEL_MOVE|ARROW_MOVE">
    <ShortcutTable>
      <Shortcut key="VK_HANGUL" usage="IME_SWITCH" value="!A"/>
      <Shortcut key="VK_HANJA" usage="KEYCHAR" value="C0|0x82"/>
    </ShortcutTable>
    <FinalConvTable>
      <FinalConv from="0x1100" to="0x3131"/>
      <FinalConv from="0x1161" to="0x314F"/>
      <FinalConv from="0x11A8" to="0x3131"/>
    </FinalConvTable>
  </EditorLayer>
  <InputLayer default="0" current="2">
    <InputEntry>
      <InputSchemeSetting flag="0" object="CBasicInputScheme">
        <KeyTable name="mini" flag="0" from="33" to="126">
          <Key at="0x6B" value="H3|G_"/>
          <Key at="0x66" value="H3|A_"/>
          <Key at="0x78" value="H3|_G"/>
          <Key at="0x24" value="T ? H3|0x1F4 : 0x24"/>
        </KeyTable>
      </InputSchemeSetting>
      <GeneratorSetting flag="0" object="CNgsImeEx" flagex="1">
        <UnitMixTable>
          <UnitMix unit="CHO" a="G_" b="G_" to="GG"/>
          <UnitMix unit="CHO" a="G_" b="500" to="GG"/>
          <UnitMix unit="JUNG" a="O_" b="A_" to="WA"/>
          <UnitMix unit="JONG" a="R_" b="S_" to="RS"/>
        </UnitMixTable>
        <VirtualUnitTable>
          <VirtualUnit unit="JUNG" from="128" to="O_"/>
          <VirtualUnit unit="JUNG" from="130" to="EU"/>
        </VirtualUnitTable>
        <AutomataTable default="0">
          <Automata state="0" value="1" default="0" remark="초기 상태"/>
          <Automata state="2" value="A&amp;&amp;A!=500 ? 0 : B||C||A==500 ? 2 : -2" default="0" remark="완성"/>
        </AutomataTable>
      </GeneratorSetting>
    </InputEntry>
    <InputEntry>
      <InputSchemeSetting object="CInputScheme"/>
      <GeneratorSetting flag="0" object="CIme"/>
    </InputEntry>
  </InputLayer>
</EditContextSetting>"#;

    #[test]
    fn parse_mini() {
        let cfg = Config::parse(MINI).unwrap();
        assert_eq!(cfg.version, "0x500");
        assert_eq!(cfg.default_entry, 0);
        assert_eq!(cfg.current_entry, 2);
        assert_eq!(cfg.entries.len(), 2);
        assert_eq!(cfg.editor.shortcuts.len(), 2);
        assert_eq!(cfg.editor.final_conv.get(&0x1100), Some(&0x3131));

        let e0 = &cfg.entries[0];
        assert_eq!(e0.scheme_object, "CBasicInputScheme");
        assert_eq!(e0.generator_object, "CNgsImeEx");
        let kt = e0.key_table.as_ref().unwrap();
        assert_eq!(kt.name, "mini");
        assert_eq!(kt.keys.len(), 4);
        assert_eq!(e0.unit_mix.len(), 4);
        assert_eq!(e0.virtual_units.len(), 2);
        assert_eq!(e0.automata.len(), 2);
        assert_eq!(e0.automata_default, "0");

        // 두 번째 항목은 패스스루
        assert_eq!(cfg.entries[1].scheme_object, "CInputScheme");
        assert!(cfg.entries[1].key_table.is_none());

        assert_eq!(cfg.first_hangul_entry(), Some(0));
    }

    #[test]
    fn compile_mini() {
        let cfg = Config::parse(MINI).unwrap();
        let layout = cfg.compile(0).unwrap();
        assert_eq!(layout.name, "mini");
        assert_eq!(layout.keys.len(), 4);

        // 갈마들이/된소리 조합: ㄱ초성 + ㄱ초성 → ㄲ초성, ㄱ + 토글 → ㄲ
        assert_eq!(layout.combine(Category::Cho, 0x1100, 0x1100), Some(0x1101));
        assert_eq!(
            layout.combine(Category::Cho, 0x1100, unit::TOGGLE),
            Some(0x1101)
        );
        // 겹모음 ㅗ+ㅏ→ㅘ
        assert_eq!(layout.combine(Category::Jung, 0x1169, 0x1161), Some(0x116A));
        // 겹받침 ㄹ+ㅅ→ㄽ (종성)
        assert_eq!(layout.combine(Category::Jong, 0x11AF, 0x11BA), Some(0x11B3));

        // 가상 단위
        assert_eq!(
            layout.virtual_units.get(&128),
            Some(&Jamo::new(Category::Jung, 0x1169))
        );
        assert_eq!(
            layout.virtual_units.get(&130),
            Some(&Jamo::new(Category::Jung, 0x1173))
        );

        // 홑낱자 출력
        assert_eq!(
            layout.standalone(Jamo::new(Category::Cho, 0x1100)),
            Some('ㄱ')
        );
        assert_eq!(
            layout.standalone(Jamo::new(Category::Jong, 0x11A8)),
            Some('ㄱ')
        );

        // 오토마타: AutomataTable 의 두 상태(0,2)가 컴파일돼 들어왔는지.
        assert_eq!(layout.automata.len(), 2);
        assert_eq!(layout.automata_start, 0);
        // 상태 2 식(A&&A!=500 ? 0 : ...): 입력이 초성 ㄱ(A=서열, !=500)이면 0(새 글자).
        use crate::expr::{Ctx, Value};
        let st2 = layout.automata.get(&2).expect("state 2");
        assert_eq!(
            st2.value.eval(&Ctx {
                a: 1,
                ..Default::default()
            }),
            Ok(Value::Int(0))
        );
    }

    #[test]
    fn compile_bksp_behavior() {
        // Bksp key=1 value1=ByUnitStep|BkspAttach value2=BySyllable 컴파일 확인.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<EditContextSetting version="0x500">
  <EditorLayer flag="0"><FinalConvTable/></EditorLayer>
  <InputLayer default="0" current="0">
    <InputEntry>
      <InputSchemeSetting object="CBasicInputScheme">
        <KeyTable name="t" flag="0" from="33" to="126"><Key at="0x6B" value="H3|G_"/></KeyTable>
      </InputSchemeSetting>
      <GeneratorSetting object="CNgsImeEx">
        <UnitMixTable/><VirtualUnitTable/><AutomataTable default="0"/>
        <Extra>
          <Bksp key="1" value1="ByUnitStep|BkspAttach" value2="BySyllable" condition1="0" condition2="0"/>
          <Bksp key="2" value1="0" value2="BySyllable" condition1="0" condition2="0"/>
        </Extra>
      </GeneratorSetting>
    </InputEntry>
  </InputLayer>
</EditContextSetting>"#;
        let layout = Config::parse(xml).unwrap().compile(0).unwrap();
        assert_eq!(layout.bksp.composing, BkspUnit::LastKey); // ByUnitStep
        assert_eq!(layout.bksp.idle, BkspUnit::Syllable); // BySyllable
        assert!(layout.bksp.attach); // BkspAttach
    }

    #[test]
    fn compile_bksp_default_when_absent() {
        // Extra/Bksp 없으면 기본(LastKey, attach 없음).
        let layout = Config::parse(MINI).unwrap().compile(0).unwrap();
        assert_eq!(layout.bksp.composing, BkspUnit::LastKey);
        assert!(!layout.bksp.attach);
    }
}
