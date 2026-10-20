//! Test-only: scores drum detectors against the labelled corpus (`corpus.rs`) and draws spectrograms with
//! the labels and detections on them, for looking at.
//!
//! A detection is one thing a scene would react to: a time, a place on the emitter line (0 bass .. 1 treble)
//! and a strength. It is matched to a label when it is reported between 20 ms before and 150 ms after the
//! drum's sample starts. The numbers a visual cares about are in the report:
//!
//! * recall on *salient* hits (velocity 64 or more, and not part of a roll: no hit of its own class in the 90 ms before)
//! * precision (every detection that matches no label, or matches a label already taken, counts against it)
//! * latency from the sample's start to the report
//! * false detections per minute on the songs that have no drums at all

use crate::analyzer::{Dsp, CURVE_POINTS};
use crate::corpus::{self, Kind, Label, Set, Sf2, Song, SR};
use crate::modulation::{RiseHits, RiseNorm};

#[derive(Clone, Copy, Debug)]
pub struct Det {
    /// When it was reported, in seconds of audio.
    pub t: f64,
    /// 0 bass .. 1 treble.
    pub pos: f32,
    pub strength: f32,
    /// 0 low, 1 snare, 2 high; 255 when the detector does not say.
    pub class: u8,
    /// The detector's confidence (1 when it does not give one).
    pub p: f32,
}

pub const CHUNK: usize = 400; // 120 reports a second, like a frame at 120 Hz
const FROM: f64 = 4.0; // detectors settle before anything is scored
const EARLY: f64 = 0.02;
const LATE: f64 = 0.10;

/// The pipeline the scenes use today: the analyzer's rise, normalized, then `RiseHits`, polled at 120 Hz.
pub fn baseline(audio: &[[f32; 2]], sens: f32) -> Vec<Det> {
    let mut dsp = Dsp::new();
    let (mut norm, mut hits) = (RiseNorm::new(), RiseHits::new());
    let mut rise = [0.0f32; CURVE_POINTS];
    let mut out = Vec::new();
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for (k, chunk) in audio.chunks(CHUNK).enumerate() {
        l.clear();
        r.clear();
        for f in chunk {
            l.push(f[0]);
            r.push(f[1]);
        }
        let now = ((k + 1) * CHUNK) as f64 / SR;
        if let (Some(lv), _) = dsp.push(&l, &r, now) {
            rise = lv.rise;
        }
        let n = norm.update(&rise, 1.0 / 120.0);
        for h in hits.update(&n, sens, now, true) {
            out.push(Det { t: now, pos: h.pos, strength: h.strength, class: 255, p: 1.0 });
        }
    }
    out
}

/// The built-in detector's hits at or above `min_p`, as the analyzer thread reports them.
pub fn builtin(audio: &[[f32; 2]], min_p: f32) -> Vec<Det> {
    let mut dsp = Dsp::new();
    let mut out = Vec::new();
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    for k in 0..audio.len() / 480 {
        for i in 0..480 {
            l[i] = audio[k * 480 + i][0];
            r[i] = audio[k * 480 + i][1];
        }
        dsp.push(&l, &r, ((k + 1) * 480) as f64 / SR);
        out.extend(dsp.take_drums().into_iter().filter(|h| h.p >= min_p).map(|h| Det { t: h.t, pos: h.pos, strength: h.strength, class: h.kind as u8, p: h.p }));
    }
    out
}

#[derive(Default, Clone, Debug)]
pub struct Report {
    pub labels: usize,
    pub salient: usize,
    pub found: usize,
    pub dets: usize,
    pub tp: usize,
    pub dup: usize,
    pub fp: usize,
    pub lat_ms: Vec<f64>,
    /// salient labels and found, by kind: kick, snare, hat, tom, cymbal
    pub by_kind: [(usize, usize); 5],
    /// matched detections: label class (low, snare, high) by detected class
    pub conf: [[usize; 3]; 3],
    /// per detected class: detections and those that matched a label (class-aware scoring)
    pub cls: [(usize, usize); 3],
    pub secs: f64,
}

impl Report {
    pub fn precision(&self) -> f64 {
        self.tp as f64 / self.dets.max(1) as f64
    }
    pub fn recall(&self) -> f64 {
        self.found as f64 / self.salient.max(1) as f64
    }
    pub fn f1(&self) -> f64 {
        let (p, r) = (self.precision(), self.recall());
        if p + r > 0.0 { 2.0 * p * r / (p + r) } else { 0.0 }
    }
    pub fn fp_per_min(&self) -> f64 {
        (self.fp + self.dup) as f64 / (self.secs / 60.0).max(1e-9)
    }
    pub fn lat(&self, p: f64) -> f64 {
        if self.lat_ms.is_empty() {
            return f64::NAN;
        }
        let mut v = self.lat_ms.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f64 * p) as usize]
    }
    pub fn add(&mut self, o: &Report) {
        self.labels += o.labels;
        self.salient += o.salient;
        self.found += o.found;
        self.dets += o.dets;
        self.tp += o.tp;
        self.dup += o.dup;
        self.fp += o.fp;
        self.lat_ms.extend(&o.lat_ms);
        for k in 0..5 {
            self.by_kind[k].0 += o.by_kind[k].0;
            self.by_kind[k].1 += o.by_kind[k].1;
        }
        for a in 0..3 {
            for b in 0..3 {
                self.conf[a][b] += o.conf[a][b];
            }
            self.cls[a].0 += o.cls[a].0;
            self.cls[a].1 += o.cls[a].1;
        }
        self.secs += o.secs;
    }
    pub fn line(&self) -> String {
        format!(
            "recall {:5.1}% ({:3}/{:3})  precision {:5.1}% ({:3}/{:3})  F1 {:4.2}  dup {:2} fp {:3}  latency ms {:3.0}/{:3.0}/{:3.0}",
            100.0 * self.recall(), self.found, self.salient, 100.0 * self.precision(), self.tp, self.dets, self.f1(), self.dup, self.fp,
            self.lat(0.1), self.lat(0.5), self.lat(0.9)
        )
    }
    pub fn per_class(&self) -> String {
        let p = |c: usize| 100.0 * self.cls[c].1 as f64 / self.cls[c].0.max(1) as f64;
        format!("precision by output: low {:.0}% ({}) snare {:.0}% ({}) high {:.0}% ({})", p(0), self.cls[0].0, p(1), self.cls[1].0, p(2), self.cls[2].0)
    }
    pub fn confusion(&self) -> String {
        let row = |n: &str, r: &[usize; 3]| format!("{n}: low {} snare {} high {}", r[0], r[1], r[2]);
        format!("matched as  {} | {} | {}", row("low-label", &self.conf[0]), row("snare-label", &self.conf[1]), row("high-label", &self.conf[2]))
    }
    pub fn kinds(&self) -> String {
        ["kick", "snare", "hat", "tom", "cymbal"]
            .iter()
            .zip(self.by_kind)
            .map(|(n, (a, b))| format!("{n} {}/{}", b, a))
            .collect::<Vec<_>>()
            .join("  ")
    }
}

