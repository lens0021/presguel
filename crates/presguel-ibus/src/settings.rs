//! 사용자 설정(`~/.config/presguel/config.ini`) 읽기.
//!
//! 형식은 `key=value`(한 줄에 하나, `#` 주석 무시) — addr.rs 의 IBus 주소 파일과 같은
//! 단순 형식이라 의존성이 없고, Python 설정창도 쉽게 읽고 쓴다.
//!
//! ```ini
//! # 간단 모드: 켜면 한글/영문 InputEntry 를 직접 지정한다.
//! simple_mode = false
//! # 간단 모드에서 쓸 한글 조합 InputEntry 인덱스.
//! hangul_entry = 0
//! # 그 한글 항목의 글쇠가 깔린 영문 배치 InputEntry 인덱스.
//! # 단축키 조합(Ctrl/Alt/Super+키)을 이 배치로 변환해 응용에 넘긴다(예: 드보락).
//! latin_entry = 1
//! ```

use std::path::PathBuf;

/// 파싱된 사용자 설정. 파일이 없거나 키가 빠지면 기본값(날개셋과 동일 동작).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// false(기본): 모든 InputEntry 를 읽어 날개셋과 똑같이 동작.
    /// true: 아래 두 항목만 써서 단순하게 동작.
    pub simple_mode: bool,
    /// 간단 모드에서 쓸 한글 조합 InputEntry 인덱스.
    pub hangul_entry: usize,
    /// 간단 모드에서 단축키 조합을 변환할 기준 영문 배치 InputEntry 인덱스.
    pub latin_entry: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { simple_mode: false, hangul_entry: 0, latin_entry: 1 }
    }
}

impl Settings {
    /// 표준 경로(`$PRESGUEL_CONFIG_INI` 또는 `~/.config/presguel/config.ini`)에서 읽는다.
    /// 파일이 없으면 기본값.
    pub fn load() -> Self {
        match Self::path().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(body) => Self::parse(&body),
            None => Self::default(),
        }
    }

    /// 설정 파일 경로.
    pub fn path() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("PRESGUEL_CONFIG_INI") {
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("presguel").join("config.ini"))
    }

    /// `key=value` 본문을 파싱. 알 수 없는 키/값은 무시하고 기본값 유지.
    pub fn parse(body: &str) -> Self {
        let mut s = Self::default();
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "simple_mode" => {
                    if let Some(b) = parse_bool(v) {
                        s.simple_mode = b;
                    }
                }
                "hangul_entry" => {
                    if let Ok(n) = v.parse() {
                        s.hangul_entry = n;
                    }
                }
                "latin_entry" => {
                    if let Ok(n) = v.parse() {
                        s.latin_entry = n;
                    }
                }
                _ => {}
            }
        }
        s
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_full_mode() {
        let s = Settings::default();
        assert!(!s.simple_mode);
    }

    #[test]
    fn parse_basic() {
        let s = Settings::parse("simple_mode = true\nhangul_entry=0\nlatin_entry = 1\n");
        assert!(s.simple_mode);
        assert_eq!(s.hangul_entry, 0);
        assert_eq!(s.latin_entry, 1);
    }

    #[test]
    fn parse_ignores_comments_and_unknown() {
        let s = Settings::parse("# 주석\nsimple_mode=on\nfoo=bar\n\n");
        assert!(s.simple_mode);
        assert_eq!(s.hangul_entry, 0); // 기본값 유지
    }

    #[test]
    fn parse_empty_is_default() {
        assert_eq!(Settings::parse(""), Settings::default());
    }

    #[test]
    fn bool_forms() {
        assert!(Settings::parse("simple_mode=1").simple_mode);
        assert!(Settings::parse("simple_mode=yes").simple_mode);
        assert!(!Settings::parse("simple_mode=off").simple_mode);
        assert!(!Settings::parse("simple_mode=garbage").simple_mode);
    }
}
