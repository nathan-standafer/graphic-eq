# Graphic EQ

An 11-band spectrum analyzer and graphic equalizer for Linux, in the spirit of a late-80s
component-stereo EQ. Bands: 40, 73, 130, 240, 440, 800, 1.5k, 2.7k, 4.8k, 8.8k, 16 kHz (log-spaced), ±12 dB.

It is a real system-wide EQ: it creates a virtual output ("Graphic EQ") in PipeWire, makes it the
default, and filters everything sent there into your real output. The analyzer listens to the result.

## Run

    ./graphic-eq          # or: python3 -m geq

Needs only: Python 3, PyGObject (GTK 3, GStreamer, `gst-plugins-good`), pycairo, PipeWire with
`pipewire-pulse` (provides `pactl`), `pw-cli`, `pw-dump`. No pip packages.

## Use

- Drag the sliders (double-click to reset one), or pick a preset. The EQ switch bypasses.
- Gear menu: style (LED / bars / line / area / full spectrum), colors, dB range, fall speed, peak hold,
  grid, dB scale, EQ-curve overlay, label format, sensitivity, analyze before/after EQ, output device.
- Settings persist in `~/.config/graphic-eq/settings.json`. Quitting restores your default output.

## Notes

- "After EQ" reads your output's monitor, so it follows the system volume; "Before EQ" doesn't.
- The filter graph stays running while the app is open (needed for gain changes to apply while idle).
- Stacked boosts add up (all bands at +6 ≈ +8 dB overall); lower the system volume if it clips.

## GPU version (Rust + wgpu)

`gpu/` is a ground-up Rust port with a GPU-rendered display: 3D LED columns on a reflective floor, a
scrolling waterfall terrain, HDR bloom, 4x MSAA. It holds 120 fps even in a 4800x1300 window.
It does everything the Python app does (same EQ engine, routing, presets); settings live in
`~/.config/graphic-eq/gpu-settings.json`.

    cd gpu && cargo run --release

Needs Rust (rustup) and a Vulkan-capable GPU. Pick the scene, colors, glow and more under "Display".
