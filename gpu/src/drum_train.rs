//! Test-only: trains the drum models (`drums::Model`, an ensemble of them) on randomized songs from the soundfont corpus and
//! evaluates it, and with `GEQ_WRITE_MODEL=1` writes `src/drum_model.rs`.
//!
//! `cargo test --release train_drums -- --ignored --nocapture`
//!
//! Environment: `GEQ_SONGS` (training songs, default 48), `GEQ_HIDDEN` (hidden units, default 16),
//! `GEQ_EPOCHS` (default 30), `GEQ_ENSEMBLE_N` (networks, default 5), `GEQ_HOLDOUT` (drum kits kept out of training,
//! comma separated, default "16,40"; "none" trains on all), `GEQ_SYNTH_KITS`, `GEQ_GAPS=1` (stretches of digital silence
//! in the training songs), `GEQ_EVAL_DIR`, `GEQ_WRITE_MODEL` (also installs `src/drum_model.bin`).

use crate::analyzer::Dsp;
use crate::corpus::{self, Rng, Sf2, Song, SR};
use crate::drum_eval::{self, class_of, Det};
use crate::drums::{Model, NF};

const HOP: usize = 480;
/// A label's frames: the hops whose newest sample is within this long after the drum starts.
const POSITIVE_S: f64 = 0.020;
/// Hits quieter than this are not asked for (no target, no penalty around them).
const MIN_VEL: u8 = 45;
const POS_WEIGHT: f32 = 3.0;

pub struct Sample {
    pub x: [f32; NF],
    pub y: [f32; 3],
    pub w: [f32; 3],
}

/// Features of every hop of a song with their targets.
pub fn samples(song: &Song, level_db: f32) -> Vec<Sample> {
    let g = 10f32.powf(level_db / 20.0);
    let mut dsp = Dsp::new();
    let hops = song.audio.len() / HOP;
    let mut out: Vec<Sample> = Vec::with_capacity(hops);
    let (mut l, mut r) = (vec![0.0f32; HOP], vec![0.0f32; HOP]);
    for k in 0..hops {
        for i in 0..HOP {
            l[i] = song.audio[k * HOP + i][0] * g;
            r[i] = song.audio[k * HOP + i][1] * g;
        }
        dsp.push(&l, &r, ((k + 1) * HOP) as f64 / SR);
        let t = (k + 1) as f64 * HOP as f64 / SR;
        out.push(Sample { x: dsp.drums.last.x, y: [0.0; 3], w: if t < 2.0 { [0.0; 3] } else { [1.0; 3] } });
    }
    for lab in &song.labels {
        let c = class_of(lab.kind);
        let first = (lab.t / (HOP as f64 / SR) - 1e-9).ceil().max(1.0) as usize; // frame n has t = n * 10 ms
        let frames = (POSITIVE_S / (HOP as f64 / SR)).round() as usize;
        for n in first..first + frames {
            if n >= 1 && n <= out.len() {
                let s = &mut out[n - 1];
                if lab.vel >= MIN_VEL {
                    s.y[c] = 1.0;
                    s.w[c] = POS_WEIGHT;
                } else {
                    s.w[c] = 0.0;
                }
            }
        }
        if lab.vel < MIN_VEL {
            // and the frames after it, where its tail may look like a hit
            for n in first + frames..first + frames + 2 {
                if n >= 1 && n <= out.len() && out[n - 1].y[c] == 0.0 {
                    out[n - 1].w[c] = 0.0;
                }
            }
        }
    }
    out
}

/// Songs are cached on disk (rendering takes a second or two each); bump the version when the renderer changes.
const CACHE_VERSION: u32 = 5;

fn cache_path(s: &corpus::Spec) -> String {
    let dir = std::env::var("GEQ_EVAL_DIR").unwrap_or_else(|_| "/tmp/geq-drum-eval".to_string());
    let _ = std::fs::create_dir_all(format!("{dir}/cache"));
    format!("{dir}/cache/v{CACHE_VERSION}-{}", s.cache_key())
}

