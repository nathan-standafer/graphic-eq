"""Audio capture + FFT via GStreamer's `spectrum` element, reduced to display levels."""

import math
from itertools import accumulate

import gi

gi.require_version("Gst", "1.0")
from gi.repository import Gst  # noqa: E402

from . import bands  # noqa: E402

Gst.init(None)

FFT_BANDS = 4096        # 5.9 Hz per bin at 48 kHz; needed to resolve the 40 Hz band
INTERVAL_NS = 33_000_000
CAL_DB = 9.9            # measured: a -6 dBFS sine must read -6 dB
CURVE_POINTS = 96
CURVE_FREQS = [30 * (20000 / 30) ** (i / (CURVE_POINTS - 1)) for i in range(CURVE_POINTS)]
FLOOR = -140.0


class Analyzer:
    def __init__(self, on_levels):
        self.on_levels = on_levels
        self.pipe = None
        self.device = None
        self._bin_hz = (bands.SAMPLE_RATE / 2) / FFT_BANDS
        half = math.sqrt(bands.STEP)
        self._band_edges = [bands.edges(f) for f in bands.FREQS]
        self._curve_edges = [(f / half, f * half) for f in CURVE_FREQS]

    def set_device(self, device):
        if device == self.device and self.pipe:
            return
        self.stop()
        self.device = device
        if not device:
            return
        desc = (
            f"pulsesrc device={device} client-name=\"Graphic EQ analyzer\" latency-time=20000 ! "
            f"audio/x-raw,rate={bands.SAMPLE_RATE},channels=2 ! audioconvert ! "
            f"spectrum bands={FFT_BANDS} threshold=-140 interval={INTERVAL_NS} "
            "post-messages=true message-magnitude=true ! fakesink sync=false"
        )
        self.pipe = Gst.parse_launch(desc)
        bus = self.pipe.get_bus()
        bus.add_signal_watch()
        bus.connect("message::element", self._on_element)
        self.pipe.set_state(Gst.State.PLAYING)

    def stop(self):
        if self.pipe:
            self.pipe.get_bus().remove_signal_watch()
            self.pipe.set_state(Gst.State.NULL)
            self.pipe = None
        self.device = None

    def _on_element(self, _bus, msg):
        s = msg.get_structure()
        if s is None or s.get_name() != "spectrum":
            return
        # PyGObject can't convert GstValueList, so read it from the text form.
        text = s.to_string()
        try:
            a = text.index("{", text.index("magnitude")) + 1
            mags = [float(x) for x in text[a:text.index("}", a)].split(",")]
        except ValueError:
            return
        self.on_levels(*self._reduce(mags))

    def _reduce(self, mags):
        n = len(mags)
        cum = [0.0]
        cum.extend(accumulate(10 ** (m * 0.1) for m in mags))
        w = self._bin_hz

        def energy(f):
            pos = f / w
            i = int(pos)
            if i >= n:
                return cum[n]
            return cum[i] + (pos - i) * (cum[i + 1] - cum[i])

        def db(lo, hi):
            e = energy(hi) - energy(lo)
            return max(FLOOR, 10 * math.log10(e) + CAL_DB) if e > 1e-14 else FLOOR

        return ([db(lo, hi) for lo, hi in self._band_edges],
                [db(lo, hi) for lo, hi in self._curve_edges])
