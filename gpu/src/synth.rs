//! Deterministic synthetic audio for visual iteration and tests (`GEQ_SYNTH`).
//!
//! `demo` is a looping, sectioned 120 BPM groove (sparse intro, groove, build, dense drop,
//! breakdown, outro with a bar of silence). The other fixtures isolate one behaviour each:
//! silence, quiet input, single impulses, sparse percussion (a kick every 3 s, a snare between), sustained tones, a log sweep, a dense
//! mixture, stereo movement and phase rotation. Everything is a pure function of the sample index
//! and a seeded noise generator, so two runs are bit-identical regardless of chunking.

use std::f64::consts::TAU;

pub const RATE: f64 = 48_000.0;
const BEAT: f64 = 0.5; // 120 BPM

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fixture {
    Demo,
    Silence,
    Quiet,
    Impulse,
    Percussion,
    Tones,
    Sweep,
    Dense,
    Stereo,
    Phase,
    /// A WAV file played on a loop (`GEQ_REPLAY`).
    Replay,
}

impl Fixture {
    pub fn parse(s: &str) -> Option<Fixture> {
        Some(match s.to_ascii_lowercase().as_str() {
            "1" | "true" | "demo" | "suite" => Fixture::Demo,
            "silence" => Fixture::Silence,
            "quiet" => Fixture::Quiet,
            "impulse" => Fixture::Impulse,
            "percussion" | "perc" => Fixture::Percussion,
            "tones" => Fixture::Tones,
            "sweep" => Fixture::Sweep,
            "dense" => Fixture::Dense,
            "stereo" => Fixture::Stereo,
            "phase" => Fixture::Phase,
            "replay" => Fixture::Replay,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Fixture::Demo => "demo",
            Fixture::Silence => "silence",
            Fixture::Quiet => "quiet",
            Fixture::Impulse => "impulse",
            Fixture::Percussion => "percussion",
            Fixture::Tones => "tones",
            Fixture::Sweep => "sweep",
            Fixture::Dense => "dense",
            Fixture::Stereo => "stereo",
            Fixture::Phase => "phase",
            Fixture::Replay => "replay",
        }
    }
}

/// Longest recording kept in memory (5 minutes at 48 kHz).
const MAX_REPLAY_FRAMES: usize = 5 * 60 * 48_000;

/// Read a PCM (16/24/32-bit) or float WAV file as 48 kHz stereo frames. Mono is duplicated,
/// extra channels are dropped, other sample rates are resampled linearly.
pub fn load_wav(path: &str) -> Result<Vec<[f32; 2]>, String> {
    parse_wav(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?)
}

fn parse_wav(b: &[u8]) -> Result<Vec<[f32; 2]>, String> {
    let u16le = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32le = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut fmt, mut data): (Option<(u16, u16, u32, u16)>, Option<&[u8]>) = (None, None);
    let mut o = 12;
    while o + 8 <= b.len() {
        let (id, size) = (&b[o..o + 4], u32le(o + 4) as usize);
        let body = &b[o + 8..(o + 8 + size).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            let mut tag = u16le(o + 8);
            if tag == 0xFFFE && body.len() >= 26 {
                tag = u16le(o + 8 + 24); // extensible: the sub-format's first two bytes
            }
            fmt = Some((tag, u16le(o + 10), u32le(o + 12), u16le(o + 22)));
        } else if id == b"data" {
            data = Some(body);
        }
        o += 8 + size + (size & 1);
    }
    let ((tag, ch, rate, bits), data) = (fmt.ok_or("no fmt chunk")?, data.ok_or("no data chunk")?);
    let ch = ch.max(1) as usize;
    let bytes = (bits / 8) as usize;
    if bytes == 0 || !matches!((tag, bits), (1, 16) | (1, 24) | (1, 32) | (3, 32)) {
        return Err(format!("unsupported WAV format (tag {tag}, {bits} bits)"));
    }
    let sample = |p: &[u8]| -> f32 {
        match (tag, bits) {
            (1, 16) => i16::from_le_bytes([p[0], p[1]]) as f32 / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, p[0], p[1], p[2]]) >> 8) as f32 / 8_388_608.0,
            (1, _) => i32::from_le_bytes([p[0], p[1], p[2], p[3]]) as f32 / 2_147_483_648.0,
            _ => f32::from_le_bytes([p[0], p[1], p[2], p[3]]),
        }
    };
    let frame_bytes = bytes * ch;
    let n = data.len() / frame_bytes;
    let frame = |i: usize| -> [f32; 2] {
        let f = &data[i * frame_bytes..];
        let l = sample(&f[..bytes]);
        [l, if ch > 1 { sample(&f[bytes..2 * bytes]) } else { l }]
    };
    if rate == 0 || n == 0 {
        return Err("empty recording".into());
    }
    let out_len = ((n as f64 * RATE / rate as f64) as usize).min(MAX_REPLAY_FRAMES);
    let mut out = Vec::with_capacity(out_len);
    for k in 0..out_len {
        let pos = k as f64 * rate as f64 / RATE;
        let (i, t) = (pos as usize, (pos - pos.floor()) as f32);
        let (a, c) = (frame(i.min(n - 1)), frame((i + 1).min(n - 1)));
        out.push([a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t]);
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Intro,
    Groove,
    Build,
    Drop,
    Breakdown,
    Outro,
}

