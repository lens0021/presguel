#!/usr/bin/env python3
"""presguel 설정창 (GTK4 + libadwaita).

GNOME 최신(49) 정석 스타일: AdwApplicationWindow + AdwHeaderBar +
AdwPreferencesGroup + AdwSwitchRow + AdwComboRow.

다루는 설정:
  - 간단 모드 on/off (기본 off = 모든 InputEntry 를 읽어 날개셋과 동일 동작)
  - 간단 모드 on 일 때: 한글 InputEntry / 영문 배치 InputEntry 를 드롭다운으로 지정

설정은 ~/.config/presguel/config.ini (key=value) 에 저장한다(엔진과 같은 형식).
드롭다운 항목은 ~/.config/presguel/nalgaeset.xml 의 InputEntry 들에서 읽는다.
"""
import os
import sys
import xml.etree.ElementTree as ET

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Gtk, Adw, Gio


def config_dir():
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return os.path.join(base, "presguel")


def ini_path():
    return os.environ.get("PRESGUEL_CONFIG_INI") or os.path.join(config_dir(), "config.ini")


def xml_path():
    return os.environ.get("PRESGUEL_CONFIG") or os.path.join(config_dir(), "nalgaeset.xml")


def load_ini():
    """key=value 설정을 dict 로. 없으면 기본값."""
    cfg = {"simple_mode": "false", "hangul_entry": "0", "latin_entry": "1"}
    try:
        with open(ini_path(), encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                k, v = line.split("=", 1)
                cfg[k.strip()] = v.strip()
    except FileNotFoundError:
        pass
    return cfg


def save_ini(simple, hangul_idx, latin_idx):
    os.makedirs(config_dir(), exist_ok=True)
    body = (
        "# presguel 설정 (presguel-setup 가 생성). key=value 형식.\n"
        "# simple_mode: 켜면 아래 두 항목만 써서 단순 동작. 끄면 모든 InputEntry(날개셋 동일).\n"
        f"simple_mode = {'true' if simple else 'false'}\n"
        "# 간단 모드에서 쓸 한글 InputEntry 인덱스.\n"
        f"hangul_entry = {hangul_idx}\n"
        "# 간단 모드에서 한/영 전환 시 쓸 영문 InputEntry 인덱스.\n"
        f"latin_entry = {latin_idx}\n"
    )
    with open(ini_path(), "w", encoding="utf-8") as f:
        f.write(body)


def load_entries():
    """nalgaeset.xml 에서 (인덱스, 표시이름) 목록을 읽는다."""
    out = []
    try:
        root = ET.parse(xml_path()).getroot()
    except (FileNotFoundError, ET.ParseError):
        return out
    layer = root.find("InputLayer")
    if layer is None:
        return out
    for i, entry in enumerate(layer.findall("InputEntry")):
        name = None
        kt = entry.find(".//KeyTable")
        if kt is not None:
            name = kt.get("name")
        if not name:
            scheme = entry.find("InputSchemeSetting")
            obj = scheme.get("object") if scheme is not None else None
            if obj == "CInputScheme":
                name = "(직접 입력 / 영문 패스스루)"
            else:
                name = obj or "(이름 없음)"
        out.append((i, f"{i}: {name}"))
    return out


def _to_bool(s):
    return str(s).lower() in ("true", "1", "yes", "on")


def _to_int(s, default=0):
    try:
        return int(s)
    except (TypeError, ValueError):
        return default


class SetupWindow(Adw.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="Presguel 설정")
        self.set_default_size(460, -1)

        cfg = load_ini()
        self.entries = load_entries()
        labels = [lbl for _, lbl in self.entries] or ["(nalgaeset.xml 을 찾을 수 없음)"]
        # 초기값 세팅 중에는 notify 핸들러가 저장하지 않도록 막는다(불필요한 쓰기 방지).
        self._loading = True

        # 헤더바 + 본문을 담는 ToolbarView (Adw 표준 레이아웃). 즉시 적용이라 저장 버튼 없음.
        toolbar = Adw.ToolbarView()
        toolbar.add_top_bar(Adw.HeaderBar())

        page = Adw.PreferencesPage()
        toolbar.set_content(page)
        self.set_content(toolbar)

        group = Adw.PreferencesGroup(
            title="입력 동작",
            description="끄면 설정의 모든 입력 항목을 읽어 날개셋과 똑같이 동작합니다. "
            "켜면 아래에서 고른 한글 항목과 영문 항목만 한/영 전환에 사용합니다.",
        )
        page.add(group)

        # 간단 모드 스위치 행.
        self.simple_row = Adw.SwitchRow(
            title="간단 모드",
            subtitle="한글 / 영문 배치 항목을 직접 지정",
        )
        self.simple_row.set_active(_to_bool(cfg.get("simple_mode", "false")))
        self.simple_row.connect("notify::active", self.on_change)
        group.add(self.simple_row)

        # 한글 항목 콤보.
        self.hangul_row = Adw.ComboRow(
            title="한글 입력 항목",
            subtitle="실제로 쓸 한글 자판",
            model=Gtk.StringList.new(labels),
        )
        self._set_combo(self.hangul_row, _to_int(cfg.get("hangul_entry", "0")))
        self.hangul_row.connect("notify::selected", self.on_change)
        group.add(self.hangul_row)

        # 영문 항목 콤보(한/영 전환 시 영문으로 쓸 항목).
        self.latin_row = Adw.ComboRow(
            title="영문 입력 항목",
            subtitle="한/영 전환 시 쓸 영문 항목",
            model=Gtk.StringList.new(labels),
        )
        self._set_combo(self.latin_row, _to_int(cfg.get("latin_entry", "1")))
        self.latin_row.connect("notify::selected", self.on_change)
        group.add(self.latin_row)

        # 키보드 배열 안내(단축키·영문은 GNOME 입력 소스에서 배열별 엔진을 골라 정한다).
        kbd_group = Adw.PreferencesGroup(
            title="키보드 배열",
            description="한글 자판은 물리 위치로 고정됩니다. 단축키(Ctrl/Alt+키)와 영문 배열은 "
            "GNOME 설정 → 키보드 → 입력 소스에서 'Presguel (Dvorak)' 처럼 원하는 배열의 항목을 "
            "골라 정하세요.",
        )
        page.add(kbd_group)

        # 안내 행.
        note = Adw.PreferencesGroup()
        lbl = Gtk.Label(
            label="입력 동작 설정은 즉시 적용됩니다(입력 중이었다면 입력창을 다시 누르면 반영).",
            xalign=0,
            wrap=True,
        )
        lbl.add_css_class("dim-label")
        note.add(lbl)
        page.add(note)

        self._sync_sensitivity()
        self._loading = False

    def _set_combo(self, row, idx):
        n = max(1, len(self.entries))
        row.set_selected(idx if 0 <= idx < n else 0)
        if not self.entries:
            row.set_sensitive(False)

    def _sync_sensitivity(self):
        on = self.simple_row.get_active() and bool(self.entries)
        self.hangul_row.set_sensitive(on)
        self.latin_row.set_sensitive(on)

    def on_change(self, *_):
        """위젯이 바뀔 때마다 즉시 config.ini 저장(GNOME instant-apply)."""
        self._sync_sensitivity()
        if self._loading:
            return
        simple = self.simple_row.get_active()
        h = self.hangul_row.get_selected() if self.entries else 0
        l = self.latin_row.get_selected() if self.entries else 1
        save_ini(simple, h, l)


class SetupApp(Adw.Application):
    def __init__(self):
        super().__init__(application_id="org.freedesktop.IBus.Presguel.Setup",
                         flags=Gio.ApplicationFlags.FLAGS_NONE)

    def do_activate(self):
        win = self.props.active_window
        if not win:
            win = SetupWindow(self)
        win.present()


def main():
    return SetupApp().run(sys.argv)


if __name__ == "__main__":
    sys.exit(main())
