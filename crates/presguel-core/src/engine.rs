//! 한글 조합 엔진: 컴파일된 `Layout` 위에서 키 입력을 받아 음절을 조합한다.
//!
//! 세벌식(3벌식) 모델을 따른다: 초성/중성/종성이 서로 다른 글쇠라 역할이 분명하므로
//! 이어치기가 자연스럽다. 완성된 음절에 새 **초성**이 오면 그 음절을 확정(commit)하고
//! 새 음절을 시작한다. 중성/종성/갈마들이 토글은 현재 음절에 붙는다. 겹낱자(겹받침,
//! 겹모음, 된소리)는 설정의 `UnitMixTable` 로 결합한다. 출력은 현대 음절이면 완성형
//! (U+AC00), 아니면 첫가끝(조합용 자모) 시퀀스, 홑낱자면 `FinalConvTable`(호환 자모).
//!
//! 참고: `research/02-config-decode.md` §C, `research/04-hangul-unicode.md`.

use crate::config::{BkspUnit, Layout};
use crate::expr::{Ctx, Value};
use crate::jamo;
use crate::ngs_seq::ngs_seq;
use crate::unit::{self, Category, Jamo, Unit};

/// 조합 중인 한 음절. 각 칸은 조합용 자모 코드포인트(겹낱자는 결합된 단일 코드포인트).
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct Syllable {
    cho: Option<u32>,
    jung: Option<u32>,
    jong: Option<u32>,
}

impl Syllable {
    fn is_empty(&self) -> bool {
        self.cho.is_none() && self.jung.is_none() && self.jong.is_none()
    }
}

/// 키 한 번 처리 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyOutcome {
    /// 응용프로그램에 확정 입력할 문자열(없으면 빈 문자열).
    pub commit: String,
    /// 현재 조합 중 표시(preedit). 없으면 빈 문자열.
    pub preedit: String,
    /// 엔진이 이 키를 소비했는지. false 면 프런트엔드가 원래 키를 응용에 넘긴다.
    pub consumed: bool,
}

/// 한글 조합 엔진.
#[derive(Debug, Clone)]
pub struct Engine {
    layout: Layout,
    cur: Syllable,
    /// 마지막 확정 이후 현재 음절에 투입된 단위들(낱자 단위 백스페이스용 재생 이력).
    history: Vec<Unit>,
    /// 오토마타 현재 상태 id. layout.automata 가 비어 있으면 미사용(기본 휴리스틱).
    auto_state: i64,
}

