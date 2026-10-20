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
It does not make the virtual output the default: your default output, and so the volume keys and the
system volume, stay where they are, and each playback stream is moved onto the EQ as it appears (and back on quit).

    cd gpu && cargo run --release

Needs Rust (rustup) and a Vulkan-capable GPU. It holds 120 fps at 5120x1440 on a Radeon RX 580.

**Scenes**, each with curated *looks* (palette, exposure, bloom, motion):

- **3D columns** - segmented LED cores in glass housings on a glossy floor. Optional **Sparks** (Display
  menu, on by default): each lit column sheds embers from the top of its strip in proportion to its
  level, and every drum hit throws a fan of sparks from the column at its place on the frequency axis (kicks
  from the bass columns, snares from the middle, hats only when accented), bigger the harder the hit; they arc and
  skitter on the floor.
- **Fountain** - sparks fly where the volume is rising. Every frequency reports how steeply its level is
  climbing right now, and a line of emitters along a glass floor throws sparks from the frequencies that
  are rising fastest: bass on the left, treble on the right. A kick is a steep rise in the lows, so it
  throws sparks on the left; a snare is a steep rise up high, so it throws them on the right; steady sound
  throws none. The steeper the rise, the more sparks and the bigger, brighter and higher they fly. Sparks
  arc, bounce on the floor and are mirrored in it. A rise is judged against the music's own hardest recent
  hits, so one setting works across recordings and volumes. Display menu: **sensitivity** sets how close
  to those a rise must get (turn it up for more sparks, down for only the hardest hits) and **density**
  scales the count.
- **Waterfall** - the spectrum history as a wireframe terrain (mixed or stereo overlay), inside glass
  walls on all four sides. The front wall carries the live spectrum line; each side wall carries the
  terrain's edge and the back wall the oldest row, so their lines follow where the waterfall meets the
  glass. It stands on a glass floor under a night sky: stars, a nebula band, a ringed planet, a small
  planet and moons, all tinted from the theme. Bass hits flare the stars and planet atmospheres and send
  a shockwave across the sky, and the floor mirrors all of it, along with the terrain and walls. A grid
  is etched into the surface at zero level, and the walls cast a little light onto the glass, with
  sheen from overhead light bars.

- **Ferrofluid Orb** - a glossy black sphere floating over a polished floor, with broad elastic lobes under
  192 fine spikes. The spikes sit on a spiral from the bottom of the sphere to the top, one per frequency of the
  spectrum, so a rising sweep climbs the orb; each is a spring that its frequency's level pulls up and that a hit
  throws out and lets settle. Eleven soft lobes, one per EQ band, bulge with their band's level (the volume stays
  constant: a louder band swells while a quieter one sinks) and wobble on onsets, and sustained bass makes the whole
  orb breathe. The surface is a black mirror: what it shows is a studio of strip lights in the theme's colours (a
  long one overhead, two tall ones at the sides, two behind it for the rim) that slowly drifts around it. The floor
  mirrors the orb and carries the ring of eleven LEDs it floats over. Each LED anchors a curved floor spike with a rounded tip,
  bass at the left and treble at the right, whose height follows its band's level with the same attack and fall
  as the 3D columns. The floor mirrors these spikes too; bass hits send a ripple out from the ring. In silence
  the floor spikes retract and the orb settles into a calm glass
  ball. Display menu: **spikes** (a smooth lobed blob to a hedgehog); **Motion** is how elastic the fluid is (spikes
  thrown out by hits, wobbling lobes, breathing, the lights' drift; 0 gives a fluid that only follows the loudness
  in still light), **Flash** is the pulse of the lights and the glow of the spike tips.

- **Aurora** - broad translucent curtains of light standing over a dark lake, under stars and behind a low
  treeline. Six vertical sheets (fewer on a narrower picture: five at 21:9, four at 16:9) stand at different
  distances; screen x is frequency on every one (bass at the left). The nearest shows the newest spectrum and
  each farther one a little older, so a hit appears near and echoes back into the distance. Each column's height
  follows its frequency's level, the mids fold the sheets in depth (a fold turned edge-on to you shows as a bright
  pleat), the sharp lower edges burn brightest, and a hit runs a small highlight up the rays where its energy
  was. The slow energy trend spreads the stack apart; in silence the curtains settle to thin, dim lines. The lake
  mirrors the sky and ripples with the bass. The curtains are drawn at half resolution (about 0.4 ms at 5120x1440)
  and the stars, treeline and lake at full resolution. Display menu: **curtains** (the most standing in the sky,
  2-6); **Motion** is how much the curtains fold and drift and how the lake stirs (0 holds the folds still; the
  columns still follow the music), **Flash** is the highlights that run up the rays.

