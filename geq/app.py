import signal
import threading

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

from . import bands, routing  # noqa: E402
from .analyzer import Analyzer  # noqa: E402
from .engine import EqEngine  # noqa: E402
from .presets import CUSTOM, PRESETS  # noqa: E402
from .settings import Settings  # noqa: E402
from .view import GUTTER, AnalyzerView  # noqa: E402

CSS = f"""
window, .root {{ background: #0c0e13; color: #c9d1e0; }}
headerbar {{ background: #12151c; border-bottom: 1px solid #1d222d; box-shadow: none; }}
headerbar label, headerbar button, headerbar combobox {{ color: #c9d1e0; }}
.gain-label {{ font-family: monospace; font-size: 11px; color: #8fa0bb; }}
.gain-label.active {{ color: #ffffff; }}
.freq-label {{ font-weight: bold; font-size: 13px; color: #e6ecf7; letter-spacing: 1px; }}
.unit-label {{ font-size: 10px; color: #6c7a92; }}
.legend {{ font-family: monospace; font-size: 10px; color: #5f6c82; }}
.status {{ font-size: 11px; color: #e0a458; padding: 2px 10px; }}
.eq-panel {{ background: #11141b; border-top: 1px solid #1d222d; padding: 10px 6px 12px 0; }}
scale trough {{ background: #202633; border-radius: 3px; min-width: 6px; min-height: 6px; border: none; }}
scale highlight {{ background: none; border: none; }}
scale slider {{
  min-width: 30px; min-height: 14px; border-radius: 4px; border: 1px solid #0a0c10;
  background-image: linear-gradient(to bottom, #5b667c, #3a4254);
  box-shadow: 0 2px 4px rgba(0,0,0,0.6);
}}
scale slider:hover {{ background-image: linear-gradient(to bottom, #7684a0, #4a556c); }}
scale slider:active {{ background-image: linear-gradient(to bottom, #4ee1c1, #22a58c); }}
scale marks, scale mark {{ color: transparent; }}
popover, popover.background {{ background: #161a22; color: #c9d1e0; }}
.pop-title {{ font-size: 10px; color: #6c7a92; letter-spacing: 1px; }}
"""

STYLES = [("led", "LED segments"), ("bars", "Solid bars"), ("line", "Line"),
          ("area", "Filled area"), ("spectrum", "Full spectrum")]
THEME_NAMES = [("classic", "Classic green/red"), ("amber", "Amber"), ("ice", "Ice"),
               ("neon", "Neon"), ("rainbow", "Rainbow"), ("mono", "Mono")]
RANGES = [(40, "40 dB"), (60, "60 dB"), (80, "80 dB")]
FALLS = [("slow", "Slow"), ("medium", "Medium"), ("fast", "Fast")]
LABELS = [("k", "1.5k"), ("hz", "1500 Hz"), ("off", "Hidden")]
SOURCES = [("post", "After EQ (what you hear)"), ("pre", "Before EQ (what apps send)")]


def _fmt_gain(g):
    return "0" if abs(g) < 0.05 else f"{g:+.1f}"


