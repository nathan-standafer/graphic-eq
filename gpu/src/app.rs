use crate::analyzer::{Levels, CURVE_POINTS, FLOOR_DB};
use crate::audio::Audio;
use crate::bands;
use crate::clock;
use crate::drums::DrumHit;
use crate::modulation::{column_pushes, spark_worthy, DrumHits, Modulation, RiseNorm, Signals};
use crate::presets::{self, Idle, Look};
use crate::render::{Draw, Gpu, PostLook, SceneCallback, BLOOM_LEVELS, HIST_W, RISE_FULL};
use crate::routing;
use crate::scenes::{self, CamState, Control, SceneParams};
use crate::settings::{Settings, Variant, PRESETS};
use crate::synth::Fixture;
use crate::thumbs::Thumbs;
use eframe::egui::{self, Color32, RichText};
use eframe::egui_wgpu;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

const CUSTOM: &str = "Custom";
const FALL_DB_PER_S: [f32; 3] = [14.0, 38.0, 110.0];
const PEAK_HOLD_S: f32 = 1.1;
const PEAK_FALL_DB_PER_S: f32 = 22.0;
const HISTORY_RATE: f32 = 60.0;
const THEMES: [&str; 5] = ["Classic green/red", "Amber", "Ice", "Neon", "Rainbow"];
const FADE_S: f64 = 0.9;
const LAST_SCENE: usize = scenes::ID_SLOTS - 1;
const OVERLAY_VISIBLE_S: f32 = 2.8;

/// A scene with its look resolved: preset defaults plus the user's live changes.
#[derive(Clone, Copy)]
struct Resolved {
    scene: u32,
    look: &'static Look,
    theme: u32,
    motion: f32,
    flash: f32,
    mode: u32,
}

struct Transition {
    from: Resolved,
    from_cam: CamState,
    start: f64,
    start_frame: u64,
    /// Both scenes would not fit the frame budget live: after the first frame the outgoing scene
    /// is held as a still image while the incoming one runs.
    frozen: bool,
}

/// GPU time the compositor itself needs (bloom, crossfade, composite), and the share of the
/// 120 Hz frame we are willing to spend before falling back to a frozen outgoing image.
const POST_MS: f32 = 1.5;
const LIVE_BUDGET_MS: f32 = 6.6;

/// The slow, bounded camera drift. It fades out while the user holds the camera and eases back
/// in a few seconds after they let go; its speed follows the signal gate so it settles on silence.
struct IdleCam {
    amount: f32,
    phase: f32,
    last_input: f64,
}

impl IdleCam {
    fn update(&mut self, now: f64, dt: f32, gate: f32) {
        let target = if now - self.last_input < 4.0 { 0.0 } else { 1.0 };
        let tau = if target < self.amount { 0.25 } else { 2.5 };
        self.amount += (target - self.amount) * (1.0 - (-dt / tau).exp());
        self.phase += dt * (0.35 + 0.65 * gate);
    }

    /// yaw, pitch, zoom, roll offsets for a scene's idle path.
    fn offsets(&self, idle: &Idle, motion: f32) -> [f32; 4] {
        let k = self.amount * motion;
        let (p, tau) = (self.phase, std::f32::consts::TAU);
        [
            idle.yaw * k * (tau * p / 37.0).sin(),
            idle.pitch * k * (tau * p / 29.0 + 1.3).sin(),
            idle.zoom * k * (tau * p / 43.0 + 2.1).sin(),
            idle.roll * k * (tau * p / 53.0 + 0.7).sin(),
        ]
    }
}

/// Frame-time statistics (GEQ_FPS), kept apart for startup, steady playback, resizing and transitions.
struct Diag {
    on: bool,
    last_ui: Instant,
    buckets: [Vec<f32>; 4],
    window_start: f64,
    resize_cooldown: u32,
    last_size: [u32; 2],
    latency: bool,
}

const BUCKETS: [&str; 4] = ["steady", "startup", "transition", "resize"];

fn percentile(v: &mut Vec<f32>, p: f32) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f32 * p).round() as usize]
}

pub struct App {
    cfg: Settings,
    audio: Audio,
    quit: Arc<AtomicBool>,
    disp: [f32; bands::COUNT],
    peak: [f32; bands::COUNT],
    peak_age: [f32; bands::COUNT],
    curve: [f32; CURVE_POINTS],
    curve_l: [f32; CURVE_POINTS],
    curve_r: [f32; CURVE_POINTS],
    /// How steeply each waterfall bin's volume is rising, relative to the music's own hardest recent hits
    /// (about 1 at those), for the history rows.
    rise: [f32; CURVE_POINTS],
    rise_norm: RiseNorm,
    modn: Modulation,
    sig: Signals,
    levels_f: [f32; 11],
    /// The 192-point spectrum as displayed (0..1), for the ferrofluid orb's spikes.
    spectrum_f: [f32; CURVE_POINTS],
    peaks_f: [f32; 11],
    /// The drum hits found since the last frame, the Sparks overlay's own view of them (`modulation::DrumHits`) and
    /// the bursts it throws this frame, per column.
    drums: Vec<DrumHit>,
    spark_hits: DrumHits,
    peak_hits: [f32; 11],
    now: f64,
    last_wall: f64,
    stepped: Option<f32>,
    frames: u64,
    hist_acc: f32,
    sinks: Vec<(String, String)>,
    dirty_since: Option<Instant>,
    shot: Option<(String, f64)>, // GEQ_SHOT=path[,seconds]: save a screenshot of this window and exit
    shot_sent: bool,
    cams: [CamState; scenes::ID_SLOTS],
    idle: IdleCam,
    fullscreen: bool,
    /// GEQ_BARE=1: scene only, no panels or overlay (captures at an exact aspect without going full screen).
    bare: bool,
    fs_since: f64,
    last_activity: f64,
    trans: Option<Transition>,
    next_cycle: f64,
    built: [bool; scenes::ID_SLOTS],
    diag: Diag,
    thumbs: Thumbs,
    browser_open: bool,
}

