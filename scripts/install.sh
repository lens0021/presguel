#!/usr/bin/env bash
# presguel-ibus 설치 스크립트.
#
#   - 릴리스 바이너리를 빌드해 /usr/local/bin 에 설치(sudo).
#   - 날개셋 설정을 ~/.config/presguel/nalgaeset.xml 에 둔다(사용자).
#   - ibus 컴포넌트를 /usr/share/ibus/component/presguel.xml 로 생성(sudo).
#     레이아웃별 엔진(QWERTY/Dvorak/Colemak/...)을 함께 등록한다.
#   - 시스템 레지스트리 캐시를 갱신하고 ibus 를 재시작.
#
# 참고: 이 환경의 ibus 는 사용자 데이터 디렉터리(~/.local/share/ibus/component)를
# 스캔하지 않고 /usr/share/ibus/component 만 스캔하므로 시스템 설치가 필요하다.
#
# 사용법:
#   scripts/install.sh [path/to/nalgaeset.xml]
# 인자가 없으면 기존 ~/.config/presguel/nalgaeset.xml 를 사용한다.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

config_dir="${XDG_CONFIG_HOME:-$HOME/.config}"
presguel_cfg_dir="$config_dir/presguel"
cfg_dst="$presguel_cfg_dir/nalgaeset.xml"
bin_dst="/usr/local/bin/presguel-ibus"
setup_dst="/usr/local/bin/presguel-setup"
setup_src="$repo_root/crates/presguel-ibus/data/presguel-setup.py"
desktop_dst="/usr/share/applications/ibus-setup-presguel.desktop"
desktop_src="$repo_root/crates/presguel-ibus/data/ibus-setup-presguel.desktop"
component_dst="/usr/share/ibus/component/presguel.xml"

# 레이아웃별 엔진 목록: "엔진이름접미|layout|variant|표시이름".
# 접미가 비면 기본 엔진 'presguel'(QWERTY). 그 외엔 'presguel:us:VARIANT'.
# 단축키·영문 keysym 은 GNOME 이 각 엔진의 <layout>/<layout_variant> 로 처리한다(research/07).
# 한글 자판 자체는 keycode(물리 위치) 기준이라 어느 배열에서도 동일하게 동작한다.
engines=(
  "|us||QWERTY"
  ":us:dvorak|us|dvorak|Dvorak"
  ":us:dvorak-intl|us|dvorak-intl|Dvorak, intl."
  ":us:dvorak-alt-intl|us|dvorak-alt-intl|Dvorak, alt. intl."
  ":us:dvorak-classic|us|dvorak-classic|Dvorak, classic"
  ":us:dvorak-l|us|dvorak-l|Dvorak, left-handed"
  ":us:dvorak-r|us|dvorak-r|Dvorak, right-handed"
  ":us:dvorak-mac|us|dvorak-mac|Dvorak, Mac"
  ":us:dvp|us|dvp|Programmer Dvorak"
  ":us:colemak|us|colemak|Colemak"
  ":us:colemak_dh|us|colemak_dh|Colemak-DH"
  ":us:colemak_dh_iso|us|colemak_dh_iso|Colemak-DH ISO"
  ":us:colemak_dh_ortho|us|colemak_dh_ortho|Colemak-DH Ortho"
  ":us:colemak_dh_wide|us|colemak_dh_wide|Colemak-DH Wide"
  ":us:workman|us|workman|Workman"
  ":us:workman-intl|us|workman-intl|Workman, intl."
  ":us:norman|us|norman|Norman"
)

echo "[1/5] 릴리스 빌드"
cargo build --release -p presguel-ibus
bin_src="$repo_root/target/release/presguel-ibus"

echo "[2/5] 바이너리·설정창·데스크톱 설치 (sudo)"
# 최신 GNOME(control-center 49+)은 컴포넌트 <setup> 이 아니라
# /usr/share/applications/ibus-setup-<engine>.desktop 의 Exec 로 설정창을 띄운다.
# (케밥 메뉴 ⋮ → Preferences). 그래서 desktop 파일이 반드시 필요하다.
sudo install -Dm755 "$bin_src" "$bin_dst"
sudo install -Dm755 "$setup_src" "$setup_dst"
sudo install -Dm644 "$desktop_src" "$desktop_dst"
sudo update-desktop-database /usr/share/applications 2>/dev/null || true

echo "[3/5] 설정 배치 → $cfg_dst"
mkdir -p "$presguel_cfg_dir"
if [[ "${1:-}" != "" ]]; then
  cp "$1" "$cfg_dst"
  echo "      $1 -> $cfg_dst"
elif [[ -f "$cfg_dst" ]]; then
  echo "      기존 설정 사용"
