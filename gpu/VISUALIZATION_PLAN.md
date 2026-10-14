# GPU analyzer — visual direction and implementation plan

Written 2026-10-11 against commit `a3579ea`. Sized for the machine it runs on: Radeon RX 580 8 GB
(RADV, Vulkan 1.4) driving a 5120×1440 display at 120 Hz.

## Recommendation

Build a polished first showcase, then use it to set the standard for the remaining scenes. Visual
direction, musical motion, lighting, and presentation are deliverables in every milestone. Scene
count is not the acceptance criterion.

The first milestone is **Spectrum Tunnel, improved lighting on the existing columns, fast onset
response, and a finished fullscreen presentation**. Reuse the existing history and mesh plumbing,
but budget explicitly for composition, material treatment, motion tuning, and visual review.

The initial showcase should grow toward:

1. **Spectrum Tunnel** — the first flagship: layered depth, sharp luminous structure, and localized
   onset rings traveling through the spectrum history.
2. **Smoke Jets** — eleven plumes tied to the EQ bands, with bright injection points and darker,
   translucent curls. Prototype its cost early before committing to the final simulation quality.
3. **Aurora prototype** — promote this from the end of the backlog. A few carefully shaped curtains
   may make better use of 32:9 than many overlapping layers. Its scope depends on the prototype.

Phosphor Vectorscope supplies a quieter instrument-like alternative. Stereo Stage and Feedback Warp
follow its shared plumbing; Ferrofluid, Fountain, and Chladni follow once the showcase is polished.

| # | Scene | What you see | New plumbing | GPU cost | Size |
|---|-------|--------------|--------------|----------|------|
| 1 | Spectrum Tunnel | Fly through layered spectrum history and traveling onset rings | existing history, fast envelopes | waterfall baseline plus effects; measure | S base / M showcase |
| 2 | Phosphor Vectorscope | Analogue X/Y scope, sharp beam and fading phosphor | waveform feed, persistence | sparse geometry plus persistence passes | M |
| 3 | Stereo Stage | Channel balance × frequency map, with phase accents and trails | per-bin stereo data, persistence | sparse sprites plus persistence passes | M |
| 4 | Feedback Warp | Swirling, kaleidoscopic waveform trails | same as 2, onsets | feedback sampling plus beam and composite | M |
| 5 | Smoke Jets | Eleven lit, translucent plumes, one per band | fluid passes, non-MSAA path | benchmark solver, dye and shading separately | L |
| 6 | Sparks / Fountain | Sparse embers off columns; spectrum-driven particle arcs | compute particles | depends on count, size and overlap | L, then S |
| 7 | Chladni Sand | Grains forming and dissolving figures on vibrating plates | reuses 6 | simulation and grain coverage; measure | M |
| 8 | Ferrofluid Orb | Glossy elastic lobes with frequency-driven fine spikes | offscreen height map and material shader | benchmark height map and displaced mesh | M |
| 9 | Aurora | A few broad, layered curtains of spectrum light | history mesh, render scale, translucency | prototype at reduced resolution | M prototype |
| 10 | Fireworks | Shells climb while the music is loud; a hit detonates one, and its sparks carry the hit's spectrum | reuses 6 (particles); shell table, per-spark trail length | sparse additive sprites, fill-bound at burst cores; measure | M |
| 11 | Lightning Storm | A hit strikes the glass floor as a branching bolt where it lands, with return strokes and a thunder ring | procedural bolt geometry; hit detector shared with 10 | a few hundred thin analytic quads; measure | M |
| 12 | Ripple Pond | Droplets fall on a glossy pool; each leaves rings that cross and interfere, a record of the last few seconds of hits | wave-equation height field on the smoke's pass helper | one fragment pass per fixed step plus a lit floor; measure | M |

Size: **S** = a new shader mapping on plumbing that exists; **M** = new shaders plus one new data
path; **L** = a new subsystem. These sizes describe implementation structure, not art-direction or
polish effort. Costs are hypotheses until measured; `GEQ_FPS=1` reports throughput, and the new
frame-time diagnostics below must establish pacing and headroom. Proposal numbers above are not
persisted scene IDs: preserve IDs 0/1 for columns/waterfall and reserve ID 2 for Tunnel.

## What the app does today

`parec` → 8192-point FFT every ~10 ms ([analyzer.rs](src/analyzer.rs)) → 11 band levels and three
192-point log-frequency curves (mixed, left, right) → smoothing and peak hold in
[app.rs](src/app.rs) → uniforms plus one row of an `Rg8Unorm` 192×256 history texture, 60 rows a
second → one 4×MSAA HDR pass → two-level bloom → ACES composite ([render.rs](src/render.rs),
[scene.wgsl](src/scene.wgsl), [post.wgsl](src/post.wgsl)).

Keep the HDR rendering foundation, procedural geometry from `vertex_index`, the ring-buffer history
texture, and the `GEQ_*` environment hooks. Refine bloom, lighting, and materials as part of the
visual direction rather than treating their current appearance as fixed.

Five things stand between this and more ambitious scenes:

1. **The GPU never sees the waveform.** Only smoothed spectra are uploaded, so nothing can draw a
   scope or show phase.
2. **Nothing reacts to transients.** The 8192-sample Hann window is 171 ms long, which smears a kick
   drum across many frames. The only envelope, `bass` (computed in `App::step_levels`), is still
   uploaded every frame, as is `time`, but no shader has read either since the background glow was
   removed.
3. **Every scene frame starts from black.** Spectrum history persists, but the rendered scene image
   does not, so there is no image persistence for trails, feedback or simulation.
4. **There are no compute passes**, so no particles. Nothing blocks them: the device eframe creates
   uses default limits and no optional features, and that is enough for compute and storage buffers.
5. **The scene switch is hard-coded**: a `mode` float, an `if p.mode == 0` in `prepare`, one
   `scene.wgsl`. That is fine for two scenes and unworkable for ten.

One smaller observation: the bloom is narrow for this display. It has two levels (½ and ¼
resolution), which reach about ±26 pixels on a screen 5120 wide. Your saved settings run Glow at 1.9
out of 2.0, making wider bloom worth testing. Its strength must be tuned independently of width so
the broad halo does not bury sharp detail (F5 below).

## Hardware budget

- A full-screen pass at 5120×1440 touches 7.4 million pixels; at 120 Hz that is 0.9 gigapixels a
  second per pass. Pixel count alone does not establish cost: texture reads, shader work, blending,
  bandwidth, and pass dependencies all matter. Benchmark expensive subsystems before polishing them.
- 120 Hz gives an **8.33 ms frame interval**. Record median, p95, p99, and worst frame times, with
  startup, steady playback, resizing, and transitions reported separately. Keep measured headroom
  for presentation and scene changes rather than targeting an average of exactly 120 FPS.
- No ray marching at full resolution. Anything that would march either renders to a half-resolution
  target or is built as a rasterised, displaced mesh instead (see the Ferrofluid Orb).
- Polaris has no hardware ray tracing and no mesh shaders, so nothing here uses them.
- 8 GB of VRAM leaves room for these techniques, but count all active targets, simulation buffers,
  and both sides of a transition. The current MSAA colour and depth targets are about 350 MB at
  full screen. Use a bounded cache for inactive scenes; lazy creation alone does not bound memory.
- The existing scenes hold 120 fps at 4800×1300 (per the README), which is the baseline to protect.

## Foundation work

### F0 — Art direction and motion studies

The app should feel like a luminous instrument: dark panels, precise detail, controlled reflections,
and deliberate empty space. Keep the field dark, while allowing localized lighting and atmospheric
depth around the scene. Phosphor, smoke, metal, sand, and LEDs each need a distinct material treatment.

Before polishing a flagship scene, produce a target image and a short motion study. The image may
be a mockup; the motion study must exercise the intended audio response in a simple prototype. Each
scene brief must specify:

- **Composition:** focal point, foreground/middle/distance, and default framing at 32:9 and 16:9.
  Use the ultrawide edges deliberately; preserve round scopes and readable silhouettes when narrow.
- **Color:** dominant, supporting, and transient accent colors. A theme supplies these roles; each
  material interprets them separately. Rainbow is an explicit alternative, not a universal default.
- **Lighting:** sharp core, narrow halo, broad glow, reflective or translucent surfaces, and the
  small regions allowed to reach the brightest values.
- **Motion:** which audio features drive each element, their response times, bounded camera motion,
  and how the scene settles during silence. Quiet, dense, and sustained music each need a treatment.
- **Controls:** a small set of meaningful scene parameters and at least one curated visual preset.

Review the scene against this brief in motion as well as in screenshots. Improving the current
columns is part of this work: separate LED faces from their housings, soften the floor reflection,
preserve segment gaps, and reserve sparse peak-triggered sparks for the later particle milestone.

### F1 — Scene modules

Move each scene into its own Rust module and `.wgsl` file under `src/scenes/`, with a shared
`common.wgsl` (the `U` struct, `palette`, `hsv`, `gridline`) concatenated in front of each at load
time — the same string approach `Gpu::new` already uses to patch `NX`. The drawing interface can stay
small; this sketch covers rendering, with lifecycle requirements listed below:

```rust
trait Scene {
    /// Offscreen work before the main pass: simulation steps, compute, persistence.
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame);
    /// Draw into the HDR scene pass.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, f: &Frame);
    /// 2D scenes anti-alias analytically and skip the 4x MSAA target.
    fn wants_msaa(&self) -> bool;
}
```

Give the scene manager explicit prepare/prewarm, activate/reset, resize, and release operations.
Prepare the incoming scene before starting a transition, retaining the active scene until it is
ready. Measure compilation and allocation stalls; do not assume lazy creation is hitch-free. Reset
must accept a deterministic seed for captures, and resizing must deliberately preserve or reset
persistence and simulation state.

The compositor owns scene output targets, transitions, bloom, and tone mapping. Support separate
resolved HDR outputs for two live scenes during a crossfade, with a frozen outgoing image as a
lower-cost fallback when running both would exceed the budget. Crossfade in linear HDR before a
single final tone map. Release inactive resources according to a bounded cache policy.

`Settings::scene` stays a `u32` with 0 and 1 keeping their meaning. Give each scene stable metadata:
name, camera limits, quality options, default visual preset, and controls. The Display menu shows
only the active scene's options. Keep visual presets separate from EQ presets and gains.

### F2 — Audio features

Extract features in the analyzer thread and timestamp them at the source. Measure the extra work
alongside the existing FFTs. Fast onsets and slow energy envelopes ship in the first milestone.

- **(a) Waveform feed.** Hand the UI thread every new stereo sample exactly once, and write them
  into an `Rg32Float` 4096×1 ring texture with a `head` uniform — the same idiom as the history
  texture. Shaders read it with `textureLoad`, so no float-filtering feature is needed.
- **(b) Onset envelopes.** A second, 1024-point FFT (21 ms window) on the newest samples, reduced to
  three low/mid/high spectral-flux envelopes with fast attack and time-based exponential decay.
  They go in a new `fx: vec4` uniform. These are frequency-region events, not instrument detection.
  The window centers differ by roughly 75 ms, but actual response latency depends on the signal,
  capture and presentation. Measure it with impulses; use the fast events for accents and the long
  spectrum for sustained geometry so a hit does not accidentally appear as two unrelated events.
- **(c) Stereo correlation**, −1 to +1, over the latest hop; the fourth component of `fx`.
- **(d) Per-bin pan frame** for Stereo Stage, described under that scene.
- **(e) Synthetic source and replay.** A `GEQ_SYNTH=1` mode feeds a deterministic sweep/percussion
  sequence in place of `parec` and does not start the EQ engine or change routing. Add selectable
  fixtures for silence, quiet input, impulses, sparse percussion, sustained tones, sweeps, dense
  mixtures, and stereo movement/phase behavior.
  Add local audio replay for representative musical passages. Fix seeds and audio/simulation time
  for captures; use real-time execution for performance measurements. Today even `GEQ_SHOT` starts
  the engine and re-routes the default sink, so this is an early prerequisite for visual iteration.
- **(f) Slow energy and visual normalization.** Supply sustained low/mid/high envelopes and a slower
  overall energy trend. Use bounded adaptive normalization with a silence gate for artistic motion,
  so quiet recordings remain expressive without making noise animate the scene. Preserve the
  calibrated analyzer levels and EQ gains; normalization never changes the audio or measurement scale.
- **(g) Optional beat timing, later.** Add beat phase/confidence only if transition and choreography
  experiments justify it. Fall back to low-energy moments or ordinary timed transitions when
  confidence is low; do not make beat detection a prerequisite for the first release.

Use a shared modulation layer with separate response times and per-scene mappings:

| Signal | Visual role | Example |
|--------|-------------|---------|
| Sustained bass | Broad deformation and momentum | Tunnel wall displacement; plume strength |
| Sharp onset | Localized, brief event | Traveling ring; puff; sparse burst |
| Midrange energy | Surface movement | Turbulence or elastic lobes |
| High-frequency transient | Fine detail | Brief highlights or small sparks |
| Slow energy trend | Gradual evolution | Density, lighting, or restrained camera framing |

Provide independent **Motion intensity** and **Flash intensity** controls. Camera motion is bounded
and can be disabled; chromatic aberration, if offered, is an optional per-preset lens effect. Use
frame-rate-independent smoothing throughout and define gate/decay behavior on silence.

Keep `Levels` small. Bulk data (samples, the pan frame) should travel in its own slot that the UI
thread swaps out, rather than being cloned under the mutex every frame.

### F3 — Persistence buffer

Two `Rgba16Float` textures at scene resolution, not multisampled, used alternately: each frame reads
last frame's image, fades or warps it into the other texture, and draws the new marks on top. A
final full-screen triangle copies the result into the existing HDR target, so bloom and tone mapping
work unchanged. Scenes 2, 3 and 4 all sit on this.

### F4 — Compute particles

One storage buffer of particles (position, velocity, life, band — 32 bytes each), one compute pass
per frame encoded in `prepare` ahead of the scene pass, and a draw of `0..6` vertices × `N`
instances whose vertex shader reads the particle by `instance_index` and billboards it. That keeps
the no-vertex-buffer idiom. Start at 262,144 particles (8 MB) and treat the count as a quality
setting; measure before raising it toward a million. Treat this as a capacity, not a requirement to
make every particle visible. Start column sparks sparsely and benchmark sprite coverage and overlap
as well as count. Bright events need room around them.

