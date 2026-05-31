# 07 - IBus 엔진과 XKB 레이아웃 페어링 (GNOME 49 / Wayland)

조사 환경: Fedora 43, `mutter-49.5`, `gnome-shell-49.6`, `ibus-1.5.33`, Wayland(mutter).
입력 소스: `[('ibus','hangul'), ('xkb','us+dvorak'), ('ibus','presguel')]`.
증상: `presguel` 활성 시 ProcessKeyEvent 가 **QWERTY(us) keysym** 을 받는다(물리 QWERTY-R → keyval `r` 0x72, dvorak `p` 아님). 즉 GNOME 이 ibus 엔진을 `us` XKB 레이아웃에 묶는다.

표기: **FACT** = 소스/문서 직접 인용 근거, **INFER** = 그 사실에서의 추론.

> ⚠️ 이 문서는 `research/05-wayland-xkb-keyval.md` 의 일부 결론을 **정정**한다. 05 는 "엔진 `<layout>` 은 GNOME Wayland 에서 무시된다, `<layout>default</layout>` 로 두면 사용자 XKB 를 상속한다"고 [STRONG] 으로 적었으나, 이번에 gnome-shell 49 소스(`status/keyboard.js`, `misc/keyboardManager.js`)를 직접 인용해 확인한 결과 **그 결론은 이 코드 경로에서 틀렸다.** 엔진 `<layout>`/`<layout_variant>` 는 **실제로 활성 XKB 레이아웃을 결정**하며(그래서 presguel=us → us keysym 이 나온 것), `default` 는 "상속"이 아니라 사실상 us 로 귀결된다. 자세한 근거는 §1, §8.

---

## 0. 한 줄 결론

**FACT** GNOME 49/Wayland 에서 ibus 입력 소스가 활성화되면, mutter 가 keysym 변환에 쓰는 XKB 레이아웃은 **그 엔진 desc 의 `layout`/`layout_variant`(컴포넌트 XML 의 `<layout>`/`<layout_variant>`)에서 직접 만들어진다.** gnome-shell 이 `engineDesc.layout`(+`variant`)로 그 소스의 `xkbId` 를 만들고, 모든 입력 소스의 `xkbId` 배열을 `KeyboardManager.setUserLayouts()` 로 mutter 에 등록한 뒤, 그 소스로 전환하면 해당 XKB 그룹이 활성화된다.

따라서:
- presguel 이 `<layout>us</layout>` → us 그룹 → 물리 QWERTY-R = `r`. (사용자 실측과 일치, §1.)
- 다른 `('xkb','us+dvorak')` 소스를 같이 두거나 순서를 바꿔도 **presguel 활성 시엔 presguel 자신의 layout(us)이 활성 그룹**이 된다. 상속 안 함. (§3)
- `<layout>default</layout>` 는 dvorak 을 주지 않는다. `default` 는 유효한 XKB 레이아웃 id 가 아니라 `get_layout_info('default')` 에서 not-found → 사실상 us. (§1.4, §8)

**dvorak 사용자가 일반 IME 를 유지하면서 dvorak keysym/단축키를 얻는 유일한 동작 방식**:
엔진 desc 의 `<layout>us</layout><layout_variant>dvorak</layout_variant>` 를 쓰되, dvorak 을 코드에 박지 말고 **사용자 설정값으로 이 두 필드를 채워 컴포넌트 XML 을 (재)생성**한다 (= 옵션 a + d, §6 권장 레시피).

---

## 1. mutter/gnome-shell 이 ibus 엔진의 XKB 레이아웃을 정하는 메커니즘 (소스 직접 인용)

### 1.1 큰 그림
**FACT** Wayland 에는 X11 의 `setxkbmap` 같은 범용 경로가 없어 레이아웃 관리는 데스크톱마다 구현된다(fcitx 위키). GNOME 에서 그 구현은 **gnome-shell 의 `InputSourceManager`(`js/ui/status/keyboard.js`) + `KeyboardManager`(`js/misc/keyboardManager.js`)** 이다. GNOME 설계 문서: "Input sources are a simple tuple of XKB layout and IBus engine that are known to work together" — 즉 입력 소스 하나가 (xkb 레이아웃)과 (ibus 엔진)을 한 묶음으로 본다. (wiki.gnome.org/ThreePointFive/Features/IBus)

