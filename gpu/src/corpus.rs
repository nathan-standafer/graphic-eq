//! Test-only: a labelled drum corpus rendered from the system's General MIDI soundfont.
//!
//! The synthetic fixtures in `synth.rs` are computed noise bursts and sine sweeps. To judge a drum detector
//! on something closer to recorded music, this module reads the GM soundfont that ships with the distribution
//! (`/usr/share/sounds/sf2/TimGM6mb.sf2`, real recorded drum and instrument samples), plays patterns through
//! it with a small sequencer, mixes the drums under a bass, chords and a lead, and runs the result through a
//! mastering chain (reverb, bus compression, soft clipping, a noise floor). The drum hits are known exactly,
//! so a detector can be scored: every label is the time a drum sample starts.
//!
//! Songs are split into a *dev* set, which detectors are tuned on, and a *test* set with other kits, tempos,
//! instruments and seeds, which is only looked at to report a result.

use std::collections::HashMap;

pub const SR: f64 = 48_000.0;
pub const SF2_PATH: &str = "/usr/share/sounds/sf2/TimGM6mb.sf2";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Kick,
    Snare,
    Hat,
    Tom,
    Cymbal,
}

impl Kind {
    pub fn of_key(key: u8) -> Kind {
        match key {
            35 | 36 => Kind::Kick,
            37..=40 => Kind::Snare,
            42 | 44 | 46 => Kind::Hat,
            41 | 43 | 45 | 47 | 48 | 50 => Kind::Tom,
            _ => Kind::Cymbal,
        }
    }
}

