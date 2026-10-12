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

`gpu/` is a ground-up Rust port with a GPU-rendered display. It does everything the Python app does
(same EQ engine, routing, presets); settings live in `~/.config/graphic-eq/gpu-settings.json`.

    cd gpu && cargo run --release

Needs Rust (rustup) and a Vulkan-capable GPU. It holds 120 fps at 5120x1440 on a Radeon RX 580.

**Scenes**, each with curated *looks* (palette, exposure, bloom, motion):

- **Spectrum Tunnel** - you look down a tube whose wall is the last three seconds of spectrum: bass along
  the floor, treble along the ceiling, left channel on the left wall, right on the right. Loud bins
  push inward, low-frequency hits launch luminous rings down the tube, treble accents flicker.
- **3D columns** - segmented LED cores in glass housings on a glossy floor.
- **Waterfall** - the spectrum history as a wireframe terrain (mixed or stereo overlay), inside glass
  walls on all four sides. The front wall carries the live spectrum line; each side wall carries the
  terrain's edge and the back wall the oldest row, so their lines follow where the waterfall meets the
  glass. It stands on a glass floor under a night sky: stars, a nebula band, a ringed planet, a small
  planet and moons, all tinted from the theme. Bass hits flare the stars and planet atmospheres and send
  a shockwave across the sky, and the floor mirrors all of it, along with the terrain and walls. A grid
  is etched into the surface at zero level, and the walls cast a little light onto the glass, with
  sheen from overhead light bars.

- **Vectorscope** - the raw stereo signal drawn the way an analogue X/Y scope draws it: mono is a vertical
  line, wide stereo opens into a ball, out-of-phase material tips toward the horizontal. The beam has a
  sharp core in a soft halo, is bright where it dwells and faint where it races, and leaves a fading
  phosphor afterglow (Display menu: **Persistence**). On wide screens the left and right waveforms run
  across the outer thirds as triggered traces. The picture follows the signal's level so quiet music
  still fills the tube; silence leaves just the graticule.

**Controls.** `F`/`F11` full screen, `Esc` leaves, `Left`/`Right` change scene, `1`-`9` pick a preset
(your favorites, else the whole catalog), `L` opens the look browser. In full screen a compact strip
appears on mouse movement and fades after a few seconds. Display menu: looks, colors, **Motion**
(camera drift, banking, wall breathing; 0 holds everything still), **Flash** (onset rings and
pulses; 0 turns them off), Glow, crossfade/cut, auto-cycle, range, sensitivity. Motion and Flash are
independent. Drag orbits, scroll zooms, middle-drag pans, double-click resets; the idle camera drift
pauses while you hold the camera and eases back afterwards.

**Developing without audio routing.** `GEQ_SYNTH` replaces the capture with deterministic synthetic
audio: no EQ engine, no routing changes, settings are read-only.

    GEQ_SYNTH=1 cargo run --release              # a looping 64 s groove (demo)
    GEQ_SYNTH=impulse|percussion|tones|sweep|dense|stereo|phase|quiet|silence cargo run --release
    GEQ_REPLAY=clip.wav cargo run --release      # loop a WAV file (PCM or float, any rate)

    # a deterministic 6 s screenshot of the tunnel at 32:9, without going full screen
    GEQ_SYNTH=1 GEQ_SETTINGS=default GEQ_SCENE=2 GEQ_BARE=1 GEQ_SIZE=2560x720 GEQ_SHOT=/tmp/t.png,6 cargo run --release
    # frame-time statistics and per-pass GPU time at 5120x1440
    GEQ_SYNTH=1 GEQ_SCENE=2 GEQ_FPS=1 GEQ_GPUTIME=1 GEQ_FULLSCREEN=1 cargo run --release

`GEQ_SHOT` with a synthetic source steps a simulated clock, so captures repeat exactly. Other hooks:
`GEQ_PRESET`, `GEQ_THEME`, `GEQ_BLOOM`, `GEQ_MOTION`, `GEQ_FLASH`, `GEQ_STEREO`, `GEQ_ORBIT`,
`GEQ_CYCLE=secs` (auto-cycle scenes), `GEQ_LATENCY=1` (onset delivery delay), `GEQ_BROWSER=1`,
`GEQ_VIEW=scene|bloom|nan` (composite debug views). `tools/make-thumbs.sh` regenerates the look
thumbnails from real captures. `cargo test` covers the synthetic source, onset detection, and the
modulation layer; `cargo test -- --ignored live_capture` smoke-tests the live `parec` path.
