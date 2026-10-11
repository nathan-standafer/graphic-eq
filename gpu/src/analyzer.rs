//! Audio capture (parec) + FFT, reduced to band levels in dB.

use crate::bands;
use rustfft::{num_complex::Complex, FftPlanner};
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub const FFT_SIZE: usize = 8192;
pub const HOP: usize = 480; // 10 ms at 48 kHz -> ~100 updates/s
pub const CURVE_POINTS: usize = 192;
pub const FLOOR_DB: f32 = -140.0;
const CAL_DB: f32 = 3.01; // so that a sine at -6 dBFS reads -6 dB

#[derive(Clone)]
pub struct Levels {
    pub bands: [f32; bands::COUNT],
    /// Waterfall curves: mixed signal, and the left / right channels on their own.
    pub curve: [f32; CURVE_POINTS],
    pub curve_l: [f32; CURVE_POINTS],
    pub curve_r: [f32; CURVE_POINTS],
    pub seq: u64,
}

impl Default for Levels {
    fn default() -> Self {
        Levels {
            bands: [FLOOR_DB; bands::COUNT],
            curve: [FLOOR_DB; CURVE_POINTS],
            curve_l: [FLOOR_DB; CURVE_POINTS],
            curve_r: [FLOOR_DB; CURVE_POINTS],
            seq: 0,
        }
    }
}

/// Width (in octaves, as f32 bits) of the analysis window behind each waterfall bin.
static CURVE_OCTAVES: AtomicU32 = AtomicU32::new(0x3eaaaaab); // 1/3 octave

pub fn set_curve_octaves(oct: f32) {
    CURVE_OCTAVES.store(oct.to_bits(), Ordering::Relaxed);
}

pub fn curve_freq(i: usize) -> f32 {
    30.0 * (20000.0f32 / 30.0).powf(i as f32 / (CURVE_POINTS as f32 - 1.0))
}

pub struct Analyzer {
    child: Child,
    pub levels: Arc<Mutex<Levels>>,
}

/// Cumulative one-sided power spectrum (mean-square contribution per bin).
fn cumulate(spec: &[Complex<f32>], norm: f32, cum: &mut [f32]) {
    cum[0] = 0.0;
    for k in 0..FFT_SIZE / 2 {
        cum[k + 1] = cum[k] + spec[k].norm_sqr() * norm;
    }
}

impl Analyzer {
    pub fn start(device: &str) -> Result<Analyzer, String> {
        let mut child = Command::new("parec")
            .args([
                &format!("--device={device}"),
                "--format=float32le",
                "--rate=48000",
                "--channels=2",
                "--latency-msec=10",
                "--client-name=Graphic EQ analyzer",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not launch parec: {e}"))?;
        let mut out = child.stdout.take().ok_or("no parec stdout")?;
        let levels = Arc::new(Mutex::new(Levels::default()));
        let shared = levels.clone();
        std::thread::spawn(move || {
            let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
            let window: Vec<f32> = (0..FFT_SIZE)
                .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos())
                .collect();
            let wsq: f32 = window.iter().map(|w| w * w).sum();
            let norm = 2.0 / (FFT_SIZE as f32 * wsq);
            let bin_hz = bands::SAMPLE_RATE / FFT_SIZE as f32;
            let band_edges: Vec<(f32, f32)> = bands::FREQS.iter().map(|f| bands::edges(*f)).collect();
            let half = bands::step().sqrt();

            let mut ring_l = vec![0f32; FFT_SIZE];
            let mut ring_r = vec![0f32; FFT_SIZE];
            let mut head = 0usize;
            let mut buf = vec![0u8; HOP * 8];
            let mut pending: Vec<u8> = Vec::new();
            let mut seq = 0u64;
            let mut spec_l = vec![Complex::new(0.0, 0.0); FFT_SIZE];
            let mut spec_r = vec![Complex::new(0.0, 0.0); FFT_SIZE];
            let mut spec_m = vec![Complex::new(0.0, 0.0); FFT_SIZE];
            let mut cum_m = vec![0f32; FFT_SIZE / 2 + 1];
            let mut cum_l = vec![0f32; FFT_SIZE / 2 + 1];
            let mut cum_r = vec![0f32; FFT_SIZE / 2 + 1];
            loop {
                let n = match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                pending.extend_from_slice(&buf[..n]);
                let frames = pending.len() / 8;
                for f in 0..frames {
                    let o = f * 8;
                    ring_l[head % FFT_SIZE] = f32::from_le_bytes(pending[o..o + 4].try_into().unwrap());
                    ring_r[head % FFT_SIZE] = f32::from_le_bytes(pending[o + 4..o + 8].try_into().unwrap());
                    head += 1;
                }
                pending.drain(..frames * 8);
                if frames == 0 || head < FFT_SIZE {
                    continue;
                }
                // one pair of FFTs per read chunk (chunks are ~HOP samples)
                for i in 0..FFT_SIZE {
                    let j = (head + i) % FFT_SIZE;
                    spec_l[i] = Complex::new(ring_l[j] * window[i], 0.0);
                    spec_r[i] = Complex::new(ring_r[j] * window[i], 0.0);
                }
                fft.process(&mut spec_l);
                fft.process(&mut spec_r);
                for i in 0..FFT_SIZE {
                    spec_m[i] = (spec_l[i] + spec_r[i]) * 0.5; // FFT is linear: mono = (L+R)/2
                }
                cumulate(&spec_m, norm, &mut cum_m);
                cumulate(&spec_l, norm, &mut cum_l);
                cumulate(&spec_r, norm, &mut cum_r);

                let energy = |cum: &[f32], f: f32| -> f32 {
                    let pos = f / bin_hz;
                    let i = pos as usize;
                    if i >= FFT_SIZE / 2 {
                        return cum[FFT_SIZE / 2];
                    }
                    cum[i] + (pos - i as f32) * (cum[i + 1] - cum[i])
                };
                let db = |cum: &[f32], (lo, hi): (f32, f32), scale: f32| -> f32 {
                    let e = (energy(cum, hi) - energy(cum, lo)) * scale;
                    if e > 1e-14 { (10.0 * e.log10() + CAL_DB).max(FLOOR_DB) } else { FLOOR_DB }
                };
                // Waterfall bins: window width is user-selectable; energy is scaled to what a
                // full band-wide window would collect so levels stay comparable with the bars.
                let w = (2f32.powf(f32::from_bits(CURVE_OCTAVES.load(Ordering::Relaxed)) * 0.5)).max(1.01);
                let curve_scale = (half - 1.0 / half) / (w - 1.0 / w);
                let mut lv = Levels::default();
                for (i, e) in band_edges.iter().enumerate() {
                    lv.bands[i] = db(&cum_m, *e, 1.0);
                }
                for i in 0..CURVE_POINTS {
                    let e = (curve_freq(i) / w, curve_freq(i) * w);
                    lv.curve[i] = db(&cum_m, e, curve_scale);
                    lv.curve_l[i] = db(&cum_l, e, curve_scale);
                    lv.curve_r[i] = db(&cum_r, e, curve_scale);
                }
                seq += 1;
                lv.seq = seq;
                *shared.lock().unwrap() = lv;
            }
        });
        Ok(Analyzer { child, levels })
    }

    pub fn latest(&self) -> Levels {
        self.levels.lock().unwrap().clone()
    }
}

impl Drop for Analyzer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