/// One drum hit: when its sample starts, what it is, how hard (MIDI velocity).
#[derive(Clone, Copy, Debug)]
pub struct Label {
    pub t: f64,
    pub kind: Kind,
    pub vel: u8,
}

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// 0 up to 1.
    pub fn f(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn gauss(&mut self) -> f64 {
        let (a, b) = (self.f().max(1e-12), self.f());
        (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
    }
    pub fn pick<T: Copy>(&mut self, s: &[T]) -> T {
        s[(self.f() * s.len() as f64) as usize % s.len()]
    }
}

// ------------------------------------------------------------------------------------------ soundfont

#[derive(Clone, Copy)]
struct Gen {
    op: u16,
    lo: u8,
    hi: u8,
}

impl Gen {
    fn val(&self) -> i32 {
        i16::from_le_bytes([self.lo, self.hi]) as i32
    }
}

struct PresetHdr {
    preset: u16,
    bank: u16,
    bag: usize,
}

struct SampleHdr {
    start: usize,
    end: usize,
    sloop: usize,
    eloop: usize,
    rate: f64,
    pitch: u8,
    corr: i32,
}

pub struct Sf2 {
    smpl: Vec<i16>,
    presets: Vec<PresetHdr>,
    pbag: Vec<usize>,
    pgen: Vec<Gen>,
    inst: Vec<usize>,
    ibag: Vec<usize>,
    igen: Vec<Gen>,
    shdr: Vec<SampleHdr>,
}

struct Zone {
    sample: usize,
    gens: HashMap<u16, i32>,
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn riff(b: &[u8], mut off: usize, end: usize) -> Vec<([u8; 4], usize, usize)> {
    let mut v = Vec::new();
    while off + 8 <= end {
        let id = [b[off], b[off + 1], b[off + 2], b[off + 3]];
        let sz = u32le(b, off + 4) as usize;
        v.push((id, off + 8, sz));
        off += 8 + sz + (sz & 1);
    }
    v
}

fn gens(b: &[u8]) -> Vec<Gen> {
    (0..b.len() / 4).map(|i| Gen { op: u16le(b, i * 4), lo: b[i * 4 + 2], hi: b[i * 4 + 3] }).collect()
}

impl Sf2 {
    pub fn load() -> Option<Sf2> {
        let b = std::fs::read(SF2_PATH).ok()?;
        let (mut smpl, mut pd): (Vec<i16>, HashMap<[u8; 4], &[u8]>) = (Vec::new(), HashMap::new());
        for (id, o, sz) in riff(&b, 12, b.len()) {
            if &id != b"LIST" {
                continue;
            }
            for (id2, o2, s2) in riff(&b, o + 4, o + sz) {
                match &b[o..o + 4] {
                    b"sdta" if &id2 == b"smpl" => smpl = (0..s2 / 2).map(|i| i16::from_le_bytes([b[o2 + i * 2], b[o2 + i * 2 + 1]])).collect(),
                    b"pdta" => {
                        pd.insert(id2, &b[o2..o2 + s2]);
                    }
                    _ => {}
                }
            }
        }
        let ph = pd.get(b"phdr")?;
        let presets = (0..ph.len() / 38).map(|i| PresetHdr { preset: u16le(ph, i * 38 + 20), bank: u16le(ph, i * 38 + 22), bag: u16le(ph, i * 38 + 24) as usize }).collect();
        let idx = |name: &[u8; 4], size: usize, at: usize| -> Vec<usize> { let d = pd[name]; (0..d.len() / size).map(|i| u16le(d, i * size + at) as usize).collect() };
        let sh = pd.get(b"shdr")?;
        let shdr = (0..sh.len() / 46)
            .map(|i| {
                let o = i * 46;
                SampleHdr {
                    start: u32le(sh, o + 20) as usize,
                    end: u32le(sh, o + 24) as usize,
                    sloop: u32le(sh, o + 28) as usize,
                    eloop: u32le(sh, o + 32) as usize,
                    rate: u32le(sh, o + 36) as f64,
                    pitch: sh[o + 40],
                    corr: sh[o + 41] as i8 as i32,
                }
            })
            .collect();
        Some(Sf2 { smpl, presets, pbag: idx(b"pbag", 4, 0), pgen: gens(pd[b"pgen"]), inst: idx(b"inst", 22, 20), ibag: idx(b"ibag", 4, 0), igen: gens(pd[b"igen"]), shdr })
    }

    fn in_range(gs: &[Gen], op: u16, v: u8) -> bool {
        gs.iter().filter(|g| g.op == op).all(|g| v >= g.lo && v <= g.hi)
    }

    fn zones(&self, bank: u16, preset: u16, key: u8, vel: u8) -> Vec<Zone> {
        let Some(pi) = self.presets.iter().position(|p| p.bank == bank && p.preset == preset) else { return Vec::new() };
        let mut out = Vec::new();
        let mut global_p: Vec<Gen> = Vec::new();
        for bag in self.presets[pi].bag..self.presets[pi + 1].bag {
            let pg = &self.pgen[self.pbag[bag]..self.pbag[bag + 1]];
            let Some(inst) = pg.iter().find(|g| g.op == 41).map(|g| g.val() as usize) else {
                global_p = pg.to_vec();
                continue;
            };
            let pzone: Vec<Gen> = global_p.iter().chain(pg.iter()).copied().collect();
            if !Self::in_range(&pzone, 43, key) || !Self::in_range(&pzone, 44, vel) {
                continue;
            }
            let mut global_i: Vec<Gen> = Vec::new();
            for ib in self.inst[inst]..self.inst[inst + 1] {
                let ig = &self.igen[self.ibag[ib]..self.ibag[ib + 1]];
                let Some(sample) = ig.iter().find(|g| g.op == 53).map(|g| u16::from_le_bytes([g.lo, g.hi]) as usize) else {
                    global_i = ig.to_vec();
                    continue;
                };
                let izone: Vec<Gen> = global_i.iter().chain(ig.iter()).copied().collect();
                if !Self::in_range(&izone, 43, key) || !Self::in_range(&izone, 44, vel) {
                    continue;
                }
                let mut gens = HashMap::new();
                for g in izone.iter().filter(|g| g.op != 43 && g.op != 44 && g.op != 53) {
                    gens.insert(g.op, g.val());
                }
                out.push(Zone { sample, gens });
            }
        }
        out
    }

    /// Render one note into `out` (stereo, 48 kHz) starting at `at` seconds. `dur` is how long the key is held
    /// (infinite for drums); the release tail is added after it.
    pub fn note(&self, out: &mut Vec<[f32; 2]>, bank: u16, preset: u16, key: u8, vel: u8, at: f64, dur: f64, gain: f32) {
        for z in self.zones(bank, preset, key, vel) {
            let g = |op: u16, d: i32| z.gens.get(&op).copied().unwrap_or(d);
            let sh = &self.shdr[z.sample];
            let start = (sh.start as i64 + g(0, 0) as i64 + 32768 * g(4, 0) as i64).max(0) as usize;
            let end = ((sh.end as i64 + g(1, 0) as i64 + 32768 * g(12, 0) as i64).max(0) as usize).min(self.smpl.len());
            let (ls, le) = (
                (sh.sloop as i64 + g(2, 0) as i64 + 32768 * g(45, 0) as i64).max(0) as usize,
                (sh.eloop as i64 + g(3, 0) as i64 + 32768 * g(50, 0) as i64).max(0) as usize,
            );
            let looped = matches!(g(54, 0), 1 | 3) && le > ls + 8;
            let root = if g(58, -1) >= 0 { g(58, -1) } else if sh.pitch == 255 { 60 } else { sh.pitch as i32 };
            let cents = (key as i32 - root) as f64 * g(56, 100) as f64 + g(51, 0) as f64 * 100.0 + g(52, 0) as f64 + sh.corr as f64;
            let ratio = sh.rate / SR * (cents / 1200.0).exp2();
            let pan = (g(17, 0) as f64 / 1000.0).clamp(-0.5, 0.5) + 0.5;
            let (gl, gr) = ((pan * std::f64::consts::FRAC_PI_2).cos() as f32, (pan * std::f64::consts::FRAC_PI_2).sin() as f32);
            let atten = 10f64.powf(-(g(48, 0).clamp(0, 1440) as f64) / 200.0) as f32;
            let vgain = (vel as f32 / 127.0).powf(1.6) * atten * gain / 32768.0;
            let tc = |op: u16| (g(op, -12000) as f64 / 1200.0).exp2();
            let (delay, attack, hold, decay, release) = (tc(33), tc(34), tc(35), tc(36), tc(38).min(3.0));
            let sustain_db = g(37, 0).clamp(0, 1440) as f64 / 10.0;
            let fc = g(8, 13500);
            let lp = if fc < 13000 { Some((1.0 - (-std::f64::consts::TAU * 8.176 * (fc as f64 / 1200.0).exp2() / SR).exp()) as f32) } else { None };
            let held = |t: f64| -> f64 {
                if t < delay {
                    return 0.0;
                }
                let t = t - delay;
                if t < attack {
                    return t / attack.max(1e-4);
                }
                let t = t - attack;
                if t < hold {
                    return 1.0;
                }
                let db = (100.0 / decay.max(1e-3) * (t - hold)).min(sustain_db);
                10f64.powf(-db / 20.0)
            };
            // a drum is never released by a note-off, but a looped sample would play for ever: hold it three seconds
            let dur = dur.min(3.0 + 6.0 * (dur < 1e8) as u8 as f64);
            let at_release = held(dur);
            let first = (at * SR) as usize;
            let (mut pos, mut y, mut n) = (start as f64, 0.0f32, 0usize);
            loop {
                let t = n as f64 / SR;
                let env = if t < dur { held(t) } else { at_release * 10f64.powf(-(100.0 / release.max(1e-3) * (t - dur)) / 20.0) };
                if (t > dur && env < 1e-4) || t > 12.0 {
                    break;
                }
                let i = pos as usize;
                if looped && pos >= le as f64 {
                    pos -= (le - ls) as f64;
                    continue;
                }
                if i + 1 >= end {
                    break;
                }
                let fr = (pos - i as f64) as f32;
                let mut s = self.smpl[i] as f32 * (1.0 - fr) + self.smpl[i + 1] as f32 * fr;
                if let Some(a) = lp {
                    y += a * (s - y);
                    s = y;
                }
                let v = s * vgain * env as f32;
                if out.len() <= first + n {
                    out.resize(first + n + 4096, [0.0; 2]);
                }
                out[first + n][0] += v * gl;
                out[first + n][1] += v * gr;
                pos += ratio;
                n += 1;
            }
        }
    }
}

// ------------------------------------------------------------------------------------------ songs

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Set {
    Dev,
    Test,
}

/// A drum lane: a General MIDI drum key and a 16-step bar. '.' is silent; g l m x X are velocities 30 55 80 100 122.
type Lane = (u8, &'static str);

#[derive(Clone, Copy)]
enum Bass {
    Root8,
    Walk,
    Sub,
    Funk16,
    Drive16,
    None,
}
#[derive(Clone, Copy)]
enum Chords {
    Pad,
    Stab,
    Arp,
    Strum,
    PowerStrum,
    None,
}

#[derive(Clone)]
pub struct Spec {
    pub name: &'static str,
    pub set: Set,
    bpm: f64,
    secs: f64,
    kit: u16,
    grooves: &'static [&'static [Lane]],
    /// 0 straight .. 0.33 hard shuffle: delays the odd sixteenths.
    swing: f64,
    rolls: bool,
    fills: bool,
    bass: (Bass, u16),
    chords: (Chords, u16),
    lead: Option<u16>,
    /// The backing's loudness against the drums' in dB (RMS over the file).
    band_db: f32,
    reverb: f32,
    comp: f32,
    /// Speech phrases laid over the mix, this many dB above (or below) it: the consonants of a voice are
    /// broadband bursts that a drum detector must not mistake for hats and snares.
    vocal_db: Option<f32>,
    /// A broadband noise bed this far below full scale (rms dB), in four bands whose levels flicker from block to
    /// block by about `flicker_db`, like a lossy codec's: real recordings have a noise floor and HF texture that
    /// sits above a detector's compression knee, where a clean synthetic mix has none.
    noise_db: Option<f32>,
    flicker_db: f32,
    /// Waveshaping applied to the bass and chords (dB of drive): distorted guitars are noise-like up high.
    drive_db: f32,
    /// A few stretches of exact digital silence (a song that stops and starts: breaks, drops, track starts), in
    /// which the labels are dropped. A hit that follows arrives in one analysis frame instead of two.
    gaps: bool,
    seed: u64,
}

const ROCK_A: &[Lane] = &[(36, "X.....x.x......."), (38, "....X.......X..."), (42, "x.l.x.l.x.l.x.l.")];
const ROCK_B: &[Lane] = &[(36, "X.......x.x.x..."), (38, "....X.......X..g"), (42, "x.l.x.l.x.l.x.l.")];
const HOUSE: &[Lane] = &[(36, "X...X...X...X..."), (39, "....x.......x..."), (46, "..x...x...x...x."), (42, "lglglglglglglglg")];
const FUNK_A: &[Lane] = &[(36, "X..x..x...x..x.."), (38, "....X..g.g..X..g"), (42, "xlxlxlxlxlxlxlxl"), (46, "..............x.")];
const FUNK_B: &[Lane] = &[(36, "X.....x..x.x...."), (38, "g...X..g....X.g."), (42, "xlxlxlxlxlxlxlxl")];
const TRAP: &[Lane] = &[(36, "X..x......x....."), (38, "........X......."), (42, "x.x.xxxxx.x.x.xx")];
const JAZZ: &[Lane] = &[(51, "x...x.l.x...x.l."), (44, "....m.......m..."), (36, "l.......l......."), (38, "..g...m.....g...")];
const METAL: &[Lane] = &[(36, "XxXxXxXxXxXxXxXx"), (38, "....X.......X..."), (49, "X..............."), (42, "x.x.x.x.x.x.x.x.")];
const BALLAD: &[Lane] = &[(36, "X.......l......."), (37, "........x......."), (42, "l...l...l...l...")];
const POP: &[Lane] = &[(36, "X...X...X...X..."), (38, "....X.......X..."), (42, "x.x.x.x.x.x.x.x."), (46, "...............l")];
const BREAK: &[Lane] = &[(36, "X..x..x...x.x..."), (38, "....X..g.g..X.g."), (51, "x.x.x.x.x.x.x.x."), (42, "..l...l...l...l.")];
const SOLO: &[Lane] = &[(36, "X..x..x.X..x..x."), (38, "....X..g....X..m"), (42, "x.x.xxx.x.x.xxx.")];

pub fn specs() -> Vec<Spec> {
    use Bass as B;
    use Chords as C;
    let base = Spec {
        name: "",
        set: Set::Dev,
        bpm: 120.0,
        secs: 48.0,
        kit: 0,
        grooves: &[ROCK_A],
        swing: 0.0,
        rolls: false,
        fills: true,
        bass: (B::Root8, 33),
        chords: (C::Pad, 48),
        lead: None,
        band_db: 0.0,
        reverb: 0.12,
        comp: 0.5,
        vocal_db: None,
        noise_db: None,
        flicker_db: 0.0,
        drive_db: 0.0,
        gaps: false,
        seed: 1,
    };
    vec![
        Spec { name: "rock_130", bpm: 130.0, kit: 0, grooves: &[ROCK_A, ROCK_B], bass: (B::Root8, 33), chords: (C::Strum, 29), lead: Some(73), band_db: 0.0, seed: 11, ..base.clone() },
        Spec { name: "house_124", bpm: 124.0, kit: 24, grooves: &[HOUSE], fills: false, bass: (B::Funk16, 38), chords: (C::Stab, 81), lead: None, band_db: 3.0, reverb: 0.15, seed: 12, ..base.clone() },
        Spec { name: "funk_98", bpm: 98.0, kit: 8, grooves: &[FUNK_A, FUNK_B], swing: 0.12, bass: (B::Funk16, 34), chords: (C::Stab, 4), lead: Some(56), band_db: 0.0, seed: 13, ..base.clone() },
        Spec { name: "trap_140", bpm: 140.0, kit: 25, grooves: &[TRAP], rolls: true, fills: false, bass: (B::Sub, 39), chords: (C::Pad, 52), lead: Some(80), band_db: 3.0, reverb: 0.1, comp: 0.7, seed: 14, ..base.clone() },
        Spec { name: "jazz_118", bpm: 118.0, kit: 32, grooves: &[JAZZ], swing: 0.22, fills: false, bass: (B::Walk, 32), chords: (C::Stab, 0), lead: Some(65), band_db: 0.0, reverb: 0.2, comp: 0.1, seed: 15, ..base.clone() },
        Spec { name: "pluck_132", bpm: 132.0, grooves: &[], bass: (B::Root8, 33), chords: (C::Arp, 0), lead: Some(12), set: Set::Dev, seed: 16, ..base.clone() },
        Spec { name: "metal_168", set: Set::Test, bpm: 168.0, kit: 16, grooves: &[METAL], bass: (B::Drive16, 33), chords: (C::PowerStrum, 30), lead: Some(81), band_db: 6.0, comp: 0.7, seed: 21, ..base.clone() },
        Spec { name: "pop_104", set: Set::Test, bpm: 104.0, kit: 8, grooves: &[POP], bass: (B::Funk16, 38), chords: (C::Pad, 48), lead: Some(73), band_db: 3.0, reverb: 0.18, seed: 22, ..base.clone() },
        Spec { name: "ballad_72", set: Set::Test, bpm: 72.0, kit: 40, grooves: &[BALLAD], fills: false, bass: (B::Root8, 32), chords: (C::Arp, 0), lead: Some(48), band_db: 0.0, reverb: 0.25, comp: 0.2, seed: 23, ..base.clone() },
        Spec { name: "break_136", set: Set::Test, bpm: 136.0, kit: 16, grooves: &[BREAK], swing: 0.08, bass: (B::Sub, 39), chords: (C::Stab, 16), lead: Some(80), band_db: 3.0, comp: 0.6, seed: 24, ..base.clone() },
        Spec { name: "solo_110", set: Set::Test, bpm: 110.0, kit: 0, grooves: &[SOLO], bass: (B::None, 33), chords: (C::None, 0), lead: None, band_db: -60.0, reverb: 0.1, seed: 25, ..base.clone() },
        Spec { name: "melodic_88", set: Set::Test, bpm: 88.0, grooves: &[], bass: (B::Walk, 32), chords: (C::Arp, 0), lead: Some(65), reverb: 0.2, comp: 0.3, seed: 27, ..base.clone() },
        Spec { name: "vox_pop_104", set: Set::Test, bpm: 104.0, kit: 8, grooves: &[POP], bass: (B::Funk16, 38), chords: (C::Pad, 48), lead: None, band_db: 3.0, reverb: 0.18, vocal_db: Some(0.0), seed: 28, ..base.clone() },
        Spec { name: "speech_only", set: Set::Test, grooves: &[], bass: (B::None, 33), chords: (C::None, 0), lead: None, band_db: -60.0, reverb: 0.1, vocal_db: Some(0.0), seed: 29, ..base.clone() },
        Spec { name: "rock_noisy", set: Set::Test, bpm: 126.0, kit: 0, grooves: &[ROCK_A, ROCK_B], bass: (B::Root8, 33), chords: (C::PowerStrum, 30), lead: Some(81), band_db: 3.0, noise_db: Some(-48.0), flicker_db: 4.0, drive_db: 14.0, seed: 30, ..base.clone() },
        Spec { name: "wall_noisy", set: Set::Test, bpm: 100.0, grooves: &[], bass: (B::Drive16, 33), chords: (C::PowerStrum, 30), lead: Some(81), band_db: 0.0, noise_db: Some(-45.0), flicker_db: 5.0, drive_db: 18.0, seed: 31, ..base.clone() },
        Spec { name: "subkick_128", set: Set::Test, bpm: 128.0, kit: 200, grooves: &[HOUSE, TRAP], fills: false, bass: (B::Sub, 39), chords: (C::Pad, 89), lead: Some(80), band_db: 3.0, noise_db: Some(-55.0), flicker_db: 2.0, gaps: true, seed: 32, ..base.clone() },
        Spec { name: "tonal_96", set: Set::Test, bpm: 96.0, grooves: &[], bass: (B::Walk, 33), chords: (C::Arp, 24), lead: Some(13), seed: 26, ..base.clone() },
    ]
}

/// A random song for training a detector: the kit, tempo, groove, instruments, mix balance, reverb and
/// compression all drawn from `seed`. About one in five has no drums at all. `kits` limits which drum kits may
/// be drawn (to hold some out).
pub fn random_spec(seed: u64, kits: &[u16]) -> Spec {
    use Bass as B;
    use Chords as C;
    let mut r = Rng::new(seed ^ 0xD00D);
    const GROOVES: [&[Lane]; 11] = [ROCK_A, ROCK_B, HOUSE, FUNK_A, FUNK_B, TRAP, JAZZ, METAL, BALLAD, POP, BREAK];
    let drumless = r.f() < 0.2;
    let g1 = GROOVES[(r.f() * GROOVES.len() as f64) as usize];
    let g2 = GROOVES[(r.f() * GROOVES.len() as f64) as usize];
    let grooves: &'static [&'static [Lane]] = if drumless {
        &[]
    } else {
        // leaked: a handful of tiny slices per run of a test
        Box::leak(vec![g1, g2].into_boxed_slice())
    };
    let bass = r.pick(&[B::Root8, B::Walk, B::Sub, B::Funk16, B::Drive16]);
    let chords = r.pick(&[C::Pad, C::Stab, C::Arp, C::Strum, C::PowerStrum]);
    Spec {
        name: "random",
        set: Set::Dev,
        bpm: 70.0 + r.f() * 100.0,
        secs: 48.0,
        kit: r.pick(kits),
        grooves,
        swing: r.pick(&[0.0, 0.0, 0.1, 0.2]),
        rolls: r.f() < 0.25,
        fills: r.f() < 0.6,
        bass: (bass, r.pick(&[32, 33, 34, 38, 39])),
        chords: (chords, r.pick(&[0, 4, 16, 24, 29, 30, 48, 52, 56, 81, 89])),
        lead: if r.f() < 0.7 { Some(r.pick(&[11, 12, 13, 56, 65, 73, 80, 81])) } else { None },
        band_db: -3.0 + r.f() as f32 * 11.0,
        reverb: r.f() as f32 * 0.25,
        comp: r.f() as f32 * 0.8,
        vocal_db: if r.f() < 0.55 { Some(-6.0 + r.f() as f32 * 10.0) } else { None },
        noise_db: if r.f() < 0.85 { Some(-65.0 + r.f() as f32 * 30.0) } else { None },
        flicker_db: r.f() as f32 * 6.0,
        drive_db: if r.f() < 0.4 { r.f() as f32 * 20.0 } else { 0.0 },
        gaps: r.f() < 0.45 && std::env::var_os("GEQ_GAPS").is_some(),
        seed,
    }
}

impl Spec {
    /// Identifies the song a spec renders, for caching.
    pub fn cache_key(&self) -> String {
        let g: String = self.grooves.iter().map(|g| g.iter().map(|(k, p)| format!("{k}{p}")).collect::<String>()).collect::<Vec<_>>().join("+");
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in format!("{g}{:?}{:?}{:?}", self.bass.1, self.chords.1, self.lead).bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
        format!("{}-{}-{}-{:.0}-{:.0}-{:x}-{:.0}-{:.0}-{:.0}-{}-{}-{:.0}-{}-{:?}-{:?}-{:.0}-{:.0}-{}", self.name, self.seed, self.kit, self.bpm * 10.0, self.secs, h, self.band_db * 10.0, self.reverb * 100.0, self.comp * 100.0, self.rolls as u8, self.fills as u8, self.swing * 100.0, matches!(self.bass.0, Bass::None) as u8, self.vocal_db.map(|v| (v * 10.0) as i32), self.noise_db.map(|v| (v * 10.0) as i32), self.flicker_db * 10.0, self.drive_db * 10.0, self.gaps as u8)
    }
}

/// A computed drum for the kits that are not in the soundfont (`kit` 200 and above): `kit` 200 is a sub-bass kick
/// with no click at all, as in much electronic music; 201 adds a click; 202 is a short, dry kit. Sine-sweep
/// kicks and toms, noise snares and claps, filtered-noise hats and cymbals.
fn synth_drum(kit: u16, key: u8, vel: u8, rng: &mut Rng) -> Vec<f32> {
    let g = (vel as f32 / 127.0).powf(1.4);
    let n = |secs: f64| (secs * SR) as usize;
    let mut out = vec![0.0f32; n(1.6)];
    // one-pole filters: lp state, and hp as input minus lp
    let sweep = |out: &mut [f32], f0: f64, f1: f64, tp: f64, ta: f64, amp: f32| {
        let mut phase = 0.0f64;
        for (i, o) in out.iter_mut().enumerate().take(n(1.2)) {
            let t = i as f64 / SR;
            let f = f1 + (f0 - f1) * (-t / tp).exp();
            phase += std::f64::consts::TAU * f / SR;
            *o += (phase.sin() * (-t / ta).exp()) as f32 * amp;
        }
    };
    let burst = |out: &mut [f32], rng: &mut Rng, lo: f64, hi: f64, ta: f64, delay: f64, amp: f32| {
        let (alo, ahi) = (1.0 - (-std::f64::consts::TAU * lo / SR).exp(), 1.0 - (-std::f64::consts::TAU * hi / SR).exp());
        let (mut slo, mut shi) = (0.0f64, 0.0f64);
        for i in 0..n(1.4).saturating_sub(n(delay)) {
            let w = rng.f() * 2.0 - 1.0;
            slo += alo * (w - slo);
            shi += ahi * (w - shi);
            let t = i as f64 / SR;
            out[i + n(delay)] += ((shi - slo) * (-t / ta).exp()) as f32 * amp;
        }
    };
    let click = if kit == 200 { 0.0 } else if kit == 201 { 0.35 } else { 0.15 };
    let (r1, r2, r3) = (rng.f(), rng.f(), rng.f());
    match key {
        35 | 36 => {
            sweep(&mut out, 110.0 + r1 * 120.0, 38.0 + r2 * 22.0, 0.012 + r3 * 0.03, 0.12 + rng.f() * 0.25, 0.9);
            if click > 0.0 {
                burst(&mut out, rng, 1500.0, 7000.0, 0.004, 0.0, click as f32 * 1.4);
            }
        }
        37 | 38 | 40 => {
            sweep(&mut out, 260.0, 180.0 + r1 * 40.0, 0.03, 0.04 + r2 * 0.04, 0.4);
            burst(&mut out, rng, 700.0, 9000.0, 0.07 + r3 * 0.08, 0.0, 1.6);
        }
        39 => {
            for k in 0..3 {
                burst(&mut out, rng, 900.0, 7000.0, 0.012, k as f64 * 0.011, 1.2);
            }
            burst(&mut out, rng, 900.0, 7000.0, 0.09, 0.033, 1.0);
        }
        42 | 44 => burst(&mut out, rng, 6000.0, 20000.0, 0.012 + r1 * 0.03, 0.0, 1.4),
        46 => burst(&mut out, rng, 5000.0, 20000.0, 0.16 + r1 * 0.15, 0.0, 1.2),
        41 | 43 | 45 | 47 | 48 | 50 => {
            let f = 70.0 + (key as f64 - 41.0) * 14.0;
            sweep(&mut out, f * 1.5, f, 0.03, 0.1 + r1 * 0.1, 0.8);
            burst(&mut out, rng, 1000.0, 5000.0, 0.01, 0.0, 0.2);
        }
        _ => burst(&mut out, rng, 3500.0, 20000.0, 0.3 + r1 * 0.6, 0.0, 1.0),
    }
    out.iter().map(|v| v * g * 0.8).collect()
}

struct Event {
    t: f64,
    bank: u16,
    program: u16,
    key: u8,
    vel: u8,
    dur: f64,
    gain: f32,
}

fn vel_of(c: u8) -> u8 {
    match c {
        b'g' => 30,
        b'l' => 55,
        b'm' => 80,
        b'x' => 100,
        b'X' => 122,
        _ => 0,
    }
}

fn midi_key_to_pc(root: i32, third: i32, fifth: i32) -> [i32; 3] {
    [root, root + third, root + fifth]
}

/// A drum stem and its labels.
fn drums(spec: &Spec, rng: &mut Rng, sf: &Sf2) -> (Vec<[f32; 2]>, Vec<Label>) {
    let beat = 60.0 / spec.bpm;
    let step = beat / 4.0;
    let bars = (spec.secs / (beat * 4.0)) as usize;
    let mut ev: Vec<Event> = Vec::new();
    let push = |ev: &mut Vec<Event>, t: f64, key: u8, vel: u8, rng: &mut Rng| {
        let t = t + rng.gauss() * 0.003;
        let vel = (vel as f64 + rng.gauss() * 5.0).clamp(10.0, 127.0) as u8;
        ev.push(Event { t: t.max(0.0), bank: 128, program: spec.kit, key, vel, dur: 1e9, gain: 1.0 });
    };
    if spec.grooves.is_empty() {
        return (Vec::new(), Vec::new());
    }
    for bar in 0..bars {
        let t0 = 0.5 + bar as f64 * beat * 4.0;
        let fill = spec.fills && bar % 4 == 3;
        let groove = spec.grooves[bar % spec.grooves.len()];
        for (key, pat) in groove {
            for (s, c) in pat.bytes().enumerate() {
                let v = vel_of(c);
                if v == 0 || (fill && s >= 12 && *key != 49) {
                    continue;
                }
                let shift = if s % 2 == 1 { spec.swing * step * 2.0 } else { 0.0 };
                if *key == 49 && bar % 2 == 1 {
                    continue;
                }
                push(&mut ev, t0 + s as f64 * step + shift, *key, v, rng);
            }
        }
        if fill {
            for (i, key) in [50u8, 47, 45, 41].into_iter().enumerate() {
                push(&mut ev, t0 + (12 + i) as f64 * step, key, 105 + 4 * i as u8, rng);
            }
        } else if spec.rolls && bar % 2 == 1 {
            // a hat roll: four thirty-seconds on each of the last two steps, swelling
            for i in 0..8 {
                push(&mut ev, t0 + 14.0 * step + i as f64 * step / 4.0, 42, 50 + 8 * i as u8, rng);
            }
        }
        if spec.name == "solo_110" && bar % 2 == 1 {
            // snare roll into a crash on the next bar
            for i in 0..8 {
                push(&mut ev, t0 + 8.0 * step + i as f64 * step, 38, 60 + 8 * i as u8, rng);
            }
            push(&mut ev, t0 + 16.0 * step, 49, 120, rng);
        }
    }
    let mut stem = Vec::new();
    let mut labels = Vec::new();
    let mut srng = Rng::new(spec.seed ^ 0x5717);
    // kits 200-202 synthesize only the kick and toms (sine sweeps) and take everything else from a soundfont kit; 210-212
    // synthesize every drum. (Synthesized noise bursts as hats and snares looked like the speckle of real recordings.)
    let hybrid_kit = |k: u16| [24u16, 0, 8][(k - 200) as usize];
    for e in &ev {
        let synth_it = match e.program {
            200..=202 => matches!(e.key, 35 | 36 | 41 | 43 | 45 | 47 | 48 | 50),
            210..=212 => true,
            _ => false,
        };
        if synth_it {
            let d = synth_drum(200 + e.program % 10, e.key, e.vel, &mut srng);
            let at = (e.t * SR) as usize;
            if stem.len() < at + d.len() {
                stem.resize(at + d.len(), [0.0; 2]);
            }
            for (i, v) in d.iter().enumerate() {
                stem[at + i][0] += v;
                stem[at + i][1] += v;
            }
        } else {
            let program = if (200..=202).contains(&e.program) { hybrid_kit(e.program) } else { e.program };
            sf.note(&mut stem, e.bank, program, e.key, e.vel, e.t, e.dur, e.gain);
        }
        labels.push(Label { t: e.t, kind: Kind::of_key(e.key), vel: e.vel });
    }
    labels.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    (stem, labels)
}

/// Bass, chords and lead.
fn band(spec: &Spec, rng: &mut Rng, sf: &Sf2) -> Vec<[f32; 2]> {
    let beat = 60.0 / spec.bpm;
    let step = beat / 4.0;
    let bars = (spec.secs / (beat * 4.0)) as usize;
    let prog: [(i32, bool); 4] = [(57, true), (53, false), (48, false), (55, false)]; // Am F C G
    let mut stem = Vec::new();
    let t_of = |bar: usize, s: f64| 0.5 + bar as f64 * beat * 4.0 + s * step;
    let scale = [0, 2, 3, 5, 7, 8, 10]; // A minor
    let mut lead_note = 0i32;
    for bar in 0..bars {
        let (root, minor) = prog[bar % 4];
        let tri = midi_key_to_pc(root, if minor { 3 } else { 4 }, 7);
        let (bk, bprog) = spec.bass;
        let bass_root = ((root - 24).rem_euclid(12) + 28) as u8; // E1..D#2
        match bk {
            Bass::Root8 => {
                for s in (0..16).step_by(2) {
                    if rng.f() < 0.85 {
                        let oct = if s == 6 && rng.f() < 0.4 { 12 } else { 0 };
                        sf.note(&mut stem, 0, bprog, bass_root + oct, 90, t_of(bar, s as f64), step * 1.7, 1.0);
                    }
                }
            }
            Bass::Walk => {
                let walk = [0, 4, 7, 5];
                for q in 0..4 {
                    sf.note(&mut stem, 0, bprog, bass_root + 12 + walk[q] as u8 - if q == 3 { 12 } else { 0 }, 85, t_of(bar, 4.0 * q as f64), beat * 0.9, 1.0);
                }
            }
            Bass::Sub => {
                for (s, l) in [(0.0, 5.0), (6.0, 3.0), (10.0, 5.0)] {
                    if rng.f() < 0.9 {
                        sf.note(&mut stem, 0, bprog, bass_root + 12, 105, t_of(bar, s), l * step, 1.0);
                    }
                }
            }
            Bass::Funk16 => {
                for (s, c) in "X..x.xx..x.x..x.".bytes().enumerate() {
                    if c != b'.' {
                        sf.note(&mut stem, 0, bprog, bass_root + 12 * (s % 5 == 4) as u8, 90, t_of(bar, s as f64), step * 0.8, 1.0);
                    }
                }
            }
            Bass::Drive16 => {
                for s in 0..16 {
                    sf.note(&mut stem, 0, bprog, bass_root + 12, 100, t_of(bar, s as f64), step * 0.9, 1.0);
                }
            }
            Bass::None => {}
        }
        let (ck, cprog) = spec.chords;
        let chord_gain = 0.55;
        match ck {
            Chords::Pad => {
                for k in tri {
                    sf.note(&mut stem, 0, cprog, (k + 12) as u8, 80, t_of(bar, 0.0), beat * 3.8, chord_gain);
                }
            }
            Chords::Stab => {
                for s in [0.0, 6.0, 10.0] {
                    for k in tri {
                        sf.note(&mut stem, 0, cprog, (k + 12) as u8, 85, t_of(bar, s), step * 2.0, chord_gain);
                    }
                }
            }
            Chords::Arp => {
                for s in 0..8 {
                    sf.note(&mut stem, 0, cprog, (tri[s % 3] + 12 + 12 * (s % 5 == 4) as i32) as u8, 80, t_of(bar, s as f64 * 2.0), step * 2.0, chord_gain * 1.4);
                }
                // a second, plucked layer on the sixteenths: the tonal lookalike of a hat pattern
                if spec.grooves.is_empty() && spec.name != "melodic_88" {
                    for s in 0..16 {
                        if rng.f() < 0.6 {
                            sf.note(&mut stem, 0, 45, (tri[s % 3] + 24) as u8, 70, t_of(bar, s as f64), step * 0.6, chord_gain);
                        }
                    }
                }
            }
            Chords::Strum | Chords::PowerStrum => {
                let notes: Vec<i32> = if matches!(ck, Chords::PowerStrum) { vec![root - 12, root - 5, root] } else { tri.iter().map(|k| k - 12 + 12).collect() };
                let hits: &[f64] = if matches!(ck, Chords::PowerStrum) { &[0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0] } else { &[0.0, 4.0, 8.0, 12.0] };
                for s in hits {
                    for (i, k) in notes.iter().enumerate() {
                        sf.note(&mut stem, 0, cprog, (*k + 12) as u8, 95, t_of(bar, *s) + i as f64 * 0.012, step * 3.0, chord_gain);
                    }
                }
            }
            Chords::None => {}
        }
        if let Some(lp) = spec.lead {
            let mut s = 0.0;
            while s < 16.0 {
                let len = rng.pick(&[2.0, 4.0, 4.0, 6.0, 8.0]);
                if rng.f() < 0.75 {
                    lead_note = (lead_note + rng.pick(&[-2i32, -1, 0, 1, 2])).clamp(-3, 9);
                    let idx = lead_note.rem_euclid(7) as usize;
                    let key = 69 + scale[idx] + 12 * lead_note.div_euclid(7);
                    sf.note(&mut stem, 0, lp, key as u8, 90, t_of(bar, s), step * len * 0.9, 0.6);
                }
                s += len;
            }
        }
    }
    stem
}

fn rms(v: &[[f32; 2]]) -> f64 {
    let s: f64 = v.iter().map(|x| (x[0] as f64).powi(2) + (x[1] as f64).powi(2)).sum();
    (s / (2.0 * v.len().max(1) as f64)).sqrt()
}

/// Reverb: four combs and two all-passes per side, fed with the mid signal.
fn reverb(x: &mut [[f32; 2]], wet: f32) {
    if wet <= 0.0 {
        return;
    }
    struct Comb(Vec<f32>, usize, f32);
    let mk = |len: usize| Comb(vec![0.0; len], 0, 0.0);
    let lens_l = [1116usize, 1188, 1277, 1356].map(|n| n * 48 / 44);
    let lens_r = lens_l.map(|n| n + 23);
    let mut combs: [Vec<Comb>; 2] = [lens_l.iter().map(|n| mk(*n)).collect(), lens_r.iter().map(|n| mk(*n)).collect()];
    let mut aps: [Vec<Comb>; 2] = [[556usize, 441].iter().map(|n| mk(n * 48 / 44)).collect(), [579usize, 464].iter().map(|n| mk(n * 48 / 44)).collect()];
    let (fb, damp) = (0.82f32, 0.3f32);
    for fr in x.iter_mut() {
        let input = (fr[0] + fr[1]) * 0.5 * 0.2;
        for side in 0..2 {
            let mut o = 0.0;
            for c in combs[side].iter_mut() {
                let y = c.0[c.1];
                c.2 = y * (1.0 - damp) + c.2 * damp;
                c.0[c.1] = input + c.2 * fb;
                c.1 = (c.1 + 1) % c.0.len();
                o += y;
            }
            for a in aps[side].iter_mut() {
                let y = a.0[a.1];
                let z = o + y * 0.5;
                a.0[a.1] = z;
                a.1 = (a.1 + 1) % a.0.len();
                o = y - o * 0.5;
            }
            fr[side] += o * wet * 3.0;
        }
    }
}

/// A stereo-linked bus compressor, then a soft clipper and normalization to `peak`.
fn master(x: &mut [[f32; 2]], comp: f32, peak: f32, rng: &mut Rng) {
    let top = x.iter().fold(1e-9f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    let (thr, ratio) = (0.1f32 * (1.0 - 0.6 * comp) + 0.02, 1.0 + 4.0 * comp);
    let (att, rel) = ((-1.0 / (0.004 * SR as f32)).exp(), (-1.0 / (0.12 * SR as f32)).exp());
    let mut env = 0.0f32;
    for f in x.iter_mut() {
        let lvl = f[0].abs().max(f[1].abs()) / top;
        env = if lvl > env { att * env + (1.0 - att) * lvl } else { rel * env + (1.0 - rel) * lvl };
        let g = if env > thr { (thr * (env / thr).powf(1.0 / ratio)) / env } else { 1.0 };
        f[0] *= g / top;
        f[1] *= g / top;
    }
    let top = x.iter().fold(1e-9f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    for f in x.iter_mut() {
        for c in f.iter_mut() {
            *c = (*c / top * 1.4).tanh() / 1.4f32.tanh() * peak + (rng.f() as f32 - 0.5) * 3e-4;
        }
    }
}

/// Speech recordings found on this machine (48 kHz stereo): a wideband text-to-speech sample and the speech
/// dispatcher's test message. Missing files are skipped.
fn voices() -> &'static Vec<Vec<[f32; 2]>> {
    static V: std::sync::OnceLock<Vec<Vec<[f32; 2]>>> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        ["/home/nate/.hermes/hermes-agent/tools/neutts_samples/jo.wav", "/usr/share/sounds/speech-dispatcher/dummy-message.wav"]
            .iter()
            .filter_map(|p| crate::synth::load_wav(p).ok())
            .collect()
    })
}

