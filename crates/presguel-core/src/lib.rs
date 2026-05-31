//! presguel-core — 날개셋(nalgaeset) 입력 설정을 해석하는 한글 조합 엔진.
//!
//! ibus 등 프런트엔드와 무관한 순수 라이브러리. 임의의 `nalgaeset.xml` 설정을
//! 파싱하여(`config`), 값-식을 평가하고(`expr`), 자모 단위를 모델링하며(`unit`),
//! 한글 오토마타로 음절을 조합한다(`automaton`, `engine`).
//!
//! 설계 근거는 저장소의 `research/01..04-*.md` 참고.

pub mod config;
pub mod engine;
pub mod evdev;
pub mod expr;
pub mod jamo;
mod ngs_seq;
pub mod unit;

#[doc(inline)]
pub use config::{Config, Layout};
#[doc(inline)]
pub use engine::{Engine, KeyOutcome};
#[doc(inline)]
pub use evdev::{evdev_to_ascii, us_qwerty_ascii};
#[doc(inline)]
pub use jamo::compose;
#[doc(inline)]
pub use unit::{Category, Jamo, Unit};