### 1.2 ibus 소스의 `xkbId` 는 엔진 desc 의 layout/variant 에서 나온다 — **FACT (소스 인용)**
gnome-shell `js/ui/status/keyboard.js` 의 `InputSource` 클래스(gnome-49 브랜치, 직접 인용):

```js
// InputSource 생성자
this.xkbId = this._getXkbId();
...
_getXkbId() {
    let engineDesc = IBusManager.getIBusManager().getEngineDesc(this.id);
    if (!engineDesc)
        return this.id;                         // xkb 타입 소스: id 그대로 (예: 'us+dvorak')
    if (engineDesc.variant && engineDesc.variant.length > 0)
        return `${engineDesc.layout}+${engineDesc.variant}`;   // 예: 'us+dvorak'
    else
        return engineDesc.layout;               // 예: 'us'
}
```
- `('xkb','us+dvorak')` → `getEngineDesc` 없음 → `xkbId = 'us+dvorak'`.
- `('ibus','presguel')` → `engineDesc.layout='us'`, `variant=''` → `xkbId = 'us'`.
- `('ibus','hangul')` → `layout='kr'`, `variant='kr104'` → `xkbId = 'kr+kr104'`.

`engineDesc.layout`/`engineDesc.variant` 는 IBus.EngineDesc 의 GObject 프로퍼티(= 컴포넌트 XML 의 `<layout>`/`<layout_variant>`)이다.

### 1.3 `xkbId` 배열이 mutter 에 등록된다 — **FACT (소스 인용)**
같은 파일 `InputSourceManager._updateMruSources()`:
```js
this._keyboardManager.setUserLayouts(sourcesList.map(x => x.xkbId));
```
즉 **모든 입력 소스(엔진 포함)의 `xkbId` 가 KeyboardManager 에 XKB 레이아웃 목록으로 들어간다.** 활성 소스로 전환하면 그 소스의 `xkbId` 에 해당하는 그룹이 활성화된다.

`js/misc/keyboardManager.js` (직접 인용):
```js
export const DEFAULT_LAYOUT = 'us';
export const DEFAULT_VARIANT = '';
...
setUserLayouts(ids) {
    this._current = null;
    this._layoutInfos = {};
    for (const id of ids) {
        let [found, , , layout, variant] = this._xkbInfo.get_layout_info(id);
        if (found)
            this._layoutInfos[id] = {id, layout, variant};
    }
    // ... group/groupIndex 배정
}
```
- 각 `id`(= xkbId)를 `GnomeDesktop.XkbInfo.get_layout_info()` 로 조회. **found 면** `_layoutInfos[id]` 에 등록. **not-found 면 그냥 건너뜀(skip).**
- `'us'`, `'us+dvorak'`, `'kr+kr104'` 등은 유효 → 등록됨.
- `'default'` 는 유효한 XKB 레이아웃 id 가 아님 → not-found → **건너뜀** (§1.4).

### 1.3b 소스로 전환할 때 실제 mutter 키맵이 바뀐다 — **FACT (소스 인용)**
`InputSourceManager.activateInputSource(is, interactive)` 는 xkb/ibus 양쪽 소스 모두에 대해:
```js
this._keyboardManager.apply(is.xkbId);     // 둘 다 동일하게 호출
// (ibus 소스면 추가로) this._ibusManager.setEngine(engine);
```
`KeyboardManager.apply(id)` (직접 인용):
```js
apply(id) {
    let info = this._layoutInfos[id];
    if (!info)
        return;                            // ← 미등록 id 면 아무것도 안 함
    if (this._current && this._current.group === info.group) {
        if (this._current.groupIndex !== info.groupIndex)
            this._applyLayoutGroupIndex(info.groupIndex).catch(logError);
    } else {
        this._doApply(info).catch(logError);
    }
    this._current = info;
}
```
`_applyLayoutGroup(group)` 가 최종적으로 mutter 컴포지터 키맵을 설정한다 (직접 인용):
```js
let [layouts, variants] = this._buildGroupStrings(group);
...
await global.backend.set_keymap_async(layouts, variants, options, model, null);
```
⇒ **엔진의 layout/variant → xkbId → `_layoutInfos` → `set_keymap_async(layouts, variants, ...)`.** 이것이 keysym 변환을 결정하는 mutter 키맵 그 자체다.