/// Phrases of speech with gaps between them, `secs` long.
fn vocal_stem(secs: f64, rng: &mut Rng) -> Vec<[f32; 2]> {
    let v = voices();
    let n = (secs * SR) as usize;
    let mut out = vec![[0.0f32; 2]; n];
    if v.is_empty() {
        return out;
    }
    let mut at = (rng.f() * 2.0 * SR) as usize;
    while at < n {
        let voice = &v[(rng.f() * v.len() as f64) as usize % v.len()];
        let len = ((2.0 + rng.f() * 6.0) * SR) as usize;
        let start = (rng.f() * voice.len().saturating_sub(len) as f64) as usize;
        let fade = (0.02 * SR) as usize;
        for i in 0..len.min(voice.len() - start).min(n - at) {
            let e = (i.min(len - i) as f32 / fade as f32).min(1.0);
            out[at + i][0] += voice[start + i][0] * e;
            out[at + i][1] += voice[start + i][1] * e;
        }
        at += len + ((0.2 + rng.f() * 2.3) * SR) as usize;
    }
    out
}

/// Four bands of noise (to 4, 8 and 12 kHz, and the rest) whose levels step at random every ~21 ms by `flicker_db`.
fn texture(n: usize, rng: &mut Rng, rms_db: f32, flicker_db: f32) -> Vec<f32> {
    let (a4, a8, a12) = [4000.0f64, 8000.0, 12000.0].map(|f| 1.0 - (-std::f64::consts::TAU * f / SR).exp()).into();
    let (mut p4, mut p8, mut p12) = (0.0f64, 0.0f64, 0.0f64);
    let base = [1.0f64, 0.6, 0.35, 0.2];
    let mut gains = base;
    let mut left = 0usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if left == 0 {
            left = 900 + (rng.f() * 240.0) as usize;
            for b in 0..4 {
                gains[b] = base[b] * 10f64.powf(rng.gauss() * flicker_db as f64 / 20.0);
            }
        }
        left -= 1;
        let w = rng.f() * 2.0 - 1.0;
        p4 += a4 * (w - p4);
        p8 += a8 * (w - p8);
        p12 += a12 * (w - p12);
        out.push((gains[0] * p4 + gains[1] * (p8 - p4) + gains[2] * (p12 - p8) + gains[3] * (w - p12)) as f32);
    }
    let rms = (out.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / n.max(1) as f64).sqrt().max(1e-12);
    let g = (10f64.powf(rms_db as f64 / 20.0) / rms) as f32;
    out.iter().map(|v| v * g).collect()
}