class MainWindow(Gtk.ApplicationWindow):
    def __init__(self, app, cfg):
        super().__init__(application=app, title="Graphic EQ")
        self.cfg = cfg
        self.engine = EqEngine()
        self.analyzer = Analyzer(self._on_levels)
        self.orig_default = None
        self.real_sink = None
        self.engine_ready = False
        self._sinks = []
        self._building = True
        self._closing = False

        w, h = cfg["size"]
        self.set_default_size(w, h)
        self.get_style_context().add_class("root")

        self.view = AnalyzerView(cfg)
        self._build_header()
        self._build_body()
        self._building = False
        self.view.set_gains(cfg["gains"])
        self.connect("delete-event", self._on_close)
        self.show_all()
        GLib.timeout_add_seconds(2, self._watchdog)
        self._bring_up_audio()

    # -- UI ---------------------------------------------------------------
    def _build_header(self):
        hb = Gtk.HeaderBar(show_close_button=True, title="Graphic EQ")
        self.set_titlebar(hb)

        self.power = Gtk.Switch(active=self.cfg["eq_on"], valign=Gtk.Align.CENTER)
        self.power.set_tooltip_text("EQ on / bypass")
        self.power.connect("notify::active", self._on_power)
        box = Gtk.Box(spacing=6)
        box.pack_start(Gtk.Label(label="EQ"), False, False, 0)
        box.pack_start(self.power, False, False, 0)
        hb.pack_start(box)

        self.preset = Gtk.ComboBoxText()
        for name in list(PRESETS) + [CUSTOM]:
            self.preset.append(name, name)
        self.preset.set_active_id(self.cfg["preset"] if self.cfg["preset"] in PRESETS or
                                  self.cfg["preset"] == CUSTOM else CUSTOM)
        self.preset.connect("changed", self._on_preset)
        hb.pack_start(self.preset)

        reset = Gtk.Button.new_from_icon_name("edit-undo-symbolic", Gtk.IconSize.BUTTON)
        reset.set_tooltip_text("Reset all bands to flat")
        reset.connect("clicked", lambda _b: self.preset.set_active_id("Flat"))
        hb.pack_start(reset)

        mb = Gtk.MenuButton()
        mb.add(Gtk.Image.new_from_icon_name("preferences-system-symbolic", Gtk.IconSize.BUTTON))
        mb.set_tooltip_text("Display and audio options")
        mb.set_popover(self._build_options())
        hb.pack_end(mb)

    def _combo(self, items, current, key, on_change=None):
        c = Gtk.ComboBoxText()
        for val, text in items:
            c.append(str(val), text)
        c.set_active_id(str(current))

        def changed(_c):
            if self._building:
                return
            raw = c.get_active_id()
            val = type(items[0][0])(raw) if raw is not None else None
            self.cfg[key] = val
            if on_change:
                on_change()
        c.connect("changed", changed)
        return c

    def _build_options(self):
        pop = Gtk.Popover()
        grid = Gtk.Grid(row_spacing=8, column_spacing=14, margin=14)
        row = [0]

        def title(text):
            lbl = Gtk.Label(label=text.upper(), xalign=0)
            lbl.get_style_context().add_class("pop-title")
            grid.attach(lbl, 0, row[0], 2, 1)
            row[0] += 1

        def add(text, widget):
            grid.attach(Gtk.Label(label=text, xalign=0), 0, row[0], 1, 1)
            widget.set_hexpand(True)
            grid.attach(widget, 1, row[0], 1, 1)
            row[0] += 1

        def toggle(text, key):
            sw = Gtk.Switch(active=self.cfg[key], halign=Gtk.Align.END)
            sw.connect("notify::active", lambda s, _p: self.cfg.__setitem__(key, s.get_active()))
            add(text, sw)

        title("Display")
        add("Style", self._combo(STYLES, self.cfg["style"], "style"))
        add("Colors", self._combo(THEME_NAMES, self.cfg["theme"], "theme"))
        add("Range", self._combo(RANGES, self.cfg["range"], "range"))
        add("Fall speed", self._combo(FALLS, self.cfg["fall"], "fall"))
        add("Frequency labels", self._combo(LABELS, self.cfg["labels"], "labels", self._refresh_labels))
        sens = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, -20, 20, 1)
        sens.set_value(self.cfg["sensitivity"])
        sens.set_size_request(160, -1)
        sens.set_value_pos(Gtk.PositionType.RIGHT)
        sens.connect("value-changed", lambda s: self.cfg.__setitem__("sensitivity", s.get_value()))
        add("Sensitivity (dB)", sens)
        toggle("Peak hold", "peak_hold")
        toggle("Grid", "grid")
        toggle("dB scale", "db_labels")
        toggle("EQ curve overlay", "eq_curve")

        title("Audio")
        add("Analyze", self._combo(SOURCES, self.cfg["source"], "source", self._retarget_analyzer))
        self.out_combo = Gtk.ComboBoxText()
        self.out_combo.connect("changed", self._on_output_changed)
        add("Output device", self.out_combo)
        grid.show_all()
        pop.add(grid)
        pop.connect("show", lambda _p: self._fill_outputs())
        return pop

    def _build_body(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.add(root)
        self.status = Gtk.Label(xalign=0)
        self.status.get_style_context().add_class("status")
        self.status.set_no_show_all(True)
        root.pack_start(self.status, False, False, 0)
        root.pack_start(self.view, True, True, 0)

        panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        panel.get_style_context().add_class("eq-panel")
        root.pack_start(panel, False, False, 0)

        self.gain_labels, self.scales, self.freq_labels = [], [], []
        cells = Gtk.Box(homogeneous=True)
        for i, f in enumerate(bands.FREQS):
            col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
            gl = Gtk.Label(label="0")
            gl.get_style_context().add_class("gain-label")
            sc = Gtk.Scale.new_with_range(Gtk.Orientation.VERTICAL, bands.GAIN_MIN, bands.GAIN_MAX, 0.5)
            sc.set_inverted(True)
            sc.set_draw_value(False)
            sc.set_size_request(-1, 190)
            sc.set_halign(Gtk.Align.CENTER)
            sc.set_value(self.cfg["gains"][i])
            sc.add_mark(0, Gtk.PositionType.RIGHT, None)
            sc.set_tooltip_text(f"{f} Hz — double-click to reset")
            sc.connect("value-changed", self._on_slider, i)
            sc.connect("button-press-event", self._on_slider_press)
            fl = Gtk.Label()
            fl.get_style_context().add_class("freq-label")
            col.pack_start(gl, False, False, 0)
            col.pack_start(sc, True, True, 0)
            col.pack_start(fl, False, False, 0)
            cells.pack_start(col, True, True, 0)
            self.gain_labels.append(gl)
            self.scales.append(sc)
            self.freq_labels.append(fl)

        # left gutter mirrors the analyzer's so sliders line up under their bands
        legend = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        legend.set_size_request(GUTTER, -1)
        legend.set_margin_top(26)
        legend.set_margin_bottom(26)
        for i, text in enumerate(("+12", "0", "-12 dB")):
            l = Gtk.Label(label=text, xalign=1)
            l.get_style_context().add_class("legend")
            l.set_margin_end(8)
            legend.pack_start(l, i == 1, True, 0)
        row = Gtk.Box()
        row.pack_start(legend, False, False, 0)
        row.pack_start(cells, True, True, 0)
        row.set_margin_end(6)
        panel.pack_start(row, False, False, 0)
        self._refresh_labels()
        for i, g in enumerate(self.cfg["gains"]):
            self._update_gain_label(i, g)

    def _refresh_labels(self):
        mode = self.cfg["labels"]
        for fl, f in zip(self.freq_labels, bands.FREQS):
            fl.set_text(bands.label(f, mode))
            fl.set_visible(mode != "off")

    def _update_gain_label(self, i, g):
        gl = self.gain_labels[i]
        gl.set_text(_fmt_gain(g))
        ctx = gl.get_style_context()
        (ctx.add_class if abs(g) >= 0.05 else ctx.remove_class)("active")

    def _show_status(self, text):
        self.status.set_text(text or "")
        self.status.set_visible(bool(text))

    # -- controls ---------------------------------------------------------
    def _gains(self):
        return [s.get_value() for s in self.scales]

    def _on_slider(self, scale, i):
        g = scale.get_value()
        self._update_gain_label(i, g)
        if self._building:
            return
        gains = self._gains()
        self.cfg["gains"] = gains
        self.view.set_gains(gains)
        self._apply_gains()
        if not getattr(self, "_setting_preset", False):
            self._set_preset_silently(CUSTOM)

    @staticmethod
    def _on_slider_press(scale, event):
        if event.type == Gdk.EventType._2BUTTON_PRESS:
            scale.set_value(0)
            return True
        return False

    def _set_preset_silently(self, name):
        self._building = True
        self.preset.set_active_id(name)
        self._building = False
        self.cfg["preset"] = name

    def _on_preset(self, combo):
        name = combo.get_active_id()
        if self._building or name is None:
            return
        self.cfg["preset"] = name
        if name in PRESETS:
            self._setting_preset = True
            for sc, g in zip(self.scales, PRESETS[name]):
                sc.set_value(g)
            self._setting_preset = False

    def _on_power(self, sw, _p):
        self.cfg["eq_on"] = sw.get_active()
        self._apply_gains()
        self.view.set_gains(self._gains() if sw.get_active() else [0.0] * bands.COUNT)

    def _apply_gains(self):
        gains = self._gains() if self.cfg["eq_on"] else [0.0] * bands.COUNT
        self.engine.set_gains(gains)

    # -- audio routing ----------------------------------------------------
    def _fill_outputs(self):
        self._sinks = routing.list_sinks()
        self._building = True
        self.out_combo.remove_all()
        for name, desc in self._sinks:
            self.out_combo.append(name, desc)
        if self.real_sink:
            self.out_combo.set_active_id(self.real_sink)
        self._building = False

    def _on_output_changed(self, combo):
        name = combo.get_active_id()
        if self._building or not name or name == self.real_sink:
            return
        self.cfg["output"] = name
        self._restart_engine(name)

    def _bring_up_audio(self):
        current = routing.default_sink()
        if current and current.startswith("geq_"):  # left over from a crash
            current = None
        self.orig_default = current
        sinks = [n for n, _ in routing.list_sinks()]
        target = self.cfg["output"] if self.cfg["output"] in sinks else (current or (sinks[0] if sinks else None))
        self.real_sink = target
        if not target:
            self._show_status("No audio output found — the analyzer has nothing to listen to.")
            return
        self._restart_engine(target)

    def _restart_engine(self, target):
        self._show_status("Starting equalizer…")
        self.engine_ready = False

        def work():
            err = self.engine.start(target)
            GLib.idle_add(self._engine_started, target, err)
        threading.Thread(target=work, daemon=True).start()

    def _engine_started(self, target, err):
        if self._closing:
            self.engine.stop()
            return
        if err:
            self._show_status(f"EQ engine unavailable ({err}). Showing the analyzer only.")
            self.power.set_sensitive(False)
            self.real_sink = target
            self._retarget_analyzer()
            return
        self.real_sink = target
        self.engine_ready = True
        self.power.set_sensitive(True)
        routing.set_default_sink(routing.VIRTUAL_SINK)
        routing.move_streams(None, routing.VIRTUAL_SINK)
        self._apply_gains()
        self._show_status("")
        self.analyzer.stop()  # re-open after the default sink moved, so it stays on our tap
        self._retarget_analyzer()

    def _retarget_analyzer(self):
        if self.engine_ready and self.cfg["source"] == "pre":
            dev = routing.VIRTUAL_SINK + ".monitor"
        elif self.real_sink:
            dev = self.real_sink + ".monitor"
        else:
            dev = None
        self.analyzer.set_device(dev)

    def _watchdog(self):
        if self._closing:
            return False
        if self.engine_ready and not self.engine.running:
            self.engine_ready = False
            if self.orig_default:
                routing.set_default_sink(self.real_sink or self.orig_default)
            self._show_status("The EQ engine stopped unexpectedly. Audio is back on your normal output.")
            self.power.set_sensitive(False)
            self._retarget_analyzer()
        return True

    def _on_levels(self, levels, curve):
        self.view.feed(levels, curve)

    # -- shutdown ---------------------------------------------------------
    def _on_close(self, *_a):
        self.shutdown()
        return False

    def shutdown(self):
        if self._closing:
            return
        self._closing = True
        w, h = self.get_size()
        self.cfg["size"] = [w, h]
        self.cfg.save()
        self.analyzer.stop()
        if self.engine_ready:
            target = self.real_sink or self.orig_default
            if target:
                routing.set_default_sink(target)
                routing.move_streams(routing.VIRTUAL_SINK, target)
        self.engine.stop()


class App(Gtk.Application):
    def __init__(self):
        super().__init__(application_id="net.local.GraphicEq")
        self.win = None

    def do_startup(self):
        Gtk.Application.do_startup(self)
        prov = Gtk.CssProvider()
        prov.load_from_data(CSS.encode())
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), prov, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        Gtk.Settings.get_default().set_property("gtk-application-prefer-dark-theme", True)

    def do_activate(self):
        if not self.win:
            self.win = MainWindow(self, Settings())
            for sig in (signal.SIGINT, signal.SIGTERM):
                GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, sig, self._on_signal)
        self.win.present()

    def _on_signal(self):
        if self.win:
            self.win.shutdown()
        self.quit()
        return False


def main():
    return App().run(None)