fn load_cached(path: &str, s: &corpus::Spec) -> Option<Song> {
    let raw = std::fs::read(format!("{path}.f32")).ok()?;
    let labels = std::fs::read_to_string(format!("{path}.txt")).ok()?;
    let audio = raw.chunks_exact(8).map(|c| [f32::from_le_bytes(c[..4].try_into().unwrap()), f32::from_le_bytes(c[4..].try_into().unwrap())]).collect();
    let labels = labels
        .lines()
        .map(|l| {
            let mut it = l.split(' ');
            let t: f64 = it.next()?.parse().ok()?;
            let kind = [corpus::Kind::Kick, corpus::Kind::Snare, corpus::Kind::Hat, corpus::Kind::Tom, corpus::Kind::Cymbal][it.next()?.parse::<usize>().ok()?];
            Some(corpus::Label { t, kind, vel: it.next()?.parse().ok()? })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Song { name: s.name, set: s.set, audio, labels })
}

fn save_cached(path: &str, song: &Song) {
    let mut raw = Vec::with_capacity(song.audio.len() * 8);
    for f in &song.audio {
        raw.extend_from_slice(&f[0].to_le_bytes());
        raw.extend_from_slice(&f[1].to_le_bytes());
    }
    let labels: String = song.labels.iter().map(|l| format!("{} {} {}\n", l.t, l.kind as usize, l.vel)).collect();
    let _ = std::fs::write(format!("{path}.f32"), raw);
    let _ = std::fs::write(format!("{path}.txt"), labels);
}

pub fn render_all(sf: &Sf2, specs: &[corpus::Spec]) -> Vec<Song> {
    let mut songs: Vec<Option<Song>> = (0..specs.len()).map(|_| None).collect();
    std::thread::scope(|sc| {
        let per = specs.len().div_ceil(10).max(1);
        for (chunk, slots) in specs.chunks(per).zip(songs.chunks_mut(per)) {
            sc.spawn(move || {
                for (s, slot) in chunk.iter().zip(slots.iter_mut()) {
                    let path = cache_path(s);
                    *slot = Some(load_cached(&path, s).unwrap_or_else(|| {
                        let song = corpus::render(sf, s);
                        save_cached(&path, &song);
                        song
                    }));
                }
            });
        }
    });
    songs.into_iter().map(|s| s.unwrap()).collect()
}

/// The standardized features of a dataset and what they were standardized by, shared by every network of an ensemble.
pub struct Prepared {
    xs: Vec<[f32; NF]>,
    mean: Vec<f32>,
    inv_std: Vec<f32>,
}

pub fn prepare(data: &[Sample]) -> Prepared {
    let mut mean = vec![0.0f32; NF];
    let mut var = vec![0.0f32; NF];
    let live: Vec<&Sample> = data.iter().filter(|s| s.w.iter().any(|w| *w > 0.0)).collect();
    for s in &live {
        for i in 0..NF {
            mean[i] += s.x[i];
        }
    }
    for m in mean.iter_mut() {
        *m /= live.len() as f32;
    }
    for s in &live {
        for i in 0..NF {
            var[i] += (s.x[i] - mean[i]).powi(2);
        }
    }
    let inv_std: Vec<f32> = var.iter().map(|v| 1.0 / (v / live.len() as f32).sqrt().max(1e-3)).collect();
    let xs = data.iter().map(|s| std::array::from_fn(|i| (s.x[i] - mean[i]) * inv_std[i])).collect();
    Prepared { xs, mean, inv_std }
}

/// `n` networks trained at once on the same data, each from its own initial weights and in its own order: independently
/// trained networks make different mistakes, so their average is steadier than any one of them.
pub fn train_ensemble(data: &[Sample], prep: &Prepared, nh: usize, epochs: usize, n: usize) -> Vec<Model> {
    std::thread::scope(|sc| {
        let hs: Vec<_> = (0..n).map(|k| sc.spawn(move || train(data, prep, nh, epochs, 7 + 101 * k as u64, k == 0))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

pub fn train(data: &[Sample], prep: &Prepared, nh: usize, epochs: usize, seed: u64, verbose: bool) -> Model {
    let n = data.len();
    let live_n = data.iter().filter(|s| s.w.iter().any(|w| *w > 0.0)).count();
    let (xs, mean, inv_std) = (&prep.xs, prep.mean.clone(), prep.inv_std.clone());
    let mut rng = Rng::new(seed);
    let mut m = Model {
        nh,
        mean,
        inv_std,
        w1: (0..nh * NF).map(|_| (rng.gauss() / (NF as f64).sqrt()) as f32).collect(),
        b1: vec![0.0; nh],
        w2: (0..3 * nh).map(|_| (rng.gauss() * 0.1) as f32).collect(),
        b2: vec![-3.0; 3],
    };
    // Adam
    let sizes = [m.w1.len(), m.b1.len(), m.w2.len(), m.b2.len()];
    let mut mom: Vec<Vec<f32>> = sizes.iter().map(|s| vec![0.0; *s]).collect();
    let mut vel: Vec<Vec<f32>> = sizes.iter().map(|s| vec![0.0; *s]).collect();
    let (b1c, b2c, eps, l2) = (0.9f32, 0.999f32, 1e-8f32, 1e-5f32);
    let mut order: Vec<usize> = (0..n).collect();
    let batch = 256;
    let mut step = 0u32;
    let mut h = vec![0.0f32; nh];
    for epoch in 0..epochs {
        for i in (1..n).rev() {
            let j = (rng.f() * (i + 1) as f64) as usize;
            order.swap(i, j);
        }
        let lr = 3e-3 * (1.0 - epoch as f32 / epochs as f32 * 0.9);
        let mut loss = 0.0f64;
        for chunk in order.chunks(batch) {
            let mut g: Vec<Vec<f32>> = sizes.iter().map(|s| vec![0.0; *s]).collect();
            for &si in chunk {
                let s = &data[si];
                let x = &xs[si];
                for j in 0..nh {
                    let mut z = m.b1[j];
                    let row = &m.w1[j * NF..(j + 1) * NF];
                    for i in 0..NF {
                        z += row[i] * x[i];
                    }
                    h[j] = z.tanh();
                }
                let p = m.out(&h);
                let mut dh = vec![0.0f32; nh];
                for c in 0..3 {
                    if s.w[c] == 0.0 {
                        continue;
                    }
                    let d = s.w[c] * (p[c] - s.y[c]);
                    loss += (s.w[c] * -(if s.y[c] > 0.5 { p[c].max(1e-7).ln() } else { (1.0 - p[c]).max(1e-7).ln() })) as f64;
                    g[3][c] += d;
                    for j in 0..nh {
                        g[2][c * nh + j] += d * h[j];
                        dh[j] += d * m.w2[c * nh + j];
                    }
                }
                for j in 0..nh {
                    let dz = dh[j] * (1.0 - h[j] * h[j]);
                    g[1][j] += dz;
                    for i in 0..NF {
                        g[0][j * NF + i] += dz * x[i];
                    }
                }
            }
            step += 1;
            let bc1 = 1.0 - b1c.powi(step as i32);
            let bc2 = 1.0 - b2c.powi(step as i32);
            let scale = 1.0 / chunk.len() as f32;
            let params: [&mut Vec<f32>; 4] = [&mut m.w1, &mut m.b1, &mut m.w2, &mut m.b2];
            for (pi, par) in params.into_iter().enumerate() {
                for k in 0..par.len() {
                    let gr = g[pi][k] * scale + if pi % 2 == 0 { l2 * par[k] } else { 0.0 };
                    mom[pi][k] = b1c * mom[pi][k] + (1.0 - b1c) * gr;
                    vel[pi][k] = b2c * vel[pi][k] + (1.0 - b2c) * gr * gr;
                    par[k] -= lr * (mom[pi][k] / bc1) / ((vel[pi][k] / bc2).sqrt() + eps);
                }
            }
        }
        if verbose && (epoch % 5 == 0 || epoch + 1 == epochs) {
            eprintln!("  epoch {epoch:2}: mean loss {:.4}", loss / live_n as f64);
        }
    }
    m
}

/// The detector as the app runs it, with this model and these thresholds, on a song's audio.
pub fn detect(models: &[Model], thr: [f32; 3], audio: &[[f32; 2]]) -> Vec<Det> {
    let mut dsp = Dsp::new();
    dsp.drums.models = models.to_vec();
    dsp.drums.picker.thr = thr;
    let mut out = Vec::new();
    let (mut l, mut r) = (vec![0.0f32; HOP], vec![0.0f32; HOP]);
    for k in 0..audio.len() / HOP {
        for i in 0..HOP {
            l[i] = audio[k * HOP + i][0];
            r[i] = audio[k * HOP + i][1];
        }
        dsp.push(&l, &r, ((k + 1) * HOP) as f64 / SR);
        for h in dsp.take_drums() {
            out.push(Det { t: h.t, pos: h.pos, strength: h.strength, class: h.kind as u8, p: h.p });
        }
    }
    out
}

/// Writes the ensemble in the layout `drums::builtin` reads.
pub fn write_model(models: &[Model], thr: [f32; 3], path: &str) {
    let mut b = (models.len() as u32).to_le_bytes().to_vec();
    for t in thr {
        b.extend_from_slice(&t.to_le_bytes());
    }
    for m in models {
        b.extend_from_slice(&m.to_bytes());
    }
    std::fs::write(path, b).unwrap();
}

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn train_drums() {
        let Some(sf) = Sf2::load() else { return };
        let songs_n: usize = env("GEQ_SONGS", 48);
        let nh: usize = env("GEQ_HIDDEN", 16);
        let epochs: usize = env("GEQ_EPOCHS", 30);
        let holdout = std::env::var("GEQ_HOLDOUT").unwrap_or_else(|_| "16,40".into());
        // GEQ_SYNTH_KITS: off (the default: soundfont kits only), kick (also kits whose kick and toms are computed), full
        // (also kits that are computed throughout). Computed noise-burst hats and snares made false hits on real
        // recordings about twice as common, and computed kicks added false kicks on bass notes: see the plan, Milestone 11.
        let extra: &[u16] = match std::env::var("GEQ_SYNTH_KITS").as_deref() {
            Ok("off") => &[],
            Ok("full") => &[210, 211, 212],
            Ok("kick") => &[200, 201, 202],
            _ => &[],
        };
        let mut all_kits = vec![0u16, 8, 16, 24, 25, 32, 40, 48];
        all_kits.extend_from_slice(extra);
        let held: Vec<u16> = if holdout == "none" { vec![] } else { holdout.split(',').filter_map(|k| k.parse().ok()).collect() };
        let kits: Vec<u16> = all_kits.iter().copied().filter(|k| !held.contains(k)).collect();
        eprintln!("training on {songs_n} random songs with kits {kits:?}, held out {held:?}; {nh} hidden units");
        let t0 = std::time::Instant::now();
        let specs: Vec<_> = (0..songs_n as u64).map(|i| corpus::random_spec(1000 + i, &kits)).collect();
        let mut data: Vec<Sample> = Vec::new();
        // every song at its own volume, from full scale down to -30 dB: the detector must not care how loud it is played;
        // rendered in batches so the audio of 200 songs is never held at once
        for (b, batch) in specs.chunks(20).enumerate() {
            for (j, s) in render_all(&sf, batch).iter().enumerate() {
                let i = (b * 20 + j) as u64;
                let level = -30.0 * (i.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f32 / (1u64 << 24) as f32;
                data.extend(samples(s, level));
            }
        }
        // GEQ_REAL=1: also train on real drums, the MDB-Drums songs that are not held out (see real_corpus.rs): each as released,
        // GEQ_REAL_VARIANTS (default 6) remixes of its drum and band stems, and the band alone as music without drums
        if std::env::var_os("GEQ_REAL").is_some() {
            let variants: usize = env("GEQ_REAL_VARIANTS", 6);
            let (mut used, mut held, mut n) = (0, Vec::new(), 0u64);
            for t in crate::real_corpus::tracks() {
                if crate::real_corpus::held_out(&t) {
                    held.push(t.name.clone());
                    continue;
                }
                used += 1;
                for song in t.training_songs(variants) {
                    let level = -30.0 * (n.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f32 / (1u64 << 24) as f32;
                    data.extend(samples(&song, level));
                    n += 1;
                }
            }
            assert!(used > 0, "GEQ_REAL set but no MDB-Drums songs found (see tools/import-mdb.py)");
            eprintln!("real drums: {used} songs x {} versions; held out: {}", variants + 2, held.join(" "));
        }
        let pos: [usize; 3] = std::array::from_fn(|c| data.iter().filter(|s| s.y[c] > 0.5).count());
        eprintln!("{} frames ({:.0} s of audio), positives low {} snare {} high {}; rendered in {:.0}s", data.len(), data.len() as f64 / 100.0, pos[0], pos[1], pos[2], t0.elapsed().as_secs_f32());
        let ensemble_n: usize = env("GEQ_ENSEMBLE_N", 5);
        let prep = prepare(&data);
        let models = train_ensemble(&data, &prep, nh, epochs, ensemble_n);
        drop(prep);
        eprintln!("trained {ensemble_n} networks in {:.0}s", t0.elapsed().as_secs_f32());

        let thr: f32 = env("GEQ_THRESH", 0.7);
        let fixed = render_all(&sf, &corpus::specs());
        let show: f32 = env("GEQ_SHOW", 0.9);
        for th in [0.8f32, 0.9, 0.95] {
            drum_eval::table_on(&format!("ensemble of {ensemble_n}, confidence >= {th}"), &fixed, &|a| detect(&models, [thr; 3], a).into_iter().filter(|d| d.p >= th).collect(), (th - show).abs() < 1e-3);
        }
        let dir = std::env::var("GEQ_EVAL_DIR").unwrap_or_else(|_| "/tmp/geq-drum-eval".to_string());
        let out = std::env::var("GEQ_MODEL_OUT").unwrap_or_else(|_| format!("{dir}/drum_model.bin"));
        write_model(&models, [thr; 3], &out);
        eprintln!("wrote {out} with picker threshold {thr}");
        if std::env::var_os("GEQ_WRITE_MODEL").is_some() {
            write_model(&models, [thr; 3], concat!(env!("CARGO_MANIFEST_DIR"), "/src/drum_model.bin"));
            eprintln!("wrote src/drum_model.bin");
        }
    }
}
