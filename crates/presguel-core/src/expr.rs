//! 날개셋 값-식(value expression) 언어: 렉서 + 우선순위 파서 + 평가기.
//!
//! KeyTable 의 `value`(예: `T ? H3|_J : 0x23`, `119^(P&1)<<5`, `C0|0x82`)와 같은
//! C 연산자 문법의 정수 식을 다룬다. 태그 `H3|`(한글 낱자)·`C0|`(제어 명령)는 식
//! 안의 일급 값으로 취급한다. 참고: `research/01-nalgaeset-format.md` §1.

use crate::unit::{self, Unit};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExprError {
    #[error("예기치 못한 문자 {0:?} (위치 {1})")]
    UnexpectedChar(char, usize),
    #[error("예기치 못한 토큰: {0}")]
    UnexpectedToken(String),
    #[error("식이 끝나지 않음")]
    UnexpectedEnd,
    #[error("알 수 없는 변수 {0:?}")]
    UnknownVar(String),
    #[error("해석할 수 없는 낱자 operand {0:?}")]
    BadUnit(String),
    #[error("C0| operand 는 숫자여야 함: {0:?}")]
    BadCommand(String),
    #[error("정수가 아닌 값에 산술/비교 연산을 적용")]
    NotInt,
}

/// 평가 문맥의 변수들. KeyTable 평가에는 T(오토마타 상태)와 P(수식어 비트마스크)가 쓰인다.
/// A..E 는 오토마타 식 평가용(엔진에서 직접 쓰지 않음).
#[derive(Clone, Copy, Debug, Default)]
pub struct Ctx {
    /// 오토마타 상태 id. 0 = 한글 조합 중이 아님. (`T`)
    pub t: i64,
    /// 수식어 비트마스크. bit0 = Shift. (`P`)
    pub p: i64,
    pub a: i64,
    pub b: i64,
    pub c: i64,
    pub d: i64,
    pub e: i64,
}

impl Ctx {
    fn var(&self, name: &str) -> Option<i64> {
        Some(match name {
            "T" => self.t,
            "P" => self.p,
            "A" => self.a,
            "B" => self.b,
            "C" => self.c,
            "D" => self.d,
            "E" => self.e,
            _ => return None,
        })
    }
}

/// 식 평가 결과. 정수, 한글 낱자(H3|), 제어 명령(C0|) 중 하나.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Unit(Unit),
    Command(u32),
}

impl Value {
    fn truthy(self) -> bool {
        match self {
            Value::Int(n) => n != 0,
            // 낱자/명령은 "존재" → 참 (실제 설정에서 조건부엔 정수만 옴)
            _ => true,
        }
    }
    fn as_int(self) -> Result<i64, ExprError> {
        match self {
            Value::Int(n) => Ok(n),
            _ => Err(ExprError::NotInt),
        }
    }
}

// ── AST ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Int(i64),
    Var(String),
    Unit(Unit),
    Command(u32),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    BitNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    BitOr,
    BitXor,
    BitAnd,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Shl,
    Shr,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl Expr {
    /// 문자열 식을 파싱한다. `H3|`/`C0|` operand 는 이 시점에 단위/명령으로 해석된다.
    pub fn parse(src: &str) -> Result<Expr, ExprError> {
        let tokens = lex(src)?;
        let mut p = Parser { tokens, pos: 0 };
        let e = p.parse_ternary()?;
        if p.pos != p.tokens.len() {
            return Err(ExprError::UnexpectedToken(format!("{:?}", p.tokens[p.pos])));
        }
        Ok(e)
    }

    /// 이 식(하위 포함)이 H3| 한글 낱자(Unit)를 만들 수 있는지. 한글 조합 항목과
    /// 로마자/패스스루 항목을 구별하는 데 쓴다(한글 항목이면 KeyTable 에 H3| 가 있음).
    pub fn contains_unit(&self) -> bool {
        match self {
            Expr::Unit(_) => true,
            Expr::Int(_) | Expr::Var(_) | Expr::Command(_) => false,
            Expr::Unary(_, x) => x.contains_unit(),
            Expr::Binary(_, a, b) => a.contains_unit() || b.contains_unit(),
            Expr::Ternary(c, t, f) => c.contains_unit() || t.contains_unit() || f.contains_unit(),
        }
    }

    /// 문맥으로 식을 평가한다.
    pub fn eval(&self, ctx: &Ctx) -> Result<Value, ExprError> {
        Ok(match self {
            Expr::Int(n) => Value::Int(*n),
            Expr::Unit(u) => Value::Unit(*u),
            Expr::Command(c) => Value::Command(*c),
            Expr::Var(name) => Value::Int(ctx.var(name).ok_or_else(|| ExprError::UnknownVar(name.clone()))?),
            Expr::Unary(op, x) => {
                let v = x.eval(ctx)?.as_int()?;
                Value::Int(match op {
                    UnOp::Not => (v == 0) as i64,
                    UnOp::Neg => -v,
                    UnOp::BitNot => !v,
                })
            }
            Expr::Ternary(cond, t, f) => {
                if cond.eval(ctx)?.truthy() {
                    t.eval(ctx)?
                } else {
                    f.eval(ctx)?
                }
            }
            Expr::Binary(op, l, r) => {
                // 단축 평가 논리 연산
                match op {
                    BinOp::And => {
                        return Ok(Value::Int(
                            (l.eval(ctx)?.truthy() && r.eval(ctx)?.truthy()) as i64,
                        ))
                    }
                    BinOp::Or => {
                        return Ok(Value::Int(
                            (l.eval(ctx)?.truthy() || r.eval(ctx)?.truthy()) as i64,
                        ))
                    }
                    _ => {}
                }
                let a = l.eval(ctx)?.as_int()?;
                let b = r.eval(ctx)?.as_int()?;
                Value::Int(match op {
                    BinOp::BitOr => a | b,
                    BinOp::BitXor => a ^ b,
                    BinOp::BitAnd => a & b,
                    BinOp::Eq => (a == b) as i64,
                    BinOp::Ne => (a != b) as i64,
                    BinOp::Lt => (a < b) as i64,
                    BinOp::Gt => (a > b) as i64,
                    BinOp::Le => (a <= b) as i64,
                    BinOp::Ge => (a >= b) as i64,
                    BinOp::Shl => a << b,
                    BinOp::Shr => a >> b,
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    BinOp::Div => a / b,
                    BinOp::Rem => a % b,
                    BinOp::And | BinOp::Or => unreachable!(),
                })
            }
        })
    }
}