/// Waveshaping of a stem: `drive_db` of gain into a tanh, scaled back to the stem's own rms.
fn distort(stem: &mut [[f32; 2]], drive_db: f32) {
    let r0 = rms(stem);
    if drive_db <= 0.0 || r0 <= 0.0 {
        return;
    }
    let g = 10f32.powf(drive_db / 20.0) / r0 as f32 * 0.3;
    for f in stem.iter_mut() {
        f[0] = (f[0] * g).tanh();
        f[1] = (f[1] * g).tanh();
    }
    let r1 = rms(stem);
    for f in stem.iter_mut() {
        f[0] *= (r0 / r1.max(1e-9)) as f32;
        f[1] *= (r0 / r1.max(1e-9)) as f32;
    }
}

pub struct Song {
    pub name: &'static str,
    pub set: Set,
    pub audio: Vec<[f32; 2]>,
    pub labels: Vec<Label>,
}

pub fn render(sf: &Sf2, spec: &Spec) -> Song {
    let mut rng = Rng::new(spec.seed);
    let (d, labels) = drums(spec, &mut rng, sf);
    let mut b = band(spec, &mut rng, sf);
    distort(&mut b, spec.drive_db);
    let n = d.len().max(b.len()).max((spec.secs * SR) as usize);
    let (rd, rb) = (rms(&d), rms(&b));
    let gb = if rb > 0.0 && rd > 0.0 { (rd / rb) as f32 * 10f32.powf(spec.band_db / 20.0) } else { 1.0 };
    let gb = if d.is_empty() { 1.0 } else { gb };
    let mut mix = vec![[0.0f32; 2]; n];
    for (i, f) in d.iter().enumerate() {
        mix[i][0] += f[0];
        mix[i][1] += f[1];
    }
    for (i, f) in b.iter().enumerate() {
        let g = if spec.band_db <= -50.0 { 0.0 } else { gb };
        mix[i][0] += f[0] * g;
        mix[i][1] += f[1] * g;
    }
    mix.truncate((spec.secs * SR) as usize);
    if let Some(db) = spec.vocal_db {
        let voice = vocal_stem(spec.secs, &mut Rng::new(spec.seed ^ 0x701CE));
        let (rm, rv) = (rms(&mix), rms(&voice));
        if rv > 0.0 {
            let target = if rm > 1e-6 { rm } else { 0.1 };
            let g = (target / rv) as f32 * 10f32.powf(db / 20.0);
            for (m, v) in mix.iter_mut().zip(&voice) {
                m[0] += v[0] * g;
                m[1] += v[1] * g;
            }
        }
    }
    reverb(&mut mix, spec.reverb);
    master(&mut mix, spec.comp, 0.89, &mut rng);
    if let Some(db) = spec.noise_db {
        let mut nr = Rng::new(spec.seed ^ 0xA1B2);
        let (tl, tr) = (texture(mix.len(), &mut nr, db, spec.flicker_db), texture(mix.len(), &mut nr, db, spec.flicker_db));
        for (i, f) in mix.iter_mut().enumerate() {
            f[0] += tl[i];
            f[1] += tr[i];
        }
    }
    let mut labels: Vec<Label> = labels.into_iter().filter(|l| l.t < spec.secs).collect();
    if spec.gaps {
        let mut gr = Rng::new(spec.seed ^ 0x6A95);
        for _ in 0..1 + (gr.f() * 3.0) as usize {
            let (start, len) = (5.0 + gr.f() * (spec.secs - 9.0), 0.6 + gr.f() * 1.9);
            let (a, b) = ((start * SR) as usize, (((start + len) * SR) as usize).min(mix.len()));
            for f in &mut mix[a..b] {
                *f = [0.0, 0.0]; // exact digital silence, as when a player stops
            }
            labels.retain(|l| l.t < start - 0.02 || l.t >= start + len);
        }
    }
    Song { name: spec.name, set: spec.set, audio: mix, labels }
}