### F5 — Lighting, color, and output path

- **Non-MSAA path.** 2D scenes (2, 3, 4, 5) should render straight to the resolved HDR texture with
  single-sample pipelines; multisampling a full-screen quad is wasted work.
- **Layered bloom.** Replace the two-level chain with a five- or six-level downsample/upsample chain.
  Keep a sharp luminous core, an independently tuned narrow halo, and a faint broad glow. Separate
  radius from strength and tune the level weights per visual preset. The extra levels are small,
  but measure the full chain instead of assuming equal cost. Check moving fine detail for shimmer.
- **Exposure and highlights.** Give each scene controlled exposure and emission ranges. Preserve
  saturated color and material detail; reserve the brightest values for small regions and musical
  accents. Avoid rapid exposure adaptation that pumps with every beat. Check transitions between
  presets with different brightness so the blend does not wash out.
- **Render scale.** Let `ensure_targets` size the scene target at a fraction of the window, for the
  scenes that need it. The composite pass already samples with a linear filter. Keep UI at native
  resolution, separate simulation resolution from display resolution, and inspect fine lines and
  silhouettes at every supported quality setting.

### F6 — Presentation, presets, and transitions

Presentation starts with the first scene. Extend the existing fullscreen mode and controls rather
than deferring this work until the scene catalog is complete.

- **Visual presets.** Curated combinations of camera, palette, material/exposure, bloom, and motion.
  Add a thumbnail browser, favorites, and save/restore of user variants. Visual preset selection must
  not change EQ gains, bypass, or routing. Store thumbnails; do not run every scene to show previews.
- **EQ strip.** Refine typography, spacing, zero marks, and subtle band accents. Match a scene's band
  positions to actual layout coordinates when appropriate; keep settings and gain values legible.
- **Fullscreen controls.** A compact overlay appears on interaction and fades after inactivity.
  Include scene/preset selection and the two intensity controls. Keep `F`, `F11`, and `Esc`; arrows
  step through scenes, and `1`–`9` select visible/favorite slots without redefining stable scene IDs.
- **Camera.** Author an idle path and bounds per scene. Idle movement is optional, pauses during
  interaction, and resumes smoothly. Scope and Stereo Stage retain stable analytical framing.
- **Transitions.** Begin with controlled crossfades and deliberate preset brightness matching.
  Prewarm the next scene using F1. Budget both live outputs or use the frozen-image fallback. Later,
  optionally time changes to low-energy moments or confident beat boundaries. Fade-through-black
  remains an artistic option rather than the universal transition.
- **Auto-cycle.** Optional, uses favorites, and respects user interaction. Add after manual changes
  are smooth; selecting the next scene must not hitch or reset the current scene prematurely.

## The scenes

### 1. Spectrum Tunnel — S base, M showcase

You fly down a tube whose wall is the last three seconds of spectrum. Bass runs along the bottom,
treble along the top, the left channel forms the left wall and the right channel the right wall, so
a panned sound visibly travels down one side.

**Visual treatment:** sharp nearby ribs, a quieter middle distance, and a softened vanishing point.
Keep the focal region readable at 32:9 and reframe it at 16:9. Sustained energy deforms the walls;
low-frequency onsets launch discrete luminous rings down the tunnel; high-frequency accents add
small highlights. Let the structure and camera settle during silence.

- A copy of `vs_ter` with a different mapping. Instead of `(x, h·TER_H, −z)` it emits
  `((R − h·D)·cos θ, (R − h·D)·sin θ, −z)`. θ runs from −90° up to +90° on the right wall sampling
  `hist.g`, and from −90° round to −270° on the left wall sampling `hist.r`, both from bin 0 at the
  bottom to the last bin at the top, so the two halves meet at both seams.
- The history rows already carry left in R and right in G when stereo is on, but only while
  `cfg.scene == 1`; widen that condition.
- Reuse `fs_ter` for the base prototype, then give the tunnel its own material treatment. Its depth
  fade provides the starting point for localized distance fog, without lighting the whole screen.
- A third preset in `camera()`. Its `min_el` clamp assumes a floor at y = 0, so skip the clamp for
  this scene.
- Use `u.t` for slow, bounded banking with a stable default horizon. Motion intensity controls the
  amplitude; zero disables camera movement. Avoid continuous uncontrolled roll.
- Keep a small timestamped list of onset rings; derive their position and decay from elapsed time.
  Tune the ring brightness separately from wall emission so overlapping hits preserve structure.

Each wall gets half the mesh columns, so either sample every second bin or double the column count;
the mesh is about 219k vertices today. Measure the doubled version together with the final shading.
The showcase needs F2b/F2f, F5, and F6 as well as the base history mapping.

### 2. Phosphor Vectorscope — M

The raw stereo signal drawn the way an analogue X/Y scope draws it. Mono collapses to a vertical
line, wide stereo opens into a ball, out-of-phase material tips over toward horizontal. The beam
behaves like real phosphor: bright where it dwells, faint where it races, with a fading trail. The
left and right waveforms run across the outer thirds of the ultrawide.

**Visual treatment:** a sharp beam core inside a soft phosphor halo, a subtle fixed graticule, and
carefully framed side waveforms. Keep the central scope circular. Reflow or simplify side waveforms
in narrower windows; preserve a faint frame/graticule when the input becomes silent.

- Each pair of consecutive samples is one instanced quad. The vertex shader loads samples `i` and
  `i+1` from the waveform ring and stretches the quad over that segment plus a beam radius.
- The fragment shader evaluates the closed-form integral of a Gaussian spot swept along the segment
  (it needs `erf`; WGSL has none, so use a polynomial approximation) and divides by segment length.
  This is the approach the woscope project popularised. It is analytically anti-aliased.
- Draw only the samples that arrived since the last frame, additively, into the persistence buffer,
  and multiply the buffer by `exp(−dt/τ)` each frame. That makes the picture independent of frame
  rate. τ is a "Persistence" slider.
- X = (R − L)/√2, Y = (L + R)/√2.

Needs F2a and F3. At 48 kHz and 120 FPS, expect about 400 new sample segments per frame, plus the
side-waveform draws and persistence/composite passes; lower frame rates receive more segments.

### 3. Stereo Stage — M

The whole 32:9 screen becomes a map of per-frequency channel balance. Horizontal position is the
relative left/right magnitude; vertical is log frequency, bass at the bottom. Every displayed FFT
bin contributes a glowing dot with a fading trail and a phase accent. A centered source may form a
column, hard-panned material may form a side cloud, and reverb may appear diffuse, but these are
possible patterns, not source separation or reliable instrument locations. Equal-magnitude signals
map to the center even when their phase differs.

**Visual treatment:** restrained persistence, a subtle center reference and frequency guides, and
legible dots before glow. Gate low-energy bins to avoid a field of noisy specks. Keep the same
analytical framing during quiet and dense passages; density and brightness should carry the change.

- In the analyzer, for each FFT bin from 30 Hz to 20 kHz (about 3,400 bins):
  `pan = (|R| − |L|) / (|R| + |L|)`, the bin's energy, and `cos Δφ = Re(L·conj(R)) / (|L|·|R|)`.
  `spec_l` and `spec_r` are already computed, so this is one extra loop. Guard both denominators near
  silence and suppress phase accents when either channel has insufficient energy.
- Upload the frame as a 4096×1 `Rgba32Float` texture whenever `seq` changes. Draw one additive
  sprite per bin into the persistence buffer: x from pan, y from log frequency, brightness through
  the existing `frac()` range and sensitivity mapping, colour from `palette()`, pushed toward red
  when `cos Δφ` is negative.
- The Fall Speed setting maps to the persistence time.
- Optional: a correlation bar along the bottom edge from F2c.

Needs F2d and F3; F2c supplies the optional correlation bar. Geometry is modest, but measure the
full-resolution persistence passes. Preserve the measurement mapping independently of artistic gain.

### 4. Feedback Warp — M (S once scene 2 exists)

The MilkDrop recipe. The waveform is drawn in the middle, and each frame the previous image is
pulled outward, twisted and dimmed, so the sound leaves spiralling trails behind it.

- One full-screen pass: `next(uv) = decay · prev(warp(uv))`, then scene 2's beam drawn on top.
- `warp` is a zoom about the centre (rate from the bass envelope), a rotation (a slow oscillation on
  `u.t`, nudged by the mid/hat balance), an optional N-fold mirror, and a small hue rotation.
- Three hand-tuned presets — Zoom, Swirl, Kaleidoscope — chosen in the menu.
- Do the warp in aspect-corrected coordinates so circles stay round at 32:9.
- Express zoom, rotation, and decay as rates over elapsed time. Bound feedback gain and injection
  brightness so sustained dense music does not fill the frame with white. Keep a legible waveform
  core inside the trails, and let the trails decay cleanly on silence.

Needs F2a, F2b and F3.

### 5. Smoke Jets — L

Eleven nozzles along the bottom edge, one per EQ band, each blowing coloured smoke upward. A louder
band is a stronger jet; a drum hit is a puff that rolls up and curls into its neighbours. It is a
real fluid simulation, so plumes shear, merge and shed vortices.

**Visual treatment:** bright injection points fading into darker translucent curls. Start with a
dominant and supporting color, plus a small transient accent; eleven emitters do not require eleven
unrelated hues. Leave gaps around the plume bases so their band relationship stays visible even
when the upper curls merge. Quiet passages produce delicate wisps; silence lets remaining smoke
dissipate without continued injection.

- A 2D stable-fluids solver in alternating fragment passes on a quarter-resolution grid (1280×360
  at full screen): advect velocity, inject the band forces, vorticity confinement, divergence, about
  20 Jacobi pressure iterations, subtract the pressure gradient, advect the dye. Keep the dye at
  half resolution for detail.
- Velocity in `Rg16Float`, pressure and divergence in `R16Float`, dye in `Rgba16Float`. All three
  can be rendered to and filtered without optional features.
- The existing `fs_pass` helper runs the passes. They need two input textures, which the existing
  `comp_bgl` layout (three textures, a sampler, a uniform) already provides.
- Step the simulation on a fixed timestep with an accumulator like `hist_acc`, so it behaves the
  same at 60 and 120 fps. Cap catch-up steps after a stall and define reset/recovery behavior.
- Carry density as well as color, and use a bounded optical-depth/transmittance approximation with
  directional shading from the density gradient. This is a 2D artistic lighting approximation, not
  a volumetric ray march. Establish dark curls and translucent edges before adding bloom; emission
  is strongest near the nozzles and brief onset puffs.
- Place the nozzles at the same x positions as `ui.columns(11)` so each jet stands over its slider
  in windowed mode. Bass nozzles are wide and slow, treble nozzles narrow and fast.

