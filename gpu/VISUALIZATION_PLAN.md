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
proportion to its level, and throws a burst when it pushes its peak cap higher. **Fountain** is its
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

**Not done yet** (as planned): waveform feed and persistence (F2a/d, F3), compute particles (F4), render
scale (F5, not needed by these scenes), optional beat timing, and the later scenes.
