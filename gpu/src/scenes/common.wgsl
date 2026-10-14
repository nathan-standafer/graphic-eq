// Shared by every scene: the uniform block, the history texture, and colour helpers.
// Each scene's own source is concatenated after this at load time.
struct U {
  vp: mat4x4<f32>,
  cam: vec4<f32>,
  t: f32, aspect: f32, mode: f32, theme: f32,
  head: f32, rows: f32, exposure: f32, flash: f32,
  res: vec4<f32>,       // width, height, fraction of the way to the next history row, row phase (0..4)
  fx: vec4<f32>,        // onset envelopes (low, mid, high), stereo correlation
  sus: vec4<f32>,       // sustained low / mid / high (normalised), slow energy trend
  ctl: vec4<f32>,       // motion intensity, scene shape a, scene shape b, signal gate
  levels: array<vec4<f32>, 3>,
  peaks: array<vec4<f32>, 3>,
  rings: array<vec4<f32>, 8>,   // age (s), strength, band (0 low .. 2 high), spectral centroid (0..1)
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var hist: texture_2d<f32>;

const BG: vec3<f32> = vec3<f32>(0.005, 0.006, 0.013);
const NX: u32 = 96u;
const NZ: u32 = 192u;
const PI: f32 = 3.14159265;

fn lvl(i: u32) -> f32 { return u.levels[i / 4u][i % 4u]; }
fn pk(i: u32) -> f32 { return u.peaks[i / 4u][i % 4u]; }

fn hsv(h: f32, s: f32, v: f32) -> vec3<f32> {
  let k = vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0);
  let p = abs(fract(vec3<f32>(h) + k) * 6.0 - vec3<f32>(3.0));
  return v * mix(vec3<f32>(1.0), clamp(p - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0)), s);
}
fn ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
  let lo = mix(a, b, clamp(t / 0.6, 0.0, 1.0));
  let hi = mix(b, c, clamp((t - 0.6) / 0.4, 0.0, 1.0));
  return select(hi, lo, t < 0.6);
}
// A theme supplies three roles along t: dominant (0), supporting (0.6) and transient accent (1).
// Each material decides how to use them. Rainbow ignores t and follows x instead.
fn palette(t: f32, x: f32) -> vec3<f32> {
  let th = i32(u.theme + 0.5);
  if (th == 0) { return ramp(vec3<f32>(0.10, 1.0, 0.35), vec3<f32>(1.0, 0.9, 0.2), vec3<f32>(1.0, 0.15, 0.2), t); }
  if (th == 1) { return ramp(vec3<f32>(1.0, 0.65, 0.0), vec3<f32>(1.0, 0.4, 0.0), vec3<f32>(1.0, 0.12, 0.0), t); }
  if (th == 2) { return ramp(vec3<f32>(0.0, 0.9, 1.0), vec3<f32>(0.3, 0.5, 1.0), vec3<f32>(0.75, 0.3, 1.0), t); }
  if (th == 3) { return ramp(vec3<f32>(1.0, 0.15, 0.85), vec3<f32>(0.5, 0.35, 1.0), vec3<f32>(0.0, 0.9, 1.0), t); }
  return hsv(fract(0.85 - 0.8 * x), 0.85, 1.0);
}

fn gridline(v: f32) -> f32 {
  let d = abs(fract(v + 0.5) - 0.5);
  let w = fwidth(v);
  return 1.0 - smoothstep(0.0, 1.5 * w + 1e-4, d);
}
// Fade grid lines out once they get too thin to resolve (kills shimmer toward the horizon).
fn line_aa(fw: f32) -> f32 { return 1.0 - smoothstep(0.25, 0.55, fw); }
fn line(v: f32) -> f32 {
  let d = min(fract(v), 1.0 - fract(v));
  let w = fwidth(v);
  return 1.0 - smoothstep(0.0, 1.3 * w + 1e-4, d);
}

// ---- spectrum history (ring buffer of rows; row 0 is the newest). R = left, G = right, B = mixed, A = how
// steeply the level is rising at that bin (see `hist_rise`).
fn hist_row(zi: u32) -> i32 {
  let rows = i32(u.rows);
  return (i32(u.head) - i32(zi) + rows) % rows;
}
fn hist_h2(xi: u32, zi: u32) -> vec2<f32> {
  return textureLoad(hist, vec2<i32>(i32(xi), hist_row(zi)), 0).rg;
}
fn hist_m(xi: u32, zi: u32) -> f32 {
  return textureLoad(hist, vec2<i32>(i32(xi), hist_row(zi)), 0).b;
}
const RISE_FULL: f32 = 1.5;   // the normalized rise in a full alpha byte; the Rust side keeps this in step
fn hist_rise(xi: u32, zi: u32) -> f32 {
  return textureLoad(hist, vec2<i32>(i32(xi), hist_row(zi)), 0).a * RISE_FULL;
}

fn hash21(p: vec2<f32>) -> f32 {
  var q = fract(p * vec2<f32>(123.34, 456.21));
  q += dot(q, q + 45.32);
  return fract(q.x * q.y);
}

const STEREO_L: vec3<f32> = vec3<f32>(0.05, 0.85, 1.0);
const STEREO_R: vec3<f32> = vec3<f32>(1.0, 0.12, 0.80);

// erf by Abramowitz and Stegun 7.1.26 (error below 1.5e-7).
fn erf_approx(x: f32) -> f32 {
  let a = abs(x);
  let t = 1.0 / (1.0 + 0.3275911 * a);
  let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
  return select(-y, y, x >= 0.0);
}

// A rectangular light of half-sizes (hw, hh) along the unit vectors ax, ay, centred on c. The ray from p along r
// meets its plane at distance t; `soft` blurs the edges by a fixed length and `foot` by a length that grows with t
// (the pixel's footprint on that plane, plus the surface's roughness), so a thin strip seen in a surface that curves
// fast fades instead of shimmering, and a rough floor smears a far light more than a near one.
fn rect_light(c: vec3<f32>, ax: vec3<f32>, ay: vec3<f32>, hw: f32, hh: f32, p: vec3<f32>, r: vec3<f32>, foot: f32, soft: f32) -> f32 {
  let nrm = cross(ax, ay);
  let dn = dot(r, nrm);
  let t = dot(c - p, nrm) / select(dn, 1e-4, abs(dn) < 1e-4);
  let tt = max(t, 0.0);
  let hit = p + r * tt - c;
  let s = soft + foot * tt;
  let mx = 1.0 - smoothstep(hw - s, hw + s, abs(dot(hit, ax)));
  let my = 1.0 - smoothstep(hh - s, hh + s, abs(dot(hit, ay)));
  return mx * my * select(0.0, 1.0, t > 0.0);
}
