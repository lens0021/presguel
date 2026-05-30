# presguel

**날개셋(nalgaeset) 입력 설정과 호환되는, 순수 Rust로 작성하는 ibus 한글 입력기.**

> 🟡 **초기 동작본.** 세벌식-맞춤 한글 조합이 ibus에서 실제로 동작합니다(완성형 +
> 옛한글 첫가끝 + 겹낱자/갈마들이). 다듬을 부분이 남아 있습니다(한자 변환, 로마자
> 드보락 항목, 일부 제어 명령 등).

---

## 무엇인가

[날개셋 한글 입력기](http://moogi.new21.org/)의 "입력 설정" XML(`nalgaeset.xml`)을
해석하여, Linux의 [ibus](https://github.com/ibus/ibus) 환경에서 **동일한 한글 입력
동작을 재현**하는 입력기 엔진입니다.

- **범용 해석기**: 특정 자판을 하드코딩하지 않고, 임의의 날개셋 설정 XML
  (`KeyTable` / `UnitMixTable` / `VirtualUnitTable` / `AutomataTable` /
  `FinalConvTable` 등)을 파싱·해석하는 것을 목표로 합니다.
- **순수 Rust**: `libibus`(C) 의존 없이 [`zbus`](https://github.com/dbus2/zbus)로
  ibus 데몬과 D-Bus를 직접 주고받습니다.
- **옛한글 완전 지원**: 첫가끝(U+1100 조합용 자모) 조합과 `FinalConvTable`을
  포함합니다. 옛한글 음절의 *표시*는 옛한글 OpenType 폰트(예: 함초롬)가 필요하며,
  이는 사용자 폰트 환경의 몫입니다. 엔진은 올바른 코드포인트를 commit 합니다.

## 배경

날개셋은 강력하지만 Windows 중심입니다. 같은 세벌식(및 임의 사용자 설정) 경험을
Linux ibus 위에서 그대로 쓰기 위해 만듭니다. 참고로 호환 대상 설정 파일은 이
저장소 바깥(사용자 환경)에 있는 사용자 소유 파일이며, 저장소에 포함하지 않습니다.

## 호환 대상 포맷

`EditContextSetting` (날개셋 입력 설정):

| 요소 | 역할 |
| --- | --- |
| `ShortcutTable` | 한/영 전환 등 특수키 동작 |
| `FinalConvTable` | 미완성/홑낱자 출력 시 조합용·옛 자모 → 호환 자모 변환 |
| `KeyTable` | 물리 키 → 자모 단위/문자 (값-식 언어) |
| `UnitMixTable` | 낱자 조합 규칙 (겹받침, 겹모음, 된소리, 갈마들이) |
| `VirtualUnitTable` | 가상 단위 |
| `AutomataTable` | 한글 조합 오토마타 (상태 전이 식) |
| `Extra` / `Bksp` | 백스페이스 동작 |

## 구조 (예정)

- **`presguel-core`** — 설정 파서 + 값-식 평가기 + 자모 모델 + 한글 오토마타.
  ibus와 무관한 순수 라이브러리. 테스트 벡터로 동작을 검증합니다.
- **`presguel-ibus`** — `zbus` 기반 ibus 프런트엔드(Factory / Engine),
  preedit·commit, 한/영 전환.

## 진행 상황

- [x] 날개셋 포맷·식 언어·오토마타 해석 명세 (`research/01-nalgaeset-format.md`)
- [x] 설정 파일 역공학 → 동작 명세·테스트 벡터(오라클) (`research/02-config-decode.md`)
- [x] `presguel-core`: 파서 / 식 평가 / 자모 / 오토마타 (단위·통합 테스트 통과)
- [x] `presguel-ibus`: zbus 프런트엔드 (실제 ibus 데몬에서 조합 검증)
- [x] 옛한글(첫가끝) 조합·출력
- [x] 설치 / 패키징(ibus 컴포넌트 등록, `scripts/install.sh`)
- [x] 한/영 전환(한글 키 / CapsLock, ShortcutTable 해석) + 패널 표시기(날개셋 방식 `가N`/`AN`)
- [x] 입력 항목 순환 전환(IME_SWITCH 로 모든 항목 순환) + 로마자 항목 글쇠 리매핑(드보락 등)
- [ ] 한자 변환(C0 명령), 항목 전환 그룹(`!A`/`!B`) 세분화
- [ ] 백스페이스 동작 방식 세분화(Bksp 표 충실 반영)

## 빌드 / 설치

```sh
# 빌드 + 테스트
cargo test

# 설치 (이 환경의 ibus 는 시스템 컴포넌트 디렉터리만 스캔하므로 sudo 필요)
#   - 바이너리 → /usr/local/bin/presguel-ibus
#   - 설정     → ~/.config/presguel/nalgaeset.xml  (인자로 경로 지정 가능)
#   - 컴포넌트 → /usr/share/ibus/component/presguel.xml
scripts/install.sh /path/to/nalgaeset.xml

# 입력기 전환
ibus engine presguel        # 또는 GNOME 설정 → 키보드 → 입력 소스에서 추가 후 Super+Space
```

### 한/영 전환과 표시기

- `IME_SWITCH` 글쇠(**한글 키 / CapsLock** 등)로 설정의 입력 항목을 **순환 전환**합니다
  (예: 세벌식 → 드보락 → 직접 → 세벌식). 한글 항목은 조합, 로마자 항목은 글쇠 리매핑,
  키표 없는 항목은 패스스루합니다.
- 패널 표시기는 날개셋(Windows) 방식을 따라 **`접두 + 항목번호`** 로 동적 표시됩니다
  (한글 항목 `가N`, 로마자/직접 항목 `AN`; 예 `가0`, `A1`). 항목을 XML 에 추가하면
  번호도 늘어납니다. (ibus 속성 `RegisterProperties`/`UpdateProperty`.)
- **CapsLock 을 한/영 키로** (GNOME/Wayland): 컴포지터가 CapsLock 잠금을 가로채므로
  XKB 레벨 재매핑이 필요합니다. 아래 스크립트가 *CapsLock 단독 → 한/영*,
  *Shift+CapsLock → 평소 대소문자 잠금* 으로 설정합니다(sudo 불필요):

  ```sh
  scripts/setup-capslock-hangul.sh          # 적용 (입력 소스 한 번 전환하면 반영)
  scripts/setup-capslock-hangul.sh --revert # 되돌리기
  ```

설정 파일 경로는 `PRESGUEL_CONFIG` 환경변수로도 지정할 수 있고, 기본값은
`~/.config/presguel/nalgaeset.xml` 입니다.

### 동작 확인 (GUI 없이)

실행 중인 엔진을 D-Bus 로 직접 구동해 조합 결과를 확인하는 예제:

```sh
cargo run -p presguel-ibus --example drive -- "kf kfhf"   # → "가가나"
```

## 라이선스

이 저장소는 **공개(public)되어 있으나 오픈소스가 아닙니다.** 자세한 내용은
[`LICENSE`](./LICENSE)를 참고하세요. 요지: 모든 권리 보유(All rights reserved),
저작권자의 사전 허가 없이는 사용·복제·수정·배포할 수 없습니다. 라이선스는 추후
확정될 수 있습니다.
