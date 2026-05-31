//! 실제 `nalgaeset.xml`(저장소 바깥, 사용자 환경)에 대한 통합 검증.
//!
//! 설정 경로는 `PRESGUEL_TEST_CONFIG` 환경변수로 지정하거나, 없으면 이 머신의
//! provision 경로를 기본값으로 쓴다. 파일이 없으면 테스트를 건너뛴다(저장소에는
//! 사용자 설정을 포함하지 않으므로 CI 에서는 자연히 skip).

use std::path::PathBuf;

use presguel_core::config::Config;
use presguel_core::Engine;

fn config_path() -> Option<PathBuf> {
    let p = std::env::var("PRESGUEL_TEST_CONFIG")
        .unwrap_or_else(|_| "/home/nemo/git/lens/provision/config/nalgaeset.xml".to_string());
    let p = PathBuf::from(p);
    p.exists().then_some(p)
}

fn load() -> Option<Config> {
    let path = config_path()?;
    let xml = std::fs::read_to_string(&path).expect("read config");
    Some(Config::parse(&xml).expect("parse config"))
}

#[test]
fn real_config_parses() {
    let Some(cfg) = load() else {
        eprintln!("skip: nalgaeset.xml 없음");
        return;
    };
    assert_eq!(cfg.version, "0x500");
    assert_eq!(cfg.default_entry, 0);
    assert_eq!(cfg.entries.len(), 3);

    // 항목 0 = 세벌식-맞춤
    let e0 = &cfg.entries[0];
    assert_eq!(e0.scheme_object, "CBasicInputScheme");
    assert_eq!(e0.generator_object, "CNgsImeEx");
    let kt = e0.key_table.as_ref().expect("키 테이블");
    assert_eq!(kt.name, "세벌식-맞춤");
    assert_eq!(kt.from, 33);
    assert_eq!(kt.to, 126);
    // 0x21..=0x7E = 94 키 전부
    assert_eq!(kt.keys.len(), 94, "키 개수");

    // 항목 1 = 로마자 드보락 (식 `^(P&1)<<5` 들이 전부 파싱되어야 함)
    assert_eq!(cfg.entries[1].scheme_object, "CAdvancedScheme");
    assert!(cfg.entries[1].key_table.is_some());

    // 항목 2 = 패스스루
    assert_eq!(cfg.entries[2].scheme_object, "CInputScheme");
    assert!(cfg.entries[2].key_table.is_none());

    // FinalConvTable 전체
    assert!(cfg.editor.final_conv.len() > 150, "FinalConv 항목 수");
    assert_eq!(cfg.editor.final_conv.get(&0x1100), Some(&0x3131));
    assert_eq!(cfg.editor.final_conv.get(&0x11A8), Some(&0x3131));

    // 단축글쇠
    assert!(cfg.editor.shortcuts.iter().any(|s| s.key == "VK_HANGUL"));

    assert_eq!(cfg.first_hangul_entry(), Some(0));
}

#[test]
fn real_config_compiles() {
    let Some(cfg) = load() else {
        eprintln!("skip: nalgaeset.xml 없음");
        return;
    };
    let layout = cfg.compile(0).unwrap();
    assert_eq!(layout.name, "세벌식-맞춤");
    assert_eq!(layout.keys.len(), 94);

    // 갈마들이 5쌍: ㄱ↔ㄲ ㄷ↔ㄸ ㅂ↔ㅃ ㅅ↔ㅆ ㅈ↔ㅉ (토글 경로)
    use presguel_core::unit::TOGGLE;
    use presguel_core::Category::*;
    assert_eq!(layout.combine(Cho, 0x1100, TOGGLE), Some(0x1101)); // ㄱ→ㄲ
    assert_eq!(layout.combine(Cho, 0x1101, TOGGLE), Some(0x1100)); // ㄲ→ㄱ
                                                                   // 겹모음 6개
    assert_eq!(layout.combine(Jung, 0x1169, 0x1161), Some(0x116A)); // ㅗ+ㅏ→ㅘ
    assert_eq!(layout.combine(Jung, 0x116E, 0x1165), Some(0x116F)); // ㅜ+ㅓ→ㅝ
                                                                    // 겹받침 3개 (RS/RT/RP)
    assert_eq!(layout.combine(Jong, 0x11AF, 0x11BA), Some(0x11B3)); // ㄹ+ㅅ→ㄽ

    // 가상 단위 128/129/130 = ㅗ/ㅜ/ㅡ
    use presguel_core::Jamo;
    assert_eq!(
        layout.virtual_units.get(&128),
        Some(&Jamo::new(Jung, 0x1169))
    );
    assert_eq!(
        layout.virtual_units.get(&129),
        Some(&Jamo::new(Jung, 0x116E))
    );
    assert_eq!(
        layout.virtual_units.get(&130),
        Some(&Jamo::new(Jung, 0x1173))
    );
}