- **Fireworks** - rockets climb a dark sky while the music is loud, and a hit sets one off. Sustained energy
  launches rockets from pads along a line (bass left, treble right, scattered in depth); a drum hit (see
  **Drum detection** below) detonates the
  rocket nearest its place on the line, at once and at the height it has reached, and with none near a mortar pops
  low, so a hit is never unanswered. A rocket nobody sets off bursts at its apex, small and dim. Where the hit lands
  sets the kind of burst: lows throw a **willow** (big, slow, heavy trails that droop to the floor), mids a
  **peony** (an even sphere), highs a **crackle** (small, fast, twinkling). A harder hit is bigger, brighter and
  throws more stars, and every star takes its colour from a frequency of the spectrum at the moment of
  detonation, so a bassy hit throws warm stars and a bright one cool ones. The stars hang for three or four seconds,
  cool toward ember orange and fall onto a glass floor that mirrors them and takes a pool of light under every
  live burst. The sky is never full: a hit beyond seven live bursts is dropped and stars thin out as more are
  alive. Silence stops launches, rockets in the air fizzle out and the stars expire. Display menu: **sensitivity**
  and **density** (as for the Fountain; density scales the stars in each burst); **Motion** is how far the stars
  drift on the wind and how far the camera wanders, **Flash** is the detonation flash and the light the bursts throw
  on the floor (0 leaves bare stars).

- **Lightning Storm** - each hit strikes the glass floor as a bolt of branching light, where it landed on the line
  (kicks on the left, snares in the middle, hats on the right; the same drum detector as the Fireworks, so **sensitivity** means the same).
  Every bolt is built on the GPU from its own seed by midpoint displacement and drawn as thin glowing lines that are
  anti-aliased analytically: the first stroke runs down the channel in about 20 ms, one to three return strokes
  flicker (a harder hit flickers more), and a dim channel stays behind for a third of a second. Where the hit landed
  sets the character: lows strike thick with few branches, from the clouds above the frame, and send a **thunder ring**
  out across the floor; highs are thin, many-branched and short. The strike throws a pool of light on the glass and a
  splash of sparks, and every bolt is mirrored in the floor. Between strikes faint discharges flicker on the spectrum
  line and grow with the level. No part of the line flashes more than three times a second: a strike that finds three
  full ones already within a fifth of the line in the last second is drawn as a dim ghost, with no flash on the floor.
  Silence strikes nothing and what is up fades out. Display menu: **sensitivity** and **density** (density scales
  the branches and sparks); **Motion** is how far the camera wanders, **Flash** is the strokes' brightness and the
  light and thunder ring on the floor (0 leaves only a dim channel).

- **Ripple Pond** - a glossy dark pool that keeps a record of the last few seconds of hits. A drum hit (the same detector as
  the Fireworks and the storm) breaks the water at once where it landed on the line, bass at the left, a broad deep
  crater for the lows and a small quick one for the highs, and throws a few droplets in arcs; each droplet that comes
  back down drops a ring where it lands (a harder hit throws more of them). The surface is a damped wave equation on a
  grid of cells 1/20 of a unit wide, stepped at 240 Hz by a fragment pass per step, with a sponge around its edge so
  rings leave instead of coming back, so rings cross and interfere and the pool is a living picture of what was
  just played. Splashes are broader and deeper, and the pond accepts softer drum hits for more frequent rings.
  The water is a mirror: it shows a faint haze along the horizon and eleven light poles beyond the far shore,
  one per EQ band (bass at the left, highs at the right). Each lights from the bottom up to its band’s level,
  using the same rise and fall as the 3D columns, and flares on onsets, so every passing
  ring bends their reflections and sweeps highlights over the pool; the droplets are mirrored too, bent by the ripples
  under them. The slope the water is shaded from is mipmapped and read anisotropically, so a thin ring far away is
  averaged over the pixel's footprint instead of aliasing. In silence the rings damp out quickly and leave calm glass with slowly drifting reflections. Display
  menu: **sensitivity** and **density** (density scales the droplets), **ripple life** (how long a ring lives: short
  is a quick clean pond, long a surface crowded with hits); **Motion** is how far the towers drift and the camera
  wanders (0 holds the reflections still; the water still follows the music), **Flash** is the towers' flares and the
  droplets' glow.

