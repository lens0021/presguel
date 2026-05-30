#!/usr/bin/env bash
# presguel-ibus 설치 스크립트.
#
#   - 릴리스 바이너리를 빌드해 /usr/local/bin 에 설치(sudo).
#   - 날개셋 설정을 ~/.config/presguel/nalgaeset.xml 에 둔다(사용자).
#   - ibus 컴포넌트를 /usr/share/ibus/component/presguel.xml 로 생성(sudo).
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
component_dst="/usr/share/ibus/component/presguel.xml"

echo "[1/5] 릴리스 빌드"
cargo build --release -p presguel-ibus
bin_src="$repo_root/target/release/presguel-ibus"

echo "[2/5] 바이너리·설정창 설치 → $bin_dst, $setup_dst (sudo)"
sudo install -Dm755 "$bin_src" "$bin_dst"
sudo install -Dm755 "$setup_src" "$setup_dst"

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

echo "[4/5] ibus 컴포넌트 생성 → $component_dst (sudo)"
sudo tee "$component_dst" > /dev/null <<XML
<?xml version="1.0" encoding="utf-8"?>
<component>
  <name>org.freedesktop.IBus.Presguel</name>
  <description>Presguel Korean Input Method (날개셋 호환)</description>
  <exec>$bin_dst --ibus</exec>
  <version>0.1.0</version>
  <author>lens0021 &lt;lorentz0021@gmail.com&gt;</author>
  <license>LicenseRef-AllRightsReserved</license>
  <homepage>https://github.com/lens0021/presguel</homepage>
  <textdomain>presguel</textdomain>
  <engines>
    <engine>
      <name>presguel</name>
      <language>ko</language>
      <icon>ibus-hangul</icon>
      <layout>us</layout>
      <longname>Presguel (날개셋 세벌식)</longname>
      <description>날개셋 설정 호환 한글 입력기</description>
      <rank>50</rank>
      <symbol>가</symbol>
      <setup>$setup_dst</setup>
    </engine>
  </engines>
</component>
XML

echo "[5/5] 레지스트리 캐시 갱신 + ibus 재시작"
sudo ibus write-cache --system || true
ibus restart 2>/dev/null || true
sleep 2
if ibus list-engine 2>/dev/null | grep -qi presguel; then
  echo "      OK: 'presguel' 엔진 등록됨."
  echo "      입력 소스에 추가: GNOME 설정 → 키보드 → 입력 소스, 또는"
  echo "      gsettings 로 추가 후 Super+Space 로 전환."
else
  echo "      경고: list-engine 에 아직 안 보임. 로그아웃/로그인 후 재시도." >&2
fi