/// 한 번에 파싱하고 평가하는 편의 함수.
pub fn eval_str(src: &str, ctx: &Ctx) -> Result<Value, ExprError> {
    Expr::parse(src)?.eval(ctx)
}

// ── 렉서 ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Num(i64),
    Ident(String),
    Pipe,
    Question,
    Colon,
    OrOr,
    AndAnd,
    Caret,
    Amp,
    EqEq,
    Ne,
    Le,
    Ge,
    Shl,
    Shr,
    Lt,
    Gt,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Tilde,
    LParen,
    RParen,
}

fn lex(src: &str) -> Result<Vec<Tok>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // 숫자 (0x.. 또는 10진)
        if c.is_ascii_digit() {
            let start = i;
            if c == '0' && i + 1 < chars.len() && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
                i += 2;
                while i < chars.len() && chars[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let s: String = chars[start + 2..i].iter().collect();
                let n = i64::from_str_radix(&s, 16).map_err(|_| ExprError::UnexpectedToken(s.clone()))?;
                out.push(Tok::Num(n));
            } else {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n: i64 = s.parse().map_err(|_| ExprError::UnexpectedToken(s.clone()))?;
                out.push(Tok::Num(n));
            }
            continue;
        }
        // 식별자 (변수/태그/니모닉): [A-Za-z_][A-Za-z0-9_]*
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        // 연산자 (다문자 우선)
        let two = |a: char, b: char| i + 1 < chars.len() && chars[i] == a && chars[i + 1] == b;
        let tok = if two('|', '|') {
            i += 2;
            Tok::OrOr
        } else if two('&', '&') {
            i += 2;
            Tok::AndAnd
        } else if two('=', '=') {
            i += 2;
            Tok::EqEq
        } else if two('!', '=') {
            i += 2;
            Tok::Ne
        } else if two('<', '=') {
            i += 2;
            Tok::Le
        } else if two('>', '=') {
            i += 2;
            Tok::Ge
        } else if two('<', '<') {
            i += 2;
            Tok::Shl
        } else if two('>', '>') {
            i += 2;
            Tok::Shr
        } else {
            let t = match c {
                '|' => Tok::Pipe,
                '?' => Tok::Question,
                ':' => Tok::Colon,
                '^' => Tok::Caret,
                '&' => Tok::Amp,
                '<' => Tok::Lt,
                '>' => Tok::Gt,
                '+' => Tok::Plus,
                '-' => Tok::Minus,
                '*' => Tok::Star,
                '/' => Tok::Slash,
                '%' => Tok::Percent,
                '!' => Tok::Bang,
                '~' => Tok::Tilde,
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                other => return Err(ExprError::UnexpectedChar(other, i)),
            };
            i += 1;
            t
        };
        out.push(tok);
    }
    Ok(out)
}