**Drum detection.** The Sparks, Fireworks, Lightning Storm and Ripple Pond react to *drum hits*, not to levels at
frequencies. `src/drums.rs` runs in the analyzer thread on the same 1024-point spectrum as the onset envelopes, one
frame every 10 ms: sixteen frequency bands described by their level against a slow average over the last four frames,
how many of their bins rose at once and a median of the rise across neighbouring frequencies (a drum lifts a whole
stretch of the spectrum, a note only its harmonics), and a loudness follower that makes it work at any volume. Five
small neural networks (one hidden layer of 48 units each, weights in `src/drum_model.bin`) are averaged into three
probabilities: **low** (kick, tom), **snare** (snare, clap, rim) and **high** (hat, cymbal); a picker reports each
hit once, about 20 ms after the drum, with a place on the frequency line, a strength and a confidence. The scenes see
hits whose confidence is at least 0.9 at Sensitivity 1 (0.76 for the more responsive Ripple Pond); the Display menu's **sensitivity** lowers or raises that (down
to 0.7) and the fountain still uses it as its rise threshold. The networks are trained on audio rendered from the system's
General MIDI soundfont with known drum times (randomized kits, tempos, instruments, mixes, noise, distortion, speech and
volumes; `src/corpus.rs`, `src/drum_train.rs`). On held-out songs it finds about three quarters of the kicks, snares and hats
and 97% of cymbals with 96% precision, against 31% recall for the rise detector it replaced; it still misses most toms,
click-less sub-bass kicks and fast double-bass, and fires a few times a minute on music without drums. It has not been
measured on real recorded drums (see `gpu/VISUALIZATION_PLAN.md`, Milestone 11, for the numbers and the gaps). Commands:

    cargo test --release dump_corpus -- --ignored --nocapture       # writes the labelled test songs as WAVs
    cargo test --release train_drums -- --ignored --nocapture       # retrains (about 12 min); GEQ_WRITE_MODEL=1 installs
    cargo test --release eval_builtin -- --ignored --nocapture      # scores the built-in model against the old detector
    GEQ_REAL_RAW=clip.raw cargo test --release real_music -- --ignored --nocapture   # a capture: hits, rhythm, onset image
    # clip.raw: parec --device=@DEFAULT_MONITOR@ --format=float32le --rate=48000 --channels=2 > clip.raw

**Labelling real songs.** To measure the drum detector on real drums, `gpu/tools/labeler.html` (a single page, open it in a
browser) shows a recording as an *onset image* (a drum is a vertical bar), plays it at reduced speed and lets you tap each
drum type along with it; taps are corrected for your latency and snapped to the sharpest onset of that drum's frequency band, so
the labels carry the sound's timing, not your finger's. It can also show what the detector heard, for review. Capture with
`gpu/tools/capture-song.sh`, then `cargo test --release propose_labels` / `eval_real` (`-- --ignored --nocapture`); labels live
in `gpu/labels/` (the audio is not committed). See `gpu/labels/README.md`.

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
`GEQ_PARTDEBUG=1` (prints where the fountain's live particles are relative to the floor), `GEQ_FWDEBUG=1` (one line per firework hit and burst: its time, kind, strength and where it came from), `GEQ_LTDEBUG=1` (one line per lightning strike: its time, place, strength, strokes and branches), `GEQ_RPDEBUG=1` (one line per pond impulse: a crater or a landing), `GEQ_RIPPLE_LIFE=n` (how long a ripple lives, 0.4-2.5),
`GEQ_GRADIENT=n` (fountain spark sensitivity), `GEQ_DENSITY=n` (fountain particle density), `GEQ_SPARKS=0` (columns without sparks),
`GEQ_SPIKES=n` (spike height of the ferrofluid orb, 0.3-1.5),
`GEQ_CURTAINS=n` (most curtains of the aurora, 2-6), `GEQ_AURORA_SCALE=n` (the aurora's render scale, 0.25-1; default 0.5),
`GEQ_CYCLE=secs` (auto-cycle scenes), `GEQ_LATENCY=1` (onset and drum-hit delivery delay: one line per drum hit with its kind, confidence and strength), `GEQ_BROWSER=1`,
`GEQ_VIEW=scene|bloom|nan` (composite debug views). `tools/make-thumbs.sh` regenerates the look
thumbnails from real captures. `cargo test` covers the synthetic source, onset detection, and the
modulation layer; `cargo test -- --ignored live_capture` smoke-tests the live `parec` path.
