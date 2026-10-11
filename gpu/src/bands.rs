//! Band layout and EQ maths shared by the engine and the display.

pub const FREQS: [f32; 11] = [40.0, 73.0, 130.0, 240.0, 440.0, 800.0, 1500.0, 2700.0, 4800.0, 8800.0, 16000.0];
pub const COUNT: usize = FREQS.len();
pub const Q: f32 = 1.6;
pub const GAIN_MIN: f32 = -12.0;
pub const GAIN_MAX: f32 = 12.0;
pub const SAMPLE_RATE: f32 = 48000.0;

/// Ratio between neighbouring band centres (~1.82).
pub fn step() -> f32 {
    (FREQS[COUNT - 1] / FREQS[0]).powf(1.0 / (COUNT as f32 - 1.0))
}

pub fn edges(freq: f32) -> (f32, f32) {
    let half = step().sqrt();
    (freq / half, freq * half)
}

pub fn label(freq: f32) -> String {
    if freq >= 1000.0 {
        format!("{}k", freq / 1000.0)
    } else {
        format!("{}", freq)
    }
}

/// Magnitude (dB) of an RBJ peaking biquad.
fn peaking_db(gain_db: f32, f0: f32, f: f32) -> f32 {
    if gain_db.abs() < 1e-6 {
        return 0.0;
    }
    let a = 10f32.powf(gain_db / 40.0);
    let w0 = 2.0 * std::f32::consts::PI * f0 / SAMPLE_RATE;
    let alpha = w0.sin() / (2.0 * Q);
    let cw = w0.cos();
    let (b0, b1, b2) = (1.0 + alpha * a, -2.0 * cw, 1.0 - alpha * a);
    let (a0, a1, a2) = (1.0 + alpha / a, -2.0 * cw, 1.0 - alpha / a);
    let w = 2.0 * std::f32::consts::PI * f / SAMPLE_RATE;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    // |b0 + b1 z^-1 + b2 z^-2|^2 / |a0 + a1 z^-1 + a2 z^-2|^2
    let num = (b0 + b1 * c1 + b2 * c2).powi(2) + (b1 * s1 + b2 * s2).powi(2);
    let den = (a0 + a1 * c1 + a2 * c2).powi(2) + (a1 * s1 + a2 * s2).powi(2);
    10.0 * (num / den).log10()
}

pub fn eq_response_db(gains: &[f32; COUNT], f: f32) -> f32 {
    gains.iter().zip(FREQS).map(|(g, f0)| peaking_db(*g, f0, f)).sum()
}