pub fn class_of(k: Kind) -> usize {
    match k {
        Kind::Kick | Kind::Tom => 0,
        Kind::Snare => 1,
        Kind::Hat | Kind::Cymbal => 2,
    }
}

/// Scores detections against labels. With `aware`, a detection that names a class (low, snare, high) can only
/// match a label of that class: a snare that also lights the kick output is then a false detection, as the
/// scenes would show it.
pub fn score(labels: &[Label], dets: &[Det], secs: f64, aware: bool) -> Report {
    let mut rep = score_range(labels, dets, FROM, f64::INFINITY, aware);
    rep.secs = secs - FROM;
    rep
}

/// The same for the stretch `from..to` of a recording: labels that start in it, and detections reported in it (the last
/// `LATE` seconds of it can only ask for what had time to be reported).
pub fn score_range(labels: &[Label], dets: &[Det], from: f64, to: f64, aware: bool) -> Report {
    let upper = if to.is_finite() { to - LATE } else { f64::INFINITY };
    let labels: Vec<(usize, &Label)> = labels.iter().enumerate().filter(|(_, l)| l.t >= from && l.t < upper).collect();
    // a hit is asked for when it is not faint (velocity 64 or more) and not part of a roll: no hit of its own class
    // (kick/tom, snare, hat/cymbal) in the 90 ms before it. A kick that lands with a hat is asked for.
    let salient: Vec<bool> = labels
        .iter()
        .enumerate()
        .map(|(i, (_, l))| l.vel >= 64 && !labels[..i].iter().any(|(_, p)| class_of(p.kind) == class_of(l.kind) && l.t - p.t < 0.09 && l.t - p.t >= 0.0))
        .collect();
    let mut taken = vec![false; labels.len()];
    let mut rep = Report { labels: labels.len(), secs: if to.is_finite() { to - from } else { 0.0 }, ..Default::default() };
    let mut dets: Vec<&Det> = dets.iter().filter(|d| d.t >= from && d.t <= to).collect();
    dets.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    rep.dets = dets.len();
    for d in dets {
        if d.class < 3 {
            rep.cls[d.class as usize].0 += 1;
        }
        let near = |i: usize| {
            let dt = d.t - labels[i].1.t;
            dt >= -EARLY && dt <= LATE && (!aware || d.class > 2 || class_of(labels[i].1.kind) == d.class as usize)
        };
        let best = (0..labels.len()).filter(|i| !taken[*i] && near(*i)).min_by(|a, b| {
            let (x, y) = ((d.t - 0.03 - labels[*a].1.t).abs(), (d.t - 0.03 - labels[*b].1.t).abs());
            x.partial_cmp(&y).unwrap()
        });
        match best {
            Some(i) => {
                taken[i] = true;
                rep.tp += 1;
                if d.class < 3 {
                    rep.cls[d.class as usize].1 += 1;
                    rep.conf[class_of(labels[i].1.kind)][d.class as usize] += 1;
                }
                rep.lat_ms.push((d.t - labels[i].1.t) * 1000.0);
            }
            None if (0..labels.len()).any(near) => rep.dup += 1,
            None => rep.fp += 1,
        }
    }
    for (i, (_, l)) in labels.iter().enumerate() {
        if salient[i] {
            rep.salient += 1;
            let k = match l.kind {
                Kind::Kick => 0,
                Kind::Snare => 1,
                Kind::Hat => 2,
                Kind::Tom => 3,
                Kind::Cymbal => 4,
            };
            rep.by_kind[k].0 += 1;
            if taken[i] {
                rep.found += 1;
                rep.by_kind[k].1 += 1;
            }
        }
    }
    rep
}

// ------------------------------------------------------------------------------------------ labelled real songs

/// A real recording someone labelled with `tools/labeler.html`: the drum hits, the stretch that was labelled and the
/// stretches known to have no drums in them.
pub struct LabelFile {
    pub audio: String,
    /// "dev" (tune on it) or "test" (look at it only for the final report).
    pub split: String,
    pub excerpt: Option<(f64, f64)>,
    pub labels: Vec<Label>,
    pub no_drums: Vec<(f64, f64)>,
}

fn kind_of_name(n: &str) -> Option<Kind> {
    Some(match n {
        "kick" => Kind::Kick,
        "tom" => Kind::Tom,
        "snare" => Kind::Snare,
        "hat" => Kind::Hat,
        "cymbal" => Kind::Cymbal,
        _ => return None,
    })
}

/// Reads the labeller's JSON. A ghost note (soft) gets velocity 40, so it is not asked for but a detection of it is not
/// wrong; everything else 100.
pub fn parse_label_file(json: &str) -> Result<LabelFile, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let audio = v["audio"]["name"].as_str().ok_or("no audio.name")?.to_string();
    let pair = |x: &serde_json::Value| Some((x.get(0)?.as_f64()?, x.get(1)?.as_f64()?));
    let excerpt = pair(&v["excerpt"]);
    let mut labels = Vec::new();
    for l in v["labels"].as_array().ok_or("no labels")? {
        let kind = l["kind"].as_str().and_then(kind_of_name).ok_or_else(|| format!("bad kind in {l}"))?;
        let t = l["t"].as_f64().ok_or_else(|| format!("bad time in {l}"))?;
        labels.push(Label { t, kind, vel: if l["ghost"].as_bool().unwrap_or(false) { 40 } else { 100 } });
    }
    labels.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    let no_drums = v["noDrums"].as_array().map(|a| a.iter().filter_map(pair).collect()).unwrap_or_default();
    Ok(LabelFile { audio, split: v["split"].as_str().unwrap_or("dev").to_string(), excerpt, labels, no_drums })
}

/// Where the recordings live (they are not in the repository), and the labels.
fn real_dir() -> String {
    std::env::var("GEQ_REAL_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/labels/audio").to_string())
}

fn label_dir() -> String {
    std::env::var("GEQ_LABEL_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/labels").to_string())
}

/// Every hit the detector would report at confidence `floor` or more, as the labeller's proposals: where it thinks the
/// drum started (the report is about 21 ms late), what it thinks it is and how sure it is.
pub fn proposals(audio: &[[f32; 2]], floor: f32) -> Vec<(f64, u8, f32)> {
    let mut dsp = Dsp::new();
    dsp.drums.picker.thr = [floor; 3];
    let mut out = Vec::new();
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    for k in 0..audio.len() / 480 {
        for i in 0..480 {
            l[i] = audio[k * 480 + i][0];
            r[i] = audio[k * 480 + i][1];
        }
        dsp.push(&l, &r, ((k + 1) * 480) as f64 / SR);
        out.extend(dsp.take_drums().into_iter().filter(|h| h.p >= floor).map(|h| (h.t - 0.021, h.kind as u8, h.p)));
    }
    out
}