fn blank_signals() -> Signals {
    Signals { fx: [0.0; 4], sus: [0.0; 4], gate: 0.0, rings: [[0.0; 4]; 8] }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        clock::init();
        let mut cfg = Settings::load();
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu backend required");
        let gpu = Gpu::new(&rs.device, &rs.queue, rs.target_format);
        rs.renderer.write().callback_resources.insert(gpu);

        let quit = Arc::new(AtomicBool::new(false));
        let q = quit.clone();
        let _ = ctrlc::set_handler(move || q.store(true, Ordering::SeqCst));

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(14, 16, 22);
        visuals.window_fill = Color32::from_rgb(22, 26, 34);
        visuals.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.55 };
        cc.egui_ctx.set_visuals(visuals);
        cc.egui_ctx.all_styles_mut(|s| s.spacing.slider_rail_height = 8.0);

        let shot = std::env::var("GEQ_SHOT").ok().map(|v| {
            let mut p = v.splitn(2, ',');
            let path = p.next().unwrap().to_string();
            (path, p.next().and_then(|s| s.parse().ok()).unwrap_or(6.0))
        });
        // GEQ_SYNTH: synthetic audio in place of parec; no EQ engine, no routing changes, settings read-only
        let replay = std::env::var("GEQ_REPLAY").ok().and_then(|path| match crate::synth::load_wav(&path) {
            Ok(frames) => {
                eprintln!("replaying {path}: {:.1} s", frames.len() as f32 / 48_000.0);
                Some(frames)
            }
            Err(e) => {
                eprintln!("GEQ_REPLAY: {e}");
                None
            }
        });
        let synth = replay.as_ref().map(|_| Fixture::Replay).or(std::env::var("GEQ_SYNTH").ok().map(|v| {
            Fixture::parse(&v).unwrap_or_else(|| {
                eprintln!("GEQ_SYNTH={v}: unknown fixture, using demo (silence quiet impulse percussion tones sweep dense stereo phase)");
                Fixture::Demo
            })
        }));
        // captures are deterministic: the synthetic source and the clock advance in fixed steps
        let stepped = (synth.is_some() && shot.is_some() && std::env::var_os("GEQ_REALTIME").is_none())
            .then(|| std::env::var("GEQ_DT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0 / 60.0));
        let audio = match synth {
            Some(fx) => {
                crate::settings::set_read_only();
                Audio::new_synth(fx, replay, stepped.is_some(), effective(&cfg))
            }
            None => Audio::new(cfg.output.clone(), cfg.source_pre, effective(&cfg)),
        };

        // environment overrides for captures, applied once so the UI stays usable
        if let Ok(v) = std::env::var("GEQ_SCENE") {
            cfg.scene = v.parse().unwrap_or(0);
        }
        cfg.scene = scenes::info(cfg.scene).id;
        if let Ok(v) = std::env::var("GEQ_PRESET") {
            cfg.look[(cfg.scene as usize).min(LAST_SCENE)] = v.parse().unwrap_or(0);
        }
        let apply_look = shot.is_some() || std::env::var("GEQ_PRESET").is_ok() || std::env::var("GEQ_SETTINGS").map_or(false, |v| v == "default");
        if std::env::var_os("GEQ_STEREO").is_some() {
            cfg.stereo = true;
        }
        if let Ok(v) = std::env::var("GEQ_BLOOM") {
            cfg.bloom = v.parse().unwrap_or(cfg.bloom);
        }
        if let Ok(v) = std::env::var("GEQ_GRADIENT") {
            cfg.spark_sensitivity = v.parse().unwrap_or(cfg.spark_sensitivity);
        }
        if let Ok(v) = std::env::var("GEQ_DENSITY") {
            cfg.density = v.parse().unwrap_or(cfg.density);
        }
        if let Ok(v) = std::env::var("GEQ_SPIKES") {
            cfg.orb_spikes = v.parse().unwrap_or(cfg.orb_spikes);
        }
        if let Ok(v) = std::env::var("GEQ_CURTAINS") {
            cfg.aurora_curtains = v.parse().unwrap_or(cfg.aurora_curtains);
        }
        if let Ok(v) = std::env::var("GEQ_RIPPLE_LIFE") {
            cfg.ripple_life = v.parse().unwrap_or(cfg.ripple_life);
        }
        if std::env::var("GEQ_SPARKS").map_or(false, |v| v == "0") {
            cfg.sparks = false;
        }
        if let Some(secs) = std::env::var("GEQ_CYCLE").ok().and_then(|v| v.parse::<f32>().ok()) {
            cfg.auto_cycle = true;
            cfg.cycle_secs = secs;
        }
        let mut cams = [CamState::default(); scenes::ID_SLOTS];
        if let Ok(v) = std::env::var("GEQ_ORBIT") {
            let mut p = v.split(',').filter_map(|s| s.parse::<f32>().ok());
            let s = (cfg.scene as usize).min(LAST_SCENE);
            (cams[s].yaw, cams[s].pitch) = (p.next().unwrap_or(0.0), p.next().unwrap_or(0.0));
        }
        if let Ok(v) = std::env::var("GEQ_PAN") {
            let mut p = v.split(',').filter_map(|s| s.parse::<f32>().ok());
            cams[(cfg.scene as usize).min(LAST_SCENE)].pan = [p.next().unwrap_or(0.0), p.next().unwrap_or(0.0)];
        }

        let mut app = App {
            cfg,
            audio,
            quit,
            disp: [FLOOR_DB; bands::COUNT],
            peak: [FLOOR_DB; bands::COUNT],
            peak_age: [0.0; bands::COUNT],
            curve: [FLOOR_DB; CURVE_POINTS],
            curve_l: [FLOOR_DB; CURVE_POINTS],
            curve_r: [FLOOR_DB; CURVE_POINTS],
            rise: [0.0; CURVE_POINTS],
            rise_norm: RiseNorm::new(),
            modn: Modulation::new(),
            sig: blank_signals(),
            levels_f: [0.0; 11],
            spectrum_f: [0.0; CURVE_POINTS],
            peaks_f: [0.0; 11],
            drums: Vec::new(),
            spark_hits: DrumHits::new(),
            peak_hits: [0.0; 11],
            now: 0.0,
            last_wall: clock::now(),
            stepped,
            frames: 0,
            hist_acc: 0.0,
            sinks: Vec::new(),
            dirty_since: None,
            shot,
            shot_sent: false,
            cams,
            idle: IdleCam { amount: 1.0, phase: 0.0, last_input: -100.0 },
            fullscreen: std::env::var_os("GEQ_FAKE_FS").is_some(), // GEQ_FAKE_FS=1: fullscreen UI state without the window change
            bare: std::env::var_os("GEQ_BARE").is_some(),
            fs_since: if std::env::var_os("GEQ_FAKE_FS").is_some() { 0.001 } else { 0.0 },
            last_activity: 0.0,
            trans: None,
            next_cycle: 0.0,
            built: [false; scenes::ID_SLOTS],
            thumbs: Thumbs::default(),
            browser_open: std::env::var_os("GEQ_BROWSER").is_some(),
            diag: Diag {
                on: std::env::var_os("GEQ_FPS").is_some() || std::env::var_os("GEQ_GPUTIME").is_some(),
                last_ui: Instant::now(),
                buckets: Default::default(),
                window_start: 0.0,
                resize_cooldown: 0,
                last_size: [0; 2],
                latency: std::env::var_os("GEQ_LATENCY").is_some(),
            },
        };
        if apply_look {
            app.apply_look();
        }
        // explicit overrides win over the preset
        if let Ok(v) = std::env::var("GEQ_THEME") {
            app.cfg.theme = v.parse().unwrap_or(app.cfg.theme);
        }
        if let Ok(v) = std::env::var("GEQ_MOTION") {
            app.cfg.motion = v.parse().unwrap_or(app.cfg.motion);
        }
        if let Ok(v) = std::env::var("GEQ_FLASH") {
            app.cfg.flash = v.parse().unwrap_or(app.cfg.flash);
        }
        app.next_cycle = app.cfg.cycle_secs as f64;
        app
    }

    fn frac(&self, db: f32) -> f32 {
        ((db + self.cfg.sensitivity + self.cfg.range) / self.cfg.range).clamp(0.0, 1.0)
    }

    fn touch(&mut self) {
        self.dirty_since = Some(Instant::now());
    }

    fn push_gains(&mut self) {
        self.audio.set_gains(effective(&self.cfg));
    }

    fn step_levels(&mut self, lv: &Levels, dt: f32) {
        let fall = FALL_DB_PER_S[self.cfg.fall as usize % 3];
        for i in 0..bands::COUNT {
            let r = lv.bands[i];
            self.disp[i] = if r >= self.disp[i] { r } else { (self.disp[i] - fall * dt).max(r) };
            if self.disp[i] >= self.peak[i] {
                self.peak[i] = self.disp[i];
                self.peak_age[i] = 0.0;
            } else {
                self.peak_age[i] += dt;
                if self.peak_age[i] > PEAK_HOLD_S {
                    self.peak[i] = (self.peak[i] - PEAK_FALL_DB_PER_S * dt).max(self.disp[i]);
                }
            }
        }
        let smooth = |disp: &mut [f32; CURVE_POINTS], raw: &[f32; CURVE_POINTS]| {
            for i in 0..CURVE_POINTS {
                disp[i] = if raw[i] >= disp[i] { raw[i] } else { (disp[i] - fall * dt).max(raw[i]) };
            }
        };
        smooth(&mut self.curve, &lv.curve);
        smooth(&mut self.curve_l, &lv.curve_l);
        smooth(&mut self.curve_r, &lv.curve_r);
        self.rise = self.rise_norm.update(&lv.rise, dt);
        for k in 0..CURVE_POINTS {
            self.spectrum_f[k] = self.frac(self.curve[k]);
        }
        for i in 0..11 {
            self.levels_f[i] = self.frac(self.disp[i]);
            self.peaks_f[i] = if self.cfg.peak_caps { self.frac(self.peak[i]) } else { 0.0 };
        }
    }

    // ------------------------------------------------------------------ looks and scenes

    fn look_key(&self, scene: u32) -> (usize, &'static Look) {
        let idx = self.cfg.look[(scene as usize).min(LAST_SCENE)] as usize;
        let l = presets::get(scene, idx);
        (idx, l)
    }

    /// Saved variant for the look, else the preset's own defaults.
    fn base_variant(&self, l: &Look) -> Variant {
        self.cfg.variants.get(&presets::key(l)).cloned().unwrap_or(Variant { theme: l.theme, motion: l.motion, flash: l.flash })
    }

    /// Load the selected preset's palette and intensities into the live settings.
    fn apply_look(&mut self) {
        let (_, l) = self.look_key(self.cfg.scene);
        let v = self.base_variant(l);
        (self.cfg.theme, self.cfg.motion, self.cfg.flash) = (v.theme, v.motion, v.flash);
    }

    fn look_modified(&self) -> bool {
        let (_, l) = self.look_key(self.cfg.scene);
        self.base_variant(l) != Variant { theme: self.cfg.theme, motion: self.cfg.motion, flash: self.cfg.flash }
    }

    fn resolved(&self) -> Resolved {
        let (_, look) = self.look_key(self.cfg.scene);
        Resolved {
            scene: self.cfg.scene,
            look,
            theme: self.cfg.theme,
            motion: self.cfg.motion,
            flash: self.cfg.flash,
            mode: if self.cfg.scene == scenes::WATERFALL && self.cfg.stereo { 1 } else { 0 },
        }
    }

    /// Switch scene and/or preset. The incoming scene is built first and the current one keeps
    /// drawing; only then does the crossfade start.
    fn switch(&mut self, frame: &eframe::Frame, scene: u32, preset: Option<usize>) {
        let preset = preset.unwrap_or(self.cfg.look[(scene as usize).min(LAST_SCENE)] as usize);
        if scene == self.cfg.scene && preset == self.cfg.look[(scene as usize).min(LAST_SCENE)] as usize {
            return;
        }
        if let Some(ms) = with_gpu(frame, |g, d| g.prepare_scene(d, scene)).flatten() {
            self.built[scene as usize] = true;
            if self.diag.on {
                eprintln!("scene {scene} built in {ms:.1} ms (on demand)");
            }
        }
        if self.cfg.crossfade && self.frames > 5 {
            // an interrupted fade completes instantly: the incoming scene becomes the outgoing one
            let from = self.resolved();
            // cost scales with pixels; the hints are for 5120x1440
            let px = (self.diag.last_size[0] as f32 * self.diag.last_size[1] as f32 / 7.37e6).clamp(0.05, 4.0);
            let incoming = self.cfg.stereo && scene == scenes::WATERFALL;
            let cost = (scene_cost(&from) + scene_cost_of(scene, incoming)) * px + POST_MS * px.max(0.4);
            let frozen = match std::env::var("GEQ_XFADE").as_deref() {
                Ok("live") => false,
                Ok("frozen") => true,
                _ => cost > LIVE_BUDGET_MS,
            };
            if self.diag.on {
                eprintln!("crossfade {} -> {}: estimated {cost:.1} ms, {}", from.scene, scene, if frozen { "outgoing frozen" } else { "both live" });
            }
            self.trans = Some(Transition { from, from_cam: self.cams[self.cfg.scene as usize], start: self.now, start_frame: self.frames, frozen });
        } else {
            self.trans = None;
        }
        // an incoming simulation scene starts from a known state (seeded, so captures repeat)
        if scene != self.cfg.scene {
            let seed = if self.stepped.is_some() { 1 } else { self.frames };
            with_gpu(frame, |g, _| g.reset_scene(scene, seed));
        }
        self.cfg.scene = scene;
        self.cfg.look[scene as usize] = preset as u32;
        self.apply_look();
        self.activity();
        self.touch();
    }

    fn activity(&mut self) {
        self.last_activity = self.now;
        self.next_cycle = self.now + self.cfg.cycle_secs as f64;
    }

    /// Presets reachable from the number keys: favorites if there are any, else the whole catalog.
    fn slots(&self) -> Vec<(u32, usize)> {
        let all: Vec<(u32, usize, &Look)> = scenes::ALL
            .iter()
            .flat_map(|s| presets::for_scene(*s).into_iter().map(move |(i, l)| (*s, i, l)))
            .collect();
        let fav: Vec<(u32, usize)> =
            all.iter().filter(|(_, _, l)| self.cfg.favorites.contains(&presets::key(l))).map(|(s, i, _)| (*s, *i)).collect();
        if fav.is_empty() { all.iter().map(|(s, i, _)| (*s, *i)).collect() } else { fav }
    }

    fn step_scene(&mut self, frame: &eframe::Frame, dir: i32) {
        let n = scenes::ALL.len() as i32;
        let cur = scenes::ALL.iter().position(|s| *s == self.cfg.scene).unwrap_or(0) as i32;
        let next = scenes::ALL[(cur + dir).rem_euclid(n) as usize];
        self.switch(frame, next, None);
    }

    fn params_for(&self, r: &Resolved, cam: &CamState, sig: &Signals) -> SceneParams {
        let mut cam = *cam;
        cam.idle = self.idle.offsets(&r.look.idle, r.motion);
        SceneParams {
            time: self.now as f32,
            mode: r.mode,
            theme: r.theme,
            exposure: r.look.exposure,
            flash: r.flash,
            motion: r.motion,
            gate: sig.gate,
            levels: self.levels_f,
            peaks: self.peaks_f,
            fx: sig.fx,
            sus: sig.sus,
            rings: sig.rings,
            cam,
            row_frac: (self.hist_acc * HISTORY_RATE).clamp(0.0, 1.0),
            sparks: self.cfg.sparks,
            density: self.cfg.density,
            gradient: self.cfg.spark_sensitivity,
            peak_hits: self.peak_hits,
            drums: std::array::from_fn(|i| self.drums.get(i).copied().unwrap_or_default()),
            n_drums: self.drums.len().min(scenes::MAX_DRUMS),
            spikes: self.cfg.orb_spikes,
            curtains: self.cfg.aurora_curtains,
            ripple_life: self.cfg.ripple_life,
            spectrum: self.spectrum_f,
            rise: self.rise,
        }
    }

    fn post_look(&self, a: &Look, b: &Look, t: f32) -> PostLook {
        let mix = |x: f32, y: f32| x + (y - x) * t;
        let mut weights = [0.0; BLOOM_LEVELS];
        for i in 0..BLOOM_LEVELS {
            weights[i] = mix(a.bloom[i], b.bloom[i]);
        }
        if let Some(only) = std::env::var("GEQ_BLOOM_ONLY").ok().and_then(|v| v.parse::<usize>().ok()) {
            for (i, w) in weights.iter_mut().enumerate() {
                *w = if i == only { 1.0 } else { 0.0 };
            }
        }
        PostLook {
            threshold: mix(a.threshold, b.threshold),
            knee: mix(a.knee, b.knee),
            weights,
            master: self.cfg.bloom,
            vignette: mix(a.vignette, b.vignette),
        }
    }
}