impl Engine {
    pub fn new(layout: Layout) -> Self {
        let auto_state = layout.automata_start;
        Self {
            layout,
            cur: Syllable::default(),
            history: Vec::new(),
            auto_state,
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// 조합 중인 내용이 없는가.
    pub fn is_empty(&self) -> bool {
        self.cur.is_empty()
    }

    /// 오토마타 상태 id (값-식의 `T`): 0=비어있음, 1=미완성(초성/홑낱자), 2=중성 있음.
    fn t_state(&self) -> i64 {
        if self.cur.is_empty() {
            0
        } else if self.cur.jung.is_some() {
            2
        } else {
            1
        }
    }

    /// 현재 음절을 문자열로 렌더링(완성형/첫가끝/호환 자모).
    fn render(&self, syl: &Syllable) -> String {
        if syl.is_empty() {
            return String::new();
        }
        // 초성+중성이 모두 있으면 음절 블록.
        if let (Some(cho), Some(jung)) = (syl.cho, syl.jung) {
            if let Some(ch) = jamo::compose(cho, jung, syl.jong) {
                return ch.to_string(); // 현대 완성형
            }
            // 옛한글: 첫가끝 조합용 자모 시퀀스
            let mut s = String::new();
            for cp in [Some(cho), Some(jung), syl.jong].into_iter().flatten() {
                if let Some(c) = char::from_u32(cp) {
                    s.push(c);
                }
            }
            return s;
        }
        // 그 외(홑낱자, 또는 중성 없는 부분 조합): 칸별로 호환 자모.
        let mut s = String::new();
        for (cat, cp) in [
            (Category::Cho, syl.cho),
            (Category::Jung, syl.jung),
            (Category::Jong, syl.jong),
        ] {
            if let Some(cp) = cp {
                if let Some(ch) = self.layout.standalone(Jamo::new(cat, cp)) {
                    s.push(ch);
                } else if let Some(ch) = char::from_u32(cp) {
                    s.push(ch);
                }
            }
        }
        s
    }

    /// 현재 조합 중 표시 문자열.
    pub fn preedit(&self) -> String {
        self.render(&self.cur)
    }

    /// 현재 음절을 확정 문자열로 만들고 버퍼를 비운다(이력은 건드리지 않음).
    fn commit_current(&mut self) -> String {
        let s = self.render(&self.cur);
        self.cur = Syllable::default();
        s
    }

    /// 현재 음절을 확정하고 버퍼·이력을 모두 비운다.
    fn commit_and_clear(&mut self) -> String {
        let s = self.commit_current();
        self.history.clear();
        s
    }

    /// 포커스 아웃/리셋 시: 현재 음절을 확정해 돌려주고 버퍼를 비운다.
    pub fn flush(&mut self) -> String {
        let s = self.commit_current();
        self.history.clear();
        self.auto_state = self.layout.automata_start;
        s
    }

    /// 조합 버퍼를 확정 없이 비운다.
    pub fn reset(&mut self) {
        self.cur = Syllable::default();
        self.history.clear();
        self.auto_state = self.layout.automata_start;
    }

    // ── 낱자 투입 ────────────────────────────────────────────────────────────

    fn feed_cho(&mut self, cp: u32) -> String {
        if self.cur.is_empty() {
            self.cur.cho = Some(cp);
            return String::new();
        }
        // 홑초성만 있는 상태: 된소리 결합 시도
        if self.cur.cho.is_some() && self.cur.jung.is_none() && self.cur.jong.is_none() {
            if let Some(r) = self
                .layout
                .combine(Category::Cho, self.cur.cho.unwrap(), cp)
            {
                self.cur.cho = Some(r);
                return String::new();
            }
        }
        // 그 외: 새 음절 시작
        let out = self.commit_current();
        self.cur.cho = Some(cp);
        out
    }

    fn feed_jung(&mut self, cp: u32) -> String {
        // 중성 칸이 비어 있으면(받침도 없으면) 그대로 채움(초성 유무 무관: 홀소리 음절 가능)
        if self.cur.jung.is_none() && self.cur.jong.is_none() {
            self.cur.jung = Some(cp);
            return String::new();
        }
        // 중성이 있고 받침이 없으면 겹모음 결합 시도
        if self.cur.jung.is_some() && self.cur.jong.is_none() {
            if let Some(r) = self
                .layout
                .combine(Category::Jung, self.cur.jung.unwrap(), cp)
            {
                self.cur.jung = Some(r);
                return String::new();
            }
        }
        // 그 외(CVC 뒤 모음 등): 새 음절(홀소리)로 (3벌식 → 도깨비불 없음)
        let out = self.commit_current();
        self.cur.jung = Some(cp);
        out
    }

    fn feed_jong(&mut self, cp: u32) -> String {
        // 초성+중성이 있고 받침이 비면 받침으로 붙임
        if self.cur.cho.is_some() && self.cur.jung.is_some() && self.cur.jong.is_none() {
            self.cur.jong = Some(cp);
            return String::new();
        }
        // 받침이 이미 있으면 겹받침 결합 시도
        if self.cur.jong.is_some() {
            if let Some(r) = self
                .layout
                .combine(Category::Jong, self.cur.jong.unwrap(), cp)
            {
                self.cur.jong = Some(r);
                return String::new();
            }
        }
        // 붙일 곳이 없으면 현재 음절 확정 후 홑받침(홑낱자)로 시작
        let out = self.commit_current();
        self.cur = Syllable {
            jong: Some(cp),
            ..Syllable::default()
        };
        out
    }

    fn feed_toggle(&mut self) -> String {
        // 갈마들이 토글: 현재 초성의 된소리/예사소리 전환(설정 UnitMix 에 (cho,500)→ 규칙)
        if let Some(cho) = self.cur.cho {
            if let Some(r) = self.layout.combine(Category::Cho, cho, unit::TOGGLE) {
                self.cur.cho = Some(r);
            }
        }
        String::new()
    }

    fn feed_jamo(&mut self, j: Jamo) -> String {
        match j.category {
            Category::Cho => self.feed_cho(j.cp),
            Category::Jung => self.feed_jung(j.cp),
            Category::Jong => self.feed_jong(j.cp),
        }
    }

    fn feed_unit(&mut self, u: Unit) -> String {
        // 오토마타가 정의돼 있으면 낱자(가상단위 포함)는 오토마타 경로로 처리한다.
        // 토글은 양쪽 모두 feed_toggle 로(현재 초성 된소리 전환), 이력만 갱신.
        if !self.layout.automata.is_empty() {
            let jamo = match u {
                Unit::Jamo(j) => Some(j),
                Unit::Virtual(id) => self.layout.virtual_units.get(&id).copied(),
                Unit::Toggle => None,
            };
            if let Some(j) = jamo {
                // 서열을 모르는 낱자(표 밖 옛한글 등)는 안전하게 휴리스틱으로.
                if ngs_seq(j.category, j.cp).is_some() {
                    return self.automaton_feed(j);
                }
            }
        }
        // 레거시(휴리스틱) 경로.
        let out = match u {
            Unit::Jamo(j) => self.feed_jamo(j),
            Unit::Toggle => self.feed_toggle(),
            Unit::Virtual(id) => match self.layout.virtual_units.get(&id).copied() {
                Some(j) => self.feed_jamo(j),
                None => String::new(),
            },
        };
        // 이력 갱신: 확정이 없었으면 현재 음절에 덧붙은 것 → push.
        // 확정이 있었으면 새 음절이 이 단위로 시작된 것 → 이력을 이 단위만으로 리셋.
        if out.is_empty() {
            self.history.push(u);
        } else if self.cur.is_empty() {
            self.history.clear();
        } else {
            self.history = vec![u];
        }
        out
    }

    // ── 오토마타 실행 (날개셋 AutomataTable) ─────────────────────────────────────

    /// 조합 중 음절의 한 칸(초/중/종) 서열번호. 비었으면 0.
    fn slot_seq(&self, cat: Category) -> i64 {
        let cp = match cat {
            Category::Cho => self.cur.cho,
            Category::Jung => self.cur.jung,
            Category::Jong => self.cur.jong,
        };
        cp.and_then(|c| ngs_seq(cat, c))
            .map(|s| s as i64)
            .unwrap_or(0)
    }

    /// 낱자를 현재 음절의 해당 칸에 넣는다(확정 없이). 칸이 차 있으면 UnitMix 결합을
    /// 시도하고, 결합 규칙이 없으면 교체한다(= 무한 낱자 수정). 빈 칸이면 그대로 채운다.
    /// 교체가 일어났으면 `true`(이력 정리용).
    fn put_modify(&mut self, j: Jamo) -> bool {
        let existing = match j.category {
            Category::Cho => self.cur.cho,
            Category::Jung => self.cur.jung,
            Category::Jong => self.cur.jong,
        };
        // 빈 칸=채움, 결합 규칙 있으면 결합, 없으면 교체(무한 낱자 수정).
        let (newcp, replaced) = match existing {
            None => (j.cp, false),
            Some(e) => match self.layout.combine(j.category, e, j.cp) {
                Some(r) => (r, false),
                None => (j.cp, true),
            },
        };
        match j.category {
            Category::Cho => self.cur.cho = Some(newcp),
            Category::Jung => self.cur.jung = Some(newcp),
            Category::Jong => self.cur.jong = Some(newcp),
        }
        replaced
    }

    /// 단위의 낱자 갈래(백스페이스 이력 정리용). 가상단위는 풀어서, 토글은 초성으로 본다.
    fn unit_cat(layout: &Layout, u: &Unit) -> Option<Category> {
        match u {
            Unit::Jamo(j) => Some(j.category),
            Unit::Virtual(id) => layout.virtual_units.get(id).map(|j| j.category),
            Unit::Toggle => Some(Category::Cho),
        }
    }

    /// 낱자를 이력에 기록한다. 무한 낱자 수정으로 같은 칸을 *교체*한 경우엔, 그 칸의
    /// 직전 단위를 이력에서 빼고 새 단위를 넣어, 낱자 단위 백스페이스가 정확히 현재
    /// 낱자만 되돌리도록 한다. (결합이면 둘 다 남겨 한 단계씩 분해.)
    fn record_unit(&mut self, u: Unit, replaced: bool, cat: Category) {
        if replaced {
            let pos = {
                let layout = &self.layout;
                self.history
                    .iter()
                    .rposition(|h| Self::unit_cat(layout, h) == Some(cat))
            };
            if let Some(p) = pos {
                self.history.remove(p);
            }
        }
        self.history.push(u);
    }

    /// 빈 음절에 낱자 하나를 넣었을 때의 오토마타 상태(시작 상태에서 평가).
    fn fresh_state(&self, j: Jamo) -> i64 {
        let seq = ngs_seq(j.category, j.cp).map(|s| s as i64).unwrap_or(0);
        let (a, b, c) = match j.category {
            Category::Cho => (seq, 0, 0),
            Category::Jung => (0, seq, 0),
            Category::Jong => (0, 0, seq),
        };
        let ctx = Ctx {
            a,
            b,
            c,
            ..Default::default()
        };
        match self.layout.automata.get(&self.layout.automata_start) {
            Some(st) => match st.value.eval(&ctx) {
                Ok(Value::Int(n)) if n > 0 => n,
                _ => self.layout.automata_start,
            },
            None => self.layout.automata_start,
        }
    }

    /// 한 낱자를 오토마타로 처리한다. 확정 문자열을 돌려준다.
    fn automaton_feed(&mut self, j: Jamo) -> String {
        let seq = ngs_seq(j.category, j.cp).map(|s| s as i64).unwrap_or(0);
        let (a, b, c) = match j.category {
            Category::Cho => (seq, 0, 0),
            Category::Jung => (0, seq, 0),
            Category::Jong => (0, 0, seq),
        };
        let ctx = Ctx {
            a,
            b,
            c,
            d: self.slot_seq(Category::Cho),
            e: self.slot_seq(Category::Jung),
            f: self.slot_seq(Category::Jong),
            ..Default::default() // o=0(세벌식), t=0(일반 상황)
        };
        // 현재 상태의 전이식 평가(실패 시 default 식, 그래도 없으면 휴리스틱).
        let r = match self.layout.automata.get(&self.auto_state) {
            Some(st) => match st.value.eval(&ctx) {
                Ok(Value::Int(n)) => n,
                _ => match st.default.eval(&ctx) {
                    Ok(Value::Int(n)) => n,
                    _ => return self.feed_jamo_tracked(j),
                },
            },
            None => return self.feed_jamo_tracked(j),
        };
        self.apply_result(r, j)
    }

    /// 오토마타 결과 코드 r 에 따라 낱자를 배치한다(research/ngs-automata-help.txt).
    /// 양수=그 상태로 조합 계속, 0=다음 글자 시작, -1=무시, -2=무한 낱자 수정,
    /// 그 외 음수=보수적으로 현재 확정 후 새 음절(점진적으로 정교화 예정).
    fn apply_result(&mut self, r: i64, j: Jamo) -> String {
        match r {
            // 조합 계속: 해당 칸에 배치(차 있으면 결합/교체) 후 상태 갱신.
            n if n > 0 => {
                let replaced = self.put_modify(j);
                self.auto_state = n;
                self.record_unit(Unit::Jamo(j), replaced, j.category);
                String::new()
            }
            // 무한 낱자 수정: 현재 음절을 확정하지 않고 칸을 결합/교체. 상태 유지.
            -2 => {
                let replaced = self.put_modify(j);
                self.record_unit(Unit::Jamo(j), replaced, j.category);
                String::new()
            }
            // 입력 무시(소비만).
            -1 => String::new(),
            // 0 및 그 외 음수: 현재 음절 확정 후 이 낱자로 새 음절 시작.
            _ => {
                let commit = self.commit_current();
                self.history.clear();
                self.put_modify(j);
                self.auto_state = self.fresh_state(j);
                self.history.push(Unit::Jamo(j));
                commit
            }
        }
    }

    /// 휴리스틱 feed_jamo + 이력 갱신(오토마타 경로의 폴백용).
    fn feed_jamo_tracked(&mut self, j: Jamo) -> String {
        let out = self.feed_jamo(j);
        if out.is_empty() {
            self.history.push(Unit::Jamo(j));
        } else if self.cur.is_empty() {
            self.history.clear();
        } else {
            self.history = vec![Unit::Jamo(j)];
        }
        out
    }

    // ── 키 처리 ──────────────────────────────────────────────────────────────

    /// KeyTable 의 ASCII 글쇠(0x21..0x7E)를 처리한다. `caps` 는 Caps Lock 점등 상태로,
    /// 값-식의 `P` (bit0)에 들어간다(세벌식 항목은 P 미사용).
    pub fn press(&mut self, ascii: u8, caps: bool) -> KeyOutcome {
        let expr = match self.layout.keys.get(&(ascii as u32)) {
            Some(e) => e.clone(),
            None => {
                // 배열에 없는 글쇠 → 현재 음절 확정 후 원래 키를 응용으로 넘김
                let commit = self.commit_and_clear();
                return KeyOutcome {
                    commit,
                    preedit: String::new(),
                    consumed: false,
                };
            }
        };
        let ctx = Ctx {
            t: self.t_state(),
            p: caps as i64,
            ..Default::default()
        };
        let val = match expr.eval(&ctx) {
            Ok(v) => v,
            Err(_) => {
                let commit = self.commit_and_clear();
                return KeyOutcome {
                    commit,
                    preedit: String::new(),
                    consumed: false,
                };
            }
        };
        self.dispatch(val)
    }

    fn dispatch(&mut self, val: Value) -> KeyOutcome {
        match val {
            Value::Unit(u) => {
                let commit = self.feed_unit(u);
                KeyOutcome {
                    commit,
                    preedit: self.preedit(),
                    consumed: true,
                }
            }
            Value::Int(n) => {
                // 문자(기호/숫자) 리터럴: 현재 음절 확정 후 그 문자를 확정.
                let mut commit = self.commit_and_clear();
                if let Some(ch) = u32::try_from(n).ok().and_then(char::from_u32) {
                    commit.push(ch);
                }
                KeyOutcome {
                    commit,
                    preedit: String::new(),
                    consumed: true,
                }
            }
            Value::Command(_cmd) => {
                // 제어 명령(C0|): 현재는 현재 음절만 확정(한자 변환 등은 추후).
                let commit = self.commit_and_clear();
                KeyOutcome {
                    commit,
                    preedit: self.preedit(),
                    consumed: true,
                }
            }
        }
    }

    /// 백스페이스: 낱자 단위로 되돌린다. 현재 음절을 만든 단위 이력에서 마지막 하나를
    /// 빼고 나머지를 다시 재생(replay)하므로, 겹낱자/겹모음/갈마들이 토글이 정확히 한
    /// 단계씩 풀린다(날개셋 ByUnitStep 에 해당).
    pub fn backspace(&mut self) -> KeyOutcome {
        if self.cur.is_empty() {
            // 조합 중이 아니면 응용이 직접 지우도록 넘김
            return KeyOutcome {
                commit: String::new(),
                preedit: String::new(),
                consumed: false,
            };
        }
        // 조합 중이므로 제1동작(composing) 삭제 단위를 따른다.
        self.bksp_remove(self.layout.bksp.composing);
        // 남은 이력을 처음부터 재생해 현재 음절을 재구성(오토마타 상태도 초기화).
        let hist = std::mem::take(&mut self.history);
        self.cur = Syllable::default();
        self.auto_state = self.layout.automata_start;
        for u in hist {
            // 한 음절 안의 단위들이므로 재생 중 확정은 발생하지 않는다.
            let _ = self.feed_unit(u);
        }
        KeyOutcome {
            commit: String::new(),
            preedit: self.preedit(),
            consumed: true,
        }
    }

    /// 백스페이스 삭제 단위에 따라 이력에서 제거할 단위를 정한다(현재 음절 기준).
    /// 제거 후 호출부가 남은 이력을 재생해 음절을 재구성한다.
    fn bksp_remove(&mut self, mode: BkspUnit) {
        match mode {
            // 글자 전체: 이력 비움 → 재생 시 빈 음절.
            BkspUnit::Syllable => self.history.clear(),
            // 직전 한 타: 마지막 단위 하나 제거.
            BkspUnit::LastKey => {
                self.history.pop();
            }
            // 최하위 낱자 관련: 종성→중성→초성 순으로 채워진 첫 칸을 대상으로.
            BkspUnit::LowestLastKey | BkspUnit::LowestWhole => {
                let cat = if self.cur.jong.is_some() {
                    Category::Jong
                } else if self.cur.jung.is_some() {
                    Category::Jung
                } else if self.cur.cho.is_some() {
                    Category::Cho
                } else {
                    self.history.pop();
                    return;
                };
                let layout = &self.layout;
                if mode == BkspUnit::LowestWhole {
                    // 그 낱자 전체: 해당 갈래 단위 모두 제거.
                    self.history
                        .retain(|u| Self::unit_cat(layout, u) != Some(cat));
                } else {
                    // 최하위 낱자의 직전 한 타: 그 갈래의 마지막 단위 하나만 제거.
                    if let Some(p) = self
                        .history
                        .iter()
                        .rposition(|u| Self::unit_cat(layout, u) == Some(cat))
                    {
                        self.history.remove(p);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    // 합성 설정으로 엔진 동작의 핵심 경로를 검증(외부 파일 불요).
    const MINI: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<EditContextSetting version="0x500">
  <EditorLayer flag="0">
    <FinalConvTable>
      <FinalConv from="0x1100" to="0x3131"/>
      <FinalConv from="0x1102" to="0x3134"/>
      <FinalConv from="0x1161" to="0x314F"/>
      <FinalConv from="0x11A8" to="0x3131"/>
    </FinalConvTable>
  </EditorLayer>
  <InputLayer default="0" current="0">
    <InputEntry>
      <InputSchemeSetting object="CBasicInputScheme">
        <KeyTable name="mini" flag="0" from="33" to="126">
          <Key at="0x6B" value="H3|G_"/>   <!-- k = 초성 ㄱ -->
          <Key at="0x68" value="H3|N_"/>   <!-- h = 초성 ㄴ -->
          <Key at="0x66" value="H3|A_"/>   <!-- f = 중성 ㅏ -->
          <Key at="0x2F" value="H3|O_"/>   <!-- / = 중성 ㅗ -->
          <Key at="0x78" value="H3|_G"/>   <!-- x = 종성 ㄱ -->
          <Key at="0x73" value="H3|_N"/>   <!-- s = 종성 ㄴ -->
          <Key at="0x24" value="T ? H3|0x1F4 : 0x24"/> <!-- $ = 갈마 토글 -->
          <Key at="0x21" value="0x21"/>    <!-- ! = 리터럴 '!' -->
        </KeyTable>
      </InputSchemeSetting>
      <GeneratorSetting object="CNgsImeEx">
        <UnitMixTable>
          <UnitMix unit="CHO" a="G_" b="500" to="GG"/>
          <UnitMix unit="CHO" a="GG" b="500" to="G_"/>
          <UnitMix unit="JUNG" a="O_" b="A_" to="WA"/>
        </UnitMixTable>
        <VirtualUnitTable/>
        <AutomataTable default="0"/>
      </GeneratorSetting>
    </InputEntry>
  </InputLayer>
</EditContextSetting>"#;

    fn engine() -> Engine {
        let cfg = Config::parse(MINI).unwrap();
        Engine::new(cfg.compile(0).unwrap())
    }

    /// 키 시퀀스를 눌러 (확정 누적, 마지막 preedit) 반환.
    fn typ(e: &mut Engine, keys: &str) -> (String, String) {
        let mut committed = String::new();
        let mut preedit = String::new();
        for ch in keys.chars() {
            let out = e.press(ch as u8, false);
            committed.push_str(&out.commit);
            preedit = out.preedit;
        }
        (committed, preedit)
    }

    #[test]
    fn simple_syllable() {
        let mut e = engine();
        let (c, p) = typ(&mut e, "kf"); // ㄱ + ㅏ
        assert_eq!(c, "");
        assert_eq!(p, "가");
        assert_eq!(e.flush(), "가");
    }

    #[test]
    fn syllable_with_jong() {
        let mut e = engine();
        let (_c, p) = typ(&mut e, "kfx"); // 가 + 받침 ㄱ
        assert_eq!(p, "각");
    }

    #[test]
    fn new_cho_commits_previous() {
        let mut e = engine();
        // kf (가) hf (나): 두 번째 초성 ㄴ 이 '가'를 확정
        let (c, p) = typ(&mut e, "kfhf");
        assert_eq!(c, "가");
        assert_eq!(p, "나");
    }

    #[test]
    fn compound_vowel() {
        let mut e = engine();
        let (_c, p) = typ(&mut e, "k/f"); // ㄱ + ㅗ + ㅏ → 과
        assert_eq!(p, "과");
    }

    #[test]
    fn galma_toggle_tense() {
        let mut e = engine();
        let (_c, p) = typ(&mut e, "k$f"); // ㄱ + 토글(→ㄲ) + ㅏ → 까
        assert_eq!(p, "까");
        // 토글 두 번 → 다시 예사소리
        let mut e2 = engine();
        let (_c2, p2) = typ(&mut e2, "k$$f"); // ㄱ→ㄲ→ㄱ + ㅏ → 가
        assert_eq!(p2, "가");
    }

    #[test]
    fn lone_jamo_finalconv() {
        let mut e = engine();
        let (_c, p) = typ(&mut e, "k"); // 홑초성 ㄱ → 호환 자모
        assert_eq!(p, "ㄱ");
        assert_eq!(e.flush(), "ㄱ");
    }

    #[test]
    fn literal_commits_and_breaks() {
        let mut e = engine();
        e.press(b'k', false); // ㄱ
        let out = e.press(b'!', false); // 리터럴 '!' → ㄱ 확정 + '!'
        assert_eq!(out.commit, "ㄱ!");
        assert_eq!(out.preedit, "");
        assert!(out.consumed);
    }

    #[test]
    fn backspace_unit_step() {
        let mut e = engine();
        typ(&mut e, "kfx"); // 각
        let o1 = e.backspace(); // 받침 ㄱ 제거 → 가
        assert_eq!(o1.preedit, "가");
        let o2 = e.backspace(); // 중성 ㅏ 제거 → ㄱ
        assert_eq!(o2.preedit, "ㄱ");
        let o3 = e.backspace(); // 초성 제거 → 빈
        assert_eq!(o3.preedit, "");
        let o4 = e.backspace(); // 더 없음 → 비소비
        assert!(!o4.consumed);
    }

    #[test]
    fn backspace_decomposes_compound() {
        let mut e = engine();
        typ(&mut e, "k$"); // ㄲ (토글로 된소리)
        assert_eq!(e.preedit(), "ㄲ");
        let o = e.backspace(); // 겹낱자 한 단계 → ㄱ
        assert_eq!(o.preedit, "ㄱ");
    }

    // 옛한글: 현대 집합 밖 자모는 완성형이 없으므로 첫가끝(조합용 자모) 시퀀스로,
    // 홑낱자면 FinalConv 호환 자모로 출력된다.
    const OLD: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<EditContextSetting version="0x500">
  <EditorLayer flag="0">
    <FinalConvTable>
      <FinalConv from="0x114C" to="0x3181"/>
      <FinalConv from="0x1161" to="0x314F"/>
    </FinalConvTable>
  </EditorLayer>
  <InputLayer default="0" current="0">
    <InputEntry>
      <InputSchemeSetting object="CBasicInputScheme">
        <KeyTable name="old" flag="0" from="33" to="126">
          <Key at="0x61" value="H3|0x114C"/> <!-- a = 옛이응 초성 (현대 밖) -->
          <Key at="0x62" value="H3|A_"/>      <!-- b = 중성 ㅏ -->
        </KeyTable>
      </InputSchemeSetting>
      <GeneratorSetting object="CNgsImeEx">
        <UnitMixTable/><VirtualUnitTable/><AutomataTable default="0"/>
      </GeneratorSetting>
    </InputEntry>
  </InputLayer>
</EditContextSetting>"#;

    fn old_engine() -> Engine {
        let cfg = Config::parse(OLD).unwrap();
        Engine::new(cfg.compile(0).unwrap())
    }

    #[test]
    fn old_hangul_lone_jamo_via_finalconv() {
        let mut e = old_engine();
        let (_c, p) = typ(&mut e, "a"); // 홑 옛이응 초성 → 호환 자모 ㆁ(U+3181)
        assert_eq!(p, "\u{3181}");
    }

    #[test]
    fn old_hangul_syllable_emits_conjoining() {
        let mut e = old_engine();
        let (_c, p) = typ(&mut e, "ab"); // 옛이응 초성 + ㅏ → 완성형 없음 → 첫가끝 시퀀스
        assert_eq!(p, "\u{114C}\u{1161}");
        assert_eq!(p.chars().count(), 2);
    }

    #[test]
    fn space_not_in_table_passes_through() {
        let mut e = engine();
        typ(&mut e, "kf"); // 가
        let out = e.press(b' ', false); // space (table 밖) → 가 확정 + 비소비
        assert_eq!(out.commit, "가");
        assert!(!out.consumed);
    }

    // 실제 AutomataTable(상태 0/1/2)을 가진 세벌식 설정. 무한 낱자 수정 검증용.
    // state1 에 ㅋㅋ/ㅎㅎ(서열 176/185) 연타 → 다음 글자 규칙도 포함(사용자 커스텀).
    const AUTO: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<EditContextSetting version="0x500">
  <EditorLayer flag="0"><FinalConvTable/></EditorLayer>
  <InputLayer default="0" current="0">
    <InputEntry>
      <InputSchemeSetting object="CBasicInputScheme">
        <KeyTable name="auto" flag="0" from="33" to="126">
          <Key at="0x67" value="H3|G_"/>  <!-- g 초 ㄱ -->
          <Key at="0x6E" value="H3|N_"/>  <!-- n 초 ㄴ -->
          <Key at="0x63" value="H3|S_"/>  <!-- c 초 ㅅ -->
          <Key at="0x6B" value="H3|K_"/>  <!-- k 초 ㅋ -->
          <Key at="0x68" value="H3|H_"/>  <!-- h 초 ㅎ -->
          <Key at="0x61" value="H3|A_"/>  <!-- a 중 ㅏ -->
          <Key at="0x65" value="H3|EO"/>  <!-- e 중 ㅓ -->
          <Key at="0x6F" value="H3|O_"/>  <!-- o 중 ㅗ -->
          <Key at="0x6D" value="H3|_N"/>  <!-- m 종 ㄴ -->
          <Key at="0x69" value="H3|AE"/> <!-- i 중 ㅐ -->
        </KeyTable>
      </InputSchemeSetting>
      <GeneratorSetting object="CNgsImeEx">
        <UnitMixTable>
          <UnitMix unit="JUNG" a="O_" b="A_" to="WA"/>
        </UnitMixTable>
        <VirtualUnitTable/>
        <AutomataTable default="0">
          <Automata state="0" value="1" default="0" remark="초기"/>
          <Automata state="1" value="D==176&amp;&amp;A==176 || D==185&amp;&amp;A==185 ? 0 : A || B || C ? (A || D)&amp;&amp;(B || E) ? 2 : 1 : -2" default="-1" remark="미완성"/>
          <Automata state="2" value="A&amp;&amp;A!=500 ? 0 : B||C||A==500 ? 2 : -2" default="0" remark="완성"/>
        </AutomataTable>
      </GeneratorSetting>
    </InputEntry>
  </InputLayer>
</EditContextSetting>"#;

    fn auto_engine() -> Engine {
        let cfg = Config::parse(AUTO).unwrap();
        Engine::new(cfg.compile(0).unwrap())
    }

    #[test]
    fn automaton_loads() {
        let e = auto_engine();
        assert_eq!(e.layout.automata.len(), 3);
        assert_eq!(e.auto_state, 0);
    }

    #[test]
    fn infinite_jamo_edit_replaces_jung() {
        // 핵심: 산(ㅅㅏㄴ) 입력 후 중성 ㅓ → 새 음절이 아니라 현재 중성 교체 → 선.
        let mut e = auto_engine();
        let (c, p) = typ(&mut e, "cam"); // ㅅ ㅏ ㄴ
        assert_eq!(c, "");
        assert_eq!(p, "산");
        let out = e.press(b'e', false); // 중성 ㅓ
        assert_eq!(out.commit, ""); // 확정 없음(현재 음절 수정)
        assert_eq!(out.preedit, "선"); // 무한 낱자 수정!
    }

    #[test]
    fn infinite_jamo_edit_jong() {
        // 안(ㅇ 없이 ㅏㄴ은 안 됨) 대신 간(ㄱㅏㄴ) 후 종성 교체: ㄱㅏㄴ → 종성 ㄴ 자리에 또?
        // 간 입력 후 중성 ㅗ → 곤? 아니라 중성 교체 → 곤.
        let mut e = auto_engine();
        typ(&mut e, "gam"); // 간
        assert_eq!(e.preedit(), "간");
        let out = e.press(b'o', false); // 중성 ㅗ → ㅏ 교체 → 곤
        assert_eq!(out.preedit, "곤");
    }

    #[test]
    fn kk_breaks_to_next_syllable() {
        // 사용자 ㅋㅋ 규칙(state1: D==176&&A==176 → 0): ㅋ 확정 후 새 ㅋ.
        let mut e = auto_engine();
        let o1 = e.press(b'k', false); // 초 ㅋ
        assert_eq!(o1.preedit, "ㅋ");
        let o2 = e.press(b'k', false); // 또 ㅋ → 앞 ㅋ 확정 + 새 ㅋ
        assert_eq!(o2.commit, "ㅋ");
        assert_eq!(o2.preedit, "ㅋ");
    }

    #[test]
    fn automaton_compound_vowel() {
        // 겹모음은 결합 유지: ㄱ ㅗ ㅏ → 과.
        let mut e = auto_engine();
        let (_c, p) = typ(&mut e, "goa");
        assert_eq!(p, "과");
    }

    #[test]
    fn automaton_new_cho_commits() {
        // 새 초성은 다음 글자: 가(ㄱㅏ) + ㄴ → 가 확정, 나 조합.
        let mut e = auto_engine();
        typ(&mut e, "ga"); // 가
        let out = e.press(b'n', false); // 초 ㄴ
        assert_eq!(out.commit, "가");
        assert_eq!(out.preedit, "ㄴ");
    }

    #[test]
    fn backspace_after_infinite_edit() {
        // ㄱ → 가 → 개(중성 ㅏ→ㅐ 무한 낱자 수정) 후 백스페이스 → ㄱ.
        // (교체된 ㅏ 가 이력에 남아 "가" 로 돌아가던 버그 방지.)
        let mut e = auto_engine();
        typ(&mut e, "ga"); // 가
        let o1 = e.press(b'i', false); // ㅐ → 개
        assert_eq!(o1.preedit, "개");
        let o2 = e.backspace(); // 중성 제거 → ㄱ
        assert_eq!(o2.preedit, "ㄱ");
    }

    #[test]
    fn backspace_after_compound_keeps_steps() {
        // 결합(겹모음)은 한 단계씩: ㄱㅗㅏ→과, 백스페이스 → 고(ㅘ→ㅗ).
        let mut e = auto_engine();
        typ(&mut e, "goa"); // 과
        assert_eq!(e.preedit(), "과");
        let o = e.backspace();
        assert_eq!(o.preedit, "고");
    }

    // Bksp 삭제 단위 모드별 테스트. AutomataTable + Extra/Bksp 를 가진 세벌식 설정.
    fn bksp_engine(value1: &str) -> Engine {
        let xml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<EditContextSetting version="0x500">
  <EditorLayer flag="0"><FinalConvTable/></EditorLayer>
  <InputLayer default="0" current="0">
    <InputEntry>
      <InputSchemeSetting object="CBasicInputScheme">
        <KeyTable name="b" flag="0" from="33" to="126">
          <Key at="0x67" value="H3|G_"/><Key at="0x61" value="H3|A_"/>
          <Key at="0x6D" value="H3|_N"/><Key at="0x73" value="H3|_S"/>
        </KeyTable>
      </InputSchemeSetting>
      <GeneratorSetting object="CNgsImeEx">
        <UnitMixTable><UnitMix unit="JONG" a="_N" b="_S" to="_NJ"/></UnitMixTable>
        <VirtualUnitTable/>
        <AutomataTable default="0">
          <Automata state="0" value="1" default="0"/>
          <Automata state="1" value="A||B||C ? (A||D)&amp;&amp;(B||E) ? 2 : 1 : -2" default="-1"/>
          <Automata state="2" value="A&amp;&amp;A!=500 ? 0 : B||C||A==500 ? 2 : -2" default="0"/>
        </AutomataTable>
        <Extra><Bksp key="1" value1="{value1}" value2="BySyllable" condition1="0" condition2="0"/></Extra>
      </GeneratorSetting>
    </InputEntry>
  </InputLayer>
</EditContextSetting>"#
        );
        let cfg = Config::parse(&xml).unwrap();
        Engine::new(cfg.compile(0).unwrap())
    }

    #[test]
    fn bksp_mode_lastkey() {
        // 직전 한 타: 간(ㄱㅏㄴ) → ㄴ 한 타만 → 가.
        let mut e = bksp_engine("ByUnitStep");
        typ(&mut e, "gam"); // 간
        let o = e.backspace();
        assert_eq!(o.preedit, "가");
    }

    #[test]
    fn bksp_mode_syllable() {
        // 글자 전체: 간 → 한 타에 통째 → 빈.
        let mut e = bksp_engine("BySyllable");
        typ(&mut e, "gam"); // 간
        let o = e.backspace();
        assert_eq!(o.preedit, "");
    }

    #[test]
    fn bksp_mode_lowest_whole() {
        // 최하위 낱자 전체: 갅(ㄱㅏ+겹받침ㄵ) → 종성 전체 제거 → 가.
        let mut e = bksp_engine("2"); // LowestWhole
        typ(&mut e, "gams"); // ㄱㅏ + ㄴ + ㅈ(겹받침 ㄵ)
        assert_eq!(e.preedit(), "갅");
        let o = e.backspace();
        assert_eq!(o.preedit, "가"); // 종성 ㄵ 통째 제거
    }

    #[test]
    fn bksp_mode_lowest_lastkey() {
        // 최하위 낱자 직전 한 타: 갅 → 종성 마지막 한 타(ㅈ) → 간.
        let mut e = bksp_engine("1"); // LowestLastKey
        typ(&mut e, "gams"); // 갅
        assert_eq!(e.preedit(), "갅");
        let o = e.backspace();
        assert_eq!(o.preedit, "간"); // ㄵ → ㄴ (한 단계)
    }
}