Benchmark velocity/pressure work, half-resolution dye, and final shading separately. Multiplying
roughly 30 passes by the quarter-resolution pixel count does not account for repeated neighbor
reads, pass dependencies, or the higher-resolution dye, and is not equivalent to two full-screen
passes. Prototype iteration count, simulation resolution, and dye resolution before committing to
quality settings. Needs F2b/F2f and the F5 non-MSAA path. The algorithmic starting point is
[GPU Gems: Fast Fluid Dynamics Simulation on the GPU](https://developer.nvidia.com/gpugems/gpugems/part-vi-beyond-triangles/chapter-38-fast-fluid-dynamics-simulation-gpu).

### 6. Sparks and Fountain — L for the particle system, then S each

**Sparks** is an overlay on the existing 3D columns: each column sheds embers from its top in
proportion to its level, and throws a burst when its peak cap jumps by five or more LED segments. **Fountain** is its
own scene: the 192-bin spectrum line launches particles upward at a speed set by the level; they arc
under gravity and bounce on the reflective floor.

- The compute pass integrates motion, adds curl-noise drift and gravity, bounces off the floor, and
  respawns dead particles at an emitter picked with probability proportional to band level, using a
  per-particle hash for randomness.
- Additive, depth-tested, no depth write — the same pipeline settings as `ter_st_pipe`.
- A peak-push condition exists as `peak_age[i] == 0.0` in `step_levels`, but it can remain true across
  frames. Generate discrete events from new peak increases/onsets and rate-limit bursts rather than
  treating every frame at the peak as a new hit.
- Sparse, bright sparks accent the columns' material and lighting improvements. Fountain separates
  broad low-frequency arcs from fine high-frequency spray. Bound emission during dense music,
  preserve negative space, and let existing particles expire when the signal gate closes.

Needs F4.

### 7. Chladni Sand — M

Three vibrating plates side by side — lows, mids, highs — covered in about half a million grains of
bright sand. Sand jumps off the parts of a plate that are moving and settles on the lines that are
still, drawing the classic Chladni figures. When the music changes the figure dissolves and a new
one forms; silence leaves the last one standing.

- The particle system from scene 6 with a 2D rule: each grain takes a random step sized by
  `|a(x, y)|` × loudness, where
  `a = Σ wᵢ · (cos(nᵢπx)·cos(mᵢπy) − cos(mᵢπx)·cos(nᵢπy))`.
- Each plate's two or three strongest bands choose `(m, n)` from a small table, higher bands
  getting higher mode numbers; the weights are the band levels. Add hysteresis and smooth mode
  transitions so small changes in the strongest band do not constantly destroy the figure.
- Three square plates fit the display (3 × 1440 < 5120). In a narrower window show one.
- Use directional glints and a darker plate surface to make grains read as sand. Limit bloom so
  fine nodal lines survive, and retain the settled figure on silence. Tune count after this treatment
  is established rather than requiring half a million visible grains at every quality setting.

### 8. Ferrofluid Orb — M

A glossy black sphere on the reflective floor with broad elastic lobes beneath fine spikes. Lay out
frequency-driven spike sites on a spiral from bass at the bottom to treble at the top, so a rising
sweep visibly climbs the orb. Strip-light reflections slide slowly across it and sustained bass
makes the broad shape breathe. Preserve its silhouette against the dark field with restrained rim
lighting; use the wide floor reflection and surrounding space as part of the composition.

- A small offscreen pass renders a 256×128 `R16Float` height map: for each direction, sum sharp
  falloffs around 192 Fibonacci-spiral sites, weighted by the curve levels. The spiral index is the
  frequency index. Add a lower-frequency lobe field with bounded, time-based elastic response;
  clamp fine displacement so dense music does not destroy the broad shape.
- The sphere mesh comes from `vertex_index` like the terrain grid (about 384×192 cells, 442k
  vertices), displaced by sampling the height map in the vertex shader with `textureSampleLevel`.
  Normals come from the height-map gradient in the fragment shader.
- Near-black base, Fresnel, and a reflected procedural environment of a few theme-coloured strip
  lights. Keep highlights sharp and bloom restrained so the surface stays glossy and readable.
- A mirrored copy under the floor with softened reflection, building on the columns' treatment.
- During silence, ease back toward a calm form while retaining slow reflections. Camera framing
  must protect the silhouette at both target aspect ratios.

Rasterisation avoids full-screen ray marching, but does not establish the frame budget. The proposed
height map alone evaluates roughly 6.3 million site contributions per update. Profile it and the
displaced mesh; precompute local site indices/weights or reduce update frequency if needed.

### 9. Aurora — M prototype, scope set by measurement

*Built, and promoted past the prototype gate: see Milestone 7 (ID 8). The text below is the original brief.*

Broad translucent curtains of light span the display, their height and folds following the spectrum
and trailing back through history. This is an early composition experiment alongside the Smoke
cost prototype, not a commitment to 24–32 full-width layers.

- Start with roughly 4–8 shaped ribbons at half resolution, using the existing history mesh as a
  starting point. Separate near, middle, and distant curtains through depth, opacity, and detail.
- Use a dominant/supporting color pair with localized bright edges. Compare ordered translucent
  compositing with restrained additive accents; overlapping curtains must retain their shapes.
- Sustained energy drives height, mids drive folds, and onsets send small traveling highlights.
  Use the slow energy trend for gradual changes in spread and lighting. On silence, ease into dim,
  slow ribbons rather than continuing bright flashes.
- Compose for 32:9 first, then reduce ribbon count/spread for 16:9 instead of merely cropping.
  Keep camera motion shallow and optional.
- Measure overlap, fill cost, and temporal stability at the intended render scale. Continue only
  if the small prototype establishes convincing depth and musical response within the frame budget.

Needs F2b/F2f and F5 render scale. Additional layers are a measured quality option, not a requirement.

### 10. Fireworks — M

*Built, ID 9: see Milestone 8 for what shipped, what was measured and where it differs from this brief. The text below is the original brief.*

Shells climb the dark sky while the music is loud, and a hit detonates one. The sparks hang, droop and fall onto
the glass floor, which lights up beneath them and mirrors them. It keeps what makes the Fountain work (rare,
discrete events, bigger for harder hits, kicks on the left and snares on the right, readable at a glance) and adds
what the Fountain cannot do: height and depth, a burst that hangs for three or four seconds, and a spectrum
carried inside every burst.

**Visual treatment:**

- **Composition:** the Fountain's dark field, glass floor and emitter line (bass left, treble right), but with the
  pads scattered about ±6 units in depth so simultaneous bursts overlap with parallax and different sizes. The
  camera sits low and aims a little above level, leaving two thirds of the frame as sky. A burst's radius scales
  with screen height, so shells stay round at 32:9, where four to six can stand side by side; at 16:9 the line
  narrows and fewer may be alive at once. Negative space is part of the design: cap simultaneous bursts so the sky
  is never full.
- **Color:** the theme ramp runs from bass to treble along the sparks' frequencies (Rainbow reads each spark's
  frequency directly). The accent role belongs to the detonation core and the brightest stars. A spark cools from
  its colour toward ember orange over its life.
- **Lighting:** a shell is a thin, sharp rising streak. A detonation is a small white-hot core, the only region
  allowed near the brightest values, with the sparks well below it. Each live burst throws a radial pool of light
  on the floor (at most eight at once, falling off with distance, fading in about 0.6 s), so lighting stays
  local and nothing pulses the whole frame. The floor mirrors the sparks as in the Fountain; there is no sky to
  reflect (the waterfall's glass no longer mirrors its sky either).

**Musical mapping:**

- **Energy launches, hits detonate.** The sustained low/mid/high envelopes (`sus`, gated) set how often shells
  leave the pads in their stretch of the line (roughly 0.4–3 a second; none while the gate is closed). A shell
  climbs for about a second, so something is always about to happen. A hit detonates, at once, the shell in flight
  nearest its position on the line, at its current height; the hit's strength sets the burst's radius, spark
  count, brightness and life. If no shell near it is high enough, a short-fuse mortar pops low, so a hit is never
  unanswered. A shell nobody detonates bursts at its apex, small and dim, so quiet music still gets delicate
  bursts. The point is timing: the flash lands on the hit, not half a second later, which is what launching a
  shell *at* each hit and waiting for its apex would do, and what the "no unrelated delayed flashes" check forbids.
- **Hits** come from the Fountain's normalized `rise` (already calibrated against real music and independent of
  level), peak-picked along the 192 bins on the CPU with a refractory time per position: one peak is one hit,
  with strength measured above the Fountain's threshold, so Sensitivity keeps its meaning. The onset rings are not
  the source because the analyzer launches none for the highs, and the rise already puts a kick on the left and a
  snare on the right. It has the same shape as `modulation::PeakHits` and gets the same kind of unit tests.
- **Position sets the type.** Lows: a willow, a big slow gold sphere with heavy drag whose long trails droop to
  the floor. Mids: a peony, an even sphere that is quicker, in theme colours. Highs: crackle, small, fast and
  bright, its stars twinkling by a per-particle hash of time. That is a flicker, not child particles; the ring
  buffer has no GPU-side spawning.
- **A burst carries the spectrum of its hit.** Each spark picks a frequency by rejection sampling on the displayed
  spectrum at the moment of detonation (the sampler in `spawn_spark`, run on level instead of rise), takes its
  colour from that frequency's place on the ramp and its speed from that level. A bassy hit throws warm sparks,
  a bright one cool ones, a broadband hit a multicolour shell, and no two bursts match. This may be subtle in a
  small burst; judge it in motion, not in stills.
- **Bounds and silence.** One detonation per position per ~120 ms, and spark counts shrink as more bursts are
  alive. Density scales the count. Silence stops launches, undetonated shells fizzle out in the air and sparks
  expire. Motion scales drift and wind; Flash scales the core and the floor light, and Flash 0 leaves bare sparks.

**Implementation:**

- `scenes/fireworks.rs`, `fireworks_sim.wgsl` and `fireworks.wgsl` on the existing `Particles` ring, built like
  `fountain.rs` (non-MSAA; a floor draw, a mirrored draw, an upright draw). A burst slot already tells each spawn
  its index within its burst, which is all a spherical burst needs: the direction from a Fibonacci spiral over that
  index, the count from the plan (up to six bursts a frame).
- The shells are plain Rust: a table of 16 closed-form ballistic arcs (position known at any time, deterministic,
  unit-tested the way `smoke.rs`'s `drive()` is). The table goes to the GPU in a small uniform of its own
  (`PartU.extra` has room for four vectors, not sixteen shells); a burst's `where` field indexes it, and the shell
  streaks and the floor's light pools read it.
- One change to the shared sprite code: the streak length is a constant (`TRAIL_S`, 0.045 s, in
  `particles_draw.wgsl`) and a willow needs about 0.3 s, so it becomes a per-particle value. A straight quad
  approximates the droop; if it reads as a polyline, fall back to a persistence buffer (F3).
- No new settings fields: Density and Sensitivity are the Fountain's `density` and `gradient`. Four looks, as for
  every scene: Gold Willow (the default), Ice Chrysanthemum, Neon Peony and Prism Finale (Rainbow).

**Cost and risks** (hypotheses until measured):

- A full-strength burst is perhaps 1,500 sparks; six live at once with 3–4 s lives is 10–20 thousand alive, far
  fewer than the Fountain at its loudest (0.3–0.8 ms), so expect 0.3–1 ms. The cost is fill, not simulation, and
  concentrates where a big burst overlaps itself while additive: start sparks small, cap the initial density, and
  measure with the dense fixture.
- Timing is the main risk. Compare detonate-in-flight against plain apex bursts with the percussion and impulse
  fixtures. The acceptance check is one burst per event, appearing within a frame or two of the rise crossing the
  threshold.
- The idea is familiar; the differentiators (hit-timed detonation, the spectrum in every burst, the long hang over
  a lit floor) have to carry it, so review stills and a short recording against the F0 brief.
- Dense music must not fill the sky, and bursts are localized light, never a whole-frame flash. The Fountain's
  sparks were once reported as passing through the floor; most firework stars burn out in the air, and those that
  land use the same floor-clipped, bouncing streaks.

Needs no new foundation: F4, F2b/F2f and F5's non-MSAA path all exist.

### 11. Lightning Storm — M

*Proposed 2026-10-13 as an alternative to Fireworks, outlined only: it needs a full F0 brief before work starts.*

The Fountain's idea with a different event: each hit strikes the glass floor as a bolt of branching light, at the
frequency where it landed (kicks on the left, snares on the right), and the floor lights up and mirrors it. A hit
is rare, brief and unmistakable, and nothing else in the catalog is drawn as sparse line art.

- **Hits** come from the same peak-picked `rise` detector as Fireworks (shared code if both are built), so
  Sensitivity means the same thing. Strength sets the bolt's thickness, branch count and number of return strokes.
- **Bolt:** a main channel from above the frame to the floor, plus a few branches, generated in the vertex shader by
  midpoint displacement from a per-hit seed (so captures repeat exactly). Segments are drawn with the Vectorscope's
  closed-form Gaussian beam, so thin bolts are analytically anti-aliased and do not need MSAA. A bolt flickers
  through one to three return strokes in about 150 ms and leaves a dim channel that fades in roughly 0.3 s.
- **Position sets the character:** lows strike thick with few branches and send a thunder ring across the floor
  (as the Tunnel's rings and the Ferrofluid's floor ripples do); highs are thin, many-branched and short. The strike
  point throws the Fountain's floor spark splash. Between strikes, faint discharges on the emitter line rise with the
  level, so the scene is not empty in sustained passages.
- **Lighting and bounds:** a radial pool of light under each strike and a mirrored bolt in the glass, never a
  whole-frame flash. Return strokes are capped so that no region of the screen flashes more than three times a
  second; Flash scales the strokes' brightness. Silence draws no new bolts and the remaining channels fade out.
- **Cost and risks (hypotheses):** a few hundred quads, so well under the Fountain's cost (expect under 0.5 ms).
  The risks are look, not cost: procedural bolts can read as a plasma-ball cliché or as flat zigzags, and a
  one-pixel bolt shimmers at 120 Hz unless its width follows the pixel footprint. Prototype the bolt shape and its
  return-stroke timing alone, in a still sequence, before building the rest.

### 12. Ripple Pond — M

*Proposed 2026-10-13 as an alternative to Fireworks, outlined only: it needs a full F0 brief before work starts.*

The Fountain's sparks fall into water instead of bouncing. Each landing sends rings across a glossy dark pool, and
because rings cross, the surface is a living record of the last few seconds of hits, the way the waterfall is a
record of the spectrum. Bass hits drop heavy and make wide, slow rings; treble makes small, quick ones.

- **Surface:** a wave-equation height field on a fixed-step grid (the smoke's accumulator, fragment-pass helper and
  `Rg16Float` ping-pong textures), with a sponge border so rings leave instead of bouncing back, and clamped
  heights so a dense passage cannot blow the field up. The pond is a perspective floor quad shaded from the height
  map's gradient; ripples are small, so it needs no displaced mesh.
- **Drops:** a hit throws a few droplets in arcs from its place on the emitter line, using the same peak-picked hit
  detector as Fireworks. The drops that make ripples are a bounded table of closed-form arcs on the CPU (the plan's
  particle buffer cannot report landings back), 64 at most; a decorative spray can use `Particles`. A landing adds
  an impulse whose size and width follow the drop. Strength sets the number and weight of the drops.
- **Look:** a near-black surface that reflects a few strip lights in the theme's colours (reuse the Ferrofluid
  studio), so a passing ring sweeps highlights across the pool. Mirror the droplets in the water, distorted by the
  local slope. Bloom stays restrained, or the fine rings smear.
- **Silence:** rings damp out and leave a calm glass surface with slow reflections, consistent with the other
  scenes' floors.
- **Cost and risks (hypotheses):** one cheap pass per step on a grid of about 1280×360 (each of the smoke's pressure
  iterations is 0.03 ms), plus the lit floor, so expect under 1 ms. The plain wave equation is not dispersive, so the
  bass/treble character comes from each impulse's size and from damping, not true water behaviour; try a damped
  higher-order term if it looks synthetic. The main risk is that it reads as generic water: specular glints on thin
  rings alias at 120 Hz unless the normal is filtered by the pixel footprint, and the drops-to-rings chain adds a
  delay between a hit and its visible ring that the timing check ("one accent per event, no delayed flashes")
  must accept or the drops must be shortened.

### Later, after the showcase meets its visual standard

- **Pitch Helix.** The spectrum wound onto a helix with one turn per octave and twelve positions per
  turn, so notes an octave apart stack vertically and chords become visible shapes. Needs a
  12-per-octave curve from the analyzer; the current FFT only separates neighbouring semitones
  cleanly above roughly 200 Hz.

## Build order

1. **Direction and baseline.** F0 target images and motion briefs for Tunnel and the current columns.
   Record the existing scenes' visual and performance baseline. Choose initial palettes, default
   framing, and intensity ranges; keep these artifacts beside the implementation for comparison.
2. **First showcase — the release standard.** F1 scene modules/lifecycle, F2b/F2e/F2f fast and slow
   audio features plus deterministic input, and F5 layered bloom/exposure. Finish Tunnel's depth,
   onset rings, and camera; improve column materials and reflections. Ship F6's refined EQ strip,
   curated presets, fullscreen overlay, and independent motion/flash controls. Add frame-time
   diagnostics. Exit only when these scenes satisfy the visual and pacing checks below.
