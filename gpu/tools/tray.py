"""Cinnamon/XApp tray bridge. The Rust app owns audio, settings and lifetime.

Line-delimited JSON arrives on stdin; commands go to stdout. EOF removes the
icon, so a crash cannot leave an orphan tray process. Embedded in the binary.
"""
import json
import os
import sys

import gi
gi.require_version('Gtk', '3.0')
gi.require_version('XApp', '1.0')
from gi.repository import GLib, Gtk, XApp


def send(command):
    print(command, flush=True)


def main():
    # XApp derives its D-Bus name from the program name. Embedded Python otherwise
    # uses "-c", colliding with other helpers and isolated desktop test instances.
    GLib.set_prgname(f'spectraforgeeq_{os.getpid()}')
    if not Gtk.init_check()[0]:
        raise RuntimeError('No desktop display available')
    icon = XApp.StatusIcon.new_with_name('SpectraForgeEQ')
    icon.set_icon_name(sys.argv[1])
    icon.set_tooltip_text('SpectraForgeEQ — starting equalizer…')
    icon.connect('activate', lambda *_: send('show'))
    icon.connect('state-changed', lambda _, state: send(
        'tray-unavailable' if state == XApp.StatusIconState.NO_SUPPORT else 'tray-ready'))
    menu = Gtk.Menu()
    status_item = Gtk.MenuItem(label='Starting equalizer…')
    status_item.set_sensitive(False)
    menu.append(status_item)
    open_item = Gtk.MenuItem(label='Open SpectraForgeEQ')
    open_item.connect('activate', lambda *_: send('show'))
    menu.append(open_item)
    toggle = Gtk.CheckMenuItem(label='Enable equalizer')
    updating = False

    def toggle_eq(item):
        if not updating:
            send('on' if item.get_active() else 'off')
    toggle.connect('toggled', toggle_eq)
    menu.append(toggle)
    presets_item = Gtk.MenuItem(label='Presets')
    presets_menu = Gtk.Menu()
    presets_item.set_submenu(presets_menu)
    menu.append(presets_item)
    menu.append(Gtk.SeparatorMenuItem())
    quit_item = Gtk.MenuItem(label='Quit SpectraForgeEQ')
    quit_item.connect('activate', lambda *_: send('quit'))
    menu.append(quit_item)
    menu.show_all()
    icon.set_secondary_menu(menu)
    icon.set_visible(True)
    buffer = bytearray()
    preset_items = []

    def read_status(fd, condition):
        nonlocal updating
        chunk = os.read(fd, 8192)
        if not chunk:
            Gtk.main_quit()
            return False
        buffer.extend(chunk)
        while b'\n' in buffer:
            line, _, rest = buffer.partition(b'\n')
            buffer[:] = rest
            state = json.loads(line)
            if not preset_items:
                for index, name in enumerate(state['presets']):
                    item = Gtk.CheckMenuItem(label=name)
                    item.set_draw_as_radio(True)
                    item.connect('activate', lambda _, i=index: send(f'preset {i}') if not updating else None)
                    presets_menu.append(item)
                    preset_items.append(item)
                presets_menu.show_all()
            updating = True
            toggle.set_active(state['eq_on'])
            for item, name in zip(preset_items, state['presets']):
                item.set_active(name == state['preset'])
            updating = False
            status = state['status'] or ('EQ active' if state['eq_on'] else 'EQ bypassed')
            status_item.set_label(status)
            icon.set_tooltip_text(f"SpectraForgeEQ — {status}\nPreset: {state['preset']}")
        return True

    GLib.io_add_watch(sys.stdin.fileno(), GLib.IO_IN | GLib.IO_HUP | GLib.IO_ERR, read_status)
    # XApp may take a moment to register with Cinnamon's panel.
    def announce():
        send('tray-unavailable' if icon.get_state() == XApp.StatusIconState.NO_SUPPORT else 'tray-ready')
        return False
    GLib.timeout_add(1500, announce)
    Gtk.main()


if __name__ == '__main__':
    main()
