# Development

[Back to the README](../README.md) · [Audio analysis](audio-analysis.md) · [Contributing](../CONTRIBUTING.md)

## Build and test

From the repository root:

```sh
cargo fetch --manifest-path gpu/Cargo.toml --locked
cargo build --manifest-path gpu/Cargo.toml --release --offline
cargo test --manifest-path gpu/Cargo.toml --offline
python3 -m unittest discover -s packaging -p 'test_*.py'
```

The development binary is `gpu/target/release/geq-gpu`. The installed launcher is
`spectraforgeeq`. Normal runs share a single-instance session with the installed app.
Changes to the source need a rebuild and reinstall before they appear in the installed app.
Quit the installed app from its tray before running a new development build.

## Desktop smoke test

```sh
python3 packaging/check_desktop.py
```

This opt-in test needs a running Cinnamon/X11 desktop and a release build. It uses synthetic
audio, temporary settings and a separate session directory to exercise hidden startup,
bypass and presets, repeated launches, closing and reopening, tray failure, explicit quit,
and recovery after a crash. It does not route real audio. `SPECTRAFORGE_RUNTIME_DIR` is a
test-only session-directory override.

## Repository map

| Location | Purpose |
|---|---|
| [`gpu/src/app.rs`](../gpu/src/app.rs) | Window, controls, scene selection and background lifecycle |
| [`gpu/src/eq_strip.rs`](../gpu/src/eq_strip.rs) | Illuminated EQ faders |
| [`gpu/src/audio.rs`](../gpu/src/audio.rs) | Audio engine and analyzer lifecycle |
| [`gpu/src/engine.rs`](../gpu/src/engine.rs) | PipeWire filter chain |
| [`gpu/src/desktop.rs`](../gpu/src/desktop.rs) | Single instance, local controls and tray bridge |
| [`gpu/src/scenes/`](../gpu/src/scenes/) | Scene renderers and WGSL shaders |
| [`gpu/src/render.rs`](../gpu/src/render.rs) | HDR rendering, bloom and compositing |
| [`gpu/tools/`](../gpu/tools/) | Capture tools, tray helper and drum-labeling tools |
| [`packaging/`](../packaging/) | User installer and desktop integration checks |

## README screenshots

```sh
python3 gpu/tools/capture-gallery.py           # all nine images
python3 gpu/tools/capture-gallery.py hero aurora  # selected images
```

The capture script uses the actual release app, a deterministic synthetic soundtrack and
fixed scene/look/time combinations. It opens temporary windows, saves screenshots directly
to `docs/images/`, and never changes your audio routing or saved settings. A graphical session
and a release build are required. Images are unretouched application captures. The fireworks
shot uses Gold Willow with sensitivity 1.7 and density 1.3; these are normal in-app controls.

## Synthetic audio and rendering diagnostics

Run the remaining commands from `gpu/`:

```sh
cd gpu
```

**Developing without audio routing.** `GEQ_SYNTH` replaces the capture with deterministic synthetic
audio: no EQ engine, no routing changes, settings are read-only.

    GEQ_SYNTH=1 cargo run --release              # a looping 64 s groove (demo)
    GEQ_SYNTH=impulse|percussion|tones|sweep|dense|stereo|phase|quiet|silence cargo run --release
    GEQ_REPLAY=clip.wav cargo run --release      # loop a WAV file (PCM or float, any rate)

    # a deterministic 6 s screenshot of the waterfall at 32:9, without going full screen
    GEQ_SYNTH=1 GEQ_SETTINGS=default GEQ_SCENE=1 GEQ_BARE=1 GEQ_SIZE=2560x720 GEQ_SHOT=/tmp/t.png,6 cargo run --release
    # frame-time statistics and per-pass GPU time at 5120x1440
    GEQ_SYNTH=1 GEQ_SCENE=1 GEQ_FPS=1 GEQ_GPUTIME=1 GEQ_FULLSCREEN=1 cargo run --release

A windowless check of any scene on the real GPU (no window, so it works with the monitor off; a synthetic signal, so it
measures the scene's own cost, not frame pacing) saves the final composite as a PNG and prints the GPU time of each pass:

    GEQ_HL_SCENE=11 GEQ_HL_SIZE=5120x1440 GEQ_HL_SECS=8 GEQ_HL_OUT=/tmp/pond.png \
        cargo test --release headless_scene -- --ignored --nocapture   # GEQ_HL_SCENE2=n crossfades a second scene

`GEQ_SHOT` with a synthetic source steps a simulated clock, so captures repeat exactly. Other hooks:
`GEQ_PRESET`, `GEQ_THEME`, `GEQ_BLOOM`, `GEQ_MOTION`, `GEQ_FLASH`, `GEQ_STEREO`, `GEQ_ORBIT`,
`GEQ_PARTDEBUG=1` (prints where the fountain's live particles are relative to the floor), `GEQ_FWDEBUG=1` (one line per firework hit and burst: its time, kind, strength and position), `GEQ_LTDEBUG=1` (one line per lightning strike: its time, place, strength, strokes and branches), `GEQ_RPDEBUG=1` (one line per pond impulse: a crater or a landing), `GEQ_RIPPLE_LIFE=n` (how long a ripple lives, 0.4-2.5),
`GEQ_GRADIENT=n` (fountain spark sensitivity), `GEQ_DENSITY=n` (fountain particle density), `GEQ_SPARKS=0` (columns without sparks),
`GEQ_SPIKES=n` (spike height of the ferrofluid orb, 0.3-1.5),
`GEQ_CURTAINS=n` (most curtains of the aurora, 2-6), `GEQ_AURORA_SCALE=n` (the aurora's render scale, 0.25-1; default 0.5),
`GEQ_CYCLE=secs` (auto-cycle scenes), `GEQ_LATENCY=1` (onset and drum-hit delivery delay: one line per drum hit with its kind, confidence and strength), `GEQ_BROWSER=1`,
`GEQ_VIEW=scene|bloom|nan` (composite debug views). `tools/make-thumbs.sh` regenerates the look
thumbnails from real captures. `cargo test` covers the synthetic source, onset detection, and the
modulation layer; `cargo test -- --ignored live_capture` smoke-tests the live `parec` path.
