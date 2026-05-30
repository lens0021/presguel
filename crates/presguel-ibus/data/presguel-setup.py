#!/usr/bin/env python3
"""presguel 설정창 (GTK).

간단한 설정만 다룬다:
  - 간단 모드 on/off (기본 off = 모든 InputEntry 를 읽어 날개셋과 동일 동작)
  - 간단 모드 on 일 때: 한글 InputEntry / 영문 배치 InputEntry 를 드롭다운으로 지정

설정은 ~/.config/presguel/config.ini (key=value) 에 저장한다(엔진과 같은 형식).
드롭다운 항목은 ~/.config/presguel/nalgaeset.xml 의 InputEntry 들에서 읽는다.
"""
import os
import sys
import xml.etree.ElementTree as ET

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib


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
        "# 간단 모드에서 단축키(Alt+P 등)를 변환할 기준 영문 배치 InputEntry 인덱스.\n"
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
        # KeyTable name 이 있으면 그걸, 없으면 scheme/generator object 로 이름을 짓는다.
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


class SetupWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Presguel 설정")
        self.set_border_width(16)
        self.set_default_size(420, -1)

        cfg = load_ini()
        self.entries = load_entries()

        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        self.add(box)

        # 간단 모드 토글
        self.simple_check = Gtk.CheckButton(label="간단 모드 (한글/영문 항목 직접 지정)")
        self.simple_check.set_active(cfg.get("simple_mode", "false").lower() in ("true", "1", "yes", "on"))
        self.simple_check.connect("toggled", self.on_toggle)
        box.pack_start(self.simple_check, False, False, 0)

        desc = Gtk.Label(xalign=0)
        desc.set_line_wrap(True)
        desc.set_markup(
            "<small>끄면 설정의 모든 입력 항목을 읽어 <b>날개셋과 똑같이</b> 동작합니다.\n"
            "켜면 아래에서 고른 <b>한글 항목</b>과, 단축키(Alt+P 등)를 맞출 "
            "<b>영문 배치 항목</b>만 사용합니다.</small>"
        )
        box.pack_start(desc, False, False, 0)

        # 드롭다운 그리드
        grid = Gtk.Grid(column_spacing=10, row_spacing=8)
        box.pack_start(grid, False, False, 0)

        grid.attach(Gtk.Label(label="한글 입력 항목", xalign=0), 0, 0, 1, 1)
        self.hangul_combo = self._make_combo(self._idx(cfg, "hangul_entry"))
        grid.attach(self.hangul_combo, 1, 0, 1, 1)

        grid.attach(Gtk.Label(label="영문 배치 항목", xalign=0), 0, 1, 1, 1)
        self.latin_combo = self._make_combo(self._idx(cfg, "latin_entry"))
        grid.attach(self.latin_combo, 1, 1, 1, 1)

        # 버튼
        btns = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        btns.set_halign(Gtk.Align.END)
        cancel = Gtk.Button(label="취소")
        cancel.connect("clicked", lambda *_: self.close())
        save = Gtk.Button(label="저장")
        save.get_style_context().add_class("suggested-action")
        save.connect("clicked", self.on_save)
        btns.pack_start(cancel, False, False, 0)
        btns.pack_start(save, False, False, 0)
        box.pack_start(btns, False, False, 0)

        note = Gtk.Label(xalign=0)
        note.set_markup("<small>저장 후 입력 소스를 한 번 전환하거나 <tt>ibus restart</tt> 하면 반영됩니다.</small>")
        box.pack_start(note, False, False, 0)

        self.on_toggle(self.simple_check)

    def _make_combo(self, active_idx):
        combo = Gtk.ComboBoxText()
        if self.entries:
            for _, label in self.entries:
                combo.append_text(label)
            # active_idx 는 InputEntry 인덱스 → 콤보 위치(동일)
            pos = active_idx if 0 <= active_idx < len(self.entries) else 0
            combo.set_active(pos)
        else:
            combo.append_text("(nalgaeset.xml 을 찾을 수 없음)")
            combo.set_active(0)
            combo.set_sensitive(False)
        return combo

    @staticmethod
    def _idx(cfg, key):
        try:
            return int(cfg.get(key, "0"))
        except ValueError:
            return 0

    def on_toggle(self, check):
        on = check.get_active()
        self.hangul_combo.set_sensitive(on and bool(self.entries))
        self.latin_combo.set_sensitive(on and bool(self.entries))

    def on_save(self, *_):
        simple = self.simple_check.get_active()
        h = self.hangul_combo.get_active()
        l = self.latin_combo.get_active()
        if h < 0:
            h = 0
        if l < 0:
            l = 1
        save_ini(simple, h, l)
        self.close()


def main():
    win = SetupWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
    return 0


if __name__ == "__main__":
    sys.exit(main())