/// Estimated GPU milliseconds of a scene at 5120x1440; the stereo waterfall draws two meshes.
fn scene_cost_of(scene: u32, stereo_waterfall: bool) -> f32 {
    scenes::info(scene).cost_ms + if scene == scenes::WATERFALL && stereo_waterfall { 1.6 } else { 0.0 }
}

fn scene_cost(r: &Resolved) -> f32 {
    scene_cost_of(r.scene, r.mode == 1)
}

fn with_gpu<R>(frame: &eframe::Frame, f: impl FnOnce(&mut Gpu, &eframe::wgpu::Device) -> R) -> Option<R> {
    let rs = frame.wgpu_render_state()?;
    let mut r = rs.renderer.write();
    let gpu: &mut Gpu = r.callback_resources.get_mut()?;
    Some(f(gpu, &rs.device))
}

fn effective(cfg: &Settings) -> [f32; bands::COUNT] {
    if cfg.eq_on { cfg.gains } else { [0.0; bands::COUNT] }
}

fn fmt_gain(g: f32) -> String {
    if g.abs() < 0.05 { "0".into() } else { format!("{g:+.1}") }
}

enum Action {
    Scene(u32),
    Look(usize),
    Favorite,
    SaveVariant,
    Revert,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.quit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // ---- time: wall clock normally, a fixed step for deterministic captures
        let wall = clock::now();
        let dt = match self.stepped {
            Some(step) => step,
            None => ((wall - self.last_wall) as f32).clamp(0.0, 0.1),
        };
        self.last_wall = wall;
        self.now = if self.stepped.is_some() { self.now + dt as f64 } else { wall };
        self.frames += 1;
        self.frame_diag(&ctx, frame);