fn section(bar: u32) -> (Section, u32) {
    match bar % 32 {
        b @ 0..=3 => (Section::Intro, b),
        b @ 4..=7 => (Section::Groove, b - 4),
        b @ 8..=11 => (Section::Build, b - 8),
        b @ 12..=19 => (Section::Drop, b - 12),
        b @ 20..=23 => (Section::Breakdown, b - 20),
        b @ 24..=27 => (Section::Groove, b - 24 + 4),
        b => (Section::Outro, b - 28),
    }
}

fn midi(n: f64) -> f64 {
    440.0 * 2f64.powf((n - 69.0) / 12.0)
}

fn pan(l: f32, r: f32, p: f32) -> (f32, f32) {
    // p in -1..1, constant power
    let a = (p + 1.0) * std::f32::consts::FRAC_PI_4;
    (l * a.cos() * 1.414, r * a.sin() * 1.414)
}

pub struct Synth {
    fx: Fixture,
    n: u64,
    rng: u32,
    hat_lp: f32,
    snare_lp: f32,
    bass_lp: f32,
    riser_lp: f32,
    pink: f32,
    replay: Vec<[f32; 2]>,
}

impl Synth {
    pub fn new(fx: Fixture) -> Synth {
        Synth { fx, n: 0, rng: 0x1234_5678, hat_lp: 0.0, snare_lp: 0.0, bass_lp: 0.0, riser_lp: 0.0, pink: 0.0, replay: Vec::new() }
    }

    /// Loop a recording (48 kHz stereo frames, see `load_wav`).
    pub fn with_replay(frames: Vec<[f32; 2]>) -> Synth {
        Synth { replay: frames, ..Synth::new(Fixture::Replay) }
    }

    /// Samples generated so far.
    pub fn position(&self) -> u64 {
        self.n
    }

    /// Append `frames` stereo samples.
    pub fn render(&mut self, frames: usize, l: &mut Vec<f32>, r: &mut Vec<f32>) {
        for _ in 0..frames {
            let (a, b) = self.sample();
            l.push(a);
            r.push(b);
            self.n += 1;
        }
    }

