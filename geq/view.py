"""The analyzer display: LED segments, bars, line, area or full spectrum."""

import math

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

from . import bands  # noqa: E402
from .analyzer import CURVE_FREQS, FLOOR  # noqa: E402

GUTTER = 46          # left strip for dB labels; the slider rows use the same width
FALL_DB_PER_S = {"slow": 14.0, "medium": 38.0, "fast": 110.0}
BAR_GAP = 10.0       # pixels between neighbouring bars
PEAK_HOLD_S = 1.1
PEAK_FALL_DB_PER_S = 22.0

# Gradient stops, bottom -> top, as RGB 0..1
THEMES = {
    "classic": [(0.0, (0.18, 0.91, 0.42)), (0.62, (0.96, 0.88, 0.29)), (1.0, (1.0, 0.25, 0.25))],
    "amber":   [(0.0, (1.0, 0.69, 0.0)), (0.6, (1.0, 0.48, 0.0)), (1.0, (1.0, 0.24, 0.0))],
    "ice":     [(0.0, (0.0, 0.90, 1.0)), (0.6, (0.30, 0.49, 1.0)), (1.0, (0.69, 0.30, 1.0))],
    "neon":    [(0.0, (1.0, 0.17, 0.84)), (0.55, (0.48, 0.36, 1.0)), (1.0, (0.0, 0.90, 1.0))],
    "mono":    [(0.0, (0.45, 0.50, 0.60)), (1.0, (0.95, 0.97, 1.0))],
}
EQ_CURVE_RGB = (0.31, 0.88, 0.76)
BG_TOP, BG_BOTTOM = (0.055, 0.062, 0.080), (0.035, 0.040, 0.052)


def _lerp_color(stops, t):
    t = min(1.0, max(0.0, t))
    for (t0, c0), (t1, c1) in zip(stops, stops[1:]):
        if t <= t1:
            k = (t - t0) / (t1 - t0) if t1 > t0 else 0
            return tuple(a + (b - a) * k for a, b in zip(c0, c1))
    return stops[-1][1]


def _hsv(h, s=0.85, v=1.0):
    import colorsys
    return colorsys.hsv_to_rgb(h % 1.0, s, v)