### 1.4 `<layout>default</layout>` 는 무엇으로 resolve 되나 — **이제 FACT 로 확정**
- **FACT** `DEFAULT_LAYOUT='us'`, `DEFAULT_VARIANT=''`. (로케일 기반 폴백 `_getLocaleLayout` 등에서 쓰이는 최종값.)
- **FACT** `setUserLayouts` 는 `get_layout_info('default')` 가 not-found 면 그 항목을 `_layoutInfos` 에 넣지 않는다(skip). keyboard.js/keyboardManager.js 어디에도 문자열 `'default'` 특별 분기는 **없다**(소스 확인).
- **FACT** `apply('default')` → `let info = this._layoutInfos['default']; if (!info) return;` → **아무 일도 안 함(return early).** 즉 presguel(`layout=default`)로 전환해도 mutter 키맵은 **직전에 적용돼 있던 그룹 그대로 유지**된다.
- **FACT (IBus 측)** ibus engine desc 의 `layout='default'` 는 "system 의 native XKB 를 그대로 쓰고 ibus 가 강제하지 말라"는 의도(ibus#1614, 기능요청). 즉 의미상으로도 "강제 안 함".
- **INFER (강)** 그래서 GNOME/Wayland 에서 presguel `<layout>default</layout>` 의 실제 결과: `xkbId='default'` 는 미등록 → presguel 활성 시 `apply` no-op → **직전 활성 그룹이 그대로 남는다.** 실무상 그 "직전 그룹"은 거의 항상 us 계열(첫 등록 레이아웃/`DEFAULT_LAYOUT='us'`)이라 결과적으로 **us 처럼 보인다**. 어느 쪽이든 **dvorak 을 능동적으로 만들어 주지 않는다.** 즉 `default` 는 "사용자 dvorak 상속" ❌.
  - 정정 포인트: §0/§1.2 에서 "default → us 로 귀결"이라 단순화했는데, 정확히는 **"default → apply no-op → 직전 그룹 유지(보통 us)"**. dvorak 이 안 나온다는 결론은 동일.

### 1.5 keysym 변환 자체를 결정한다 (전환 전용 아님)
질문: "per-engine `<layout>` 은 Wayland 에서 전환용으론 무시된다던데, keysym 변환 레이아웃은 거기서 오는 것 아니냐, 둘은 다른 경로 아니냐."
- **FACT/INFER (강)** 여기선 같은 경로다. gnome-shell 이 엔진 layout 으로 `xkbId` 를 만들어 `setUserLayouts` 로 등록하고, 엔진 활성 시 그 그룹을 활성 그룹으로 만든다. 활성 그룹이 곧 mutter 가 `xkb_state_key_get_one_sym()` 로 keysym 을 뽑는 키맵이다(05 문서의 inputMethod.js 경로: `process_key_event_async(event.get_key_symbol(), ...)`). 그러므로 엔진 `<layout>` 은 **변환 레이아웃을 결정**한다.
- "Wayland 에서 per-engine layout 무시"라는 통설(ibus#2408 등)은 **ibus-ui-gtk3 가 직접 `setxkbmap` 하는 X11 식 경로가 Wayland 에서 no-op** 이라는 뜻이지, **gnome-shell 이 엔진 layout 을 안 쓴다는 뜻이 아니다.** GNOME 은 ibus 의 setxkbmap 대신 자기 `setUserLayouts` 로 동일 효과를 낸다. (이것이 05 문서가 혼동한 지점.)

---

## 2. 엔진이 `<layout>us</layout><layout_variant>dvorak</layout_variant>` 를 선언하면?

**INFER (확신 매우 높음, §1.2/1.3 의 직접 귀결)** `_getXkbId()` 가 `'us+dvorak'` 를 만들고, `setUserLayouts(['...','us+dvorak','...'])` 로 등록되며, `get_layout_info('us+dvorak')` 은 found(유효한 xkb 레이아웃) → presguel 활성 시 활성 XKB 그룹 = `us(dvorak)`. → **물리 QWERTY-R 키가 keysym `p`(0x70)로 들어온다.** Ctrl/Alt+키 단축키도 dvorak 위치.

**FACT (실증 사례)** `hangul.xml` 은 `<layout>kr</layout><layout_variant>kr104</layout_variant>` 를 선언한다(파일 직접 확인). `_getXkbId()` 가 이를 `'kr+kr104'` 로 만들어 등록한다. 즉 GNOME 은 엔진 desc 의 layout_variant 를 실제 활성 XKB 결정에 쓴다 — variant 가 무시된다면 ibus-hangul 의 kr104 지정이 의미 없을 것.
또한 `simple.xml` 의 xkb 엔진들(예: `xkb:us:dvorak:eng`)도 `<layout>us</layout><layout_variant>dvorak</layout_variant>` 형태(05 문서 §B 에 캡처).

⇒ **"Wayland 에서 layout_variant 가 keysym 변환에 반영되나?" 답: 예.** 단, 옵션 (a) 를 정적으로 박으면 dvorak 하드코딩 → §6 에서 런타임 설정으로 해결.

---

## 3. ibus 엔진이 "현재 활성 XKB 레이아웃을 따라가게" 하는 GNOME 설정이 있나? — **없음**

- **FACT** 각 입력 소스의 `xkbId` 는 그 소스 자체(엔진이면 엔진 layout)로 고정된다(§1.2). `_updateMruSources` 가 모든 소스의 `xkbId` 를 등록하지만, **활성 그룹은 "현재 선택된 소스"의 xkbId** 다.
- **INFER (강)** 따라서 `sources` 에서 `us+dvorak` 을 presguel 앞에 둬도, presguel 로 전환하는 순간 활성 그룹은 presguel 의 `xkbId`(=us). **앞 소스의 dvorak 을 상속하지 않는다.**
- **FACT** GNOME 입력 소스 모델은 "하나의 소스 = xkb 레이아웃 **또는** ibus 엔진". `org.gnome.desktop.input-sources` 스키마에 "ibus 엔진 + 임의 xkb 레이아웃을 한 소스로 결합"하는 키는 없다. 결합은 **오직 엔진 desc 의 layout 필드**로만.

---

## 4. 다른 IME 들은 어떻게 하나 (typing-booster / m17n)

- **ibus-m17n**(`m17n.xml`): `<engines exec=".../ibus-engine-m17n --xml" />` 로 엔진 목록을 동적 생성. 각 m17n 엔진의 layout 은 그 엔진 정의에서 온다 → 역시 **엔진 desc 의 layout 경로**.
- **ibus-typing-booster**(`typing-booster.xml`, `--xml` 동적 엔진 + 자체 GSettings 스키마 `org.freedesktop.ibus.engine.typing-booster` 설치 확인): GNOME 입력 소스의 XKB 레이아웃 위에서 동작하고, 키보드/사전 관련 옵션을 **자체 설정으로 노출**해 setup 에서 사용자가 고른다. = **옵션 (d) 패턴의 선례.**
  - **INFER** dvorak 사용자에게 권하는 실용 경로는 "GNOME 입력 소스를 dvorak 베이스로 두고 typing-booster 가 그 위에서 동작" 또는 "setup 에서 레이아웃 지정". 핵심 교훈: **레이아웃 선택을 엔진 설정으로 노출**.

시사점: presguel 도 사용자가 베이스 레이아웃을 고르는 설정을 두고, 그 값을 **엔진 desc 의 `<layout>`/`<layout_variant>` 로 반영**하면 일반성을 지키며 dvorak 을 얻는다.

---

## 5. dconf/gsettings 각도

**FACT** 관련 스키마: `org.gnome.desktop.input-sources`. 본 환경 측정값:
- `sources = [('ibus','hangul'), ('xkb','us+dvorak'), ('ibus','presguel')]`
- `xkb-options = @as []`
- `mru-sources`/`per-window` 는 런타임 상태.
- `localectl`: X11 keymap 미설정, `/etc/X11/xorg.conf.d/00-keyboard.conf` 부재(=시스템 기본).

옵션 b/c 직접 판정:
- **(b)** `sources` 에서 dvorak 을 presguel 앞에 배치 → §3 대로 presguel 활성 시 us 로 덮임. **동작 안 함.**
- **(c)** `localectl set-x11-keymap dvorak` / 시스템 기본 XKB → "xkb 타입 소스가 없을 때의 시드/로그인 화면" 에만 영향. ibus 엔진 활성 시 그룹은 엔진 layout(=us)이 덮으므로 **ibus 엔진엔 영향 없음.** **동작 안 함.**
- Wayland 정답 메커니즘 = **엔진 desc 의 layout/layout_variant**(X11 시절의 "ibus engine + setxkbmap" 대체). dconf 가 아니라 컴포넌트 XML 에 있다.

---

## 6. 권장 레시피 (dvorak 사용자, 일반 IME 유지) — 옵션 (a)+(d)

핵심: **엔진의 `<layout>`/`<layout_variant>` 가 변환 레이아웃을 결정한다**(§1,§2)는 *사실*을 이용하되, dvorak 을 코드에 박지 말고 **사용자 설정으로 그 두 필드를 채워 XML 을 (재)생성**한다. presguel 은 이미 `install.sh` 가 `~/.config/ibus/component/presguel.xml` 를 생성하므로(파일 헤더 주석 확인) 거기에 끼워넣으면 된다.

### 6.1 XML (dvorak 사용자 예)
```xml
<engine>
  <name>presguel</name>
  ...
  <layout>us</layout>
  <layout_variant>dvorak</layout_variant>
  ...
</engine>
```
- 일반 us 사용자: `<layout>us</layout>` + variant 줄 생략(또는 빈 값).
- colemak: `<layout>us</layout><layout_variant>colemak</layout_variant>`.
- **`<layout>default</layout>` 은 쓰지 말 것** → us 로 귀결(§1.4), dvorak 안 됨. (05 문서의 "default 권장"은 폐기.)

### 6.2 사용자 레이아웃 자동 추론 (선택, 하드코딩 회피)
install/setup 시 GNOME 입력 소스에서 첫 `xkb` 엔트리의 variant 를 읽어 기본값으로 채운다:
```bash
# sources 에서 첫 ('xkb','LAYOUT[+VARIANT]') 를 파싱
gsettings get org.gnome.desktop.input-sources sources
#  예: ('xkb','us+dvorak')  → layout=us, variant=dvorak
#      ('xkb','us')         → layout=us, variant=(빈값)
```
이 값을 컴포넌트 XML 의 `<layout>`/`<layout_variant>` 에 기록한다 → dvorak 이 코드 어디에도 박히지 않음.

### 6.3 적용 시퀀스 (이 repo install.sh 방식에 맞춤)
```bash
# 1) 사용자 layout/variant 를 ~/.config/ibus/component/presguel.xml 의
#    <layout>/<layout_variant> 에 기록 (위 6.2 로 추론 또는 setup 에서 선택)

# 2) ibus 레지스트리 캐시 갱신
ibus write-cache --system 2>/dev/null || true

# 3) ibus 재시작 (실패 시 데몬 replace 재시작 - 기존 install.sh 패턴)
ibus restart 2>/dev/null || { pkill -f ibus-daemon; ibus-daemon -drxR & }

# 4) (Wayland) gnome-shell 이 새 engine desc 의 layout 을 다시 읽도록
#    재로그인 권장. ibus restart 만으로 xkbId 가 안 바뀌면 로그아웃/로그인.
```

### 6.4 검증
```bash
# presguel 활성 상태에서 물리 QWERTY-R 을 눌렀을 때 ProcessKeyEvent keyval:
#   성공(dvorak): 0x70 ('p')
#   실패(us):     0x72 ('r')
# presguel ProcessKeyEvent 진입부 keyval 로깅으로 확인 (05/06 의 PRESGUEL_DEBUG_KEYS 방식).
```

### 6.5 한 단계 더: 진짜 일반화 (옵션 d 완성)
presguel-setup(GTK 설정창)에 "베이스 키보드 레이아웃" 드롭다운(us / us+dvorak / us+colemak / ...)을 추가하고, 저장 시 컴포넌트 XML 의 layout/variant 를 다시 써서 ibus 재시작 + 재로그인. dvorak 하드코딩이 전혀 없음.

> 06 문서의 설계(한글 글자는 keycode 기반 KeyTable 로 물리위치 고정, 단축키는 통과)와 충돌하지 않는다. 06 은 "엔진은 레이아웃을 강제하지 말고 단축키는 XKB 가 처리"라 했는데, 본 문서는 그 "XKB" 를 dvorak 으로 만들려면 **엔진 layout 을 dvorak 으로 줘야 한다**는 점을 추가한다(그게 유일한 동작 경로). 한글 자모는 여전히 keycode 기반으로 고정하면 베이스 레이아웃과 무관하게 안정적이다.

---

## 7. 옵션 평가 요약표

| 옵션 | 내용 | GNOME49/Wayland 동작? | 일반 IME 유지? | 판정 |
|---|---|---|---|---|
| (a) | 엔진에 `<layout>us</layout><layout_variant>dvorak</layout_variant>` 정적 선언 | ✅ dvorak keysym (INFER 강, §2) | ❌ dvorak 하드코딩 | 동작하나 그대로는 부적합 |
| (b) | 엔진 `default`, `sources` 에 dvorak 을 앞 배치 | ❌ presguel 활성 시 us 로 덮임 | ✅ | **안 됨** |
| (c) | `localectl set-x11-keymap dvorak` 등 시스템 기본 | ❌ ibus 엔진 그룹에 영향 없음 | ✅ | **안 됨** |
| (d) | 엔진이 런타임 설정으로 layout 결정 → XML 에 반영 | ✅ (= a 와 동일 경로) | ✅ | **권장** |
| **(a)+(d)** | install/setup 가 사용자 레이아웃을 XML layout/variant 로 기록 | ✅ | ✅ | **최종 권장 (§6)** |

---

## 8. 검증 한계 / 신뢰도 노트

- **FACT (소스 직접 인용)**: `js/ui/status/keyboard.js` 의 `InputSource._getXkbId()`(=`engineDesc.layout`/`engineDesc.variant` → `xkbId`)와 `_updateMruSources()` 의 `this._keyboardManager.setUserLayouts(sourcesList.map(x => x.xkbId))`; `js/misc/keyboardManager.js` 의 `DEFAULT_LAYOUT='us'`/`DEFAULT_VARIANT=''` 와 `setUserLayouts(ids)`(각 id 를 `get_layout_info` 로 조회, not-found 면 skip, `'default'` 특별 분기 없음). gnome-49 브랜치에서 3회 독립 인용으로 일치.
- **FACT (로컬 파일)**: presguel.xml=`<layout>us</layout>`, hangul.xml=`kr`/`kr104`, m17n/typing-booster 동적엔진+자체 스키마, gsettings 현재값, localectl 시스템 기본.
- **FACT (문서)**: IBusEngineDesc `layout` 기본값 `'us'`, `layout-variant` 기본값 `''` (lazka pgi-docs). GNOME 설계: "input source = (xkb layout, ibus engine) tuple". ibus#1614: `default` = 레이아웃 강제 안 함 센티넬.
- **INFER (확신 높음)**: (1) 엔진 `<layout>us</layout><layout_variant>dvorak</layout_variant>` → 활성 그룹 us(dvorak) → dvorak keysym (a/d 의 핵심, §2). 근거: `_getXkbId`/`setUserLayouts` 인용 + hangul kr104 가 실제 동작한다는 사실 + 사용자 실측(us→us). (2) `default`/누락 → 사실상 us (§1.4). 근거: not-found skip + `DEFAULT_LAYOUT='us'` 폴백 + ibus#1614.
- **`apply(id)` 도 이제 FACT 로 인용 확인**: `let info = this._layoutInfos[id]; if (!info) return;` → 미등록 id(=default 처럼 skip 된 경우)는 **return early, 키맵 변경 없음**. `_applyLayoutGroup` 가 `global.backend.set_keymap_async(layouts, variants, options, model, null)` 로 mutter 키맵을 실제 설정. `activateInputSource` 가 xkb/ibus 양쪽 모두 `this._keyboardManager.apply(is.xkbId)` 호출. (gnome-49 소스, 다회 독립 인용 일치.) 따라서 §1.4 의 종착점은 "default→apply no-op→직전 그룹 유지(보통 us)"로 FACT 화. 단 dvorak 이 안 나온다는 결론은 불변. 권장 레시피(옵션 a+d)는 `'us+dvorak'`(항상 found)에 의존하므로 확실히 동작.
- **05 문서 정정 요지**: 05 의 "엔진 `<layout>` Wayland 무시 / `default` 로 상속" [STRONG] 은 **이 코드 경로에선 오류**. 정정: 엔진 layout/variant 가 활성 XKB(=keysym 변환)를 결정하며, dvorak 을 원하면 엔진에 `us+dvorak` 를 지정해야 한다(런타임 설정으로). 05 의 다른 결론(keyval = 활성 레이아웃 keysym, keycode = 물리위치, ForwardKeyEvent remap 불가, 한글은 keycode 기반)은 유효.

### 근거 링크
- gnome-shell `js/ui/status/keyboard.js` (InputSource `_getXkbId`: engineDesc.layout/variant → xkbId; `_updateMruSources` → `setUserLayouts`): https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/gnome-49/js/ui/status/keyboard.js
- gnome-shell `js/misc/keyboardManager.js` (`DEFAULT_LAYOUT='us'`, `setUserLayouts` → `get_layout_info`, default 특별분기 없음): https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/gnome-49/js/misc/keyboardManager.js
- IBusEngineDesc layout/layout-variant 프로퍼티(기본값 'us'/''): https://lazka.github.io/pgi-docs/IBus-1.0/classes/EngineDesc.html
- GNOME IBus 통합 설계 ("input source = tuple of XKB layout and IBus engine"): https://wiki.gnome.org/ThreePointFive/Features/IBus
- ibus#1614 ('default' XKB layout 센티넬 의미): https://github.com/ibus/ibus/issues/1614
- per-engine setxkbmap 이 Wayland 에서 no-op (gnome-shell 의 setUserLayouts 경로와 별개): https://github.com/ibus/ibus/issues/2408
- Wayland 에 범용 setxkbmap 경로 없음, 데스크톱별 구현: https://fcitx-im.org/wiki/Using_Fcitx_5_on_Wayland
- ArchWiki IBus (ibus 가 xkb 설정을 관리): https://wiki.archlinux.org/title/IBus
- 로컬 비교 파일: `/usr/share/ibus/component/hangul.xml`(layout=kr, layout_variant=kr104), `m17n.xml`, `/usr/share/glib-2.0/schemas/org.freedesktop.ibus.engine.typing-booster.gschema.xml`

---

## 9. 최종 채택: 레이아웃별 다중 엔진 등록 (옵션 a+d 대체)

§6 의 "설정창에서 레이아웃 골라 단일 엔진 XML 재생성(pkexec)" 대신, **컴포넌트에 레이아웃별
엔진을 미리 여러 개 등록**하는 방식으로 변경(사용자 요청). 이유: GNOME 입력 소스 모델이
"엔진 = (xkb 레이아웃, ibus 엔진) 튜플"(§1.1)이므로, 레이아웃마다 엔진을 두면 사용자가
**GNOME 입력 소스에서 직접 고르기만 하면** 되고 pkexec·XML 재생성·재로그인 요구가 사라진다.

- 엔진 이름: 기본 `presguel`(layout=us), 변형 `presguel:us:<variant>`(예 `presguel:us:dvorak`).
  Factory 는 `presguel` 또는 `presguel:` 접두를 모두 수락(`create_engine`). 조합 로직은 동일,
  영문 배열 차이는 GNOME 이 각 엔진의 `<layout_variant>` 로 처리(§1,§2).
- 등록 목록(이 시스템 `GnomeDesktop.XkbInfo` 의 us 계열 영문 대체배열 전부, 17개):
  us(QWERTY) + dvorak/dvorak-intl/dvorak-alt-intl/dvorak-classic/dvorak-l/dvorak-r/dvorak-mac/dvp
  + colemak/colemak_dh/colemak_dh_iso/colemak_dh_ortho/colemak_dh_wide + workman/workman-intl + norman.
- `install.sh` 가 이 목록으로 `<engine>` 블록들을 생성. longname `Presguel`/`Presguel (Dvorak)` 등.
- 폐기: `base_layout`/`base_variant`(config.ini), `scripts/presguel-apply-layout`(pkexec 헬퍼),
  설정창의 "단축키 키보드 배열" 드롭다운/적용 버튼. (§6 레시피는 이 방식으로 대체됨.)
- 실기 검증: 17개 엔진 등록 확인, 데몬이 각 엔진의 layout/variant 를 올바로 보고
  (`presguel:us:colemak`→variant=colemak 등; 데몬 완전 재시작 후 정확).

> ⚠️ **정정(§10):** 아래 §10 에서 실증한 결과, `<layout_variant>` 방식은 gnome-shell 49 버그로
> **실제로 동작하지 않았다.** variant 를 `<layout>us+dvorak</layout>` 처럼 layout 필드에 합쳐야 한다.
> 위 "데몬이 variant 를 올바로 보고" 는 사실이나(데몬 레벨은 정상), gnome-shell 이 그 variant 를
> 안 읽는 게 문제였다. §2 의 "INFER (확신 매우 높음)" 도 이 지점에서 틀렸다.

---

## 10. 실증: `<layout_variant>` 는 gnome-shell 49 에서 무시된다 (GObject 인트로스펙션으로 확정)

**증상(사용자 보고, 2026-05-31):** 컴포넌트 XML 17엔진·캐시·데몬 desc 가 전부 정확한데도
(재로그인까지 했는데) `Presguel (Dvorak)` 활성 시 물리 QWERTY-R 이 여전히 `r`. dvorak 안 먹음.

### 10.1 근본 원인 — **FACT (인트로스펙션 직접 확인)**
gnome-shell `js/ui/status/keyboard.js` 의 `_getXkbId()`(§1.2 인용)는 `engineDesc.variant` 를 읽는다:
```js
if (engineDesc.variant && engineDesc.variant.length > 0)
    return `${engineDesc.layout}+${engineDesc.variant}`;
else
    return engineDesc.layout;
```
그런데 **`IBus.EngineDesc` 에는 `variant` 라는 GObject 속성이 없다.** 실제 속성은 `layout-variant`다.
`python3 -c "GObject.list_properties(IBus.EngineDesc)"` 결과(직접 실행):
```
[... 'layout', 'layout-option', 'layout-variant', ...]   # 'variant' 없음
has 'variant' prop?        False
has 'layout-variant' prop? True
```
gjs 로 `engineDesc.variant` 접근 시 **`undefined`** 반환(예외 아님, JS 라 조용히 undefined):
```
presguel:us:dvorak:  .variant = undefined,  .layout_variant = "dvorak",  .layout = "us"
>>> _getXkbId() = "us"           # variant=undefined → else 분기 → layout 만!
```
⇒ `<layout_variant>dvorak</layout_variant>` 는 `_getXkbId` 에서 **항상 무시**되고 `xkbId="us"`(QWERTY)로
귀결. mutter 가 us 키맵을 깐다. 이게 증상의 정확한 원인. (hangul 의 `kr104` 도 같은 이유로 무시되나
kr 과 거의 같아 아무도 못 느낀 잠복 버그. 이 사실이 §2 의 "hangul kr104 가 동작하므로 variant 가
반영된다"는 *방증을 무효화*한다 — kr104 는 사실 반영된 적이 없다.)

### 10.2 해결 — variant 를 `<layout>` 에 합치기. **FACT (데몬+gjs 로 실증)**
`<layout>us+dvorak</layout>` 로 두고 `<layout_variant>` 는 생략한다. 그러면:
- 데몬 desc: `get_layout()="us+dvorak"`, `get_layout_variant()=""`.
- `_getXkbId()`: variant 빈 값 → `else` → `engineDesc.layout` = `"us+dvorak"` 그대로 반환.
- `GnomeDesktop.XkbInfo.get_layout_info("us+dvorak")` → **found=true**, layout=us, variant=dvorak
  (gjs 직접 확인. `us+colemak`, `us+workman` 등 16개 변형 전부 found). ⇒ `setUserLayouts` 에 등록 →
  `apply` → `set_keymap_async("us","dvorak")`. 사용자의 잘 되던 `('xkb','us+dvorak')` 소스와 **동일 xkbId**.
- 검증 명령: `python3 /tmp/presguel_dump_engines.py`(데몬 desc), `gjs /tmp/probe_enginedesc.js`(_getXkbId 재현).
  수정 후: `presguel:us:dvorak | layout='us+dvorak'`, `_getXkbId()="us+dvorak"`. ✅

### 10.3 적용
`install.sh` 의 엔진 XML 생성에서 `xkb_layout="${layout}+${variant}"`(variant 있을 때)로 `<layout>` 에
합쳐 쓰고 `<layout_variant>` 줄을 제거. 재설치 후 데몬/gjs 로 위 값 확인. **재로그인 1회 필요**
(gnome-shell 이 `InputSource.xkbId` 를 생성 시 1회 캐시 — §1.2). 그 뒤 물리 QWERTY-R → dvorak `p`.

### 10.4 신뢰도
- **FACT**: `variant` 속성 부재(`list_properties`), `_getXkbId` 가 `us` 반환(gjs), `us+dvorak` 합본이
  `get_layout_info` found(gjs), 수정 후 데몬 desc=`us+dvorak`·`_getXkbId="us+dvorak"`(실행 확인).
- **남은 1점(사용자 확인 대기)**: 재로그인 후 실제 타이핑에서 물리 R→`p`. 위 체인이 사용자의 dvorak
  xkb 소스와 동일 경로·동일 xkbId 로 수렴하므로 동작 확신 매우 높음.