        if let Some((path, after)) = self.shot.clone() {
            if self.now > after && !self.shot_sent {
                self.shot_sent = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            let img = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = img {
                let [w, h] = img.size;
                // GEQ_THUMB_W=n saves a downscaled copy (look thumbnails)
                match std::env::var("GEQ_THUMB_W").ok().and_then(|v| v.parse::<u32>().ok()) {
                    Some(tw) => {
                        if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, img.as_raw().to_vec()) {
                            let th = (tw as f32 * h as f32 / w as f32).round() as u32;
                            let small = image::imageops::resize(&buf, tw, th, image::imageops::FilterType::Lanczos3);
                            let _ = small.save(&path);
                        }
                    }
                    None => {
                        let _ = image::save_buffer(&path, img.as_raw(), w as u32, h as u32, image::ColorType::Rgba8);
                    }
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        crate::analyzer::set_curve_octaves([1.0 / 6.0, 1.0 / 3.0, 2.0 / 3.0][self.cfg.detail as usize % 3]);

        // ---- keyboard: F / F11 toggle full screen, Esc leaves, arrows step scenes, 1-9 pick presets
        let (toggle, esc, left, right, digit, user_input, browse) = ctx.input(|i| {
            let digits = [
                egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5,
                egui::Key::Num6, egui::Key::Num7, egui::Key::Num8, egui::Key::Num9,
            ];
            let plain = !i.modifiers.any();
            (
                i.key_pressed(egui::Key::F11) || (i.key_pressed(egui::Key::F) && plain),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::ArrowLeft) && plain,
                i.key_pressed(egui::Key::ArrowRight) && plain,
                digits.iter().position(|k| i.key_pressed(*k) && plain),
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::PointerMoved(_) | egui::Event::Key { .. } | egui::Event::MouseWheel { .. } | egui::Event::PointerButton { .. }
                    )
                }),
                i.key_pressed(egui::Key::L) && plain,
            )
        });
        if browse {
            self.browser_open = !self.browser_open;
        }
        if user_input {
            self.last_activity = self.now;
            self.next_cycle = self.now + self.cfg.cycle_secs as f64;
        }
        if toggle || (esc && self.fullscreen) || (std::env::var_os("GEQ_FULLSCREEN").is_some() && self.fs_since == 0.0 && !self.fullscreen) {
            self.set_fullscreen(&ctx, if esc { false } else { !self.fullscreen });
        }
        if left {
            self.step_scene(frame, -1);
        }
        if right {
            self.step_scene(frame, 1);
        }
        if let Some(d) = digit {
            if let Some((s, p)) = self.slots().get(d).copied() {
                self.switch(frame, s, Some(p));
            }
        }

        // ---- audio, levels, modulation
        self.audio.poll();
        self.audio.advance(self.now);
        let lv: Levels = self.audio.levels().unwrap_or_default();
        let events = self.audio.take_events();
        self.drums = self.audio.take_drums();
        if self.diag.latency {
            for d in &self.drums {
                eprintln!("drum {:?} p {:.2} strength {:.2} pos {:.2} at {:.3}s, delivered {:.1} ms later", d.kind, d.p, d.strength, d.pos, d.t, (self.now - d.t) * 1000.0);
            }
            for e in &events {
                eprintln!("onset band {} strength {:.2} at {:.3}s, delivered {:.1} ms later", e.band, e.strength, e.t, (self.now - e.t) * 1000.0);
            }
        }
        self.step_levels(&lv, dt);
        let sparks = self.spark_hits.update(&spark_worthy(&self.drums), self.cfg.spark_sensitivity.clamp(0.2, 5.0), self.now, self.sig.gate > 0.5);
        self.peak_hits = column_pushes(&sparks);
        self.sig = self.modn.update(self.now, dt, &lv, &events, &self.levels_f);
        self.idle.update(self.now, dt, self.sig.gate);
        ctx.request_repaint();

        // build scenes ahead of use, one per frame, so switching never stalls on compilation
        if self.frames > 2 {
            if let Some(id) = scenes::ALL.iter().copied().find(|id| !self.built[*id as usize]) {
                self.built[id as usize] = true;
                if let Some(ms) = with_gpu(frame, |g, d| g.prepare_scene(d, id)).flatten() {
                    if self.diag.on {
                        eprintln!("scene {id} prewarmed in {ms:.1} ms");
                    }
                }
            }
        }
        if self.cfg.auto_cycle && self.now >= self.next_cycle && self.trans.is_none() {
            let slots = self.slots();
            let cur = (self.cfg.scene, self.cfg.look[self.cfg.scene as usize] as usize);
            let at = slots.iter().position(|s| *s == cur);
            let (s, p) = slots[(at.map_or(0, |i| i + 1)) % slots.len()];
            self.switch(frame, s, Some(p));
            self.next_cycle = self.now + self.cfg.cycle_secs as f64;
        }

        // debounce settings writes
        if self.dirty_since.map_or(false, |t| t.elapsed().as_millis() > 500) {
            self.cfg.save();
            self.dirty_since = None;
        }

        // ---------------------------------------------------------------- header
        if !self.fullscreen && !self.bare { egui::Panel::top("top").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                let mut on = self.cfg.eq_on;
                if ui.add_enabled(self.audio.ready, egui::Checkbox::new(&mut on, "EQ")).changed() {
                    self.cfg.eq_on = on;
                    self.push_gains();
                    self.touch();
                }
                egui::ComboBox::from_id_salt("preset").selected_text(self.cfg.preset.clone()).show_ui(ui, |ui| {
                    for (name, g) in PRESETS.iter() {
                        if ui.selectable_label(self.cfg.preset == *name, *name).clicked() {
                            self.cfg.preset = (*name).into();
                            self.cfg.gains = *g;
                            self.push_gains();
                            self.touch();
                        }
                    }
                    ui.add_enabled(false, egui::Button::new(CUSTOM));
                });
                if ui.button("Reset").on_hover_text("All bands to 0 dB").clicked() {
                    self.cfg.preset = "Flat".into();
                    self.cfg.gains = [0.0; bands::COUNT];
                    self.push_gains();
                    self.touch();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    self.options_menu(ui, frame);
                    if ui.button("Full screen").on_hover_text("F or F11 (Esc to leave)").clicked() {
                        self.set_fullscreen(&ctx, true);
                    }
                    if !self.audio.status.is_empty() {
                        ui.label(RichText::new(&self.audio.status).color(Color32::from_rgb(224, 164, 88)));
                    }
                });
            });
        }); }

        // --------------------------------------------------------------- sliders
        if !self.fullscreen && !self.bare { self.eq_strip(ui); }

        // ------------------------------------------------------------------ scene
        egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            let s = self.cfg.scene as usize;
            let lim = &scenes::info(self.cfg.scene).cam;
            let mut touched = false;
            if resp.dragged_by(egui::PointerButton::Primary) {
                let d = resp.drag_delta();
                self.cams[s].yaw -= d.x * 0.006;
                self.cams[s].pitch += d.y * 0.006;
                touched = true;
            }
            if resp.dragged_by(egui::PointerButton::Middle) {
                let d = resp.drag_delta();
                self.cams[s].pan[0] += d.x;
                self.cams[s].pan[1] += d.y;
                touched = true;
            }
            if resp.hovered() {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    self.cams[s].zoom = (self.cams[s].zoom * (1.0 - scroll * 0.0015)).clamp(lim.zoom.0, lim.zoom.1);
                    touched = true;
                }
            }
            if resp.double_clicked() {
                self.cams[s] = CamState::default();
                touched = true;
            }
            if lim.yaw.is_finite() {
                self.cams[s].yaw = self.cams[s].yaw.clamp(-lim.yaw, lim.yaw);
            }
            self.cams[s].pitch = self.cams[s].pitch.clamp(lim.pitch.0, lim.pitch.1);
            if touched {
                self.idle.last_input = self.now;
            }
            if resp.hovered() || resp.dragged() {
                ctx.set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            }
            let ppp = ctx.pixels_per_point();
            let size_px = [(rect.width() * ppp) as u32, (rect.height() * ppp) as u32];
            if size_px != self.diag.last_size {
                self.diag.last_size = size_px;
                self.diag.resize_cooldown = 12;
            }

            // history rows at a fixed rate, independent of frame rate. Every row carries
            // left, right and mixed levels so any scene can read any of them.
            self.hist_acc += dt;
            let mut rows = Vec::new();
            while self.hist_acc >= 1.0 / HISTORY_RATE {
                self.hist_acc -= 1.0 / HISTORY_RATE;
                let mut row = vec![0u8; HIST_W as usize * 4];
                for k in 0..HIST_W as usize {
                    row[4 * k] = (self.frac(self.curve_l[k]) * 255.0) as u8;
                    row[4 * k + 1] = (self.frac(self.curve_r[k]) * 255.0) as u8;
                    row[4 * k + 2] = (self.frac(self.curve[k]) * 255.0) as u8;
                    row[4 * k + 3] = (self.rise[k] / RISE_FULL * 255.0).clamp(0.0, 255.0) as u8;
                }
                rows.push(row);
            }

            // one scene, or the outgoing and incoming scene during a crossfade
            if self.trans.as_ref().map_or(false, |t| self.now - t.start >= FADE_S) {
                self.trans = None;
            }
            let to = self.resolved();
            let mut draws = Vec::new();
            let mut mix = 1.0;
            let mut post = self.post_look(to.look, to.look, 0.0);
            if let Some(tr) = &self.trans {
                let p = ((self.now - tr.start) / FADE_S).clamp(0.0, 1.0) as f32;
                mix = p * p * (3.0 - 2.0 * p);
                draws.push(Draw { scene: tr.from.scene, p: self.params_for(&tr.from, &tr.from_cam, &self.sig) });
                post = self.post_look(tr.from.look, to.look, mix);
            }
            draws.push(Draw { scene: to.scene, p: self.params_for(&to, &self.cams[s], &self.sig) });
            let freeze = self.trans.as_ref().map_or(false, |t| t.frozen && self.frames > t.start_frame);
            let cb = SceneCallback { draws, mix, freeze_outgoing: freeze, post, time: self.now as f32, size_px, rows };
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, cb));
        });

        if self.fullscreen {
            self.overlay(&ctx, frame);
        }
        self.looks_window(&ctx, frame);
    }

    fn on_exit(&mut self) {
        self.cfg.save();
        self.audio.shutdown();
    }
}