3. **Instrument view and early experiments.** F2a/F2c, F3, and F5 non-MSAA/render scale; finish
   Phosphor Vectorscope. Prototype Smoke's solver/shading cost and Aurora's small set of ribbons.
   Use a target image and motion study for each; decide feasible quality settings from measurements
   before expanding the techniques.
4. **Complete the initial showcase.** Polish Smoke; promote Aurora if its prototype meets the
   composition and performance gates, otherwise keep it experimental. Finish the thumbnail browser,
   favorites/user visual presets, and smooth manual crossfades with prewarming and budget fallback.
   Tune lighting and brightness across scenes so the presentation feels consistent.
5. **Extend the catalog.** Stereo Stage with F2d, Feedback Warp, then Ferrofluid Orb. Give each an F0
   brief, its own material and response tuning, and the same acceptance checks. Shared plumbing is
   a starting point; it does not replace visual polish.
6. **Particles and optional show features.** F4, sparse Sparks on the improved columns, then Fountain
   and Chladni. Add auto-cycle and optional F2g beat-aware transition timing once manual transitions
   are reliable. Pitch Helix remains a later experiment.
7. **Fireworks (done, Milestone 8).** No new foundation. Prototype the detonate-in-flight timing against the percussion and
   impulse fixtures before any polish, compare it with plain apex bursts, then add the shell table, willow
   trails and floor light pools. Same F0 brief and acceptance checks as the other scenes.
8. **Lightning Storm and Ripple Pond.** Alternatives to Fireworks, outlined only. Build one at most as the
   next experiment, starting from its F0 brief, and share the peak-picked hit detector with Fireworks.

The app is shippable after each implementation milestone. Defer additional scenes when the current
showcase still has unresolved composition, timing, or frame-pacing problems.

## Checking each scene

### Repeatable input and captures

Implement the F2e synthetic source before using the commands below. They illustrate the planned
workflow from `gpu/`; scene ID 2 is reserved for Tunnel. Existing screenshot/FPS hooks remain, while
the synthetic suite, replay, frame-sequence capture, and detailed timing reports are new work.

```bash
GEQ_SYNTH=1 GEQ_SCENE=2 GEQ_SHOT=/tmp/scene2.png,4 cargo run --release
```

```bash
GEQ_SYNTH=1 GEQ_SCENE=2 GEQ_FPS=1 GEQ_FULLSCREEN=1 cargo run --release
```

The first saves a screenshot after four seconds and exits; the second reports real-time performance
at fullscreen resolution. Add fixed-step frame-sequence capture for short motion reviews, using
fixed audio, seeds, preset, camera, and timestamps. Frame-sequence capture may run slower than real
time and is not a performance benchmark. Capture runs must not persist temporary settings.

Each scene gets the following input suite at 32:9 and 16:9, including windowed UI and fullscreen:

| Input | What to check |
|-------|---------------|
| Silence, then quiet input | Gate behavior, calm residual image, no amplified noise or abrupt restart |
| Isolated impulses and sparse percussion | Visible onset timing, one discrete accent per event, clean decay |
| Sustained tones and slow sweeps | Smooth deformation, stable lines/normals, no shimmer or mode chatter |
| Dense mixtures and representative music replay | Preserved detail/color, bounded brightness and density |
| Left/right sweeps, mono, inverted phase | Correct stereo mapping, guarded silence cases, legible phase cues |
| Preset/scene changes and resize | Correct framing, controlled brightness, valid history reset, smooth presentation |

### Visual and musical acceptance

- Compare a still and a short recording with the scene's F0 target image and motion study. Record
  the chosen preset and quality settings so comparisons are repeatable.
- The focal point and material remain recognizable under quiet and dense input. Fine LED gaps,
  scope lines, smoke curls, and orb highlights survive bloom and tone mapping as appropriate.
- Brightness and motion have hierarchy: sustained forms, brief accents, and slow evolution remain
  distinguishable. Audio impulses do not cause unrelated delayed flashes or repeated bursts.
- No distracting shimmer, muddy accumulation, unbounded feedback, blown highlights, or abrupt camera
  jumps. Silence has a deliberate settling behavior; zero motion/flash settings do what they say.
- UI remains sharp and legible at native resolution. Visual presets never alter EQ gains or routing.

### Performance and lifecycle acceptance

- Record CPU frame work, presented/update intervals, median/p95/p99/worst timings, and missed frame
  intervals over at least a minute of real-time playback. Target 120 Hz with work fitting the 8.33 ms
  budget and measured headroom; report long-tail stalls rather than hiding them in average FPS.