// ── 파서 (재귀 하강 + 이항 우선순위 등반) ────────────────────────────────────

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }
    fn bump(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn expect(&mut self, t: &Tok) -> Result<(), ExprError> {
        match self.bump() {
            Some(ref got) if got == t => Ok(()),
            Some(got) => Err(ExprError::UnexpectedToken(format!("{got:?}"))),
            None => Err(ExprError::UnexpectedEnd),
        }
    }

    /// ternary := binary ( '?' ternary ':' ternary )?
    fn parse_ternary(&mut self) -> Result<Expr, ExprError> {
        let cond = self.parse_binary(0)?;
        if let Some(Tok::Question) = self.peek() {
            self.bump();
            let then = self.parse_ternary()?;
            self.expect(&Tok::Colon)?;
            let els = self.parse_ternary()?;
            Ok(Expr::Ternary(Box::new(cond), Box::new(then), Box::new(els)))
        } else {
            Ok(cond)
        }
    }

    /// 이항 연산: 최소 우선순위 `min_prec` 이상만 흡수.
    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, ExprError> {
        let mut left = self.parse_unary()?;
        while let Some(tok) = self.peek() {
            let (op, prec) = match binop_of(tok) {
                Some(v) => v,
                None => break,
            };
            if prec < min_prec {
                break;
            }
            self.bump();
            // 모든 이항 연산자는 좌결합 → 오른쪽은 prec+1
            let right = self.parse_binary(prec + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// unary := ('!' | '-' | '~')? unary | primary
    fn parse_unary(&mut self) -> Result<Expr, ExprError> {
        match self.peek() {
            Some(Tok::Bang) => {
                self.bump();
                Ok(Expr::Unary(UnOp::Not, Box::new(self.parse_unary()?)))
            }
            Some(Tok::Minus) => {
                self.bump();
                Ok(Expr::Unary(UnOp::Neg, Box::new(self.parse_unary()?)))
            }
            Some(Tok::Tilde) => {
                self.bump();
                Ok(Expr::Unary(UnOp::BitNot, Box::new(self.parse_unary()?)))
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ExprError> {
        match self.bump() {
            Some(Tok::Num(n)) => Ok(Expr::Int(n)),
            Some(Tok::LParen) => {
                let e = self.parse_ternary()?;
                self.expect(&Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::Ident(name)) => {
                // 태그 H3| / C0| 처리
                if (name == "H3" || name == "C0") && matches!(self.peek(), Some(Tok::Pipe)) {
                    self.bump(); // '|'
                    self.parse_tagged(&name)
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Some(other) => Err(ExprError::UnexpectedToken(format!("{other:?}"))),
            None => Err(ExprError::UnexpectedEnd),
        }
    }

    /// 태그 operand 를 읽어 단위/명령으로 해석. operand 는 숫자 또는 식별자(니모닉).
    fn parse_tagged(&mut self, tag: &str) -> Result<Expr, ExprError> {
        match self.bump() {
            Some(Tok::Num(n)) => {
                let n = n as u32;
                if tag == "C0" {
                    Ok(Expr::Command(n))
                } else {
                    let u = unit::resolve_numeric(n).ok_or_else(|| ExprError::BadUnit(format!("0x{n:X}")))?;
                    Ok(Expr::Unit(u))
                }
            }
            Some(Tok::Ident(s)) => {
                if tag == "C0" {
                    Err(ExprError::BadCommand(s))
                } else {
                    let u = unit::resolve_mnemonic(&s, None).ok_or(ExprError::BadUnit(s))?;
                    Ok(Expr::Unit(u))
                }
            }
            Some(other) => Err(ExprError::UnexpectedToken(format!("{other:?}"))),
            None => Err(ExprError::UnexpectedEnd),
        }
    }
}

/// 토큰 → (이항 연산자, 우선순위). 큰 값이 더 강하게 결합.
fn binop_of(t: &Tok) -> Option<(BinOp, u8)> {
    Some(match t {
        Tok::OrOr => (BinOp::Or, 1),
        Tok::AndAnd => (BinOp::And, 2),
        Tok::Pipe => (BinOp::BitOr, 3),
        Tok::Caret => (BinOp::BitXor, 4),
        Tok::Amp => (BinOp::BitAnd, 5),
        Tok::EqEq => (BinOp::Eq, 6),
        Tok::Ne => (BinOp::Ne, 6),
        Tok::Lt => (BinOp::Lt, 7),
        Tok::Gt => (BinOp::Gt, 7),
        Tok::Le => (BinOp::Le, 7),
        Tok::Ge => (BinOp::Ge, 7),
        Tok::Shl => (BinOp::Shl, 8),
        Tok::Shr => (BinOp::Shr, 8),
        Tok::Plus => (BinOp::Add, 9),
        Tok::Minus => (BinOp::Sub, 9),
        Tok::Star => (BinOp::Mul, 10),
        Tok::Slash => (BinOp::Div, 10),
        Tok::Percent => (BinOp::Rem, 10),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit::{Category, Jamo};

    fn ev(src: &str, ctx: &Ctx) -> Value {
        eval_str(src, ctx).unwrap_or_else(|e| panic!("eval {src:?}: {e}"))
    }

    #[test]
    fn literals_and_arith() {
        let ctx = Ctx::default();
        assert_eq!(ev("0xB7", &ctx), Value::Int(0xB7));
        assert_eq!(ev("500", &ctx), Value::Int(500));
        assert_eq!(ev("-2", &ctx), Value::Int(-2));
        // C 우선순위: << 가 ^ 보다 강함 → 119 ^ (1<<5) = 119 ^ 32 = 87 = 'W'
        let shifted = Ctx { p: 1, ..Default::default() };
        assert_eq!(ev("119^(P&1)<<5", &shifted), Value::Int('W' as i64));
        let unshifted = Ctx { p: 0, ..Default::default() };
        assert_eq!(ev("119^(P&1)<<5", &unshifted), Value::Int('w' as i64));
    }

    #[test]
    fn tagged_units() {
        let ctx = Ctx::default();
        // H3|_GG → 종성 ㄲ
        assert_eq!(ev("H3|_GG", &ctx), Value::Unit(Unit::Jamo(Jamo::new(Category::Jong, 0x11A9))));
        // H3|O_ → 중성 ㅗ
        assert_eq!(ev("H3|O_", &ctx), Value::Unit(Unit::Jamo(Jamo::new(Category::Jung, 0x1169))));
        // H3|0x820000 → 가상 단위 130
        assert_eq!(ev("H3|0x820000", &ctx), Value::Unit(Unit::Virtual(130)));
        // C0|0x82 → 한자 명령
        assert_eq!(ev("C0|0x82", &ctx), Value::Command(0x82));
    }

    #[test]
    fn ternary_with_t() {
        // T ? H3|_J : 0x23  (조합 중이면 종성 ㅈ, 아니면 '#')
        let composing = Ctx { t: 1, ..Default::default() };
        assert_eq!(
            ev("T ? H3|_J : 0x23", &composing),
            Value::Unit(Unit::Jamo(Jamo::new(Category::Jong, 0x11BD)))
        );
        let idle = Ctx { t: 0, ..Default::default() };
        assert_eq!(ev("T ? H3|_J : 0x23", &idle), Value::Int(0x23));
        // T ? H3|0x1F4 : 0x24  ($ 키: 조합 중이면 갈마들이 토글)
        assert_eq!(ev("T ? H3|0x1F4 : 0x24", &composing), Value::Unit(Unit::Toggle));
        assert_eq!(ev("T ? H3|0x1F4 : 0x24", &idle), Value::Int(0x24));
    }

    #[test]
    fn automata_expr_parses_and_evaluates() {
        // 상태 2 식: A&&A!=500 ? 0 : B||C||A==500 ? 2 : -2
        let src = "A&&A!=500 ? 0 : B||C||A==500 ? 2 : -2";
        // 들어온 게 초성(A!=0,!=500) → 0
        assert_eq!(ev(src, &Ctx { a: 0x1100, ..Default::default() }), Value::Int(0));
        // 들어온 게 종성(C!=0) → 2
        assert_eq!(ev(src, &Ctx { c: 0x11A8, ..Default::default() }), Value::Int(2));
        // 토글(A==500) → 2
        assert_eq!(ev(src, &Ctx { a: 500, ..Default::default() }), Value::Int(2));
        // 아무 낱자도 아님 → -2
        assert_eq!(ev(src, &Ctx::default()), Value::Int(-2));
    }

    #[test]
    fn contains_unit_detects_hangul_keys() {
        // 한글 키: H3| 포함 → true
        assert!(Expr::parse("H3|_GG").unwrap().contains_unit());
        assert!(Expr::parse("T ? H3|_J : 0x23").unwrap().contains_unit());
        // 로마자/리터럴 키: Unit 없음 → false
        assert!(!Expr::parse("119^(P&1)<<5").unwrap().contains_unit());
        assert!(!Expr::parse("0x5B").unwrap().contains_unit());
        assert!(!Expr::parse("C0|0x82").unwrap().contains_unit());
    }

    #[test]
    fn precedence_sanity() {
        let ctx = Ctx::default();
        assert_eq!(ev("1+2*3", &ctx), Value::Int(7));
        assert_eq!(ev("(1+2)*3", &ctx), Value::Int(9));
        assert_eq!(ev("1<2 && 3>2", &ctx), Value::Int(1));
        assert_eq!(ev("0 || 0 || 5", &ctx), Value::Int(1)); // 논리 결과는 0/1
        assert_eq!(ev("7 & 3", &ctx), Value::Int(3));
        assert_eq!(ev("1 << 4", &ctx), Value::Int(16));
    }
}