class AnalyzerView(Gtk.DrawingArea):
    def __init__(self, settings):
        super().__init__()
        self.cfg = settings
        n = bands.COUNT
        self.raw = [FLOOR] * n
        self.disp = [FLOOR] * n
        self.peak = [FLOOR] * n
        self.peak_age = [0.0] * n
        self.raw_curve = [FLOOR] * len(CURVE_FREQS)
        self.disp_curve = [FLOOR] * len(CURVE_FREQS)
        self.peak_curve = [FLOOR] * len(CURVE_FREQS)
        self.peak_curve_age = [0.0] * len(CURVE_FREQS)
        self.gains = [0.0] * n
        self._eq_curve = [0.0] * len(CURVE_FREQS)
        self._last = None
        self.set_size_request(480, 200)
        self.connect("draw", self._draw)
        self.add_tick_callback(self._tick)

    # -- data in ----------------------------------------------------------
    def feed(self, levels, curve):
        self.raw, self.raw_curve = levels, curve

    def set_gains(self, gains):
        self.gains = list(gains)
        self._eq_curve = bands.eq_response_db(self.gains, CURVE_FREQS)

    # -- ballistics -------------------------------------------------------
    @staticmethod
    def _ballistics(disp, peak, age, raw, dt, fall, hold):
        for i, r in enumerate(raw):
            if r >= disp[i]:
                disp[i] = r
            else:
                disp[i] = max(r, disp[i] - fall * dt)
            if disp[i] >= peak[i]:
                peak[i], age[i] = disp[i], 0.0
            else:
                age[i] += dt
                if age[i] > PEAK_HOLD_S:
                    peak[i] = max(disp[i], peak[i] - PEAK_FALL_DB_PER_S * dt)

    def _tick(self, widget, clock):
        now = clock.get_frame_time() / 1e6
        dt = min(0.1, now - self._last) if self._last else 0.016
        self._last = now
        fall = FALL_DB_PER_S[self.cfg["fall"]]
        self._ballistics(self.disp, self.peak, self.peak_age, self.raw, dt, fall, True)
        self._ballistics(self.disp_curve, self.peak_curve, self.peak_curve_age,
                         self.raw_curve, dt, fall, True)
        self.queue_draw()
        return True

    # -- drawing ----------------------------------------------------------
    def _color_at(self, frac, band=None):
        theme = self.cfg["theme"]
        if theme == "rainbow":
            if band is not None:
                return _hsv(0.95 - 0.8 * band / (bands.COUNT - 1), 0.85, 1.0)
            return _hsv(0.33 - 0.33 * frac)
        return _lerp_color(THEMES.get(theme, THEMES["classic"]), frac)

    def _frac(self, db):
        rng = float(self.cfg["range"])
        return min(1.0, max(0.0, (db + self.cfg["sensitivity"] + rng) / rng))

    def _vgradient(self, cr_mod, y_bottom, y_top, alpha=1.0, band=None):
        import cairo
        g = cairo.LinearGradient(0, y_bottom, 0, y_top)
        theme = self.cfg["theme"]
        if theme == "rainbow" and band is not None:
            c = self._color_at(0, band)
            g.add_color_stop_rgba(0, *c, alpha * 0.55)
            g.add_color_stop_rgba(1, *c, alpha)
        else:
            stops = THEMES.get(theme, THEMES["classic"]) if theme != "rainbow" else \
                [(i / 6, _hsv(0.33 - 0.33 * i / 6)) for i in range(7)]
            for t, c in stops:
                g.add_color_stop_rgba(t, *c, alpha)
        return g

    def _draw(self, _w, cr):
        import cairo
        w, h = self.get_allocated_width(), self.get_allocated_height()
        top, bottom = 14.0, 10.0
        x0, x1 = float(GUTTER), float(w) - 6
        y_top, y_bot = top, h - bottom
        ph = y_bot - y_top
        cw = (x1 - x0) / bands.COUNT

        # panel
        bg = cairo.LinearGradient(0, 0, 0, h)
        bg.add_color_stop_rgb(0, *BG_TOP)
        bg.add_color_stop_rgb(1, *BG_BOTTOM)
        cr.set_source(bg)
        cr.paint()

        def cx(i):
            return x0 + (i + 0.5) * cw

        def fx(f):  # frequency -> x on the same log axis as the band centres
            return x0 + (math.log(f / bands.FREQS[0]) / math.log(bands.STEP) + 0.5) * cw

        def fy(frac):
            return y_bot - frac * ph

        self._draw_grid(cr, x0, x1, y_top, y_bot, cw)

        style = self.cfg["style"]
        peaks_on = self.cfg["peak_hold"]
        if style in ("led", "bars"):
            self._draw_bars(cr, style, cx, fy, cw, y_top, y_bot, peaks_on)
        elif style in ("line", "area"):
            pts = [(cx(i), fy(self._frac(d))) for i, d in enumerate(self.disp)]
            pts = [(x0, pts[0][1])] + pts + [(x1, pts[-1][1])]
            self._draw_curve(cr, pts, y_top, y_bot, fill=(style == "area"))
            if peaks_on:
                for i, p in enumerate(self.peak):
                    self._peak_tick(cr, cx(i), fy(self._frac(p)), cw * 0.5, i, self._frac(p))
        else:  # spectrum
            pts = [(fx(f), fy(self._frac(d))) for f, d in zip(CURVE_FREQS, self.disp_curve)]
            self._draw_curve(cr, pts, y_top, y_bot, fill=True, smooth=False)
            if peaks_on:
                cr.set_line_width(1.2)
                cr.set_source_rgba(1, 1, 1, 0.55)
                for k, (f, p) in enumerate(zip(CURVE_FREQS, self.peak_curve)):
                    (cr.move_to if k == 0 else cr.line_to)(fx(f), fy(self._frac(p)))
                cr.stroke()

        if self.cfg["eq_curve"]:
            self._draw_eq_curve(cr, fx, (y_top + y_bot) / 2, ph, x1)

    def _draw_grid(self, cr, x0, x1, y_top, y_bot, cw):
        import cairo
        rng = int(self.cfg["range"])
        ph = y_bot - y_top
        sens = self.cfg["sensitivity"]
        cr.select_font_face("monospace", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_NORMAL)
        cr.set_font_size(10)
        if self.cfg["grid"]:
            cr.set_line_width(1)
            for i in range(1, bands.COUNT):
                x = round(x0 + i * cw) + 0.5
                cr.set_source_rgba(1, 1, 1, 0.035)
                cr.move_to(x, y_top)
                cr.line_to(x, y_bot)
                cr.stroke()
        step = 10
        db = 0
        while db >= -rng:
            frac = (db + sens + rng) / rng
            if 0 <= frac <= 1:
                y = round(y_bot - frac * ph) + 0.5
                if self.cfg["grid"]:
                    cr.set_source_rgba(1, 1, 1, 0.07 if db else 0.16)
                    cr.set_line_width(1)
                    cr.move_to(x0, y)
                    cr.line_to(x1, y)
                    cr.stroke()
                if self.cfg["db_labels"]:
                    txt = f"{db}"
                    ext = cr.text_extents(txt)
                    cr.set_source_rgba(0.62, 0.68, 0.78, 0.8)
                    cr.move_to(x0 - 8 - ext.width, y + 3.5)
                    cr.show_text(txt)
            db -= step
        if self.cfg["db_labels"]:
            cr.set_source_rgba(0.62, 0.68, 0.78, 0.5)
            cr.move_to(6, y_top + 3.5)
            cr.show_text("dB")

    def _draw_bars(self, cr, style, cx, fy, cw, y_top, y_bot, peaks_on):
        import cairo
        bw = max(4.0, cw - BAR_GAP)  # fixed gap; bars take the rest of each cell
        ph = y_bot - y_top
        if style == "led":
            pitch = 7.0
            n = max(1, int(ph // pitch))
            seg_h = pitch - 2
            unlit, lit_img = self._led_layers(cx, cw, bw, n, pitch, seg_h, y_bot)
            cr.set_source_surface(unlit, 0, 0)
            cr.paint()
            for i, d in enumerate(self.disp):
                x = cx(i) - bw / 2
                lit = int(self._frac(d) * n + 0.5)
                pk = int(self._frac(self.peak[i]) * n + 0.5) - 1
                cr.set_source_surface(lit_img, 0, 0)
                if lit > 0:
                    cr.save()
                    cr.rectangle(x, y_bot - lit * pitch + 2, bw, lit * pitch - 2)
                    cr.clip()
                    cr.paint()
                    cr.restore()
                if peaks_on and pk >= lit:
                    cr.save()
                    cr.rectangle(x, y_bot - (pk + 1) * pitch + 2, bw, seg_h)
                    cr.clip()
                    cr.paint_with_alpha(0.95)
                    cr.restore()
        else:
            for i, d in enumerate(self.disp):
                x = cx(i) - bw / 2
                y = fy(self._frac(d))
                if y_bot - y > 0.5:
                    cr.save()
                    cr.rectangle(x, y, bw, y_bot - y)
                    cr.clip()
                    cr.set_source(self._vgradient(cr, y_bot, y_top, 1.0, i))
                    self._rrect(cr, x, y_top, bw, ph + 4, 3)
                    cr.fill()
                    cr.restore()
                if peaks_on:
                    self._peak_tick(cr, cx(i), fy(self._frac(self.peak[i])), bw, i,
                                    self._frac(self.peak[i]))

    def _led_layers(self, cx, cw, bw, n, pitch, seg_h, y_bot):
        """Dim and bright copies of every LED segment, rebuilt only when the geometry changes."""
        import cairo
        w, h = self.get_allocated_width(), self.get_allocated_height()
        key = (w, h, self.cfg["theme"], round(bw, 1), n)
        if getattr(self, "_led_key", None) != key:
            def build(alpha_fn):
                s = cairo.ImageSurface(cairo.FORMAT_ARGB32, w, h)
                c = cairo.Context(s)
                for i in range(bands.COUNT):
                    x = cx(i) - bw / 2
                    for j in range(n):
                        col = self._color_at((j + 0.5) / n, i)
                        c.set_source_rgba(*col, alpha_fn)
                        self._rrect(c, x, y_bot - (j + 1) * pitch + 2, bw, seg_h, 1.5)
                        c.fill()
                return s
            self._led_key = key
            self._led_unlit = build(0.075)
            self._led_lit = build(1.0)
        return self._led_unlit, self._led_lit

    def _peak_tick(self, cr, x, y, width, band, frac):
        c = self._color_at(frac, band)
        cr.set_source_rgba(*c, 0.95)
        self._rrect(cr, x - width / 2, y - 3, width, 2.5, 1)
        cr.fill()

    def _draw_curve(self, cr, pts, y_top, y_bot, fill, smooth=True):
        def path():
            cr.move_to(*pts[0])
            if not smooth:
                for p in pts[1:]:
                    cr.line_to(*p)
                return
            for i in range(len(pts) - 1):  # Catmull-Rom -> cubic Bezier
                p0 = pts[max(i - 1, 0)]
                p1, p2 = pts[i], pts[i + 1]
                p3 = pts[min(i + 2, len(pts) - 1)]
                c1 = (p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6)
                c2 = (p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6)
                # keep control points inside the plot so peaks don't overshoot
                c1 = (c1[0], min(max(c1[1], y_top), y_bot))
                c2 = (c2[0], min(max(c2[1], y_top), y_bot))
                cr.curve_to(*c1, *c2, *p2)

        if fill:
            path()
            cr.line_to(pts[-1][0], y_bot)
            cr.line_to(pts[0][0], y_bot)
            cr.close_path()
            cr.set_source(self._vgradient(cr, y_bot, y_top, 0.38))
            cr.fill()
        path()
        cr.set_line_width(5)
        cr.set_source(self._vgradient(cr, y_bot, y_top, 0.16))
        cr.stroke_preserve()
        cr.set_line_width(2)
        cr.set_source(self._vgradient(cr, y_bot, y_top, 1.0))
        cr.stroke()

    def _draw_eq_curve(self, cr, fx, y_mid, ph, x1):
        """The EQ's own response, scaled so +/-12 dB spans the middle 80% of the plot.

        It has its own scale (not the dB axis), so it gets its own colour and caption.
        """
        scale = 0.4 * ph / bands.GAIN_MAX
        r, g, b = EQ_CURVE_RGB
        cr.set_line_width(1.8)
        cr.set_dash([6, 4])
        cr.set_source_rgba(r, g, b, 0.9)
        for k, (f, db) in enumerate(zip(CURVE_FREQS, self._eq_curve)):
            (cr.move_to if k == 0 else cr.line_to)(fx(f), y_mid - db * scale)
        cr.stroke()
        cr.set_dash([])
        cr.set_font_size(10)
        txt = "EQ curve  \u00b112 dB"
        ext = cr.text_extents(txt)
        cr.set_source_rgba(r, g, b, 0.85)
        cr.move_to(x1 - ext.width - 6, y_mid - 5)
        cr.show_text(txt)

    @staticmethod
    def _rrect(cr, x, y, w, h, r):
        r = min(r, w / 2, h / 2)
        cr.new_sub_path()
        cr.arc(x + w - r, y + r, r, -math.pi / 2, 0)
        cr.arc(x + w - r, y + h - r, r, 0, math.pi / 2)
        cr.arc(x + r, y + h - r, r, math.pi / 2, math.pi)
        cr.arc(x + r, y + r, r, math.pi, 3 * math.pi / 2)
        cr.close_path()