/// A recording's drums and the rest of the band (separate stems at 48 kHz) mixed the way `render` mixes a synthetic song, as
/// training material with real drums in it: the band at a random level against the drums, now and then distorted, speech
/// laid over it, bus compression and a noise floor, all drawn from `seed`. With no drums it is the band alone, which is
/// real music for the detector to leave quiet on.
pub fn remix(drums: &[[f32; 2]], rest: &[[f32; 2]], seed: u64) -> Vec<[f32; 2]> {
    let mut rng = Rng::new(seed ^ 0x5EA1);
    let n = drums.len().max(rest.len());
    let mut band = rest.to_vec();
    if rng.f() < 0.3 {
        distort(&mut band, 6.0 + rng.f() as f32 * 10.0);
    }
    let band_db = -6.0 + rng.f() as f32 * 16.0;
    let (rd, rb) = (rms(drums), rms(&band));
    let gb = if !drums.is_empty() && rb > 0.0 && rd > 0.0 { (rd / rb) as f32 * 10f32.powf(band_db / 20.0) } else { 1.0 };
    let mut mix = vec![[0.0f32; 2]; n];
    for (m, f) in mix.iter_mut().zip(drums) {
        m[0] += f[0];
        m[1] += f[1];
    }
    for (m, f) in mix.iter_mut().zip(&band) {
        m[0] += f[0] * gb;
        m[1] += f[1] * gb;
    }
    if rng.f() < 0.4 {
        let voice = vocal_stem(n as f64 / SR, &mut rng);
        let (rm, rv) = (rms(&mix), rms(&voice));
        if rv > 0.0 && rm > 1e-6 {
            let g = (rm / rv) as f32 * 10f32.powf((-6.0 + rng.f() as f32 * 10.0) / 20.0);
            for (m, v) in mix.iter_mut().zip(&voice) {
                m[0] += v[0] * g;
                m[1] += v[1] * g;
            }
        }
    }
    master(&mut mix, rng.f() as f32 * 0.8, 0.89, &mut rng);
    if rng.f() < 0.85 {
        let (db, flicker) = (-65.0 + rng.f() as f32 * 30.0, rng.f() as f32 * 5.0);
        let mut nr = Rng::new(seed ^ 0xA1B2);
        let (tl, tr) = (texture(n, &mut nr, db, flicker), texture(n, &mut nr, db, flicker));
        for (i, f) in mix.iter_mut().enumerate() {
            f[0] += tl[i];
            f[1] += tr[i];
        }
    }
    mix
}

