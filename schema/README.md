# schema/

`nalgaeset-no-namespace.xsd` 는 presguel 이 읽는 자판 설정(`layout.xml`, 날개셋
종합 설정 `.set` 형식)을 기계 검증하기 위한 XML Schema 다. 런타임이 아니라
개발/CI 린트에서만 쓴다(`crates/presguel-core/tests/schema.rs`).

- 출처: [chaotic-ground/nalgaeset-reverse-spec](https://github.com/chaotic-ground/nalgaeset-reverse-spec)
- 라이선스: CC BY 4.0 (© 2026 lens0021). 본 파일은 그 저장소의 정본
  `nalgaeset.xsd` 에서 네임스페이스 헤더만 제거한 동기화본으로, 네임스페이스가
  없는 실제 날개셋 출력 파일을 그대로 검증한다.
- 갱신: 상위 사양이 바뀌면 위 저장소에서 다시 받아 이 파일을 교체한다.