/// A row of mutually exclusive choices (no nested popups, which would close the parent menu).
fn choice<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (v, label) in options {
            if ui.selectable_label(*value == *v, *label).clicked() && *value != *v {
                *value = *v;
                changed = true;
            }
        }
    });
    changed
}

impl App {
    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        self.fs_since = self.now.max(0.001);
        self.last_activity = self.now;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    /// Frame-time statistics, reported every two seconds with GEQ_FPS=1.
    fn frame_diag(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if !self.diag.on {
            return;
        }
        let d = &mut self.diag;
        let interval = d.last_ui.elapsed().as_secs_f32() * 1000.0;
        d.last_ui = Instant::now();
        if self.frames > 1 {
            let bucket = if d.resize_cooldown > 0 {
                d.resize_cooldown -= 1;
                3
            } else if self.now < 3.0 {
                1
            } else if self.trans.is_some() {
                2
            } else {
                0
            };
            d.buckets[bucket].push(interval);
        }
        if self.now - d.window_start >= 2.0 {
            let size = ctx.content_rect();
            for (name, b) in BUCKETS.iter().zip(d.buckets.iter_mut()) {
                if b.is_empty() {
                    continue;
                }
                let total: f32 = b.iter().sum();
                let n = b.len();
                let fps = n as f32 / (total / 1000.0);
                println!(
                    "{name:>10}: {fps:6.1} fps  ({n} frames, {}x{})  median {:.2}  p95 {:.2}  p99 {:.2}  max {:.2} ms",
                    size.width() as u32,
                    size.height() as u32,
                    percentile(b, 0.5),
                    percentile(b, 0.95),
                    percentile(b, 0.99),
                    b.last().copied().unwrap_or(0.0),
                );
                b.clear();
            }
            if let Some(Some(rep)) = with_gpu(frame, |g, _| g.timer.as_mut().map(|t| t.take_report())) {
                if !rep.is_empty() {
                    let total: f32 = rep.iter().map(|(_, v)| v).sum();
                    let parts: Vec<String> = rep.iter().map(|(n, v)| format!("{n} {v:.2}")).collect();
                    println!("       gpu: {} | sum {total:.2} ms", parts.join("  "));
                }
            }
            d.window_start = self.now;
        }
    }

    // ------------------------------------------------------------------- EQ strip

