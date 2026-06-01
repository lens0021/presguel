//! 자판 설정 XML 을 날개셋 reverse-spec 스키마(no-namespace XSD)로 검증하는
//! 개발/CI 린트. 런타임 의존성이 아니라 테스트 전용이며, `xmllint`(libxml2)가
//! PATH 에 있을 때만 실행하고 없으면 건너뛴다.
//!
//! 검증 대상:
//!  1. `tests/fixtures/*.xml` — 저장소 동봉 clean-room 예제(.set/.ist/.key). 항상 검사.
//!  2. 사용자 실제 설정 — `PRESGUEL_TEST_CONFIG` 또는 provision `layout.xml` 가
//!     있으면 추가 검사(없으면 skip, CI 에서는 자연히 skip).
//!
//! 스키마 출처: chaotic-ground/nalgaeset-reverse-spec (CC BY 4.0). `schema/` 참조.

use std::path::{Path, PathBuf};
use std::process::Command;

fn xmllint_available() -> bool {
    Command::new("xmllint")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 저장소 루트의 vendored no-namespace XSD 경로.
fn schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/nalgaeset-no-namespace.xsd")
}

fn validate(xsd: &Path, xml: &Path) -> Result<(), String> {
    let out = Command::new("xmllint")
        .arg("--noout")
        .arg("--schema")
        .arg(xsd)
        .arg(xml)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[test]
fn fixtures_validate_against_schema() {
    if !xmllint_available() {
        eprintln!("skip: xmllint(libxml2) 없음");
        return;
    }
    let xsd = schema_path();
    assert!(xsd.exists(), "스키마 없음: {}", xsd.display());

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    let mut fails = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("fixtures 디렉터리") {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }
        checked += 1;
        if let Err(e) = validate(&xsd, &p) {
            fails.push(format!(
                "  {}:\n{}",
                p.file_name().unwrap().to_string_lossy(),
                e
            ));
        }
    }
    assert!(checked > 0, "검사할 픽스처가 없음: {}", dir.display());
    assert!(
        fails.is_empty(),
        "{} 개 픽스처가 스키마 위반:\n{}",
        fails.len(),
        fails.join("\n")
    );
}

#[test]
fn real_config_validates_against_schema() {
    if !xmllint_available() {
        eprintln!("skip: xmllint 없음");
        return;
    }
    let path = std::env::var("PRESGUEL_TEST_CONFIG")
        .unwrap_or_else(|_| "/home/nemo/git/lens/provision/config/layout.xml".to_string());
    let path = PathBuf::from(path);
    if !path.exists() {
        eprintln!("skip: 실제 설정 없음 ({})", path.display());
        return;
    }
    if let Err(e) = validate(&schema_path(), &path) {
        panic!("실제 설정이 스키마 위반:\n{e}");
    }
}