/// A named song of the corpus, cut to `secs`; None when the soundfont is not installed.
pub fn song(name: &str, secs: f64) -> Option<Song> {
    let sf = Sf2::load()?;
    let spec = specs().into_iter().find(|s| s.name == name)?;
    Some(render(&sf, &Spec { secs, ..spec }))
}

pub fn write_wav(path: &str, audio: &[[f32; 2]]) {
    let n = audio.len() * 4;
    let mut b = Vec::with_capacity(44 + n);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + n as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&48000u32.to_le_bytes());
    b.extend_from_slice(&(48000u32 * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(n as u32).to_le_bytes());
    for f in audio {
        for c in f {
            b.extend_from_slice(&((c.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    std::fs::write(path, b).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_soundfont_renders_labelled_drums() {
        let Some(sf) = Sf2::load() else {
            eprintln!("no soundfont at {SF2_PATH}; skipping");
            return;
        };
        let spec = specs().into_iter().find(|s| s.name == "rock_130").unwrap();
        let song = render(&sf, &Spec { secs: 8.0, ..spec });
        assert_eq!(song.audio.len(), 8 * 48_000);
        let peak = song.audio.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.5 && peak <= 1.0, "peak {peak}");
        assert!(song.labels.iter().filter(|l| l.kind == Kind::Kick).count() >= 4);
        assert!(song.labels.iter().filter(|l| l.kind == Kind::Snare).count() >= 2);
        // a kick sample starts exactly where its label says: the mix is loud there and quiet just before
        let k = song.labels.iter().find(|l| l.kind == Kind::Kick && l.t > 1.0).unwrap();
        let at = (k.t * SR) as usize;
        let loud = song.audio[at..at + 2400].iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!(loud > 0.05, "no energy after the kick label: {loud}");
    }
}