    fn eq_strip(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("sliders").show_inside(ui, |ui| {
            ui.add_space(8.0);
            let mut changed = false;
            let (_, look) = self.look_key(self.cfg.scene);
            let accent = |i: usize| -> Color32 {
                // each band's accent follows the active theme along its position
                let t = i as f32 / (bands::COUNT - 1) as f32;
                theme_color(self.cfg.theme, t)
            };
            let _ = look;
            ui.columns(bands::COUNT, |cols| {
                for (i, col) in cols.iter_mut().enumerate() {
                    col.vertical_centered(|ui| {
                        let mut g = self.cfg.gains[i];
                        let lit = self.levels_f[i];
                        ui.label(RichText::new(fmt_gain(g)).monospace().size(12.5).color(
                            if g.abs() < 0.05 { Color32::from_gray(120) } else { Color32::from_gray(235) },
                        ));
                        ui.spacing_mut().slider_width = 150.0;
                        let pad = (ui.available_width() - ui.spacing().interact_size.y) / 2.0;
                        let r = ui
                            .horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.add_space(pad.max(0.0));
                                ui.add(
                                    egui::Slider::new(&mut g, bands::GAIN_MIN..=bands::GAIN_MAX)
                                        .vertical()
                                        .step_by(0.5)
                                        .show_value(false),
                                )
                            })
                            .inner;
                        // zero mark and 6 dB ticks beside the rail
                        let rect = r.rect;
                        let half = ui.spacing().interact_size.y * 0.5 * 0.55;
                        let (top, bot) = (rect.top() + half, rect.bottom() - half);
                        let y_at = |db: f32| bot + (top - bot) * ((db - bands::GAIN_MIN) / (bands::GAIN_MAX - bands::GAIN_MIN));
                        let p = ui.painter();
                        for db in [-12.0f32, -6.0, 6.0, 12.0] {
                            let y = y_at(db);
                            p.line_segment([egui::pos2(rect.center().x + 9.0, y), egui::pos2(rect.center().x + 14.0, y)], egui::Stroke::new(1.0, Color32::from_gray(60)));
                        }
                        let y0 = y_at(0.0);
                        p.line_segment(
                            [egui::pos2(rect.center().x - 16.0, y0), egui::pos2(rect.center().x - 10.0, y0)],
                            egui::Stroke::new(1.5, Color32::from_gray(140)),
                        );
                        p.line_segment(
                            [egui::pos2(rect.center().x + 10.0, y0), egui::pos2(rect.center().x + 16.0, y0)],
                            egui::Stroke::new(1.5, Color32::from_gray(140)),
                        );
                        if r.double_clicked() {
                            g = 0.0;
                        }
                        if r.changed() || r.double_clicked() {
                            self.cfg.gains[i] = g;
                            changed = true;
                        }
                        ui.add_space(2.0);
                        ui.label(RichText::new(bands::label(bands::FREQS[i])).strong().size(14.0));
                        // a small accent that brightens with the band's level
                        let (bar, _) = ui.allocate_exact_size(egui::vec2(22.0, 3.0), egui::Sense::hover());
                        let a = accent(i);
                        let k = 0.18 + 0.82 * lit;
                        ui.painter().rect_filled(bar, 1.5, Color32::from_rgb((a.r() as f32 * k) as u8, (a.g() as f32 * k) as u8, (a.b() as f32 * k) as u8));
                    });
                }
            });
            if changed {
                self.cfg.preset = CUSTOM.into();
                self.push_gains();
                self.touch();
            }
            ui.add_space(6.0);
        });
    }

    // ------------------------------------------------- fullscreen controls overlay

    /// A compact control strip that appears on interaction and fades after inactivity.
    fn overlay(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let idle = (self.now - self.last_activity) as f32;
        let alpha = 1.0 - ((idle - OVERLAY_VISIBLE_S) / 0.8).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            ctx.set_cursor_icon(egui::CursorIcon::None);
            return;
        }
        let mut act: Option<Action> = None;
        let mut touched = false;
        let mut exit = false;
        let hovered = egui::Area::new(egui::Id::new("fs_overlay"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -26.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(8, 10, 14, 215))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 140, 170, 60)))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::symmetric(14, 9))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            self.scene_and_look_row(ui, &mut act);
                            if ui.button("Looks").on_hover_text("Browse looks (L)").clicked() {
                                self.browser_open = !self.browser_open;
                            }
                            ui.separator();
                            ui.label(RichText::new("Motion").small().weak());
                            touched |= ui.add(egui::Slider::new(&mut self.cfg.motion, 0.0..=1.5).show_value(false).fixed_decimals(2)).changed();
                            ui.label(RichText::new("Flash").small().weak());
                            touched |= ui.add(egui::Slider::new(&mut self.cfg.flash, 0.0..=1.5).show_value(false).fixed_decimals(2)).changed();
                            ui.separator();
                            touched |= ui.checkbox(&mut self.cfg.auto_cycle, "Auto").on_hover_text("Cycle through favorites (or every preset)").changed();
                            if ui.button("Exit").on_hover_text("Esc or F").clicked() {
                                exit = true;
                            }
                        });
                    });
            })
            .response
            .hovered();
        if hovered || touched {
            self.last_activity = self.now;
            self.next_cycle = self.now + self.cfg.cycle_secs as f64;
        }
        if touched {
            self.touch();
        }
        if exit {
            self.set_fullscreen(ctx, false);
        }
        self.run_action(frame, act);
        // key hints for the first seconds
        let age = (self.now - self.fs_since) as f32;
        if age < 3.5 {
            let a = (1.0 - ((age - 2.5) / 1.0).clamp(0.0, 1.0)) * 0.7 * alpha;
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("fs_hint"))).text(
                egui::pos2(18.0, 14.0),
                egui::Align2::LEFT_TOP,
                "Esc or F: leave   ·   Left / Right: scene   ·   1-9: preset   ·   drag: rotate   scroll: zoom",
                egui::FontId::proportional(14.0),
                Color32::from_white_alpha((a * 255.0) as u8),
            );
        }
    }

    /// Scene buttons and the active scene's look chips; shared by the overlay and the menu.
    fn scene_and_look_row(&self, ui: &mut egui::Ui, act: &mut Option<Action>) {
        for id in scenes::ALL {
            if ui.selectable_label(self.cfg.scene == id, scenes::info(id).name).clicked() {
                *act = Some(Action::Scene(id));
            }
        }
        ui.separator();
        let (cur, _) = self.look_key(self.cfg.scene);
        for (i, l) in presets::for_scene(self.cfg.scene) {
            let fav = self.cfg.favorites.contains(&presets::key(l));
            let label = if fav { format!("★ {}", l.name) } else { l.name.to_string() };
            if ui.selectable_label(cur == i, label).clicked() {
                *act = Some(Action::Look(i));
            }
        }
    }

    fn run_action(&mut self, frame: &eframe::Frame, act: Option<Action>) {
        let Some(act) = act else { return };
        let (cur, look) = self.look_key(self.cfg.scene);
        match act {
            Action::Scene(id) => self.switch(frame, id, None),
            Action::Look(i) => self.switch(frame, self.cfg.scene, Some(i)),
            Action::Favorite => {
                let k = presets::key(look);
                if let Some(p) = self.cfg.favorites.iter().position(|f| *f == k) {
                    self.cfg.favorites.remove(p);
                } else {
                    self.cfg.favorites.push(k);
                }
                self.touch();
            }
            Action::SaveVariant => {
                self.cfg.variants.insert(
                    presets::key(look),
                    Variant { theme: self.cfg.theme, motion: self.cfg.motion, flash: self.cfg.flash },
                );
                self.touch();
            }
            Action::Revert => {
                self.cfg.variants.remove(&presets::key(look));
                self.apply_look();
                self.touch();
            }
        }
        let _ = cur;
    }

    // ------------------------------------------------------------------ look browser

    /// A window of stored thumbnails, one row per scene. Clicking a card applies that look; the
    /// star toggles it as a favorite.
    fn looks_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if !self.browser_open {
            return;
        }
        self.last_activity = self.now; // keep the fullscreen controls up while browsing
        let mut open = true;
        let mut pick: Option<(u32, usize)> = None;
        let mut fav: Option<String> = None;
        let card = egui::vec2(224.0, 63.0);
        egui::Window::new("Looks")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
            .show(ctx, |ui| {
                for scene in scenes::ALL {
                    ui.label(RichText::new(scenes::info(scene).name.to_uppercase()).small().weak());
                    ui.horizontal(|ui| {
                        for (i, l) in presets::for_scene(scene) {
                            let cat = presets::catalog_index(l);
                            let (rect, resp) = ui.allocate_exact_size(card + egui::vec2(0.0, 22.0), egui::Sense::click());
                            let img = egui::Rect::from_min_size(rect.min, card);
                            let p = ui.painter();
                            p.rect_filled(img, 4.0, Color32::from_gray(10));
                            if let Some(t) = self.thumbs.get(ctx, cat) {
                                p.image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            let current = self.cfg.scene == scene && self.cfg.look[scene as usize] as usize == i;
                            let stroke = if current {
                                egui::Stroke::new(2.0, Color32::from_rgb(70, 170, 230))
                            } else if resp.hovered() {
                                egui::Stroke::new(1.5, Color32::from_gray(150))
                            } else {
                                egui::Stroke::new(1.0, Color32::from_gray(50))
                            };
                            p.rect_stroke(img, 4.0, stroke, egui::StrokeKind::Inside);
                            p.text(
                                rect.left_bottom() + egui::vec2(2.0, -2.0),
                                egui::Align2::LEFT_BOTTOM,
                                l.name,
                                egui::FontId::proportional(13.0),
                                if current { Color32::WHITE } else { Color32::from_gray(190) },
                            );
                            let key = presets::key(l);
                            let is_fav = self.cfg.favorites.contains(&key);
                            let star = egui::Rect::from_min_size(img.right_top() + egui::vec2(-26.0, 4.0), egui::vec2(22.0, 22.0));
                            let sr = ui.interact(star, ui.id().with(("fav", cat)), egui::Sense::click());
                            ui.painter().text(
                                star.center(),
                                egui::Align2::CENTER_CENTER,
                                if is_fav { "★" } else { "☆" },
                                egui::FontId::proportional(18.0),
                                if is_fav { Color32::from_rgb(255, 210, 80) } else { Color32::from_white_alpha(if sr.hovered() { 230 } else { 120 }) },
                            );
                            if sr.clicked() {
                                fav = Some(key);
                            } else if resp.clicked() {
                                pick = Some((scene, i));
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
                ui.label(RichText::new("Click to apply  ·  star = favorite (number keys and Auto cycle use favorites)").small().weak());
            });
        if !open {
            self.browser_open = false;
        }
        if let Some(k) = fav {
            if let Some(p) = self.cfg.favorites.iter().position(|f| *f == k) {
                self.cfg.favorites.remove(p);
            } else {
                self.cfg.favorites.push(k);
            }
            self.touch();
        }
        if let Some((s, i)) = pick {
            self.switch(frame, s, Some(i));
        }
    }

    // ----------------------------------------------------------------- Display menu

    fn options_menu(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        ui.menu_button("Display", |ui| {
            ui.set_min_width(340.0);
            let mut touched = false;
            let mut act: Option<Action> = None;
            let info = scenes::info(self.cfg.scene);

            ui.label(RichText::new("SCENE").small().weak());
            ui.horizontal_wrapped(|ui| {
                for id in scenes::ALL {
                    if ui.selectable_label(self.cfg.scene == id, scenes::info(id).name).clicked() {
                        act = Some(Action::Scene(id));
                    }
                }
            });
            if ui.button("Browse looks…").on_hover_text("Thumbnails of every look (L)").clicked() {
                self.browser_open = true;
                ui.close();
            }
            ui.add_space(4.0);

            ui.label(RichText::new("LOOK").small().weak());
            let (cur, look) = self.look_key(self.cfg.scene);
            ui.horizontal_wrapped(|ui| {
                for (i, l) in presets::for_scene(self.cfg.scene) {
                    let fav = self.cfg.favorites.contains(&presets::key(l));
                    let label = if fav { format!("★ {}", l.name) } else { l.name.to_string() };
                    if ui.selectable_label(cur == i, label).clicked() {
                        act = Some(Action::Look(i));
                    }
                }
            });
            let fav = self.cfg.favorites.contains(&presets::key(look));
            let modified = self.look_modified();
            let has_variant = self.cfg.variants.contains_key(&presets::key(look));
            ui.horizontal(|ui| {
                if ui.button(if fav { "★ Favorite" } else { "☆ Favorite" }).on_hover_text("Favorites are what the number keys and Auto cycle through").clicked() {
                    act = Some(Action::Favorite);
                }
                if ui.add_enabled(modified, egui::Button::new("Save variant")).on_hover_text("Keep the current colors, Motion and Flash for this look").clicked() {
                    act = Some(Action::SaveVariant);
                }
                if ui.add_enabled(modified || has_variant, egui::Button::new("Revert")).on_hover_text("Back to the preset's own settings").clicked() {
                    act = Some(Action::Revert);
                }
            });
            ui.add_space(4.0);

            for c in info.controls {
                match c {
                    Control::WaterfallChannels => {
                        ui.label(RichText::new("CHANNELS").small().weak());
                        touched |= choice(ui, &mut self.cfg.stereo, &[(false, "Mixed"), (true, "Stereo overlay (L cyan / R magenta)")]);
                    }
                    Control::WaterfallDetail => {
                        ui.label(RichText::new("DETAIL").small().weak());
                        touched |= choice(ui, &mut self.cfg.detail, &[(0, "Fine"), (1, "Medium"), (2, "Smooth")]);
                    }
                    Control::PeakCaps => {
                        ui.checkbox(&mut self.cfg.peak_caps, "Peak hold").changed().then(|| touched = true);
                    }
                    Control::Sparks => {
                        ui.checkbox(&mut self.cfg.sparks, "Sparks").on_hover_text("Embers off the lit columns, and a burst of sparks from the column at its place on the frequency axis when a drum hits (kicks from the bass columns, hats from the treble ones; the harder the hit, the bigger the burst)").changed().then(|| touched = true);
                    }
                    Control::Gradient => {
                        ui.label(RichText::new(if self.cfg.scene == scenes::RIPPLE { "RIPPLES" } else { "SPARKS" }).small().weak());
                        touched |= ui
                            .add(egui::Slider::new(&mut self.cfg.spark_sensitivity, 0.4..=3.0).logarithmic(true).text("sensitivity"))
                            .on_hover_text("The fountain: how steeply the volume at a frequency must rise to throw sparks. Column sparks, fireworks, storm and pond: how sure the drum detector must be that it heard a drum (a kick, snare, tom, hat or cymbal) before it sets off a burst, a strike or a splash. Higher accepts less certain hits, lower only the clearest. The pond starts more sensitive for frequent ripples.")
                            .changed();
                    }
                    Control::Density => {
                        ui.label(RichText::new("PARTICLES").small().weak());
                        touched |= ui
                            .add(egui::Slider::new(&mut self.cfg.density, 0.25..=1.5).text("density"))
                            .on_hover_text("How many particles the fountain launches, stars in each firework, branches and sparks in each lightning strike, or droplets in each splash of the pond")
                            .changed();
                    }
                    Control::OrbSpikes => {
                        ui.label(RichText::new("FLUID").small().weak());
                        touched |= ui
                            .add(egui::Slider::new(&mut self.cfg.orb_spikes, 0.3..=1.5).text("spikes"))
                            .on_hover_text("How tall the spikes stand. Lower gives a smooth, lobed blob; higher a hedgehog.")
                            .changed();
                    }
                    Control::AuroraCurtains => {
                        ui.label(RichText::new("AURORA").small().weak());
                        touched |= ui
                            .add(egui::Slider::new(&mut self.cfg.aurora_curtains, 2..=6).text("curtains"))
                            .on_hover_text("The most curtains standing in the sky. A narrower picture shows fewer of them anyway; fewer curtains are quieter and cheaper.")
                            .changed();
                    }
                    Control::RippleLife => {
                        ui.label(RichText::new("POND").small().weak());
                        touched |= ui
                            .add(egui::Slider::new(&mut self.cfg.ripple_life, 0.4..=2.5).logarithmic(true).text("ripple life"))
                            .on_hover_text("How long a ripple lives. Short gives a quick, clean pond; long a surface crowded with the last several seconds of hits.")
                            .changed();
                    }
                }
                ui.add_space(4.0);
            }

            ui.label(RichText::new("COLORS").small().weak());
            let themes: Vec<(u32, &str)> = THEMES.iter().enumerate().map(|(i, n)| (i as u32, *n)).collect();
            touched |= choice(ui, &mut self.cfg.theme, &themes);
            ui.add_space(4.0);
            egui::Grid::new("look_opts").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label("Motion");
                let motion_tip = if self.cfg.scene == scenes::AURORA {
                    "How much the curtains fold and drift, how the lake stirs, and how far the camera wanders. 0 holds the folds still (the columns still follow the music)."
                } else if self.cfg.scene == scenes::FIREWORKS {
                    "How far the stars drift on the wind and how far the camera wanders. 0 holds the sky still."
                } else if self.cfg.scene == scenes::LIGHTNING {
                    "How far the camera wanders. 0 holds the view still."
                } else if self.cfg.scene == scenes::RIPPLE {
                    "How far the light towers drift and the camera wanders. 0 holds the reflections still (the water still follows the music)."
                } else if self.cfg.scene == scenes::FERROFLUID {
                    "How elastic the fluid is: spikes thrown out by a hit, wobbling lobes, breathing, and how fast the lights drift around it. 0 gives a fluid that only follows the loudness, in still light."
                } else {
                    "Camera drift and movement. 0 holds everything still."
                };
                touched |= ui.add(egui::Slider::new(&mut self.cfg.motion, 0.0..=1.5)).on_hover_text(motion_tip).changed();
                ui.end_row();
                ui.label("Flash");
                touched |= ui.add(egui::Slider::new(&mut self.cfg.flash, 0.0..=1.5)).on_hover_text("Brightness of onset rings, sparks and pulses (the lights and tip glow of the ferrofluid orb, the highlights that run up the aurora's rays, the flash of a firework and the light it throws on the floor, the strokes of a lightning bolt with the light and thunder ring it throws on the floor, the flares of the pond's light towers and the glow of its droplets; 0 leaves only a dim channel). 0 turns them off.").changed();
                ui.end_row();
                ui.label("Glow");
                touched |= ui.add(egui::Slider::new(&mut self.cfg.bloom, 0.0..=2.0)).changed();
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.label(RichText::new("TRANSITIONS").small().weak());
            touched |= choice(ui, &mut self.cfg.crossfade, &[(true, "Crossfade"), (false, "Cut")]);
            ui.horizontal(|ui| {
                touched |= ui.checkbox(&mut self.cfg.auto_cycle, "Auto-cycle every").changed();
                touched |= ui.add(egui::DragValue::new(&mut self.cfg.cycle_secs).range(10.0..=600.0).suffix(" s")).changed();
            });
            ui.separator();

            ui.label(RichText::new("RANGE").small().weak());
            touched |= choice(ui, &mut self.cfg.range, &[(40.0, "40 dB"), (60.0, "60 dB"), (80.0, "80 dB")]);
            ui.add_space(4.0);
            ui.label(RichText::new("FALL SPEED").small().weak());
            touched |= choice(ui, &mut self.cfg.fall, &[(0, "Slow"), (1, "Medium"), (2, "Fast")]);
            ui.add_space(4.0);
            egui::Grid::new("opts").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label("Sensitivity (dB)");
                touched |= ui.add(egui::Slider::new(&mut self.cfg.sensitivity, -20.0..=20.0)).changed();
                ui.end_row();
            });
            ui.separator();
            if !self.audio.is_synth() {
                ui.label(RichText::new("ANALYZE").small().weak());
                let mut pre = self.cfg.source_pre;
                choice(ui, &mut pre, &[(false, "After EQ (what you hear)"), (true, "Before EQ")]);
                if pre != self.cfg.source_pre {
                    self.cfg.source_pre = pre;
                    self.audio.set_source_pre(pre);
                    touched = true;
                }
                ui.add_space(4.0);
                ui.label(RichText::new("OUTPUT DEVICE").small().weak());
                if self.sinks.is_empty() {
                    self.sinks = routing::list_sinks();
                }
                let current = self.audio.real_sink.clone().unwrap_or_default();
                let mut pick: Option<String> = None;
                for (name, desc) in &self.sinks {
                    if ui.selectable_label(*name == current, desc).clicked() && *name != current {
                        pick = Some(name.clone());
                    }
                }
                if let Some(n) = pick {
                    self.cfg.output = Some(n.clone());
                    self.audio.set_output(n);
                    touched = true;
                }
            }
            if touched {
                self.touch();
            }
            self.run_action(frame, act);
        })
        .response
        .on_hover_text("Display and audio options");
    }
}

/// A theme's colour along its dominant -> supporting -> accent ramp, for UI accents (0..1).
fn theme_color(theme: u32, t: f32) -> Color32 {
    let ramp = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| -> [f32; 3] {
        let (lo, hi) = (((t / 0.6).clamp(0.0, 1.0)), ((t - 0.6) / 0.4).clamp(0.0, 1.0));
        let m = |x: [f32; 3], y: [f32; 3], k: f32| [x[0] + (y[0] - x[0]) * k, x[1] + (y[1] - x[1]) * k, x[2] + (y[2] - x[2]) * k];
        if t < 0.6 { m(a, b, lo) } else { m(b, c, hi) }
    };
    let c = match theme {
        0 => ramp([0.10, 1.0, 0.35], [1.0, 0.9, 0.2], [1.0, 0.15, 0.2]),
        1 => ramp([1.0, 0.65, 0.0], [1.0, 0.4, 0.0], [1.0, 0.12, 0.0]),
        2 => ramp([0.0, 0.9, 1.0], [0.3, 0.5, 1.0], [0.75, 0.3, 1.0]),
        3 => ramp([1.0, 0.15, 0.85], [0.5, 0.35, 1.0], [0.0, 0.9, 1.0]),
        _ => {
            let h = (0.85 - 0.8 * t).rem_euclid(1.0);
            let hsv = egui::ecolor::Hsva::new(h, 0.85, 1.0, 1.0);
            let c = Color32::from(hsv);
            return c;
        }
    };
    Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
}