- Where supported, request the optional timestamp-query feature at device creation and measure GPU
  simulation, scene drawing, bloom, and composite separately. Keep CPU/frame diagnostics available
  when it is unsupported. See [wgpu timestamp queries](https://docs.rs/wgpu/latest/wgpu/enum.QueryType.html#variant.Timestamp).
- Measure cold preparation, warmed transitions, and resize separately from steady playback. Verify
  the outgoing scene remains responsive during preparation, the transition respects the frame
  budget using its selected fallback, and repeated switches do not grow memory without bound.
- Test dense input at 5120×1440 and at the stated render/simulation scale. A reduced-resolution
  result must pass the visual checks too; meeting FPS with blurred structure is not sufficient.
- Report capture/replay buffering and audio-to-visual latency with the impulse fixture. The short
  FFT's window length alone is not proof of end-to-end responsiveness.
- Record the measured settings and results per scene. A scene is done only when its visual,
  musical, interaction, and pacing criteria pass together.

## Working defaults

- **No whole-screen pulsing by default.** You removed the bass-driven background glow in `a3579ea`.
  Preserve a dark field, while allowing localized lighting, reflections, and atmosphere around the
  scene. Start Motion intensity and Flash intensity conservatively; camera movement can be disabled.
- **120 fps at full screen is the target for every scene**, using a reduced render scale where a
  scene needs it, with visual quality and frame pacing validated at that scale.
- **Visual controls and measurement/audio controls stay separate.** Artistic normalization affects
  motion only; visual presets do not change EQ or routing. Analytical views retain their mappings.
- **The plan file lives in `gpu/`** because it only concerns the Rust version. The Python app is
  untouched.

## Milestone 1 results (2026-10-11)

Built as specified: Spectrum Tunnel, improved column lighting, fast onset response, and a finished
fullscreen presentation. Everything below was measured on the target machine (RX 580, 5120x1440 at
120 Hz, real time, synthetic demo signal; frame times are wall-clock intervals, GPU times come from
timestamp queries).

**Shipped**

- F1: `src/scenes/` (one Rust module and `.wgsl` per scene over a shared `common.wgsl`), a scene
  manager that builds scenes on demand, prewarms them one per frame, and bounds how many stay alive.
  Two uniform sets per scene so a crossfade can draw one scene twice. IDs 0/1 unchanged, 2 = Tunnel.
- F2b/c/e/f: 1024-point onset analysis (three flux envelopes, adaptive trigger with decaying-peak
  suppression, timestamped events), stereo correlation, a deterministic synthetic source with ten
  fixtures plus WAV replay, a frame-stepped mode for repeatable captures, and a modulation layer with
  a silence gate and bounded adaptive normalization. Unit tests cover all of it.
- F5: six-level bloom (13-tap Karis-averaged downsample, tent upsample) with per-level weights chosen
  per look, soft-knee threshold, per-look exposure, triangular dither, NaN/Inf-safe input.
- F6: ten looks, favorites, saved variants, thumbnail browser (stored captures), fullscreen strip,
  number/arrow keys, idle camera path, crossfade, auto-cycle, a refined EQ strip.

**Measured** (steady playback, 5120x1440)

| Scene | fps | frame median / p99 | Scene GPU | Bloom GPU |
|-------|-----|--------------------|-----------|-----------|
| 3D columns | 120.0 | 8.33 / 9.1 ms | 2.1 ms | 0.45 ms |
| Waterfall (mixed / stereo), with glass floor, walls and sky | 120.0 | 8.33 / 9.0 ms (stereo 9.1) | 3.5 / 5.0 ms | 0.45 ms |
| Spectrum Tunnel | 120.0 | 8.33 / 9.0 ms | 2.75 ms | 0.45 ms |

Transitions also hold 120.0 fps (p99 9-11 ms). The compositor estimates the cost of both scenes and
crossfades live only if it fits 6.6 ms of GPU time; otherwise the outgoing scene is rendered once and
held as a still image (this is the plan's frozen-image fallback; with today's costs
tunnel/tunnel, waterfall/waterfall and any pair that includes the stereo waterfall take it). Source-to-UI latency for an impulse is
about 4 ms median (12 ms worst) after a detection delay of up to one 10 ms hop.

**Findings worth keeping**

- The tunnel's first version cost 4.4 ms, mostly one loop: evaluating every ring's three `exp()` per
  pixel. Culling by distance from each ring took it to 2.75 ms. Bisecting with GPU timings beat
  guessing: palette work and MSAA were both smaller than expected (MSAA off saves about a third of the tunnel).
- The waterfall's night sky first cost 3 ms (the scene went from 3.1 to 6.1 ms). What brought it back to
  3.5 ms: every part of the sky bounds its own work (stars skip the horizon, a planet is one dot product
  away from its disc, the nebula lives in its band); stars sit on a cube-face grid instead of needing
  `asin`/`atan2`/`cos` per pixel, and most pixels in a star's cell leave early; the backdrop is a quad
  clipped to the horizon rather than the whole screen; and the mirror skips what a blurred image does not
  show. The biggest single gains were structural: drawing the opaque terrain first so the depth test skips
  the floor and reflections behind it (-0.6 ms), and discarding the stereo overlay's empty pixels instead of
  blending zeros (-1.2 ms in stereo, which ended up cheaper than before the sky).
- The glass floor no longer mirrors the sky (the owner asked for it removed): the floor shader drops the
  three light-bar reflections and the mirrored `sky_color` call, and the sky's reduced "mirror" variant
  (`full = false`: no nebula, no shockwave, fewer stars) is gone with its only caller. The terrain, wall
  and edge-line reflections stay. Scene GPU time at 5120x1372 fell by about 0.4 ms in every timing window
  of a synthetic run (4.35 to 3.95, 4.86 to 4.47, 5.69 to 5.30 ms).
- The camera's far plane (100 units) clips the floor long before its 600-unit edge. Once the fog at the
  horizon was lighter than the clear colour that showed as a black strip, so the sky quad now reaches below
  the horizon far enough to paint the fog there.
- `pow(x, y)` with a slightly negative `x` is NaN in WGSL, and 4x MSAA can extrapolate an attribute
  just outside its range at triangle edges. One NaN pixel spread through the bloom pyramid into a
  screen-sized black block. Interpolated values are now clamped and the bloom input is sanitized.
- The ring leads the wall deformation: events come from a 21 ms window and the history from a 171 ms
  window, so a hit's ring launches roughly 75 ms before its ridge reaches the newest row. Rings travel
  slightly faster than the history scrolls, so they read as a pulse running ahead of the wall.
- Quiet material: geometry keeps the calibrated scale, so a very quiet source (below about -45 dB)
  draws little wall relief; rings, breathing and lamp level follow the normalized envelopes and stay
  alive. The Sensitivity setting is the intended control for the walls.

**Not done yet** (as planned): the per-bin pan frame (F2d), render scale (F5), optional beat timing, and
the later scenes.

## Milestone 2: Phosphor Vectorscope (scene 3)

The first instrument scene, with the two pieces of plumbing it needed.

**Shipped**

- F2a, the waveform feed: the analyzer thread queues every stereo sample for the UI exactly once
  (a bounded queue, swapped out under the lock, so a stalled UI never makes it grow); the synthetic
  sources do the same. The scene uploads the new samples each frame into a 4096 x 1 `Rgba32Float`
  ring (left, right, and each channel's trace position) and draws the segments between them.
- F3, persistence, as the scene's own offscreen pass: a single-sample HDR buffer whose red channel is the
  fast flash and green the slow afterglow. Each frame it is multiplied by `exp(-dt/tau)` per channel (a
  full-screen pass with a blend constant, so it is independent of frame rate) and the beam is added on top.
  The scene shows it through its own composite, so bloom and tone mapping run unchanged. The scene trait
  gained what this needed: samples in `Frame`, the uniform bind group and a timestamp slot for `update`,
  and scene-owned bind groups. A scene that is skipped for a frame (a transition, another scene) restarts
  the beam instead of joining across the gap. Resizing resets the buffer.
- The beam follows the plan: one instanced quad per sample pair, a closed-form Gaussian integral along the
  segment (a polynomial `erf`) divided by its length, so it is anti-aliased and bright where it dwells.
  Each segment is drawn three times for the X/Y scope (X = (R - L)/sqrt2, Y = (L + R)/sqrt2) and the two
  side traces, which are triggered sweeps on a rising zero crossing that free-run on a signal with none.
- A display gain follows the recent peak (instant up, six seconds down, limited to 0.55x-24x), so quiet
  recordings fill the tube and loud ones do not overshoot; it only scales the picture. The signal gate
  dims the beam on silence. The graticule (rings, axes, tick marks, trace frames) stays on screen when the
  input stops, and bass onsets flick the outer ring.
- Four looks (P31 Green, Amber Scope, Ice Trace, Neon Lissajous), a Persistence slider in the Display
  menu, thumbnails, settings that load files from before the fourth scene (their `look` array grows).
  The circle stays round at every aspect: the traces take the outer thirds when there is room (21:9 and
  wider) and 16:9 shows the scope alone.

**Measured** (5120x1440, real time, synthetic demo and dense signals)

| Scene | fps | frame median / p99 | Scene GPU | Bloom GPU |
|-------|-----|--------------------|-----------|-----------|
| Vectorscope | 120.0 | 8.33 / 9.2-10.5 ms | 0.95 ms (persistence 0.35-0.4, screen 0.55) | 0.45 ms |

With about 400 segments a frame at 120 fps (3 draws each) the beam pass costs 0.35-0.4 ms; the full-screen
persistence fade and the composite make up the rest. A scene cycle through all four scenes held 120.0 fps
with live crossfades for every pair that fits the budget.

## Milestone 3: Sparks and Fountain (scene 6)

F4, compute particles, and the two things the plan hangs on it.

**Shipped**

- F4: `scenes/particles.rs` and three shader pieces. One storage buffer is a ring of particles (48 bytes
  each: position and life, velocity and total life, four scene-defined values). The CPU hands out the next
  `n` ring slots each frame, split into a continuous stream and up to six bursts; a compute pass integrates
  every particle that can be alive and rebirths the slots it was told to, drawing each spawn's randomness
  from a hash of its serial number, so a run repeats exactly (the capture tests below). Because a particle
  lives at most `max_life` seconds, everything alive sits in a contiguous stretch of the ring behind the
  head; both the compute dispatch and the draw cover only that stretch, so a quiet sky costs nothing.
  Draw is vertex pulling from the same buffer: each particle is a streak (a Gaussian dot dragged along its
  screen-space velocity, so fast ones read as motion at 120 Hz), additive. A scene that is skipped for
  twenty frames starts again from an empty sky. The shared uniform group became visible to compute, and
  `Frame` gained a compute timestamp slot.
- Fountain (scene 4): 262,144 particles of capacity, thrown where the volume is rising (see Milestone 3b for
  how it got there). The analyzer reports, for each of the 192 bins, how steeply its level is climbing;
  the rise rides in the history texture's spare alpha channel, and a spawn picks a frequency by rejection
  sampling on it (sixteen tries, above an adjustable threshold) and launches from that frequency's place on
  the emitter line. The steeper the rise, the bigger, brighter and faster the spark. Gravity, a gentle
  divergence-free wind (the curl of sines), a floor bounce with a splash flash, and a settle rule. Non-MSAA
  with analytic grid lines, mirrored in the floor by a second draw. Four looks; a sensitivity slider (the
  threshold) and a density slider.
- Sparks (an overlay on the columns): 16,384 particles on the same machinery, drawn inside the columns'
  multisampled pass so their depth hides what is behind them. Embers shed from the top of each lit strip
  (a column is chosen by running weights the CPU sends), sparks burst from a peak cap when it jumps at
  least five LED segments, bigger the further it jumped (`modulation::PeakHits`, see the findings below).
  A Display-menu checkbox.

**Measured** (5120x1440, real time, the demo signal's loudest seconds, and the dense fixture)

| Scene | fps | frame median / p99 | Scene GPU | Compute |
|-------|-----|--------------------|-----------|---------|
| Fountain, sensitivity 1.0 | 120.0 | 8.33 / 9.1 ms | 0.3 ms | 0.01-0.02 ms |
| Fountain, sensitivity 1.8 | 120.0 | 8.33 / 8.9 ms | 0.8 ms | 0.02 ms |
| 3D columns with Sparks (without) | 120.0 | 8.33 / 10 ms | 2.2 ms (2.1) | 0.01 ms |

(The fountain figures are for the rise-driven version. The first, spectrum-driven version cost up to 2.5 ms
because thousands of jets were alive at once; sparks only fly when something is rising now, so far fewer are.)

**Findings worth keeping**

- Column sparks are rare and big on purpose. The first rule (a cap rising about one segment, 3% of the
  scale) fired 520 times in 60 s of the demo signal, on almost every column at every beat, and a single hit
  was reported as two bursts because the rate limit split its climb (a hit's cap rises over about four
  frames: 3.3, 3.8, 1.1 and 1.0 segments). Now a push has to add up to five of the strip's 30 segments within
  a quarter second, and is reported once it has settled (the cap has not risen for 25 ms, or 80 ms into a long
  climb), so one hit is one burst measured by the whole push: 108 bursts in the same 60 s, and a slow climb
  or a staircase of single segments never adds up to one. Burst size runs from 5 segments (45 particles) to
  25 or more (300), and the sparks' size, speed and life grow with it, so a 5-segment hit is a small
  spray and a 20-segment one a bright flash. Constants: `PEAK_HIT_*` in `modulation.rs`, `BURST_*` and
  `FULL_PUSH_SEGMENTS` in `sparks.rs`. Frame time and scene GPU time (2.1 ms) did not change.
- Particle density, not particle count, is the design variable. The first version emitted 52,000 a second
  and was a white cloud at every setting; what reads as a fountain is a few thousand alive at a time, with
  streaks long enough to show the arcs. "Capacity" in the plan is just headroom for the density slider.
- The cost is fragment fill, not the simulation. The compute pass is 0.02 ms for 262k particles; trimming
  the streak quads' margins from 5 sigma to 4 sigma cut the fountain's loudest cost by a third (3.15 to
  2.05 ms). An early measurement on the demo's quiet intro said 1.1 ms; the hint needs the loudest second.
- Two looks of one scene crossfading draw its particles twice but simulate once, so the pair costs two
  draws, and the compositor budgets for that.

## Milestone 3b: from spectrum driven to beat driven (the Fountain)

The first Fountain launched jets by *level* (frequency driven). The request was to make it beat driven:
bass drums toward the left, snares toward the right, sparks flying on a hit and bigger the more pronounced
the hit. Two routes were tried.

**Drum classification (tried, discarded).** Classify the analyzer's onset events into kicks, snares and
hats, and throw a burst per drum from a pad on the left or right. Measured against an exact ground truth
for the synthetic songs it was precise (no false kicks or snares at any level) but missed half the kicks
and a third of the snares in a dense mix, and the three coarse frequency regions cannot tell a snare from a
kick that coincides with a hat (both fire all three). It also needed thresholds that follow the signal level
(a threshold relative to the strongest recent hit fails when a song has no snares for a while), and it never
looked right. It was reverted.

**Rate of rise per frequency (shipped).** The idea that replaced it: measure how steeply the volume at each
frequency is rising, and throw sparks from the frequencies that are rising fast, above a threshold the user
can move. There is nothing to classify. The position along the line is already the frequency, so a kick (a
steep rise in the lows) lands on the left and a snare (a rise up high) on the right on their own.

- `analyzer::Levels::rise`: for each of the 192 waterfall bins, the mean positive change of the
  log-compressed 1024-point magnitude since the previous hop, with a fast attack and a 50 ms fall so a hit
  survives to the next 60 Hz history row. It travels in the history texture's spare alpha byte.
- Measured on the percussion fixture, whose hit times are exact: a kick rises 6.6 nats in the lows, 3.6 at
  500 Hz-2 kHz, 1.1 at 2-6 kHz and 0.2 above; a snare 5.4, 3.4, 3.4 and 3.3. Steady tones read below 0.15.
- The mean over a bin's FFT bins, not the maximum: a hit lifts all of them together while the jitter of
  noise-like sounds (hats, cymbals) lifts some and drops others, and a maximum kept the jitter. Switching
  to the mean cut the dense demo's background (median 0.71 to 0.30 nats, p90 1.9 to 1.1) and left the hits.
- A fixed tilt weights the higher frequencies up (0.85 to 1.8): a snare's rise up high is half a kick's in
  the lows, and without the tilt the right side was nearly dark.
- The threshold is the adjustable gradient: sensitivity 0.6, 1.0 and 1.8 on the same moment of the same song
  go from nearly nothing, to a clear kick burst on the left and a snare burst on the right, to a full
  firework. (Its units changed once real music was tried; see below.)

**Findings worth keeping**

- A pure tone starting from digital silence splatters across the whole spectrum, and the log compression
  magnifies any energy above zero; a test needs the noise floor of a real recording.
- The rise is measured in nats of a log-compressed magnitude, so it also falls as the signal gets quieter;
  the sensitivity slider is the remedy for a low system volume.
- A snare is broadband, so it also lights the lows; the kick-left, snare-right split is a matter of which
  side *only* one of them reaches (the right side is snares and hats; the left is kicks and snare bodies).
- Nothing here has been heard on real music, which this machine does not have; only the synthetic songs.

**Real music, and normalizing the rise.** The first version used an absolute threshold (2.6 nats, tuned on the
synthetic songs), and the owner had to turn the sensitivity slider to its maximum (3.0) on real music to get
it to work. The live-capture test (`cargo test --release live_rise_stats -- --ignored --nocapture`, a
read-only tap on the default output that prints only statistics) showed why: a real recording's hardest hits
rise about 3.3 nats against the synthetic kick's 6.6, so the default sat above nearly everything real. A
different absolute default would fit one song, so the rise is now judged against the music's own recent
hits:

- `modulation::RiseNorm` divides each (tilted) rise by a reference: the peak of its own recent maximum,
  decaying with an 8 s time constant, never below 3.0 nats (so steady music, whose rise is only jitter, is
  not scaled up until the jitter looks like hits) and never above 7.0 (so one click cannot silence the
  fountain for long). The history texture's alpha byte now carries this normalized value, and the shader's
  thresholds are fractions of the hardest hit.
- Level independence, measured at the final thresholds: the same synthetic song at 1, 1/2, 1/4 and 1/10 of
  its level gives sparks in nearly the same share of frames (demo 29.9, 29.8, 30.4, 32.2 %; dense 36.9,
  36.4, 37.7, 40.0 %; the clean percussion track 4.4 % at every level).
- The thresholds (0.28 to 0.72 of the hardest hit, at sensitivity 1) were set by reproducing the owner's
  setting on the same real audio. The old rule at sensitivity 3.0 lit sparks in 85.5% of frames (23
  frequencies when they flew, full strength in 2.4%); the new default at sensitivity 1.0: 86.9%, 24, 2.1%.
  That is a lot of sparks, but they are scaled by the rise: most frames carry a few small ones, and hard hits
  throw many large ones.
- One real recording (30 s) is the whole real-world evidence. The slider stays for everything else.

**Sparks and the floor.** The owner reported fountain sparks "pass through the floor and come back up instead
of bouncing". A bounce is a position put back above the floor, and the simulation already did that:
`GEQ_PARTDEBUG=1` reads the particle buffer back and across tens of snapshots no live particle was ever below
the floor (lowest height 0.0000-0.0006), with about a third of them within 0.06 of it, half of those rising.
The bounce now sets the height to its absolute value, as the owner described, and a streak's tail is clipped
at the floor (a spark that has just bounced is moving up, so its tail trailed below the glass).

What can still look like a spark passing through the floor, and what was tried: sparks fly toward and away
from the camera (a mean 1.8 units from the emitter line, up to 18), so one that lands on the floor in front
of the line appears below it on screen, and a falling spark's mirror image continues its line below the
contact point. Keeping the sparks in a thin slab around the line (a spring in depth) removed both, and was
rejected by the owner: the sparks should fly toward and away from the camera. If it still reads as passing
through, the remaining options that keep that freedom are dimming and shortening the reflection, or drawing a
faint spot on the floor under each spark so the point it will hit is visible.

## Milestone 4: Smoke Jets (scene 5)

The plan's `L` subsystem: eleven plumes tied to the EQ bands, on a 2D stable-fluids solver. IDs: 5 = Smoke Jets.

**Shipped**

- `scenes/smoke.rs`, `smoke_sim.wgsl` (every offscreen pass), `smoke.wgsl` (the screen pass) and `smoke_common.wgsl`
  (the uniform block). Fragment passes on a grid of square cells about four screen pixels wide (1280 x 360 at
  full screen, never fewer than 192 rows, so a small window shows an enlarged simulation instead of a coarse
  one), with the dye advected at twice that resolution. Velocity is `Rg16Float`, pressure/curl/divergence
  `R16Float`, dye `Rgba16Float` (density, heat, band position x density, accent). All filterable and
  renderable with no optional features; the one bind group layout (uniform, sampler, three textures) serves
  every pass, with a 1x1 texture filling unused slots.
- One step: curl, forces (vorticity confinement, buoyancy of the hot smoke, the nozzles), divergence, 18
  Jacobi iterations warm-started from the previous step's pressure, gradient subtraction, MacCormack
  velocity advection (two passes), MacCormack dye advection with decay and injection (two passes). The
  solver runs on a fixed 1/120 s step through an accumulator that rounds instead of flooring (a steady 120 Hz
  is exactly one step a frame whatever the clock jitter; 60 fps runs two, a stall is capped at four and the
  backlog dropped). A scene absent for 30 frames, a reset or a resize starts from clear air; `reset(seed)`
  seeds the wobble, and two captures of the same run are byte-identical.
- Open top, closed free-slip floor and side walls. The nozzles stand half a spacing from each side wall, so a
  wall is a mirror image at exactly the neighbours' spacing and the outer plumes behave like the inner ones.
- The drive is plain Rust (`drive()`, unit-tested): band level -> jet strength above a noise floor (a quiet
  band is a faint wisp, a closed gate blows nothing); the onset envelope of the band's region (low for the
  bass bands, mid, high, blended over the seams, times Flash) -> a puff that flares the nozzle and adds speed,
  density and accent; bass nozzles wide and slow, treble narrow and fast; Thickness scales the density. The
  nozzles take their x from the EQ strip's measured column centres in windowed mode (`SceneParams::nozzle_x`),
  evenly spaced otherwise. Silence raises the smoke's decay so what is left dissipates.
- Lighting at dye resolution: a bump normal from the density gradient, self-shadowing by the smoke below (four
  taps), light from lamps on the floor that widens and fades with height, a haze behind the smoke. The result
  is four scalars (brightness, emission, accent, band position); the screen pass colours them from the theme
  (dominant to supporting along the bands, a small accent for puffs; Rainbow follows the band), so two looks
  of the scene crossfade with their own palettes. The screen pass also draws the floor strip and a lamp in
  every nozzle, which stay on screen in silence.
- Four looks (Ice Vent, Ember Smoke, Neon Haze, Prism Plumes), stored thumbnails, a Thickness slider, Motion
  as turbulence (it scales the confinement and the nozzles' wobble) with its own tooltip, settings that
  load files from before the sixth scene (`look` grows, `smoke_density` defaults to 1).

**Measured** (5120x1440, real time, synthetic demo and dense signals, GPU times from timestamp queries)

| Scene | fps | Solver + shading (`scene_upd`) | Screen pass | Bloom |
|-------|-----|--------------------------------|-------------|-------|
| Smoke Jets, 18 iterations | 120.0 | 1.45 ms | 0.33 ms | 0.45 ms |
| 6 iterations / 40 iterations | 120.0 | 1.05 / 2.15 ms | 0.33 ms | 0.45 ms |

A pressure iteration costs 0.03 ms, and everything else in the step about 0.85 ms (the four MacCormack
passes about 0.3 of it). The dense fixture costs the same as the demo (the cost does not depend on the
picture). Crossfades into the scene and between two looks of it (one simulation, two draws) held 120 fps with
p99 9-14 ms. The textures come to about 70 MB at full screen (computed from their sizes, not measured). The frame-time tail during these runs (p99 9-20 ms, max
17-23 ms) is not the scene's: the Fountain, measured minutes before in the same session, showed the same
hitches (p99 up to 18 ms) where it had been 9.1 ms on another day, so this machine was busy; the first
smoke run, before MacCormack, had p99 9.2 ms.

**Findings worth keeping**

- Vorticity confinement is unstable at a useful strength. Its force is proportional to the curl, so a thin
  shear layer grows by (strength x dt) a step: at 13 cells/s that is 10 % a step, and after 30 s the field
  held single-row jets pinned at the speed limit, which drew as hard axis-aligned voids in the smoke. A
  clamped curl and a much smaller strength fixed it. Plain semi-Lagrangian advection at 120 steps a second
  also blurs every step, which is why the first version needed confinement at all; with MacCormack (a forward
  pass, a backward pass to estimate the error, a correction held inside the range the trace-back read)
  vortices keep their strength, a puff rolls up into a ring that actually travels, and the confinement could
  drop to almost nothing.
- Non-dissipative advection needs a little dissipation of its own. Without it the dye turned to fine marbling
  and the velocity kept every cell-sized ripple, and the screen filled with noise. A small viscosity and dye
  diffusion (scaled with the grid, so a small window does not look fatter), a faster density decay and a
  thin-smoke loss gave dark space around the plumes back. Delicate wisps come from low injection, not from
  how long the leftovers live.
- The pressure iteration count is an art parameter, not only a quality one: 4 iterations stand the plumes up as
  tall columns that reach the top (the projection is leaky), 18 gives S-curves and caps about half the height,
  60 is lower still. Jet speed was raised (0.64 to 1.10 screen heights a second, bass to treble) to get tall
  plumes with 18.
- The wobble that makes plumes sway must not have regular phases: with phases a fixed step apart every plume
  flapped the same way, the same way in every capture. Irregular phases, slow for bass and quicker for treble,
  give each plume its own meander.
- Colouring in the screen pass instead of in the simulation passes is what lets a look change crossfade
  correctly; the lit field costs one bilinear read per full-resolution pixel.
- Resolution independence needs every length in cells to scale with the grid height (speeds, widths, buoyancy,
  confinement) and the numerical smoothing to scale with it too; the nozzles' vertical extent is the one thing
  still fixed in cells.
- Nothing here has been heard on real music, which this machine does not have; only the synthetic songs. The
  jets follow the calibrated levels (a recording quieter than about -45 dB blows little smoke; Sensitivity is the
  control) while puffs follow the normalized onset envelopes.

**Not done yet**: a quality setting for the grid and iteration count (the cost is low enough to leave at 1.45 ms),
a render scale for the shading pass, and tuning on real music.

## Milestone 5: Chladni Sand (scene 7 in the table, ID 6)

Three plates of sand that draw the classic figures. IDs: 6 = Chladni Sand.

**Shipped**

- `scenes/sand.rs` (layout, mode selection, drive, the scene; unit-tested), `sand_common.wgsl` (the uniform block
  and the plate's vibration pattern), `sand_sim.wgsl` (compute) and `sand.wgsl` (the plates). One storage buffer
  holds 737,280 grains (24 bytes: position on its plate, velocity, the angle of the facet that catches the lamp, a
  tone); they are never born or killed. 163,840 per plate at the designed density, three plates, so about half a
  million; the Grains setting runs from 0.25 to 1.5 of that, and a smaller plate gets fewer grains in proportion
  to its area (the same grains per pixel at any size, and a grain's light follows, so a small window is not
  brighter). Plates are three squares evenly spaced across the width when the picture is at least 2.6:1, one in
  the middle otherwise (a lone plate is driven by all eleven bands), with whole-pixel geometry.
- The pattern is `a(x, y) = sum w_i (cos(n_i pi x) cos(m_i pi y) + s_i cos(m_i pi x) cos(n_i pi y))` over four
  modes per plate. Each EQ band owns a mode (m, n, s), higher bands higher modes: (1,2,+) at 40 Hz up to (6,9,+)
  at 16 kHz. A plate's modes are four *voices*: the loudest three bands of its group (lows 40-240 Hz, mids 440 Hz-
  2.7 kHz, highs 4.8-16 kHz) claim a voice, a band already vibrating counts 1.3x louder than it is so a
  challenger must clearly beat it, a band must be within 55% of the leader to take part, and weights follow
  `(level / leader)^2` with a 0.30 s rise and a 0.55 s fall. Nothing changes while the signal gate is closed
  (silence leaves the last figure standing), and a plate whose region has nothing above a displayed level of 0.14
  keeps its figure instead of reshaping it from noise. Constants are at the top of the section in `sand.rs`.
- The compute pass kicks each grain with a velocity impulse of `|a|` times the plate's energy (damped, so it hops
  in short arcs rather than flickering; the noise scales with sqrt(dt), every rate is per second, so 60 and 120 fps
  agree) and draws it toward the nearest zero of `a` (the Newton step `a grad(a) / (|grad a|^2 + 1)`, capped, and
  ignored when that zero lies outside the plate). Where the plate is still nothing moves, so sand collects on the
  nodal lines; when the weights change the old lines start to shake and the sand runs to the new ones. A plate's
  energy is the sustained level of its region (the normalized `sus` envelope, gated); a hit in that region
  multiplies the shaking (Motion) and weakens the pull while the sand is airborne, and makes the sand glow
  (Flash). The scene starts from a figure that is already formed: each grain tries up to 32 random places and keeps
  the first near a nodal line (the rest are loose grains), so there is no settling period; the pass is spread over
  four frames after a reset.
- Rendering: the compute pass adds every grain into a per-pixel density buffer of fixed-point atomics (bilinear
  over four pixels, two u32 per pixel: the body of the grains and the glints), and each plate pixel turns its sums
  into light, softly saturating where sand piles up (a crossing of two lines does not burn out). The sand is a
  pale body in the plate's theme colour (lows, mids and highs take successive hues of the theme; Rainbow follows
  the plate), plus glints when a grain's facet faces a lamp that drifts slowly (Motion), and a moving grain
  tumbles so it flashes. The plates are dark brushed metal with a lit rim and a spill of their colour on the dark
  field, vibration faintly tinting the antinodes, and a strip of 32 lamps under each that fills with the plate's
  energy and flares on a hit; the strips and rims stay on screen in silence. Non-MSAA.
- Four looks (Moonlit Sand, Amber Dust, Neon Grit, Prism Sand), a Grains slider, `sand_grains` in the settings
  (older files load; `look` grows to seven), thumbnails, `GEQ_GRAINS`, `GEQ_SANDDEBUG`. `prepare_scene` now gives a
  scene built after the targets exist its size, so the 35 MB density buffer is allocated at prewarm.

**Measured** (5120x1440, real time, demo and dense signals, GPU times from timestamp queries, taken while the
display was on)

| Scene | fps | Compute (`scene_upd`) | Plates + sand resolve (`scene`) | Bloom |
|-------|-----|-----------------------|---------------------------------|-------|
| Chladni Sand, grains 1.0 (491,520 on screen) | 120.0 | 1.22 ms | 0.58 ms | 0.45 ms |
| grains 0.5 / 1.5 | 120.0 / 120.0 | 0.73 / 2.15 ms | 0.58 / 0.59 ms | 0.45 ms |

The dense fixture costs the same as the demo. `cost_ms` is 1.9. Nothing below was re-measured after the last two
changes (the setup pass spread over four frames, and allocating at prewarm); see the end of this section.

**Findings worth keeping**

- Drawing each grain as a tiny quad cost 7.2 ms (a first version: 491k instances, two triangles each, additive
  blend) and scaled linearly with grains (2.15 ms at a quarter, 3.85 at half). Bisecting: constant colour in the
  vertex shader changed nothing (7.23 ms), shrinking the quads from 4.1 to 1.2 px halved it (3.80 ms), so it was
  rasterising a million tiny triangles and blending them, not vertex work. Splatting in the compute pass with
  atomics and shading per pixel costs 1.22 + 0.58 ms and also gave the soft saturation for free.
- Every antisymmetric mode (`s = -1`) keeps the diagonal x = y still and the corner (0,0), so sand collected there
  and the same diagonal burned in every figure; and an antisymmetric mode with m + n even is a blob at the centre
  (a zero of high order, `(1,3)`, `(3,5)`). The table now mixes signs (mostly symmetric, with `(3,4)` and
  `(5,8)` antisymmetric) and a test checks that no plate keeps one line still in all of its modes.
- Mode sums can cancel: with near-equal weights `(1,2,+)`, `(1,3,+)` and `(2,3,-)` add up to zero along the whole
  bottom edge, which drew a band of sand along the wall that looked like a bug. It is a real nodal line; changing
  the third to `(2,3,+)` removed that case, but any table can have such coincidences.
- A pull toward the nearest zero must ignore zeros outside the plate, or sand piles against the wall wherever a
  nodal line passes just beyond the edge (first with a 2% margin, then none).
- Louder music shakes harder, so lines fattened with level. The noise (4.4) is now low enough, and the pull to
  the lines (3.2 per second at energy 1) strong enough, that lines stay thin at full energy; a hit is what
  breaks them: the shake is `1 + 4.5 x Motion x onset` (3.7x for a full-strength hit at Motion 0.6) with the
  pull weakened by its square root.
- The demo signal's spectrum is nearly flat (bands within 0.5-0.65 of each other during the groove), so three
  modes keep equal shares and the figure holds for tens of seconds; the percussion fixture changes figures every
  few seconds. Real, tilted music should lead with one band more often (the weights are squared to help); none
  has been heard.
- Frame-rate independence only holds if the noise scales with sqrt(dt) and the tumbling and pull with dt; the
  clamp on dt (50 ms) bounds a stall.

**Not done yet / not verified**

- The display went to sleep during this session ("Monitor is Off", DPMS timeouts disabled, so it was switched
  off rather than timed out), which stops presentation and drops the GPU clocks; nothing after that point could be
  measured or captured. Still to do on the real machine: re-run the cycle (`GEQ_CYCLE=3 GEQ_SCENE=5 GEQ_PRESET=3
  ... GEQ_FPS=1`). Before the display slept, the crossfade from Smoke into the sand showed two transition windows
  with p95 24-25 ms (about three frames in 60), both before and after the allocation moved to prewarm, while later
  crossfades inside the sand and out of it were at 120 fps with p99 9-14 ms; its cause is not established (the
  init pass was the suspect and is now spread over four frames, but that was not measured).
- The thumbnails in `assets/thumbs/6-*.png` are the 14 s captures of each look at 2560x720, cropped and scaled to
  336x109; run `tools/make-thumbs.sh` to replace them with the standard 1600x450 captures.
- Another `geq-gpu` instance was running on this GPU during the session (started before it), so earlier frame-time
  tails (the median column read 10-13 ms at 120.0 fps for every scene, Smoke included) are not comparable with
  the plan's earlier tables; the GPU-time columns and fps are.
- Quality settings beyond Grains (a render scale, a plate resolution), and tuning on real music.

## Milestone 6: Ferrofluid Orb (scene 8 in the table, ID 7)

A glossy black sphere floating over a polished floor, with broad elastic lobes under fine spikes. IDs: 7 = Ferrofluid Orb.

**Shipped**

- `scenes/ferro.rs` (the springs, the sites, the scene; unit-tested), `ferro_common.wgsl` (the uniform block and the
  map between the height texture and directions), `ferro_height.wgsl` (the offscreen pass) and `ferro.wgsl` (the
  mesh, the studio, the floor). The camera is a free orbit; the scene is MSAA (it is rasterised geometry).
- The shape. 192 spike sites on a Fibonacci spiral from the bottom pole to the top, site `i` = frequency bin `i` of the
  waterfall's spectrum (so a rising sweep climbs the orb), about 0.256 rad apart; each spike is a cone with concave
  flanks, `h = A (1 - alpha / w)^2` with `w` = 0.17 rad (a sharp tip and a base that runs into the surface). Eleven lobes
  (one per EQ band, on their own spiral) are broad bumps, `A exp(-kappa (1 - cos alpha))`, which wrap the sphere without
  a seam. The height pass turns both into a 256 x 128 `Rgba16Float` longitude / latitude map every frame: the height in
  `w` and the *analytic gradient* on the sphere in `xyz` (a world-space tangent vector, so there is no seam or pole
  singularity in it). The plan asked for an `R16Float` height map with normals differenced from it; bilinear filtering
  of a height map has a kink at every texel edge, and a sharp strip light reflected in the surface shows each one,
  where interpolating the stored gradient does not. The mesh (384 x 192 cells, 442k vertices from `vertex_index`)
  samples the map in the vertex shader, and the normal of the surface `r = R (b + h)` is `(b + h) dir - gradient`.
- The motion is plain Rust, in fixed 1/240 s steps (so 30, 60 and 120 fps agree, tested). Each spike is a spring
  (7 Hz) pulled toward its level, `0.15 + 0.85 smoothstep(level, 0.22, 0.95)` while the signal gate is open (every site
  carries a short spike, the loud ones rise out of it) and thrown out by a force proportional to its rise (the same
  normalized rise that drives the fountain, above 0.22). Each lobe is a softer spring (2 Hz) aimed at how far its band is
  above the others' average, so a louder band swells and a quieter one sinks and the volume holds; the onset envelopes
  kick the lobes and hop the whole orb. Sustained bass breathes it (4%). Silence retracts the spikes and the orb settles
  into a smooth sphere. Everything is bounded: springs are clamped (a spike may overshoot to 1.3x of its height; nothing is drawn below the
  surface), and at Spikes 1 the lowest point the shape can reach still clears the floor (checked in a test).
- The material. A black mirror (`F0` 0.07, Fresnel) showing a procedural studio of six rectangular strip lights in the
  theme's colours: a long one overhead (bass), two tall ones at the sides (mids), two behind it for the rim (highs), a
  thin one low in front. The ray from the surface is intersected with each light's plane, with edges blurred by the
  pixel's footprint on that plane (so thin strips seen in spike flanks fade rather than shimmer) and by the surface's
  roughness times the ray's length. The whole rig turns slowly (`0.05 rad/s x Motion`, a third as fast in silence), so
  reflections keep sliding when the music stops. The tallest spikes glow at their tips in the colour of their frequency
  (the latitude of the tip is its frequency, so Rainbow runs bass to treble up the orb). A faint dome gives the
  silhouette a rim against the dark.
- The floor. Dark and polished: a faint polar grid, the ring of eleven LEDs the orb floats over (bass at the left on the
  front arc, each filling with its band's level and flaring on onsets, also seen in the orb's underside), a soft pool of
  light, and ripples out from the ring on low-frequency onsets. The mirror image is the same mesh at half density,
  mirrored in y and shaded from the camera's reflection (so it shows the underside), additive, with the opposite face
  culled (an additive pass would otherwise draw the back of the sphere on top of its front). It fades with depth and is
  strong at grazing angles. The floor itself reflects only the overhead light, at a third strength.
- `Ctx::pipeline_cull` (a face-culling variant of `pipeline_with`), `SceneParams` gained `spikes`, `spectrum` (the 192
  displayed levels) and `rise`, a Spikes slider (`Control::OrbSpikes`, `orb_spikes` in the settings, `GEQ_SPIKES`), four
  looks (Obsidian Ice, Ember Core, Neon Magnet, Prism Oil), stored thumbnails, Motion and Flash tooltips for the scene.
  Motion is how elastic the fluid is (hits, lobe wobble, breathing, the light rig; 0 holds a fluid that only follows the
  loudness in still light) and Flash the pulse of the lights, the ripples and the tip glow.

**Measured** (5120x1440, real time, demo signal, GPU times from timestamp queries)

| Scene | fps | frame median / p99 | Scene GPU | Height pass | Bloom |
|-------|-----|--------------------|-----------|-------------|-------|
| Ferrofluid Orb | 120.0 | 8.2-8.5 / 9.0-9.2 ms | 1.70-1.77 ms | 0.06 ms | 0.43 ms |

Bisecting the scene pass: with nothing drawn the 4x MSAA clear and resolve at this size already cost 0.71 ms; the orb adds
about 0.34 ms, the floor 0.45 ms and the mirror image 0.2 ms. The height pass the plan worried about (6.3 million site
contributions) is 0.06 ms, so nothing needed precomputing; the budget has room for a 512 x 256 map or a denser mesh
if they are ever wanted. A cycle through sand, the orb's four looks and the columns held 120.0 fps in every transition
(p99 9.0-10 ms; the worst single frames were 14.9 ms going into the orb and 15.8 ms in one window of its own steady
playback, once each). The orb is the slowest scene to build, 46 ms at prewarm (the others are 3-12 ms), which happens
one scene per frame at startup; the worst frame of the startup window was 9.8 ms.

**Findings worth keeping**

- The first version read as a sea urchin, not a black mirror: spike bases 0.205 rad wide on sites 0.256 apart, and
  almost every bin tall in dense music, left no smooth surface between them for a reflection to show. Narrower spikes
  (0.17 rad), taller at the top of the range (0.40 of the radius), and a short spike on every site (0.15) with only the
  loud ones rising out of it gave the glossy gaps back. The lobes give the orb its larger shape; the spikes are the
  detail.
- Physically correct reflections of the tall strip lights in a glossy floor are three bright pillars, which pulled the
  eye from the orb, so the floor shows the overhead light only and the tall ones stay on the orb. A long horizon light
  behind the orb gave a hard bright rod at the horizon that did not change when it was dimmed to 40% and made three times
  taller (the cause is not established); it was removed, and the floor without it is the better composition at 32:9.
- A rough surface blurs a reflection in proportion to how far the reflected ray travels, not by a fixed width: a fixed
  blur left the pillars' edges hard and softened nearby lights too much.
- Stored analytic gradients (above) matter; so does shading the mirror image from the mirrored eye with the real
  normals, instead of mirroring the normals.
- A hit has to throw a spike out by roughly a third of its range at the default Motion to read as a hit; the springs'
  force is proportional to the rise (not an impulse per frame), so it does not depend on how many frames the rise spans.
- Nothing here has been heard on real music, which this machine does not have; only the synthetic songs. The motion was
  judged from stills and frame sequences (spike lengths changing frame to frame without flicker), not watched live.

**Not done yet**: a quality setting for mesh density (the mirror image is already half density), a real blur for the
mirror image (it fades and roughens, it is not filtered), tuning on real music, and portrait-shaped windows (the camera
backs off so the orb fits, but only 32:9, 21:9, 16:9 and the default window were looked at).

## Milestone 7: Aurora (scene 9 in the table, ID 8)

Broad translucent curtains over a dark lake. This was the plan's "prototype, scope set by measurement"; the gate
(convincing depth and musical response inside the frame budget) passed, so it ships as a scene. IDs: 8 = Aurora.

**Shipped**

- `scenes/aurora.rs` (the ribbon table, clocks, view, scene; unit-tested), `aurora_common.wgsl` (the uniform block and
  noise), `aurora.wgsl` (the ribbon pass) and `aurora_screen.wgsl` (the screen pass). Up to six vertical sheets,
  farthest first, each a 256-column strip drawn as one instanced draw of 1,530 vertices. Screen x is frequency on every
  sheet. The nearest shows the newest history row and each farther one a little older (0 to 1.3 s), so a hit appears
  near and echoes back; the rows are read with time interpolation one row behind the newest, so 60 Hz rows give smooth
  120 Hz motion. A column's height is its bin's level smoothed over a few bins (a floor keeps a dim low glow in silence).
- A sheet is folded in depth by two slow waves (the mids deepen them; Motion 0 keeps a small fixed fold) and its base
  wanders. Where a fold turns edge-on the light is brighter (the pleat: `sqrt(1 + slope^2)^1.5`, capped at 2.2). Light
  is a sharp bright lower edge plus rays that rise from it, lean with height and fade; the edge is anti-aliased
  analytically (the quad reaches below the base and the shader draws the edge), so the ribbons need no MSAA. Rays
  are 1D value noise in three octaves; each finer octave fades out as it nears a pixel in size.
- Depth: farther sheets stand lower in the sky (the same altitude seen from further away), smaller, fainter (gain
  0.22 to 1.0), softer (lower ray contrast, a thicker edge) and carry older spectrum. Compositing is premultiplied
  "over" in farthest-first order: each ribbon adds its light and dims what is behind it by up to 0.6 of its own
  opacity, weighted toward the near ones, so overlapping curtains keep their shapes.
- Onset highlights: for each ring (low and mid onsets) a narrow pale bundle of rays runs up the sheet at the hit's
  spectral centroid, reaching each farther ribbon a little later (0.1 to 0.55 s), dying in about 0.6 s. Flash scales
  them; Flash 0 removes them. The high-frequency envelope only sharpens the rays for a moment.
- Signals: sustained loudness lifts the columns, mids fold the sheets, bass lifts the stack by about a degree at most and stirs
  the lake, the slow energy trend spreads the stack apart (bases +/- 8%, heights +/- 15%), the signal gate dims the light
  to 35% and slows every clock to a quarter. Clocks (`Clocks`) are integrated with `dt` times a speed, so 30 and 120 fps
  agree and Motion 0 stands still.
- The ribbon target is half resolution (`DEFAULT_SCALE`, `GEQ_AURORA_SCALE`) and is theme-free: it stores the weights of
  the palette's three roles (dominant, supporting, accent) plus the opacity, and the screen pass turns them into colour.
  The palette ramp is linear in the three roles, so nothing is lost, and Rainbow takes its hue from screen x. That makes a
  crossfade between two looks of the scene correct from one render (the second draw's `update` is skipped).
- Screen pass (full resolution, non-MSAA): the target over a night sky (the shared `sky.wgsl` stars in three layers, a pale
  Milky Way band, a faint glow along the horizon), a procedural treeline (low hills with clumps of pines) and a lake that
  mirrors the target about the horizon with ripples that stretch and wobble the image more toward the viewer (three taps,
  fresnel falling with depth, the trees reflected as dark). Colour where ribbons pile up is compressed by `1/(1 + 0.45 sum)`
  and only the brightest light burns toward white.
- Camera: a fixed spot at the water's edge looking 7.8 degrees above the horizon, a constant 40 degree vertical field of
  view (a wider picture shows more sky sideways, not more height), so the horizon is at 69% of the height whatever the
  aspect. The user may turn it by 0.09 rad, pitch it by 0.05 and zoom 0.85-1.2; the total turn is capped at 0.11 rad (a test
  checks the ribbons' ends stay off screen at 32:9 and 21:9).
- Composition by aspect, not by cropping: six ribbons at 32:9, five from 2.1:1 (21:9), four from 1.5:1 (16:9), three below
  that; the user's Curtains slider (2-6, `aurora_curtains`, `GEQ_CURTAINS`) caps it. The set always keeps the nearest and the
  farthest.
- Four looks (Boreal Green, Polar Ice, Neon Veil, Prism Curtains), stored thumbnails, `look` grows to nine, settings from
  before the ninth scene load, Motion and Flash tooltips for the scene, README.

**Measured** (5120x1440, real time, GPU times from timestamp queries, no other `geq-gpu` running)

| Case | fps | frame median / p99 | Ribbons (`scene_upd`) | Screen pass (`scene`) | Bloom |
|------|-----|--------------------|-----------------------|-----------------------|-------|
| Demo signal, 6 curtains, scale 0.5 | 120.0 | 8.33 / 8.6-8.7 ms | 0.43 ms | 1.31 ms | 0.45 ms |
| Dense fixture | 120.0 | 8.33 / 8.7 ms | 0.45 ms | 1.31 ms | 0.45 ms |
| Render scale 1.0 / 0.25 (dense) | 120.0 / 120.0 | 8.33 / 8.6-8.7 ms | 1.59 / 0.14 ms | 1.31 ms | 0.45 ms |
| 3 curtains (dense) | 120.0 | 8.33 / 8.7-8.9 ms | 0.24 ms | 1.31 ms | 0.45 ms |

`cost_ms` is 1.8. A scene cycle (Ferrofluid, Aurora, Columns, with the auto-cycle's look changes inside each) held 120.0 fps
in every transition (p99 8.5-8.9 ms): live crossfades into and out of the scene and between two looks of it (one render,
two screen passes). The one hitch of the run (p95 25 ms over 20 frames, the log's `resize` window) came with entering fullscreen, which is a window change and not the scene.

**Findings worth keeping**

- The first captures had two defects that looked like one. Cream-coloured masses in the loud bass columns were not the body
  of the ribbons: they were the onset highlights (a kick and snare landing at 13.6 s of the demo), too wide and too bright
  and summed over six ribbons. Narrowing them (0.035 to 0.011 of the width), cutting their strength and making them follow the
  rays fixed that. Separately, the base edge had a brightness floor, so quiet columns drew as bright as loud ones and the
  stack read as six parallel stripes; making edge and body follow the column's level, widening the wander of each base
  (0.006-0.013 to 0.018-0.044 of a tangent) and deepening the folds turned the stripes into curtains.
- Colour of the body: with the accent role at v = 0.5 the tall columns went cream where green, yellow and red add. The ramp
  now reaches the supporting colour only near the thin ray tops (`t = v^2.2`). Overlap was tamed by three changes made
  together (body 0.55 to 0.30, opacity 0.30 to 0.45 capped at 0.6, the colour compression); they were not tried separately,
  and no pure-additive comparison was kept.
- Isolated events were checked with the percussion fixture (a kick at t = 6 s): calm at 5.9, the bass side rises and a
  highlight leaves the near ribbon at 6.1, its echo is on the second ribbon at 6.25-6.45, a trace remains at 6.8, calm at 7.4.
  The demo's kick repeats every 0.5 s at the same frequency, so the first version (life `exp(-1.8 t)`) piled the
  highlights into a near-permanent patch; `exp(-3.4 t)` and a faster climb gave one accent per hit.
- Half resolution is nearly free of visible loss: next to scale 1.0 only the sharp lower edge is a little softer (one
  full-resolution pixel), the rays are the same. The ribbon pass is fill-bound (0.14, 0.43 and 1.59 ms at 0.25, 0.5 and 1.0, so
  about 1.15 ms saved at the default); the picture came out right at 0.5 and the scale stays an environment setting.
- Fine ray detail aliased into a moire on the nearest ribbon at half resolution (the finest octave at about one cell per
  pixel). Fading each octave by `fwidth` of its coordinate removed it. A derivative must be taken before the early return, or
  the shader leaves uniform control flow.
- The screen pass is the expensive half, not the ribbons. Bisecting its 1.46 ms: the three star layers 0.52 ms, the Milky Way
  band 0.27 ms (two cloud evaluations over a third of the sky for a barely visible effect; one evaluation over a narrower
  band is 0.12), the lake 0.11 ms, and 0.57 ms for everything else (the sky direction, the palette lookups, the treeline, the
  clear and the target read). The stars are the next saving if the budget ever needs one.
- The ribbons reach 1.4 screen half widths; turning the view sideways shows their end where they fade (from 1.12), which at
  32:9 and a 0.16 rad turn would put the screen edge at 1.41 (worked out, not seen). The turn is capped at 0.11 rad and a test computes the edge for 21:9
  and 32:9 at the widest zoom.

**Not done yet / not verified**

- Nothing here has been heard on real music, which this machine does not have; only the synthetic songs. The motion was
  judged from stepped captures (the kick timeline above, five frames around the demo's hit) and the frame-time runs, not
  watched live. Real, tilted music may want the treble lift in the vertex shader (`0.80 + 0.55 x`) tuned.
- A bass hit does not ripple the lake (the lake's ripples follow sustained bass); the Ferrofluid floor's expanding rings would
  carry over. The highlights use only the low and mid onset rings (the analyzer launches none for the highs).
- Windowed 2400x1000, 21:9, 16:9, 1:1 and 32:9 were looked at; tall portrait windows were not. The look browser thumbnails were
  captured at 1600x450 with `tools/make-thumbs.sh`'s settings.
- Quality settings beyond Curtains: a render-scale control and a star-layer count are the obvious ones if a weaker GPU needs
  them (the ribbon pass and the stars are the two costs that scale).


## Milestone 8: Fireworks (scene 10 in the table, ID 9)

Rockets climb a dark sky while the music is loud, and a hit sets one off. IDs: 9 = Fireworks.

**Shipped**

- `scenes/fireworks.rs` (the choreography `Show`, the scene; unit-tested), `fireworks_sim.wgsl` (compute) and `fireworks.wgsl`
  (floor, rockets, stars), on the existing `Particles` ring (65,536 particles, 4.3 s longest life, non-MSAA). Camera: eye
  (0, 2, 23) aimed at (0, 5.6, 0), 40 degrees, two thirds sky. The pad line is 12.5 x aspect wide, 14 to 40 (see the findings).
- Hits: `modulation::RiseHits`, the shared detector the plan asked for (same shape and tests as `PeakHits`). It works on the Fountain's
  normalized rise, so Sensitivity keeps its meaning: strength is 0 at the Fountain's `SPARK_START` and 1 at `SPARK_FULL`, at the
  sensitivity. Peaks along the 192 bins, merged unless the rise dips below 0.7 of the lower peak between them, a hit reported once its
  rise has peaked (or after 15 ms), one per position per 120 ms, and an event (hits starting within 60 ms) is at most its two
  strongest parts. Each bin is judged after taking off twice its own slow average (0.6 s). Dense music is thinned: every hit in the last
  second raises the strength the next needs by 0.12.
- Rockets: a table of 16 closed-form arcs in Rust (apex 1.1-1.5 s, 5-10 units high), launched from pads scattered +-6 in depth along
  the line, at 0.12 + 0.95 x the sustained level of the lows, mids and highs (about 0.4 to 3 a second in all, none with the gate
  closed). A hit sets off the rocket in the air nearest its place on the line, in the same frame, where it is; with none within
  0.28 of the line's width and above 3.5 units, a mortar pops low (3.8-6). A rocket nobody sets off bursts at its apex, small and
  dim, unless four bursts are already alive, when it fades out instead. In silence the rockets fizzle in 0.35 s and nothing launches.
- Bursts: the CPU sends a burst's line position, strength, seed and star count in `PartU.bursts` and its centre and radius in
  `PartU.extra` (so at most four a frame; the queue carries the rest). Slot 0 of a burst is the detonation flash, the others are
  stars: directions from a Fibonacci spiral over the star's index (a round shell at any size), each star's frequency picked by
  rejection sampling on the displayed spectrum at that moment (colour on the theme ramp, a little of its speed), with a mild treble
  tilt. The hit's place sets the kind: willow below 0.30 of the line (big, heavy drag, 3-4 s, trails up to 1.2 units that droop),
  peony to 0.66 (even sphere, 2-3 s), crackle above (small, fast, 0.8-1.5 s, twinkling by a hash of time). A star is born white-hot,
  takes its colour, cools toward ember orange, lights up over 0.22 s as the shell opens, and the brightest 7% use the accent role.
  Sparks that reach the floor bounce and burn out, and are mirrored in the glass like the Fountain's.
- Bounds: a hit beyond seven live bursts (2.6 s) is dropped, stars per burst thin by 1/(1 + 0.2 x alive), Density scales them
  (100-900 each), the floor keeps at most eight pools of light (a quick flash plus a glow that rises over 0.6 s and fades with the
  burst), radius shrinks to 0.7 of full toward 16:9. Motion scales the wind on the stars and the idle camera, Flash scales the
  detonation flash and all floor light (Flash 0 leaves bare stars).
- `particles_draw.wgsl` gained `sprite_make_trail` (the streak length per particle; `sprite_make` is the old call with `TRAIL_S`).
- Four looks (Gold Willow, Ice Chrysanthemum, Neon Peony, Prism Finale), stored thumbnails, Display menu Sensitivity and Density
  (the Fountain's, tooltips updated), a Motion tooltip, README, `GEQ_FWDEBUG=1` (one line per hit and burst).
  `settings` needed no new field (`look` grows to ten).

**Measured** (5120x1440, real time, GPU times from timestamp queries, no other `geq-gpu` running)

| Case | fps | frame median / p99 | Scene (`scene`) | Compute (`scene_upd`) | Bloom |
|------|-----|--------------------|-----------------|-----------------------|-------|
| Dense fixture, designed settings | 120.0 (119.9 in one window) | 8.0-8.9 / 9.4-9.7 ms | 0.25-0.53 ms (median 0.47) | 0.00 ms | 0.45 ms |
| Dense, Density 1.5, Sensitivity 1.8 | 120.0 | 8.33 / 8.7-9.0 ms | 0.26-0.83 ms (median 0.61) | 0.01 ms | 0.45 ms |

`cost_ms` is 0.8 (the brief guessed 0.3-1 ms). The choreography and detector cost 0.24 microseconds a frame on the CPU
(`cargo test --release cpu_cost -- --ignored --nocapture`). The frame-time tail moved between runs in this session while the scene's own
cost did not: the same scene gave p95 8.6-8.7 ms earlier, 9.3-9.4 ms in the run above and 8.9-9.0 ms next to a Fountain control at
8.5, so the 0.3-0.8 ms of tail is the machine (Spotify and others were running), not the scene. All of them kept 120.0 fps with the
worst frame under 10.3 ms. Transitions on the final build held 120.0 fps with p99 8.4-9.5 ms: Aurora into Fireworks, between two looks of
it (one simulation, two draws) and out to the columns; two isolated single frames of 15.6 and 17.4 ms in a 55 s cycle were not tied to
a transition. Prewarm is 6.6-7.5 ms. Tests: 87 pass (69 before: 8 for the detector, one of them on the real analyzer and the percussion
fixture, and 10 for the choreography).

Timing, with the percussion fixture's exact event times: every kick is one hit and one burst, 25 ms after the drum in the 120 Hz unit test
(the rise is first measured one hop after it and then settles) and 33 ms in the stepped 60 Hz captures (two frames), every snare is two
(its body on the left with the kicks, its crack on the right) and nothing falls between the drums. The impulse fixture gives one burst per
impulse at the same delay. A burst is set off in the frame that reports the hit. The apex-only alternative was not built; the log says what
it would be: a rocket reaches its apex 1.1-1.5 s after its launch whatever the music does, and the 16 apex bursts of the percussion
log landed 0.05-0.57 s (median 0.20) from the nearest drum, which is an unrelated flash next to the real ones. So apex bursts are small,
dim and kept to the quiet moments (fewer than four bursts alive), and the hits are what detonate.

| Fixture, 40 s | Hits | Detonated | Mortar | Apex |
|---------------|------|-----------|--------|------|
| Demo (groove) | 80 (2.0 a second) | 61 | 17 | 3 |
| Dense | 82 | 62 | 17 | 2 |
| Percussion | 40 | 21 | 19 | 16 |
| Tones (steady chord) | 1 | 0 | 1 | 51 |
| Sweep | 12 | 5 | 7 | 36 |
| Quiet (the demo at 3%) | 38 | 23 | 15 | 13 |
| Silence | 0 | 0 | 0 | 0 |

At most 23% of the sky region is lit (any channel above 45/255) in the dense captures, and 1-2.6% near white.

**Findings worth keeping**

- A broadband snare cleared the threshold in its body, its noisy middle and its crack, and gave three or four hits for one event. The
  valley rule could not split or merge them, because strength saturates at 1 and the plateau is noise. Capping an event at its two
  strongest parts left a kick as one burst and a snare as two. Strength is tracked above 1 (to 2) for finding the peak and clamped on
  output: with the clamp a hit that kept climbing past full looked flat and was reported a frame late.
- A steady chord whose partials sit within about two bins of the 21 ms window beats inside it, so its rise jitters at the chord's own
  frequencies; with only a threshold it detonated 1.8 times a second (44 hits in 24 s on the tones fixture). A slow per-bin
  background (twice its 0.6 s average) took it to zero and the demo from 3.3 to 1.9 hits a second, with kicks and snares unchanged
  (swept in `dump_hit_tuning`: minimum strength 0.18/0.30/0.45 against background factor 1/1.5/2).
- Streak length proportional to speed made hedgehogs: a willow star leaves at 13 units a second and a 0.34 s trail is a line most of
  the way to the burst's edge. Each kind caps its streak in world units (1.2, 0.38, 0.25), so only the slow stars of a falling willow
  reach the long droop. Straight quads read as drooping trails in stills; the F3 persistence fallback was not needed.
- 1,200-1,500 overlapping additive stars burn to a flat yellow disc (centre pixels 250/255), with the detonation flash lost inside it.
  Dimmer stars (0.30 + 0.32 x strength), a shell that lights up over 0.22 s from 30%, smaller counts and a bigger flash gave dark gaps
  between stars and a core that stands apart. The amber theme through ACES reads gold-yellow, with the orange only in the cooling tails.
- A rectilinear view at 32:9 stretches whatever stands near the edges. The first camera (50 degrees, 17.5 away, a line 46 wide) put
  the outer pads 52 degrees off the axis and the far-right burst came out an ellipse about 1.6 times too wide. Backing off to 23 units at
  40 degrees and holding the line to 40 (41 degrees at the outermost pad, 1.3x) made the shells round. Parallax is weaker at the
  longer distance, and the depth scatter of +-6 still gives sizes 0.8-1.35.
- Detonating the shell in flight needs rockets in the air: on the sparse percussion fixture half the hits find none and pop a mortar,
  on the demo and dense songs 78% find one.
- A kick on every beat is a willow on the left every beat, so the left side is rarely empty on four-on-the-floor music; the
  count thinning (1/(1 + 0.2 x alive)) and the seven-burst cap are what keep that to a dome, not a wall.
- A capture is perturbed when the pointer is over the capture window: one of five thumbnail candidates showed a different camera.
  Two runs that hash identically is the check, and the thumbnails above were only accepted when a pair matched. `GEQ_SIZE=1280x360`
  gets a window 520 high (the minimum), so aspect tests need 2560x720.
- `tools/make-thumbs.sh` had a duplicate block that called `shot` before it was defined (it could not have run to the end); removed.

**Differences from the brief**

- A burst's `where` is its place on the line, not an index into a shell table (the table is a uniform of its own, `FwU`, that only the
  rockets and the floor read): the centre and radius travel in `PartU.extra`, which caps a frame at four bursts.
- A snare is two bursts (body left, crack right), where the brief says one per event; a kick is one.
- Hits are judged against their own background, which the Fountain does not do, so Sensitivity moves both thresholds the same way
  but a hit must also stand out from steady sound.
- The spectrum sampler runs on the displayed level with a mild treble tilt (0.8 to 2.0), because real music's highs are quiet.
- The floor's pool is a 0.1 s flash plus a glow that rises over 0.6 s and fades with the burst, not a single fade of 0.6 s.
- A rocket turned away at its apex in a busy sky fades out instead of vanishing.

**Not done / not verified**

- Nothing here has been heard on real music, which this machine does not have; only the synthetic songs. The motion was judged from
  stills at fixed times and the timing log, not watched live; the 33 ms was measured in stepped captures, not with a real-time
  recording of the output.
- Windowed 2560x720, 1600x900 and the windowed UI were looked at; tall portrait windows were not. The look browser thumbnails are
  19.9 s of the demo.
- The apex burst of a quiet passage is a small dim sphere (about 100 stars); it was seen in the tones and stereo logs and the unit
  tests, and only glimpsed in stills.
- No render scale or star-count quality setting beyond Density (the scene costs 0.5 ms).

## Fix: quieter sound while the app is open (2026-10-13)

- Cause: the "Graphic EQ" virtual sink did not start at unity. WirePlumber's `restore-stream` saves a sink's volume under
  `Audio/Sink:media.name:Graphic EQ` and restores it when the sink next appears. The sink was the default, so the volume keys
  changed it and the change was saved. Here it held 0.5116 linear ("80%", -5.82 dB) and came back every start, stacked on the
  real output's own volume. Every band at 0 dB is a pass-through, so EQ off made no difference. At 100% the sink is exactly
  transparent (a 0.2 peak tone came out at 0.2, and at 50% it came out at 0.025, an eighth, on the cubic scale).
- First fix, kept: `Engine::start` sets the EQ sink to 100% and unmutes it once it exists, and again 250 ms later in case the
  restore lands late (`routing::reset_sink_volume`). It only touches the app's own sink. `state.restore-props = false` in the
  node properties does nothing on WirePlumber 0.4.17 (a node property arrives as the string "false"; only a WirePlumber rule
  gives the boolean), and neither `channelmix.disable` nor `audioconvert.volume` makes the sink ignore its volume.
- Second fix, the real one: the EQ is no longer the default output. With it as the default the volume keys acted on it, which
  stayed capped at 100% (Cinnamon's amplified volume is off), so volume-up did nothing while the app was open and volume-down
  stacked on the speaker level. Now the default stays the real output and `routing::StreamRedirect` (a `pactl subscribe`
  watcher) moves each new playback stream onto the EQ sink as it appears; `move_streams` at start-up and quit does the rest.
  If the chosen output is not the system default, the default is set to it while the app runs and put back on quit.
- Checks (all against the real PipeWire): `cargo test --release sink_starts_at_unity -- --ignored` leaves the sink at 80% and
  restarts the engine, expecting 100% (fails with `Some("80%")` without the reset). `new_streams_follow_the_eq` starts a
  silent player on the real output explicitly and expects it on the EQ sink within 3 s while the default does not change:
  49 ms with the watcher, never without it. (A first version passed without the watcher because WirePlumber's `restore-target`
  puts a player it saw being moved straight onto the EQ sink; the explicit device avoids that.) The real app was run for 12 s
  with Spotify and two silent players: default and speaker volume unchanged, EQ sink at 100%, all three streams on the EQ,
  one started after the app included, and back on the speakers on quit, with no engine or watcher left. If the engine dies,
  its streams fall back to the default output by themselves.
- Not done / not verified: a new stream plays on the real output until it is moved, 49 ms in the test; not listened for, and a
  stream that starts loudly at its first sample could be heard for that moment. Streams you had sent to another device are moved
  too, as before. The level was read from the sinks and a recorded tone, not heard on this machine.