/// 키 시퀀스를 눌러 (확정 누적 + 마지막 flush) 전체 출력 문자열을 만든다.
fn run(cfg: &Config, keys: &str) -> String {
    let layout = cfg.compile(0).unwrap();
    let mut e = Engine::new(layout);
    let mut out = String::new();
    for ch in keys.chars() {
        out.push_str(&e.press(ch as u8, false).commit);
    }
    out.push_str(&e.flush());
    out
}

/// `research/02-config-decode.md` §D 의 테스트 벡터(도달 가능한 것)를 실제 설정으로 검증.
#[test]
fn decode_test_vectors() {
    let Some(cfg) = load() else {
        eprintln!("skip: nalgaeset.xml 없음");
        return;
    };

    // (키 시퀀스, 기대 출력)
    let cases: &[(&str, &str)] = &[
        // 기본 음절
        ("kf", "가"),
        ("hf", "나"),
        ("uf", "다"),
        // 받침
        ("kfx", "각"),
        ("kfs", "간"),
        ("ifs", "만"),
        ("hfs", "난"),
        // 겹모음
        ("k/f", "과"),
        ("k9t", "궈"),
        ("j9t", "워"),
        ("j/d", "외"),
        ("j9d", "위"),
        ("j8", "의"),
        // 겹받침 (전용 키)
        ("kf@", "갉"),
        ("ufF", "닮"),
        ("kfV", "갃"),
        ("jfS", "않"),
        // 된소리: 연타 / 갈마들이 토글
        ("kkf", "까"),
        ("k$f", "까"),
        ("n$f", "싸"),
        ("k$$f", "가"),
        ("u$f", "따"),
        ("l$f", "짜"),
        // 이어치기: 새 초성이 앞 음절 확정
        ("kfkf", "가가"),
        ("kfhf", "가나"),
        // 홑낱자 → 호환 자모(FinalConv)
        ("k", "ㄱ"),
        ("f", "ㅏ"),
        ("x", "ㄱ"),
        // `@` = `T ? H3|_RG : 0x40` → 빈 상태(T=0)에서는 리터럴 '@'. (decode 벡터 29 정정:
        // 조건식을 빠뜨려 ㄺ 로 적었으나, 비조합 상태의 `@`는 '@' 가 맞다.)
        ("@", "@"),
        ("X", "ㅄ"),
        // ㄹ 겹받침 (UnitMix RS/RT)
        ("yfw", "랄"),
        ("yfwq", "랈"),
        ("yfwW", "랉"),
        ("ifQ", "맢"),
        // CVC(각) 뒤 모음 ㅏ: 실제 AutomataTable 상태2 식
        // (A&&A!=500 ? 0 : B||C||A==500 ? 2 : -2)에서 중성(B≠0)은 결과 2 = 무한 낱자 수정
        // → 새 음절이 아니라 현재 음절 중성을 교체. 각은 이미 중성 ㅏ라 그대로 "각".
        // (옛 휴리스틱은 "각ㅏ"였으나 오토마타 정식 구현 후엔 이쪽이 날개셋 실제 동작.)
        ("kfxf", "각"),
        // 가상 단위: v=ㅗ, b=ㅜ, g=ㅡ
        ("kvf", "과"),
        ("jg", "으"),
        ("kg", "그"),
    ];

    let mut fails = Vec::new();
    for (keys, expected) in cases {
        let got = run(&cfg, keys);
        if got != *expected {
            fails.push(format!("  {keys:?}: 기대 {expected:?}, 실제 {got:?}"));
        }
    }
    assert!(
        fails.is_empty(),
        "{} 개 벡터 불일치:\n{}",
        fails.len(),
        fails.join("\n")
    );
}