pub fn songs(set: Option<Set>) -> Vec<Song> {
    let Some(sf) = Sf2::load() else { return Vec::new() };
    corpus::specs().iter().filter(|s| set.map_or(true, |w| s.set == w)).map(|s| corpus::render(&sf, s)).collect()
}

/// Score `detect` on every song of a set and print a table. Returns the overall report of the songs with drums.
pub fn table(name: &str, set: Option<Set>, detect: &(dyn Fn(&[[f32; 2]]) -> Vec<Det> + Sync)) -> Report {
    table_on(name, &songs(set), detect, true)
}

pub fn table_on(name: &str, songs: &[Song], detect: &(dyn Fn(&[[f32; 2]]) -> Vec<Det> + Sync), verbose: bool) -> Report {
    let dets: Vec<Vec<Det>> = std::thread::scope(|sc| {
        let hs: Vec<_> = songs.iter().map(|s| sc.spawn(move || detect(&s.audio))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut all = Report::default();
    let mut all_aware = Report::default();
    let (mut none_secs, mut none_fp) = (0.0, 0usize);
    eprintln!("== {name}");
    for (song, dets) in songs.iter().zip(&dets) {
        let secs = song.audio.len() as f64 / SR;
        let rep = score(&song.labels, dets, secs, false);
        if song.labels.is_empty() {
            let by: [usize; 3] = std::array::from_fn(|c| dets.iter().filter(|d| d.t >= FROM && d.class as usize == c).count());
            if verbose {
            eprintln!("{:12} {:?}  no drums: {} detections in {:.0} s ({:.0} a minute; low {} snare {} high {})", song.name, song.set, rep.dets, secs - FROM, rep.dets as f64 / ((secs - FROM) / 60.0), by[0], by[1], by[2]);
            }
            none_secs += secs - FROM;
            none_fp += rep.dets;
        } else {
            let aw = score(&song.labels, dets, secs, true);
            if verbose {
                eprintln!("{:12} {:?}  {}", song.name, song.set, rep.line());
                eprintln!("{:20}   class-aware: P {:4.1}% R {:4.1}% F1 {:4.2} dup {} fp {} | {}", "", 100.0 * aw.precision(), 100.0 * aw.recall(), aw.f1(), aw.dup, aw.fp, rep.kinds());
            }
            all.add(&rep);
            all_aware.add(&aw);
        }
    }
    eprintln!("{:12} ALL    {}", "", all.line());
    eprintln!("{:20}   class-aware: P {:4.1}% R {:4.1}% F1 {:4.2} dup {} fp {} | {}", "", 100.0 * all_aware.precision(), 100.0 * all_aware.recall(), all_aware.f1(), all_aware.dup, all_aware.fp, all.kinds());
    if all_aware.cls.iter().map(|c| c.0).sum::<usize>() > 0 {
        eprintln!("{:20}   {}", "", all_aware.per_class());
    }
    if none_secs > 0.0 {
        eprintln!("{:12} no-drum songs: {:.1} false detections a minute", "", none_fp as f64 / (none_secs / 60.0));
    }
    all_aware
}

/// A log-frequency spectrogram of `from..to` seconds with the labels (top: kick red, snare yellow, hat cyan,
/// tom magenta, cymbal white) and detections (bottom, green) drawn on it.
pub fn spectrogram(path: &str, audio: &[[f32; 2]], labels: &[Label], dets: &[Det], from: f64, to: f64) {
    use rustfft::{num_complex::Complex, FftPlanner};
    let n = 2048;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(n);
    let hann: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
    let hop = 480usize;
    let cols = ((to - from) * 100.0) as usize;
    let rows = 192usize;
    let scale = 3usize;
    let (w, h) = (cols * 3, rows * 2 + 2 * 12 * scale);
    let mut img = image::RgbImage::new(w as u32, h as u32);
    let mut buf = vec![Complex::new(0.0f32, 0.0); n];
    for c in 0..cols {
        let s0 = (from * SR) as usize + c * hop;
        if s0 + n > audio.len() {
            break;
        }
        for i in 0..n {
            buf[i] = Complex::new((audio[s0 + i][0] + audio[s0 + i][1]) * 0.5 * hann[i], 0.0);
        }
        fft.process(&mut buf);
        for r in 0..rows {
            let f = 30.0 * (16000.0f32 / 30.0).powf(r as f32 / (rows - 1) as f32);
            let k = (f / (SR as f32 / n as f32)) as usize;
            let (lo, hi) = (k.saturating_sub(1).max(1), (k + 1).min(n / 2));
            let m = (lo..=hi).map(|k| buf[k].norm()).fold(0.0f32, f32::max);
            // 0 dB is a full-scale sine's bin; the picture spans 75 dB below that
            let db = 20.0 * (m / (n as f32 * 0.25) + 1e-9).log10();
            let v = (((db + 75.0) / 75.0).clamp(0.0, 1.0).powf(1.6) * 255.0) as u8;
            let px = image::Rgb([v, (v as f32 * 0.8) as u8, (v as f32 * 0.5 + 20.0) as u8]);
            for dx in 0..3 {
                for dy in 0..2 {
                    img.put_pixel((c * 3 + dx) as u32, ((rows - 1 - r) * 2 + dy + 12 * scale) as u32, px);
                }
            }
        }
    }
    let x_of = |t: f64| ((t - from) * 100.0 * 3.0) as i64;
    for l in labels.iter().filter(|l| l.t >= from && l.t < to) {
        let c = match l.kind {
            Kind::Kick => [255, 60, 60],
            Kind::Snare => [255, 220, 40],
            Kind::Hat => [60, 220, 255],
            Kind::Tom => [255, 80, 255],
            Kind::Cymbal => [255, 255, 255],
        };
        let hgt = 6 + (l.vel as usize / 12);
        for y in 0..hgt.min(12 * scale) {
            if x_of(l.t) >= 0 && (x_of(l.t) as usize) < w {
                img.put_pixel(x_of(l.t) as u32, y as u32, image::Rgb(c));
            }
        }
    }
    for d in dets.iter().filter(|d| d.t >= from && d.t < to) {
        for y in 0..12 * scale {
            let x = x_of(d.t);
            if x >= 0 && (x as usize) < w {
                let c = match d.class {
                    0 => [255, 60, 60],
                    1 => [255, 220, 40],
                    2 => [60, 220, 255],
                    _ => [60, 255, 90],
                };
                img.put_pixel(x as u32, (h - 1 - y) as u32, image::Rgb(c));
            }
        }
    }
    img.save(path).unwrap();
}

/// An onset image: how much each frequency rose since two frames ago (half-wave rectified, log magnitude), 30 Hz
/// at the bottom to 16 kHz at the top, with the detections ticked at the bottom. Drum hits are bars across many
/// rows; a sustained sound is dark, and a note's attack is a short bar at its harmonics.
pub fn onset_image(path: &str, audio: &[[f32; 2]], dets: &[Det], from: f64, to: f64) {
    use rustfft::{num_complex::Complex, FftPlanner};
    let n = 1024;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(n);
    let hann: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
    let cols = ((to - from) * 100.0) as usize;
    let rows = 160usize;
    let (w, h) = (cols * 3, rows * 2 + 30);
    let mut img = image::RgbImage::new(w as u32, h as u32);
    let mut buf = vec![Complex::new(0.0f32, 0.0); n];
    let mut hist: Vec<Vec<f32>> = Vec::new();
    for c in 0..cols + 2 {
        let s0 = ((from * SR) as usize + c * 480).saturating_sub(2 * 480);
        if s0 + n > audio.len() {
            break;
        }
        for i in 0..n {
            buf[i] = Complex::new((audio[s0 + i][0] + audio[s0 + i][1]) * 0.5 * hann[i], 0.0);
        }
        fft.process(&mut buf);
        let lv: Vec<f32> = (0..rows)
            .map(|r| {
                let f = 30.0 * (16000.0f32 / 30.0).powf(r as f32 / (rows - 1) as f32);
                let k = ((f / (SR as f32 / n as f32)) as usize).clamp(1, n / 2 - 1);
                (1.0 + 2000.0 * buf[k - 1..=k + 1].iter().map(|x| x.norm()).fold(0.0f32, f32::max) / 256.0).ln()
            })
            .collect();
        hist.push(lv);
        if hist.len() >= 3 {
            let (now, then) = (&hist[hist.len() - 1], &hist[hist.len() - 3]);
            let col = hist.len() - 3;
            if col < cols {
                for r in 0..rows {
                    let v = (((now[r] - then[r]).max(0.0) / 2.5).clamp(0.0, 1.0).powf(0.7) * 255.0) as u8;
                    for dx in 0..3 {
                        for dy in 0..2 {
                            img.put_pixel((col * 3 + dx) as u32, ((rows - 1 - r) * 2 + dy) as u32, image::Rgb([v, v, v]));
                        }
                    }
                }
            }
        }
    }
    for d in dets.iter().filter(|d| d.t >= from && d.t < to) {
        // the onset itself is about 20 ms before it was reported
        let x = ((d.t - 0.02 - from) * 300.0) as i64;
        let c = match d.class {
            0 => [255, 60, 60],
            1 => [255, 220, 40],
            2 => [60, 220, 255],
            _ => [60, 255, 90],
        };
        for y in 0..28 {
            if x >= 0 && (x as usize) < w {
                img.put_pixel(x as u32, (h - 1 - y) as u32, image::Rgb(c));
            }
        }
    }
    img.save(path).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out_dir() -> String {
        let d = std::env::var("GEQ_EVAL_DIR").unwrap_or_else(|_| "/tmp/geq-drum-eval".to_string());
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Writes every corpus song as a WAV and its label list, for listening or inspecting.
    #[test]
    #[ignore]
    fn dump_corpus() {
        let dir = out_dir();
        for s in songs(None) {
            corpus::write_wav(&format!("{dir}/{}.wav", s.name), &s.audio);
            // the drum times in the labeller's format, for testing it and the scorer against something exact
            let name = |k: Kind| ["kick", "snare", "hat", "tom", "cymbal"][k as usize];
            let labels: Vec<serde_json::Value> = s.labels.iter().map(|l| serde_json::json!({"t": l.t, "kind": name(l.kind), "ghost": l.vel < 64})).collect();
            let doc = serde_json::json!({"version": 1, "audio": {"name": format!("{}.wav", s.name)}, "split": "test", "excerpt": null, "labels": labels, "noDrums": []});
            std::fs::write(format!("{dir}/{}.truth.json", s.name), serde_json::to_string(&doc).unwrap()).unwrap();
            let peak = s.audio.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
            let mut kinds = [0usize; 5];
            for l in &s.labels {
                kinds[l.kind as usize] += 1;
            }
            eprintln!("{:12} {:?} {:.0}s peak {peak:.2} labels {} (kick {} snare {} hat {} tom {} cymbal {})", s.name, s.set, s.audio.len() as f64 / SR, s.labels.len(), kinds[0], kinds[1], kinds[2], kinds[3], kinds[4]);
        }
    }

    /// The built-in detector (the model in `drum_model.rs`) on a recording captured as raw little-endian f32 stereo at
    /// 48 kHz (`GEQ_REAL_RAW`): how often it fires, how regular the hits are, and spectrograms with the hits drawn on.
    /// `parec --device=@DEFAULT_MONITOR@ --format=float32le --rate=48000 --channels=2 > file.raw`
    #[test]
    #[ignore]
    fn real_music() {
        let Ok(path) = std::env::var("GEQ_REAL_RAW") else { return };
        let raw = std::fs::read(&path).unwrap();
        let audio: Vec<[f32; 2]> = raw.chunks_exact(8).map(|c| [f32::from_le_bytes(c[..4].try_into().unwrap()), f32::from_le_bytes(c[4..].try_into().unwrap())]).collect();
        let secs = audio.len() as f64 / SR;
        let mut dsp = Dsp::new();
        let mut hits = Vec::new();
        let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
        for k in 0..audio.len() / 480 {
            for i in 0..480 {
                l[i] = audio[k * 480 + i][0];
                r[i] = audio[k * 480 + i][1];
            }
            dsp.push(&l, &r, ((k + 1) * 480) as f64 / SR);
            hits.extend(dsp.take_drums());
        }
        let dets: Vec<Det> = hits.iter().map(|h| Det { t: h.t, pos: h.pos, strength: h.strength, class: h.kind as u8, p: h.p }).collect();
        let name = ["low", "snare", "high"];
        eprintln!("{path}: {secs:.0} s");
        for th in [0.8f32, 0.85, 0.9, 0.95] {
            let n = |c: usize| hits.iter().filter(|h| h.kind as usize == c && h.p >= th).count();
            eprintln!("  confidence >= {th}: low {} snare {} high {}  ({:.1} hits a minute)", n(0), n(1), n(2), (n(0) + n(1) + n(2)) as f64 / (secs / 60.0));
        }
        for c in 0..3 {
            let ts: Vec<f64> = dets.iter().filter(|d| d.class as usize == c).map(|d| d.t).collect();
            // inter-onset intervals in 20 ms bins: a drum part repeats a few intervals; chance spreads them out
            let mut hist = std::collections::BTreeMap::new();
            for w in ts.windows(2) {
                *hist.entry(((w[1] - w[0]) / 0.02).round() as i64).or_insert(0usize) += 1;
            }
            let mut top: Vec<_> = hist.iter().collect();
            top.sort_by(|a, b| b.1.cmp(a.1));
            let covered: usize = top.iter().take(3).map(|(_, n)| **n).sum();
            eprintln!(
                "{:6} {:4} hits ({:.1}/s); commonest intervals ms {:?}; top three cover {:.0}% of intervals",
                name[c], ts.len(), ts.len() as f64 / secs,
                top.iter().take(3).map(|(b, n)| format!("{}ms x{}", **b * 20, n)).collect::<Vec<_>>(),
                100.0 * covered as f64 / ts.len().saturating_sub(1).max(1) as f64
            );
        }
        // drums with this recording laid over them (it is speech: held out of training), at the same level
        if let (Some(sf), true) = (Sf2::load(), std::env::var_os("GEQ_REAL_IS_SPEECH").is_some()) {
            let rms = |v: &[[f32; 2]]| (v.iter().map(|f| (f[0] * f[0] + f[1] * f[1]) as f64).sum::<f64>() / (2.0 * v.len().max(1) as f64)).sqrt() as f32;
            for spec in corpus::specs().iter().filter(|s| ["pop_104", "rock_130", "house_124"].contains(&s.name)) {
                let song = corpus::render(&sf, spec);
                let voice = &audio[(5.0 * SR) as usize..((5.0 * SR) as usize + song.audio.len()).min(audio.len())];
                let g = rms(&song.audio) / rms(voice).max(1e-6);
                let mix: Vec<[f32; 2]> = song.audio.iter().zip(voice).map(|(a, v)| [(a[0] + v[0] * g) * 0.6, (a[1] + v[1] * g) * 0.6]).collect();
                let secs_m = mix.len() as f64 / SR;
                let d = |a: &[[f32; 2]]| {
                    let mut dsp = Dsp::new();
                    let mut out = Vec::new();
                    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
                    for k in 0..a.len() / 480 {
                        for i in 0..480 {
                            l[i] = a[k * 480 + i][0];
                            r[i] = a[k * 480 + i][1];
                        }
                        dsp.push(&l, &r, ((k + 1) * 480) as f64 / SR);
                        out.extend(dsp.take_drums().into_iter().filter(|h| h.p >= 0.9).map(|h| Det { t: h.t, pos: h.pos, strength: h.strength, class: h.kind as u8, p: h.p }));
                    }
                    out
                };
                let (clean, mixed) = (score(&song.labels, &d(&song.audio), secs_m, true), score(&song.labels, &d(&mix), secs_m, true));
                let old = score(&song.labels, &baseline(&mix, 1.0), secs_m, false);
                eprintln!("{:10} clean:      P {:4.1}% R {:4.1}%  F1 {:.2}", spec.name, 100.0 * clean.precision(), 100.0 * clean.recall(), clean.f1());
                eprintln!("{:10} + speech:   P {:4.1}% R {:4.1}%  F1 {:.2}   (old detector on the mix: P {:4.1}% R {:4.1}%)", spec.name, 100.0 * mixed.precision(), 100.0 * mixed.recall(), mixed.f1(), 100.0 * old.precision(), 100.0 * old.recall());
            }
        }
        // the same recording through the detector the scenes used before, for comparison
        let old = baseline(&audio, 1.0);
        eprintln!("old RiseHits at sensitivity 1: {} hits ({:.1}/s)", old.len(), old.len() as f64 / secs);
        // is there a rhythm in the hits? autocorrelation of the hit times (10 ms bins) at lags 120 ms .. 1.5 s
        for c in 0..3 {
            let mut sig = vec![0.0f32; (secs * 100.0) as usize + 2];
            for d in dets.iter().filter(|d| d.class as usize == c) {
                sig[(d.t * 100.0) as usize] += 1.0;
            }
            let n = sig.len();
            let ac: Vec<f32> = (12..150).map(|lag| (0..n - lag).map(|i| sig[i] * sig[i + lag]).sum::<f32>()).collect();
            let mean = ac.iter().sum::<f32>() / ac.len() as f32;
            let (best, v) = ac.iter().enumerate().fold((0, 0.0f32), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
            eprintln!("{:6} hit autocorrelation: strongest at {} ms, {:.1}x the average lag", name[c], (best + 12) * 10, v / mean.max(1e-9));
        }
        let dir = out_dir();
        let mut t = 6.0;
        while t + 8.0 <= secs && t < 70.0 {
            spectrogram(&format!("{dir}/real_{:03.0}.png", t), &audio, &[], &dets, t, t + 8.0);
            onset_image(&format!("{dir}/real_onset_{:03.0}.png", t), &audio, &dets, t, t + 8.0);
            t += 16.0;
        }
    }

    /// Distribution of the high bands' flux and breadth on a real recording against corpus songs, to see what a model
    /// trained on the corpus is up against: `GEQ_REAL_RAW=... cargo test --release compare_hf -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn compare_hf() {
        let stats = |name: &str, audio: &[[f32; 2]]| {
            let mut dsp = Dsp::new();
            let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
            let (mut flux, mut broad, mut quiet) = (Vec::new(), Vec::new(), Vec::new());
            for k in 0..audio.len() / 480 {
                for i in 0..480 {
                    l[i] = audio[k * 480 + i][0];
                    r[i] = audio[k * 480 + i][1];
                }
                dsp.push(&l, &r, ((k + 1) * 480) as f64 / SR);
                if k > 400 {
                    flux.push(dsp.drums.last.flux[2]);
                    broad.push(dsp.drums.last.x[crate::drums::NB * crate::drums::PER_BAND + 4]);
                    quiet.push(dsp.drums.last.x[crate::drums::PER_BAND * 14 + 5]); // share of bins risen, band 14 (13-17 kHz)
                }
            }
            let pct = |v: &mut Vec<f32>, p: f64| { v.sort_by(|a, b| a.partial_cmp(b).unwrap()); v[((v.len() - 1) as f64 * p) as usize] };
            eprintln!(
                "{name:12} hi-flux p50 {:.2} p90 {:.2} p99 {:.2} p99.9 {:.2} | breadth p50 {:.2} p90 {:.2} p99 {:.2} | band14 broad p50 {:.2} p90 {:.2} p99 {:.2}",
                pct(&mut flux, 0.5), pct(&mut flux, 0.9), pct(&mut flux, 0.99), pct(&mut flux, 0.999),
                pct(&mut broad, 0.5), pct(&mut broad, 0.9), pct(&mut broad, 0.99),
                pct(&mut quiet, 0.5), pct(&mut quiet, 0.9), pct(&mut quiet, 0.99)
            );
        };
        if let Ok(path) = std::env::var("GEQ_REAL_RAW") {
            let raw = std::fs::read(&path).unwrap();
            let audio: Vec<[f32; 2]> = raw.chunks_exact(8).map(|c| [f32::from_le_bytes(c[..4].try_into().unwrap()), f32::from_le_bytes(c[4..].try_into().unwrap())]).collect();
            stats("real", &audio);
        }
        for s in songs(None).iter().filter(|s| ["rock_130", "pop_104", "melodic_88", "metal_168"].contains(&s.name)) {
            stats(s.name, &s.audio);
        }
    }

    /// The exact drum times of the synthetic fixtures (`synth.rs`): a kick, snare or hat starts at each of these.
    pub(super) fn fixture_labels(fx: crate::synth::Fixture, secs: f64) -> Vec<Label> {
        use crate::synth::Fixture;
        let beat = 0.5f64;
        let mut out = Vec::new();
        let mut push = |t: f64, kind: Kind, vel: u8| {
            if t < secs {
                out.push(Label { t, kind, vel });
            }
        };
        match fx {
            Fixture::Percussion => {
                let mut k = 0.0;
                while k < secs {
                    push(k, Kind::Kick, 110);
                    push(k + 1.5, Kind::Snare, 100);
                    k += 3.0;
                }
            }
            Fixture::Demo | Fixture::Dense => {
                // sections by bar (2 s) of a 32-bar loop; Dense is the drop section throughout
                let n_beats = (secs / beat) as u64;
                for bi in 0..n_beats {
                    let bar = bi / 4;
                    let in_bar = bi % 4;
                    let (sec, sb) = if fx == Fixture::Dense {
                        ('D', bar % 8)
                    } else {
                        match bar % 32 {
                            b @ 0..=3 => ('I', b),
                            b @ 4..=7 => ('G', b - 4),
                            b @ 8..=11 => ('B', b - 8),
                            b @ 12..=19 => ('D', b - 12),
                            b @ 20..=23 => ('K', b - 20),
                            b @ 24..=27 => ('G', b - 24 + 4),
                            b => ('O', b - 28),
                        }
                    };
                    let t = bi as f64 * beat;
                    let kick = match sec {
                        'I' | 'K' => in_bar == 0,
                        'O' => sb < 2,
                        _ => true,
                    };
                    if kick {
                        push(t, Kind::Kick, 110);
                    }
                    let roll = sec == 'B' && sb == 3;
                    let snare = match sec {
                        'G' | 'D' => in_bar == 1 || in_bar == 3,
                        'B' => roll || (sb >= 1 && (in_bar == 1 || in_bar == 3)),
                        _ => false,
                    };
                    if snare && roll {
                        for q in 0..4 {
                            push(t + q as f64 * beat / 4.0, Kind::Snare, 60);
                        }
                    } else if snare {
                        push(t, Kind::Snare, 100);
                    }
                    // hats are quiet (a fifth of the kick's level): labelled so that a detection counts as a hit, not as a miss
                    match sec {
                        'I' if in_bar % 2 == 1 => push(t, Kind::Hat, 50),
                        'G' | 'B' => {
                            push(t, Kind::Hat, 50);
                            push(t + beat / 2.0, Kind::Hat, 50);
                        }
                        'O' if sb < 3 => {
                            push(t, Kind::Hat, 50);
                            push(t + beat / 2.0, Kind::Hat, 50);
                        }
                        'D' => {
                            for q in 0..4 {
                                push(t + q as f64 * beat / 4.0, Kind::Hat, 50);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        out
    }

    /// Scores the built-in detector on the repo's own synthetic songs, whose drum times are exact. They were made by
    /// other code than the corpus and never seen in training.
    #[test]
    #[ignore]
    fn score_fixtures() {
        use crate::synth::{Fixture, Synth};
        for (fx, secs) in [(Fixture::Demo, 64.0), (Fixture::Dense, 40.0), (Fixture::Percussion, 40.0)] {
            let mut synth = Synth::new(fx);
            let (mut l, mut r) = (Vec::new(), Vec::new());
            synth.render((secs * SR) as usize, &mut l, &mut r);
            let audio: Vec<[f32; 2]> = l.iter().zip(&r).map(|(a, b)| [*a, *b]).collect();
            let mut dsp = Dsp::new();
            let mut hits = Vec::new();
            let (mut cl, mut cr) = (vec![0.0f32; 480], vec![0.0f32; 480]);
            for k in 0..audio.len() / 480 {
                for i in 0..480 {
                    cl[i] = audio[k * 480 + i][0];
                    cr[i] = audio[k * 480 + i][1];
                }
                dsp.push(&cl, &cr, ((k + 1) * 480) as f64 / SR);
                hits.extend(dsp.take_drums().into_iter().filter(|h| h.p >= 0.9));
            }
            let dets: Vec<Det> = hits.iter().map(|h| Det { t: h.t, pos: h.pos, strength: h.strength, class: h.kind as u8, p: h.p }).collect();
            let labels = fixture_labels(fx, secs);
            let agnostic = score(&labels, &dets, secs, false);
            let aware = score(&labels, &dets, secs, true);
            eprintln!("{:10} any drum: {}", fx.name(), agnostic.line());
            eprintln!("{:10} class-aware: P {:.1}% R {:.1}% F1 {:.2} | {}", "", 100.0 * aware.precision(), 100.0 * aware.recall(), aware.f1(), aware.kinds());
        }
    }

    /// The built-in detector on the repo's own synthetic fixtures (computed sounds, not soundfont samples): hits per
    /// class, and for the percussion fixture (a kick at 3k s, a snare at 3k + 1.5 s) the report time after each drum.
    #[test]
    #[ignore]
    fn fixtures_with_the_drum_detector() {
        use crate::synth::{Fixture, Synth};
        for (fx, secs) in [(Fixture::Percussion, 40.0), (Fixture::Impulse, 20.0), (Fixture::Demo, 64.0), (Fixture::Dense, 40.0), (Fixture::Tones, 30.0), (Fixture::Sweep, 40.0), (Fixture::Quiet, 40.0), (Fixture::Silence, 10.0)] {
            let mut synth = Synth::new(fx);
            let mut dsp = Dsp::new();
            let (mut l, mut r) = (Vec::new(), Vec::new());
            let mut hits = Vec::new();
            for k in 0..(secs * 100.0) as usize {
                l.clear();
                r.clear();
                synth.render(480, &mut l, &mut r);
                dsp.push(&l, &r, (k + 1) as f64 * 0.01);
                hits.extend(dsp.take_drums().into_iter().filter(|h| h.t > 4.0 && h.p >= 0.9));
            }
            let n = |c: usize| hits.iter().filter(|h| h.kind as usize == c).count();
            // and how many a scene would see, through the adapter at Sensitivity 1
            let mut dh = crate::modulation::DrumHits::new();
            let reached: usize = hits.iter().map(|h| dh.update(std::slice::from_ref(h), 1.0, h.t, true).len()).sum();
            eprintln!("{:10} {:3} hits at confidence >= 0.9 in {:.0} s: low {} snare {} high {}; {} reach a scene ({:.1}/s)", fx.name(), hits.len(), secs - 4.0, n(0), n(1), n(2), reached, reached as f64 / (secs - 4.0));
            if fx == Fixture::Percussion {
                for k in 2..8 {
                    for (t, name) in [(3.0 * k as f64, "kick "), (3.0 * k as f64 + 1.5, "snare")] {
                        let found: Vec<String> = hits.iter().filter(|h| h.t >= t && h.t < t + 0.25).map(|h| format!("{:?} +{:.0}ms p {:.2}", h.kind, (h.t - t) * 1000.0, h.p)).collect();
                        eprintln!("    {name} {t:5.1}: {}", found.join(" | "));
                    }
                }
            }
        }
    }

    /// The model's three probabilities, the loudness and the features that matter, frame by frame around the
    /// percussion fixture's kick (t = 6.0 s) and snare (7.5 s).
    #[test]
    #[ignore]
    fn percussion_probs() {
        use crate::synth::{Fixture, Synth};
        let mut synth = Synth::new(Fixture::Percussion);
        let mut dsp = Dsp::new();
        let (mut l, mut r) = (Vec::new(), Vec::new());
        for k in 0..800 {
            l.clear();
            r.clear();
            synth.render(480, &mut l, &mut r);
            dsp.push(&l, &r, (k + 1) as f64 * 0.01);
            let t = (k + 1) as f64 * 0.01;
            if (5.97..6.2).contains(&t) || (7.47..7.62).contains(&t) {
                let d = &dsp.drums;
                eprintln!("t {t:.2} loud {:.4} p [{:.2} {:.2} {:.2}] flux lo/mid/hi [{:.2} {:.2} {:.2}] peak {:.3}", d.last.loud, d.probs[0], d.probs[1], d.probs[2], d.last.flux[0], d.last.flux[1], d.last.flux[2], l.iter().fold(0.0f32, |m, v| m.max(v.abs())));
            }
        }
    }

    /// The built-in detector (the model in `drum_model.rs`) on the whole corpus and the repo's synthetic songs, at the
    /// confidence the scenes ask for at Sensitivity 1 and at two others.
    #[test]
    #[ignore]
    fn eval_builtin() {
        let mut all = crate::drum_train::render_all(&Sf2::load().unwrap(), &corpus::specs());
        for (fx, secs) in [(crate::synth::Fixture::Demo, 64.0), (crate::synth::Fixture::Dense, 40.0)] {
            let mut synth = crate::synth::Synth::new(fx);
            let (mut l, mut r) = (Vec::new(), Vec::new());
            synth.render((secs * SR) as usize, &mut l, &mut r);
            all.push(Song { name: if fx == crate::synth::Fixture::Demo { "synth_demo" } else { "synth_dense" }, set: Set::Test, audio: l.iter().zip(&r).map(|(a, b)| [*a, *b]).collect(), labels: fixture_labels(fx, secs) });
        }
        for min_p in [0.9f32, 0.8] {
            table_on(&format!("built-in model, confidence >= {min_p}"), &all, &|a| builtin(a, min_p), min_p > 0.85);
        }
        table_on("old RiseHits, sensitivity 1 (the baseline)", &all, &|a| baseline(a, 1.0), false);
    }

    /// The app's own path for hits, without a window: the stepped synthetic source advanced at 60 Hz, `take_drums` each
    /// frame and the adapter, as `App::update` does. Counts what reaches a scene.
    #[test]
    #[ignore]
    fn app_path_hits() {
        use crate::audio::Audio;
        use crate::synth::Fixture;
        for (fx, secs) in [(Fixture::Demo, 44.0), (Fixture::Dense, 44.0), (Fixture::Percussion, 44.0)] {
            let mut audio = Audio::new_synth(fx, None, true, [0.0; 11]);
            let mut dh = crate::modulation::DrumHits::new();
            let (mut taken, mut passed) = ([0usize; 3], [0usize; 3]);
            for f in 0..(secs * 60.0) as usize {
                let now = f as f64 / 60.0;
                audio.advance(now);
                let drums = audio.take_drums();
                for d in drums.iter().filter(|d| d.t > 4.0) {
                    taken[d.kind as usize] += 1;
                }
                for h in dh.update(&drums, 1.0, now, true).iter().filter(|_| now > 4.0) {
                    passed[((h.pos * 3.0) as usize).min(2)] += 1;
                }
            }
            eprintln!("{:10} taken (all confidences) low {} snare {} high {} | passed the adapter by place: low {} mid {} high {} ({:.1}/s)", fx.name(), taken[0], taken[1], taken[2], passed[0], passed[1], passed[2], passed.iter().sum::<usize>() as f64 / (secs - 4.0));
        }
    }

    /// Writes `<song>.proposals.json` next to every WAV in `GEQ_REAL_DIR` (default `labels/audio`): everything the detector
    /// hears at confidence 0.4 or more, for the labeller to show as ghost marks while you review.
    #[test]
    #[ignore]
    fn propose_labels() {
        let dir = real_dir();
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{dir}: {e}")).flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("wav") {
                continue;
            }
            let audio = crate::synth::load_wav(path.to_str().unwrap()).unwrap();
            let hits: Vec<serde_json::Value> = proposals(&audio, 0.4).iter().map(|(t, c, p)| serde_json::json!({"t": (t * 1e4).round() / 1e4, "class": c, "p": (*p as f64 * 1e3).round() / 1e3})).collect();
            let name = path.file_name().unwrap().to_str().unwrap().to_string();
            let out = path.with_extension("proposals.json");
            std::fs::write(&out, serde_json::to_string(&serde_json::json!({"audio": name, "hits": hits})).unwrap()).unwrap();
            eprintln!("{name}: {} proposals -> {}", hits.len(), out.display());
            n += 1;
        }
        assert!(n > 0, "no WAV files in {dir}");
    }

    /// Scores the built-in detector (and the old one) on every labelled real song: `labels/*.labels.json` against the
    /// audio in `GEQ_REAL_DIR`. Dev and test songs are reported separately; look at the test line only for a final report.
    /// `GEQ_CONF` is the confidence (default 0.9).
    #[test]
    #[ignore]
    fn eval_real() {
        let conf: f32 = std::env::var("GEQ_CONF").ok().and_then(|v| v.parse().ok()).unwrap_or(0.9);
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(label_dir()).unwrap().flatten().map(|e| e.path()).filter(|p| p.to_str().map_or(false, |s| s.ends_with(".labels.json"))).collect();
        files.sort();
        let (mut dev, mut test) = (Report::default(), Report::default());
        let (mut dev_old, mut test_old) = (Report::default(), Report::default());
        let mut nodrum = [(0usize, 0.0f64); 2];
        eprintln!("== real songs, confidence >= {conf}");
        for f in files {
            let lf = match parse_label_file(&std::fs::read_to_string(&f).unwrap()) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}: {e}", f.display());
                    continue;
                }
            };
            let wav = format!("{}/{}", real_dir(), lf.audio);
            let Ok(audio) = crate::synth::load_wav(&wav) else {
                eprintln!("{:24} audio not found: {wav}", lf.audio);
                continue;
            };
            let secs = audio.len() as f64 / SR;
            let (from, to) = lf.excerpt.unwrap_or((FROM, secs));
            if from < 3.0 {
                eprintln!("{:24} warning: the excerpt starts at {from:.1} s; the detector needs about 3 s of lead-in", lf.audio);
            }
            let dets = builtin(&audio, conf);
            let old = baseline(&audio, 1.0);
            let (rep, rep_old) = (score_range(&lf.labels, &dets, from, to, true), score_range(&lf.labels, &old, from, to, false));
            let split = if lf.split == "test" { 1 } else { 0 };
            // false hits where the labeller said there are no drums
            let (mut n, mut s) = (0usize, 0.0f64);
            for (a, b) in lf.no_drums.iter().map(|(a, b)| (a.max(from), b.min(to))).filter(|(a, b)| b > a) {
                n += dets.iter().filter(|d| d.t >= a && d.t <= b).count();
                s += b - a;
            }
            nodrum[split].0 += n;
            nodrum[split].1 += s;
            eprintln!("{:24} {:4}  {}", lf.audio, lf.split, rep.line());
            eprintln!("{:29}class-aware P {:4.1}% R {:4.1}% F1 {:.2} | {}", "", 100.0 * rep.precision(), 100.0 * rep.recall(), rep.f1(), rep.kinds());
            if s > 0.0 {
                eprintln!("{:29}no-drum stretches: {n} hits in {s:.0} s ({:.0} a minute)", "", n as f64 / (s / 60.0));
            }
            if split == 1 {
                test.add(&rep);
                test_old.add(&rep_old);
            } else {
                dev.add(&rep);
                dev_old.add(&rep_old);
            }
        }
        for (name, rep, old, nd) in [("dev", &dev, &dev_old, nodrum[0]), ("test", &test, &test_old, nodrum[1])] {
            if rep.labels == 0 {
                continue;
            }
            eprintln!("{name:5} ALL   {}", rep.line());
            eprintln!("{:6}      {}", "", rep.kinds());
            eprintln!("{:6}      old detector: recall {:4.1}% precision {:4.1}% F1 {:.2}", "", 100.0 * old.recall(), 100.0 * old.precision(), old.f1());
            if nd.1 > 0.0 {
                eprintln!("{:6}      false hits in no-drum stretches: {:.0} a minute", "", nd.0 as f64 / (nd.1 / 60.0));
            }
        }
    }

    /// False hits on real music with no drums in it: each MDB-Drums song's band (every stem but the drums), after the first 4 s,
    /// at `GEQ_CONF` (default 0.9). The songs training held out (`GEQ_REAL_HOLDOUT`, default the test split) are totalled apart.
    #[test]
    #[ignore]
    fn eval_band_only() {
        let conf: f32 = std::env::var("GEQ_CONF").ok().and_then(|v| v.parse().ok()).unwrap_or(0.9);
        let tracks = crate::real_corpus::tracks();
        assert!(!tracks.is_empty(), "no MDB-Drums songs found (see tools/import-mdb.py)");
        let mut tot = [(0usize, 0.0f64); 2];
        eprintln!("== the band alone (no drums), confidence >= {conf}");
        for t in &tracks {
            let band = t.band();
            let secs = (band.len() as f64 / SR - FROM).max(0.0);
            let n = builtin(&band, conf).iter().filter(|d| d.t >= FROM).count();
            let held = crate::real_corpus::held_out(t) as usize;
            tot[held].0 += n;
            tot[held].1 += secs;
            eprintln!("{:24} {:9} {:3} hits in {:4.0} s ({:3.0} a minute)", t.name, if held == 1 { "held out" } else { "trained" }, n, secs, n as f64 / (secs / 60.0).max(1e-9));
        }
        for (name, (n, s)) in [("trained on", tot[0]), ("held out  ", tot[1])] {
            if s > 0.0 {
                eprintln!("{name} ALL  {n} hits in {s:.0} s: {:.1} a minute", n as f64 / (s / 60.0));
            }
        }
    }

    #[test]
    fn label_files_are_read_and_scored_within_their_excerpt() {
        let json = r#"{"version":1,"audio":{"name":"a.wav"},"split":"test","excerpt":[10,20],
            "labels":[{"t":9.0,"kind":"kick"},{"t":12.0,"kind":"kick"},{"t":13.0,"kind":"snare"},{"t":14.0,"kind":"hat","ghost":true},{"t":16.0,"kind":"tom"}],
            "noDrums":[[18,19]]}"#;
        let lf = parse_label_file(json).unwrap();
        assert_eq!((lf.audio.as_str(), lf.split.as_str(), lf.excerpt), ("a.wav", "test", Some((10.0, 20.0))));
        assert_eq!(lf.labels.len(), 5);
        assert_eq!(lf.labels[3].vel, 40, "a ghost note is soft");
        assert_eq!(lf.no_drums, vec![(18.0, 19.0)]);
        // the kick at 9 s is outside the excerpt; the kick, snare and tom are asked for, the ghost hat is not
        let det = |t: f64, class: u8| Det { t, pos: 0.5, strength: 1.0, class, p: 1.0 };
        let rep = score_range(&lf.labels, &[det(12.02, 0), det(13.02, 1), det(14.02, 2), det(16.02, 1), det(17.5, 0)], 10.0, 20.0, true);
        assert_eq!(rep.salient, 3, "{rep:?}");
        assert_eq!(rep.found, 2, "kick and snare found; the tom was heard as a snare: {rep:?}");
        assert_eq!((rep.tp, rep.fp), (3, 2), "the ghost hat is no false hit, the stray ones are: {rep:?}");
        assert!(parse_label_file(r#"{"audio":{"name":"a.wav"},"labels":[{"t":1,"kind":"cowbell"}]}"#).is_err());
    }

    /// The detector the scenes use today, on the whole corpus.
    #[test]
    #[ignore]
    fn eval_baseline() {
        for sens in [1.0f32, 1.8] {
            table(&format!("baseline RiseHits, sensitivity {sens}"), None, &|a| baseline(a, sens));
        }
    }

    /// Spectrograms of eight seconds of each song with the baseline's detections.
    #[test]
    #[ignore]
    fn plot_baseline() {
        let dir = out_dir();
        for s in songs(None) {
            let dets = baseline(&s.audio, 1.0);
            spectrogram(&format!("{dir}/{}_baseline.png", s.name), &s.audio, &s.labels, &dets, 10.0, 18.0);
        }
    }
}
