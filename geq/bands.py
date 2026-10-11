"""Band layout and the maths shared by the engine and the display."""

import cmath
import math

FREQS = [40, 73, 130, 240, 440, 800, 1500, 2700, 4800, 8800, 16000]
COUNT = len(FREQS)
STEP = (FREQS[-1] / FREQS[0]) ** (1 / (COUNT - 1))  # ~1.82x between neighbours
Q = 1.6  # ~0.86 octave wide, so neighbouring sliders overlap smoothly
GAIN_MIN, GAIN_MAX = -12.0, 12.0
SAMPLE_RATE = 48000


def edges(freq):
    """Lower/upper edge of the analyzer band centred on freq."""
    half = math.sqrt(STEP)
    return freq / half, freq * half


def label(freq, mode):
    if mode == "off":
        return ""
    if mode == "hz":
        return f"{freq:g}"
    return f"{freq / 1000:g}k" if freq >= 1000 else f"{freq:g}"


def _peaking_response(gain_db, f0, f, fs=SAMPLE_RATE, q=Q):
    """Magnitude (dB) of an RBJ peaking biquad at frequency f."""
    if abs(gain_db) < 1e-6:
        return 0.0
    a = 10 ** (gain_db / 40)
    w0 = 2 * math.pi * f0 / fs
    alpha = math.sin(w0) / (2 * q)
    cw = math.cos(w0)
    b = (1 + alpha * a, -2 * cw, 1 - alpha * a)
    d = (1 + alpha / a, -2 * cw, 1 - alpha / a)
    z = cmath.exp(-1j * 2 * math.pi * f / fs)
    num = b[0] + b[1] * z + b[2] * z * z
    den = d[0] + d[1] * z + d[2] * z * z
    return 20 * math.log10(abs(num / den))


def eq_response_db(gains, freqs):
    """Combined response of all bands at each frequency in freqs."""
    return [sum(_peaking_response(g, f0, f) for g, f0 in zip(gains, FREQS)) for f in freqs]