else
  echo "      오류: nalgaeset.xml 경로를 인자로 주거나 $cfg_dst 에 미리 두세요." >&2
  exit 1
fi

echo "[4/5] ibus 컴포넌트 생성(레이아웃별 엔진 ${#engines[@]}개) → $component_dst (sudo)"
# 엔진 블록들을 만든다.
engine_xml=""
for spec in "${engines[@]}"; do
  IFS='|' read -r suffix layout variant disp <<< "$spec"
  name="presguel${suffix}"
  # 기본 엔진만 longname 'Presguel', 나머지는 'Presguel (배열)'.
  if [[ -z "$suffix" ]]; then
    longname="Presguel"
  else
    longname="Presguel (${disp})"
  fi
  # gnome-shell(49) status/keyboard.js 의 _getXkbId 는 EngineDesc 의 `layout-variant`
  # 가 아니라 *존재하지 않는* `variant` 속성을 읽는다(undefined → else 분기 → layout 만).
  # 즉 <layout_variant> 는 무시되고 QWERTY 로 떨어진다(검증: GObject 인트로스펙션,
  # research/07 §10). 그래서 variant 를 <layout> 에 `us+dvorak` 형태로 합쳐 둔다.
  # _getXkbId 가 그 문자열을 그대로 쓰고 get_layout_info("us+dvorak") 가 found 다
  # (= 사용자의 ('xkb','us+dvorak') 소스와 동일한 xkbId). <layout_variant> 는 두지 않는다.
  xkb_layout="$layout"
  [[ -n "$variant" ]] && xkb_layout="${layout}+${variant}"
  engine_xml+="
    <engine>
      <name>${name}</name>
      <language>ko</language>
      <icon>ibus-hangul</icon>
      <layout>${xkb_layout}</layout>
      <longname>${longname}</longname>
      <description>Presguel 한글 입력기 (${disp})</description>
      <rank>50</rank>
      <symbol>글</symbol>
      <setup>${setup_dst}</setup>
    </engine>"
done

sudo tee "$component_dst" > /dev/null <<XML
<?xml version="1.0" encoding="utf-8"?>
<component>
  <name>org.freedesktop.IBus.Presguel</name>
  <description>Presguel Korean Input Method</description>
  <exec>$bin_dst --ibus</exec>
  <version>0.1.0</version>
  <author>lens0021 &lt;lorentz0021@gmail.com&gt;</author>
  <license>LicenseRef-AllRightsReserved</license>
  <homepage>https://github.com/lens0021/presguel</homepage>
  <textdomain>presguel</textdomain>
  <engines>$engine_xml
  </engines>
</component>
XML

echo "[5/5] 레지스트리 캐시 갱신 + ibus 재시작"
# 시스템 캐시(엔진 탐색)와 사용자 캐시(GNOME 패널이 읽음, <setup> 경로 포함) 둘 다 갱신.
sudo ibus write-cache --system 2>/dev/null || true
ibus write-cache 2>/dev/null || true

# 데몬을 실제로 재시작해야 새 엔진 목록이 반영된다. 이 환경(GNOME/Wayland)에선
# `ibus restart` 가 데몬을 교체하지 못하는 경우가 있어, PID 가 안 바뀌면 데몬의 실제
# 실행 인자를 그대로 -r(replace) 로 재실행한다.
dpid_before="$(pgrep -x ibus-daemon | head -1)"
ibus restart 2>/dev/null || true
sleep 2
dpid_after="$(pgrep -x ibus-daemon | head -1)"
if [[ -n "$dpid_before" && "$dpid_before" == "$dpid_after" ]]; then
  echo "      ibus restart 가 데몬을 교체하지 못함 → 실행 인자 그대로 replace 재시작"
  args="$(tr '\0' ' ' < "/proc/$dpid_before/cmdline" 2>/dev/null)"
  if [[ -n "$args" ]]; then
    setsid $args -r -d </dev/null >/dev/null 2>&1 || true
    sleep 2
  fi
fi
sleep 2
if ibus list-engine 2>/dev/null | grep -qi presguel; then
  echo "      OK: presguel 엔진들이 등록됨."
  echo "      입력 소스 추가: GNOME 설정 → 키보드 → 입력 소스 → 한국어에서"
  echo "      'Presguel' / 'Presguel (Dvorak)' / 'Presguel (Colemak)' 등 원하는 배열 선택."
  echo "      (자판 배열이 곧 단축키·영문 배열입니다. 한글 자판은 어느 것이든 동일.)"
else
  echo "      경고: list-engine 에 presguel 이 안 보임. 로그아웃/로그인 후 재시도." >&2
fi
