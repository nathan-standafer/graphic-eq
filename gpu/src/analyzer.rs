//! Audio capture (parec) + FFT, reduced to band levels in dB.

use crate::bands;
use rustfft::{num_complex::Complex, FftPlanner};
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

pub const FFT_SIZE: usize = 8192;
pub const HOP: usize = 480; // 10 ms at 48 kHz -> ~100 updates/s
pub const CURVE_POINTS: usize = 192;
pub const FLOOR_DB: f32 = -140.0;
const CAL_DB: f32 = 3.01; // so that a sine at -6 dBFS reads -6 dB

#[derive(Clone)]
pub struct Levels {
    pub bands: [f32; bands::COUNT],
    pub curve: [f32; CURVE_POINTS],
    pub seq: u64,
}

impl Default for Levels {
    fn default() -> Self {
        Levels { bands: [FLOOR_DB; bands::COUNT], curve: [FLOOR_DB; CURVE_POINTS], seq: 0 }
    }
}

pub fn curve_freq(i: usize) -> f32 {
    30.0 * (20000.0f32 / 30.0).powf(i as f32 / (CURVE_POINTS as f32 - 1.0))
}

pub struct Analyzer {
    child: Child,
    pub levels: Arc<Mutex<Levels>>,
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
            let bin_hz = bands::SAMPLE_RATE / FFT_SIZE as f32;
            let band_edges: Vec<(f32, f32)> = bands::FREQS.iter().map(|f| bands::edges(*f)).collect();
            // The waterfall curve uses a narrow window (two point-spacings wide) for real
            // resolution, scaled to the energy a full band-wide window would collect so levels
            // stay comparable with the bars.
            let half = bands::step().sqrt();
            let r = (20000.0f32 / 30.0).powf(1.0 / (CURVE_POINTS as f32 - 1.0));
            let curve_scale = (half - 1.0 / half) / (r - 1.0 / r);
            let curve_edges: Vec<(f32, f32)> =
                (0..CURVE_POINTS).map(|i| (curve_freq(i) / r, curve_freq(i) * r)).collect();

            let mut ring = vec![0f32; FFT_SIZE];
            let mut head = 0usize;
            let mut buf = vec![0u8; HOP * 8];
            let mut pending: Vec<u8> = Vec::new();
            let mut seq = 0u64;
            let mut scratch = vec![Complex::new(0.0, 0.0); FFT_SIZE];
            let mut cum = vec![0f32; FFT_SIZE / 2 + 1];
            loop {
                let n = match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                pending.extend_from_slice(&buf[..n]);
                let frames = pending.len() / 8;
                let mut produced = 0;
                for f in 0..frames {
                    let o = f * 8;
                    let l = f32::from_le_bytes(pending[o..o + 4].try_into().unwrap());
                    let r = f32::from_le_bytes(pending[o + 4..o + 8].try_into().unwrap());
                    ring[head % FFT_SIZE] = 0.5 * (l + r);
                    head += 1;
                    produced += 1;
                }
                pending.drain(..frames * 8);
                if produced == 0 || head < FFT_SIZE {
                    continue;
                }
                // one FFT per read chunk (chunks are ~HOP samples)
                for (i, s) in scratch.iter_mut().enumerate() {
                    *s = Complex::new(ring[(head + i) % FFT_SIZE] * window[i], 0.0);
                }
                fft.process(&mut scratch);
                // one-sided power per bin: mean-square contribution
                let norm = 2.0 / (FFT_SIZE as f32 * wsq);
                cum[0] = 0.0;
                for k in 0..FFT_SIZE / 2 {
                    cum[k + 1] = cum[k] + scratch[k].norm_sqr() * norm;
                }
                let energy = |f: f32| -> f32 {
                    let pos = f / bin_hz;
                    let i = pos as usize;
                    if i >= FFT_SIZE / 2 {
                        return cum[FFT_SIZE / 2];
                    }
                    cum[i] + (pos - i as f32) * (cum[i + 1] - cum[i])
                };
                let db = |(lo, hi): (f32, f32), scale: f32| -> f32 {
                    let e = (energy(hi) - energy(lo)) * scale;
                    if e > 1e-14 { (10.0 * e.log10() + CAL_DB).max(FLOOR_DB) } else { FLOOR_DB }
                };
                let mut lv = Levels::default();
                for (i, e) in band_edges.iter().enumerate() {
                    lv.bands[i] = db(*e, 1.0);
                }
                for (i, e) in curve_edges.iter().enumerate() {
                    lv.curve[i] = db(*e, curve_scale);
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