    fn noise(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn kick(age: f64) -> f32 {
        if !(0.0..0.6).contains(&age) {
            return 0.0;
        }
        let (f0, f1, tau) = (46.0, 160.0, 0.026);
        let phase = TAU * (f0 * age + (f1 - f0) * tau * (1.0 - (-age / tau).exp()));
        (phase.sin() * (-age / 0.16).exp()) as f32
    }

    fn snare(&mut self, age: f64) -> f32 {
        if !(0.0..0.4).contains(&age) {
            return 0.0;
        }
        let x = self.noise();
        self.snare_lp += 0.14 * (x - self.snare_lp);
        let hp = x - self.snare_lp;
        let tone = (TAU * 185.0 * age).sin() as f32 * (-age / 0.05).exp() as f32;
        hp * (-age / 0.09).exp() as f32 * 0.7 + tone * 0.45
    }

    fn hat(&mut self, age: f64, open: bool) -> f32 {
        let tau = if open { 0.11 } else { 0.028 };
        if !(0.0..0.5).contains(&age) {
            return 0.0;
        }
        let x = self.noise();
        self.hat_lp += 0.6 * (x - self.hat_lp);
        (x - self.hat_lp) * (-age / tau).exp() as f32
    }

    fn song(&mut self, t: f64, force: Option<Section>, density: f32) -> (f32, f32) {
        let bar = (t / (BEAT * 4.0)) as u32;
        let (sec, sb) = match force {
            Some(s) => (s, bar % 8),
            None => section(bar),
        };
        let beat_i = (t / BEAT) as u64;
        let beat_age = t - beat_i as f64 * BEAT;
        let in_bar = (beat_i % 4) as u32;
        let eighth_age = (t % (BEAT / 2.0)) as f64;
        let sixteenth_age = (t % (BEAT / 4.0)) as f64;
        let sixteenth_i = (t / (BEAT / 4.0)) as u64;

        let (mut out_l, mut out_r) = (0.0f32, 0.0f32);

        // ---- kick
        let kick_on = match sec {
            Section::Intro | Section::Breakdown => in_bar == 0,
            Section::Outro => sb < 2,
            _ => true,
        };
        let outro_gain = match sec {
            Section::Outro => [1.0, 0.8, 0.45, 0.0][sb as usize],
            _ => 1.0,
        };
        if kick_on && !(sec == Section::Outro && sb >= 2) {
            let k = Self::kick(beat_age) * 0.85 * outro_gain;
            out_l += k;
            out_r += k;
        }
        // ---- snare: beats 2 and 4, plus a 16th roll in the last bar of the build
        let roll = sec == Section::Build && sb == 3;
        let snare_age = if roll { sixteenth_age } else { beat_age };
        let snare_on = match sec {
            Section::Groove | Section::Drop => in_bar == 1 || in_bar == 3,
            Section::Build => roll || (sb >= 1 && (in_bar == 1 || in_bar == 3)),
            _ => false,
        };
        if snare_on {
            let ramp = ((t / (BEAT * 4.0)) % 1.0) as f32;
            let s = self.snare(snare_age) * if roll { 0.25 + 0.55 * ramp } else { 0.8 };
            out_l += s * 0.9;
            out_r += s * 0.9;
        }
        // ---- hats
        let hat_mode = match sec {
            Section::Intro => 1,
            Section::Groove | Section::Build => 2,
            Section::Drop => 3,
            Section::Outro if sb < 3 => 2,
            _ => 0,
        };
        if hat_mode > 0 {
            let (age, on, open) = match hat_mode {
                1 => (beat_age, in_bar % 2 == 1, false),
                2 => (eighth_age, true, false),
                _ => {
                    let open = sixteenth_i % 8 == 6;
                    (sixteenth_age, true, open)
                }
            };
            if on {
                let h = self.hat(age, open) * 0.22 * outro_gain;
                let p = if sixteenth_i % 2 == 0 { -0.3 } else { 0.3 };
                let (a, b) = pan(h, h, p);
                out_l += a;
                out_r += b;
            }
        }
        // ---- bass: eighth-note pattern, root follows the chord
        let bass_on = matches!(sec, Section::Groove | Section::Build | Section::Drop) || (sec == Section::Outro && sb == 0);
        let chord = ((t / (BEAT * 8.0)) as usize) % 4; // Am F C G, two bars each
        let roots = [33.0, 29.0, 36.0, 31.0]; // A1 F1 C2 G1
        let eighth_i = (t / (BEAT / 2.0)) as u64;
        const PAT: [bool; 8] = [true, false, true, true, false, true, true, false];
        if bass_on && PAT[(eighth_i % 8) as usize] {
            let f = midi(roots[chord] + if eighth_i % 8 == 6 { 7.0 } else { 0.0 });
            let saw = ((f * t).fract() * 2.0 - 1.0) as f32;
            self.bass_lp += 0.07 * (saw - self.bass_lp);
            let b = self.bass_lp * (-(eighth_age as f32) / 0.22).exp() * 0.6;
            out_l += b;
            out_r += b;
        } else {
            self.bass_lp *= 0.98;
        }
        // ---- pad: sustained chord, slow tremolo, wide
        let pad_gain = match sec {
            Section::Intro => 0.9,
            Section::Breakdown => 1.2,
            Section::Outro => outro_gain,
            _ => 0.6,
        } * 0.045;
        let chords: [[f64; 4]; 4] = [[57.0, 60.0, 64.0, 67.0], [53.0, 57.0, 60.0, 64.0], [55.0, 60.0, 64.0, 67.0], [55.0, 59.0, 62.0, 67.0]];
        for (i, n) in chords[chord].iter().enumerate() {
            let f = midi(*n + 12.0);
            let trem = 0.75 + 0.25 * (TAU * (0.13 + 0.04 * i as f64) * t).sin() as f32;
            let a = (TAU * f * 1.002 * t).sin() as f32 + 0.5 * (TAU * f * 2.0 * t).sin() as f32;
            let b = (TAU * f * 0.998 * t + 1.1).sin() as f32 + 0.5 * (TAU * f * 2.0 * t + 0.4).sin() as f32;
            let w = if i % 2 == 0 { 0.7 } else { 0.3 };
            out_l += a * pad_gain * trem * (0.5 + w);
            out_r += b * pad_gain * trem * (1.5 - w);
        }
        // ---- lead: 16th arpeggio that pans slowly across the field
        let lead_on = matches!(sec, Section::Build | Section::Drop | Section::Breakdown) && !(sec == Section::Build && sb < 1)
            || (sec == Section::Groove && sb >= 2);
        if lead_on {
            const NOTES: [f64; 8] = [69.0, 72.0, 76.0, 72.0, 79.0, 76.0, 72.0, 74.0];
            let n = NOTES[(sixteenth_i % 8) as usize] + [0.0, -4.0, 3.0, -2.0][chord] + if sec == Section::Drop { 12.0 } else { 0.0 };
            let f = midi(n);
            let env = (-(sixteenth_age as f32) / 0.11).exp();
            let s = ((TAU * f * t).sin() as f32 + 0.35 * (TAU * 2.0 * f * t).sin() as f32 + 0.15 * (TAU * 3.0 * f * t).sin() as f32) * env * 0.16;
            let p = 0.85 * (TAU * t / 7.0).sin() as f32;
            let (a, b) = pan(s, s, p);
            out_l += a;
            out_r += b;
        }
        // ---- riser in the build
        if sec == Section::Build && sb >= 2 {
            let x = ((t / (BEAT * 4.0)) % 4.0 - 2.0) as f32 / 2.0 + 0.0; // 0..1 across the last two bars
            let x = x.clamp(0.0, 1.0);
            let n = self.noise();
            self.riser_lp += (0.02 + 0.9 * x * x) * (n - self.riser_lp);
            let g = 0.05 + 0.3 * x * x;
            out_l += self.riser_lp * g;
            out_r -= self.riser_lp * g * 0.9;
        }
        let g = density;
        ((out_l * g).tanh(), (out_r * g).tanh())
    }

    fn sample(&mut self) -> (f32, f32) {
        let t = self.n as f64 / RATE;
        match self.fx {
            Fixture::Silence => (0.0, 0.0),
            Fixture::Replay => match self.replay.len() {
                0 => (0.0, 0.0),
                len => {
                    let f = self.replay[self.n as usize % len];
                    (f[0], f[1])
                }
            },
            Fixture::Demo => self.song(t, None, 1.0),
            Fixture::Quiet => {
                let (a, b) = self.song(t, None, 1.0);
                let n = self.noise() * 4e-5;
                (a * 0.03 + n, b * 0.03 - n)
            }
            Fixture::Dense => {
                let (a, b) = self.song(t, Some(Section::Drop), 1.3);
                let n = self.noise();
                self.pink += 0.08 * (n - self.pink);
                ((a + self.pink * 0.12) * 0.8, (b - self.pink * 0.12) * 0.8)
            }
            Fixture::Impulse => {
                if self.n % 48_000 == 24_000 { (0.9, 0.9) } else { (0.0, 0.0) }
            }
            Fixture::Percussion => {
                let k = Self::kick(t % 3.0) * 0.9;
                let s = self.snare((t + 1.5) % 3.0) * 0.7;
                (k + s, k + s)
            }
            Fixture::Tones => {
                let mut a = 0.0f32;
                for (i, n) in [57.0, 60.0, 64.0, 67.0, 71.0].iter().enumerate() {
                    let f = midi(*n);
                    a += (TAU * f * t).sin() as f32 * 0.06 * (0.8 + 0.2 * (TAU * 0.2 * t + i as f64).sin() as f32);
                }
                (a, a)
            }
            Fixture::Sweep => {
                let tau = t % 6.0;
                let ratio: f64 = 20000.0 / 30.0;
                let phase = TAU * 30.0 * 6.0 / ratio.ln() * (ratio.powf(tau / 6.0) - 1.0);
                let s = phase.sin() as f32 * 0.4;
                (s, s)
            }
            Fixture::Stereo => {
                // a tone and a noisy shimmer swing left and right; every 12 s a hard-left then hard-right burst
                let p = (TAU * t / 6.0).sin() as f32;
                let s = (TAU * 330.0 * t).sin() as f32 * 0.25 + (TAU * 1320.0 * t).sin() as f32 * 0.08;
                let (mut a, mut b) = pan(s, s, p);
                let ph = t % 12.0;
                let burst = (TAU * 880.0 * t).sin() as f32 * 0.3 * (-((ph % 1.0) as f32) / 0.2).exp();
                if ph < 1.0 { a += burst } else if (6.0..7.0).contains(&ph) { b += burst }
                (a, b)
            }
            Fixture::Phase => {
                // rotate the right channel's phase against the left: 0 deg (mono) -> 180 deg (out of phase) -> back
                let phi = std::f64::consts::PI * (0.5 - 0.5 * (TAU * t / 6.0).cos());
                let a = (TAU * 220.0 * t).sin() as f32 * 0.3 + (TAU * 440.0 * t).sin() as f32 * 0.1;
                let b = (TAU * 220.0 * t + phi).sin() as f32 * 0.3 + (TAU * 440.0 * t + phi).sin() as f32 * 0.1;
                (a, b)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(fx: Fixture, secs: f64) -> f32 {
        let mut s = Synth::new(fx);
        let (mut l, mut r) = (Vec::new(), Vec::new());
        s.render((secs * RATE) as usize, &mut l, &mut r);
        l.iter().chain(&r).fold(0.0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn output_is_bounded_and_not_silent() {
        for fx in [Fixture::Demo, Fixture::Dense, Fixture::Percussion, Fixture::Tones, Fixture::Sweep, Fixture::Stereo, Fixture::Phase] {
            let p = peak(fx, 20.0);
            assert!(p > 0.05 && p <= 1.0, "{fx:?} peak {p}");
        }
        assert_eq!(peak(Fixture::Silence, 1.0), 0.0);
        assert!(peak(Fixture::Quiet, 20.0) < 0.05);
    }

    fn wav16(rate: u32, ch: u16, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&ch.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2 * ch as u32).to_le_bytes());
        b.extend_from_slice(&(2 * ch).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b
    }

    #[test]
    fn wav_replay_is_resampled_and_loops() {
        // 0.1 s of a 441 Hz stereo tone at 44.1 kHz becomes 4800 frames at 48 kHz
        let tone: Vec<i16> = (0..4410).flat_map(|i| {
            let v = ((i as f32 / 100.0 * std::f32::consts::TAU).sin() * 16000.0) as i16;
            [v, -v]
        }).collect();
        let frames = parse_wav(&wav16(44_100, 2, &tone)).unwrap();
        assert!((frames.len() as i64 - 4800).abs() <= 1, "{} frames", frames.len());
        assert!(frames.iter().all(|f| (f[0] + f[1]).abs() < 1e-3), "channels should mirror each other");
        let peak = frames.iter().fold(0.0f32, |m, f| m.max(f[0].abs()));
        assert!((peak - 0.488).abs() < 0.02, "peak {peak}");
        // mono is duplicated and playback loops
        let mono = parse_wav(&wav16(48_000, 1, &[1000, -1000, 2000])).unwrap();
        assert_eq!(mono.len(), 3);
        assert_eq!(mono[0][0], mono[0][1]);
        let mut s = Synth::with_replay(mono.clone());
        let (mut l, mut r) = (Vec::new(), Vec::new());
        s.render(7, &mut l, &mut r);
        assert_eq!(l[3], mono[0][0]);
        assert!(parse_wav(b"nonsense").is_err());
    }

    #[test]
    fn rendering_is_chunk_independent() {
        let whole = {
            let mut s = Synth::new(Fixture::Demo);
            let (mut l, mut r) = (Vec::new(), Vec::new());
            s.render(100_000, &mut l, &mut r);
            l
        };
        let chunked = {
            let mut s = Synth::new(Fixture::Demo);
            let (mut l, mut r) = (Vec::new(), Vec::new());
            for _ in 0..100 {
                s.render(1000, &mut l, &mut r);
            }
            l
        };
        assert_eq!(whole, chunked);
    }

    #[test]
    fn demo_ends_a_loop_in_silence() {
        // the last bar of the 64 s loop is silent so the gate and decay behaviour get exercised
        let mut s = Synth::new(Fixture::Demo);
        let (mut l, mut r) = (Vec::new(), Vec::new());
        s.render((62.5 * RATE) as usize, &mut l, &mut r);
        l.clear();
        r.clear();
        s.render((1.0 * RATE) as usize, &mut l, &mut r);
        let p = l.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(p < 0.01, "last bar peak {p}");
    }
}
