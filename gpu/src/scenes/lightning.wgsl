// Lightning Storm's floor, bolts and splash. The floor is the fountain's glossy dark glass with the spectrum line
// the strikes land on; every live strike throws a pool of light on it, and a low strike sends a thunder ring across
// it. A bolt is drawn from the table the CPU keeps (`lightning.rs`): a main channel of 64 segments and up to six
// branches of 16, each generated here by midpoint displacement from the strike's seed, so the same seed draws the
// same bolt in every capture. Segments are capsules with a Gaussian profile, integrated in closed form along their
// length (the vectorscope's beam), so thin lines are anti-aliased analytically and need no MSAA.
const FLOOR_EXTENT: f32 = 600.0;
const MAX_BOLTS: u32 = 8u;
const MAIN_LEVELS: u32 = 6u;
const MAIN_SEGS: u32 = 64u;
const BR_LEVELS: u32 = 4u;
const BR_SEGS: u32 = 16u;
const MAX_BR: u32 = 6u;
const SEGS: u32 = 160u;            // MAIN_SEGS + MAX_BR * BR_SEGS
const CORONAS: u32 = 24u;
const CORONA_SEGS: u32 = 8u;
const VISIBLE_S: f32 = 0.55;       // a channel is gone this long after the strike
const LEADER_S: f32 = 0.022;       // the first stroke runs down the channel in this long

// per bolt, four vec4:
//   a: (x, z, age, strength)            strength 0 = unused slot
//   b: (line, seed, strokes, top)       top = height the channel starts at
//   c: (branches, ghost, sigma, ring)   sigma = world radius of the main channel's core
//   d: (branch length scale, pool sigma, 0, 0)
struct LtU { bolts: array<vec4<f32>, 32> };
@group(1) @binding(2) var<uniform> lt: LtU;

struct Bolt {
  x: f32, z: f32, age: f32, strength: f32,
  line: f32, seed: u32, strokes: u32, top: f32,
  branches: u32, ghost: bool, sigma: f32, ring: f32,
  brlen: f32, pool: f32,
};

fn load_bolt(i: u32) -> Bolt {
  let a = lt.bolts[4u * i];
  let b = lt.bolts[4u * i + 1u];
  let c = lt.bolts[4u * i + 2u];
  let d = lt.bolts[4u * i + 3u];
  return Bolt(a.x, a.y, a.z, a.w, b.x, u32(b.y), u32(b.z), b.w, u32(c.x), c.y > 0.5, c.z, c.w, d.x, d.y);
}

// ---- midpoint displacement
// The classic recursion adds a random value at the midpoint of every segment; summed over levels that is a hat
// function per new node, so one vertex needs only the nodes whose hats cover it, one per level.
fn rn2(seed: u32, a: u32, b: u32) -> vec2<f32> {
  let s = seed * 7919u + a * 104729u + b * 1299709u;
  return vec2<f32>(rnd(s, 0u), rnd(s, 1u)) * 2.0 - vec2<f32>(1.0);
}

// Displacement of vertex j of a polyline of 2^levels segments (zero at both ends), each level `rough` times
// the one before.
fn frac_off(seed: u32, j: u32, levels: u32, rough: f32) -> vec2<f32> {
  let n = 1u << levels;
  var o = vec2<f32>(0.0);
  var amp = 1.0;
  for (var l = 1u; l <= levels; l++) {
    let step = n >> l;
    let m = j / step;
    let rem = j - m * step;
    var k = m + 1u;
    var w = f32(rem) / f32(step);
    if ((m & 1u) == 1u) { k = m; w = 1.0 - w; }
    o += amp * w * rn2(seed, l, k);
    amp *= rough;
  }
  return o;
}

// The main channel: from the cloud (which wanders a little from the strike) down to the strike on the glass.
fn main_point(bl: Bolt, j: u32) -> vec3<f32> {
  let t = f32(j) / f32(MAIN_SEGS);
  let drift = (vec2<f32>(rnd(bl.seed, 90u), rnd(bl.seed, 91u)) - vec2<f32>(0.5)) * vec2<f32>(5.0, 2.5);
  let base = vec3<f32>(mix(bl.x + drift.x, bl.x, t), mix(bl.top, 0.0, t), mix(bl.z + drift.y, bl.z, t));
  let o = frac_off(bl.seed, j, MAIN_LEVELS, 0.62) * (0.075 * bl.top);
  return base + vec3<f32>(o.x, 0.0, 0.6 * o.y);
}

// A branch leaves the main channel somewhere in its lower two thirds (the part in the frame) and runs down and to one side.
fn branch_j(bl: Bolt, br: u32) -> u32 {
  return 26u + u32(rnd(bl.seed, 100u + br * 8u) * 28.0);
}

fn branch_end(bl: Bolt, br: u32, o: vec3<f32>) -> vec3<f32> {
  let side = rnd(bl.seed, 101u + br * 8u) - 0.5;
  let lat = select(-1.0, 1.0, side > 0.0) * (0.30 + 2.0 * abs(side));
  let dir = normalize(vec3<f32>(lat, -1.0, rnd(bl.seed, 102u + br * 8u) - 0.5));
  var len = bl.top * (0.09 + 0.20 * rnd(bl.seed, 103u + br * 8u)) * bl.brlen;
  len = min(len, max(o.y - 0.03, 0.0) / -dir.y);       // never below the glass
  return o + dir * len;
}

fn branch_point(bl: Bolt, br: u32, o: vec3<f32>, e: vec3<f32>, j: u32) -> vec3<f32> {
  let t = f32(j) / f32(BR_SEGS);
  let off = frac_off(bl.seed ^ (br * 40503u + 17u), j, BR_LEVELS, 0.62) * (0.11 * length(e - o));
  return mix(o, e, t) + vec3<f32>(off.x, 0.0, 0.6 * off.y);
}

// ---- strokes
// A strike is one to three return strokes: the first runs down the channel in `LEADER_S`, each one flares and
// falls off in about 30 ms, and the channel keeps a dim glow that fades in a few tenths of a second. A branch
// joins some of the later strokes and skips others, so the picture flickers instead of repeating itself.
fn stroke_time(seed: u32, k: u32) -> f32 {
  if (k == 0u) { return 0.0; }
  var t = 0.055 + 0.020 * rnd(seed, 300u);
  if (k == 2u) { t += 0.045 + 0.020 * rnd(seed, 301u); }
  return t;
}

// `br` is 0 for the main channel, else the branch number; `path` is how far along the leader's route a point is.
fn bolt_light(bl: Bolt, br: u32, path: f32) -> f32 {
  let reveal = smoothstep(0.0, 0.07, (bl.age + 0.002) / LEADER_S * 1.07 - path);
  var e = 0.20 * exp(-bl.age / 0.11) * reveal;                   // the dim channel that stays behind
  for (var k = 0u; k < 3u; k++) {
    if (k >= bl.strokes) { break; }
    var on = true;
    if (br > 0u && k > 0u) { on = rnd(bl.seed, 400u + br * 4u + k) < 0.7; }
    if (!on) { continue; }
    let d = bl.age - stroke_time(bl.seed, k);
    if (d < 0.0) { continue; }
    var amp = 1.0 - 0.25 * f32(k);
    if (k == 0u) { amp *= reveal; }
    e += amp * exp(-d / 0.030);
  }
  return e;
}

// ---- segments as capsules
struct BoltOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) loc: vec2<f32>,                       // px along the segment from its start, and across it
  @location(1) @interpolate(flat) len: f32,
  @location(2) @interpolate(flat) sig: f32,          // core sigma, px
  @location(3) @interpolate(flat) inten: vec2<f32>,  // brightness at the two ends
  @location(4) @interpolate(flat) tint: vec3<f32>,
};

fn bolt_dead() -> BoltOut {
  var o: BoltOut;
  o.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
  o.loc = vec2<f32>(0.0);
  o.len = 0.0;
  o.sig = 1.0;
  o.inten = vec2<f32>(0.0);
  o.tint = vec3<f32>(0.0);
  return o;
}

// A mirror image in the glass is weak from above, stronger at grazing angles and fades with depth.
fn mirror_gain(p: vec3<f32>) -> f32 {
  let v = normalize(u.cam.xyz - p);
  return (0.10 + 0.90 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 3.0)) * exp(-0.35 * p.y) * 0.9;
}

fn bolt_quad(pa: vec3<f32>, pb: vec3<f32>, ia_in: f32, ib_in: f32, wsig: f32, tint: vec3<f32>, vi: u32, mirror: bool) -> BoltOut {
  var ia = ia_in;
  var ib = ib_in;
  var a3 = pa;
  var b3 = pb;
  if (mirror) {
    ia *= mirror_gain(pa);
    ib *= mirror_gain(pb);
    a3.y = -a3.y;
    b3.y = -b3.y;
  }
  let ca = u.vp * vec4<f32>(a3, 1.0);
  let cb = u.vp * vec4<f32>(b3, 1.0);
  if (ca.w < 0.2 || cb.w < 0.2 || ia + ib < 0.004) { return bolt_dead(); }
  let half = u.res.xy * 0.5;
  let a = ca.xy / ca.w * half;
  let b = cb.xy / cb.w * half;
  let focal = length(vec3<f32>(u.vp[0][1], u.vp[1][1], u.vp[2][1]));
  let true_px = wsig * focal * half.y / (0.5 * (ca.w + cb.w));      // the world radius in pixels
  let sig = max(true_px, 0.7);
  let kk = min(1.0, true_px / sig);                                  // a line thinner than a pixel keeps its energy by dimming
  let d = b - a;
  let len = length(d);
  var dir = vec2<f32>(1.0, 0.0);
  if (len > 1e-3) { dir = d / len; }
  let nrm = vec2<f32>(-dir.y, dir.x);
  let reach = 10.0 * sig + 1.5;                                      // far enough to catch the halo
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let k = q[vi];
  let along = select(-reach, len + reach, k.x > 0.5);
  let across = k.y * reach;
  let p = a + dir * along + nrm * across;
  var o: BoltOut;
  o.pos = vec4<f32>(p / half, 0.5 * (ca.z / ca.w + cb.z / cb.w), 1.0);
  o.loc = vec2<f32>(along, across);
  o.len = len;
  o.sig = sig;
  o.inten = vec2<f32>(ia, ib) * kk;
  o.tint = tint;
  return o;
}

// How far a stroke's light reaches in the world: Flash scales it, but a dim channel always stays
fn flash_gain() -> f32 { return 0.30 + 0.70 * clamp(u.flash, 0.0, 1.5); }

fn bolt_segment(vi: u32, ii: u32, mirror: bool) -> BoltOut {
  let bl = load_bolt(ii / SEGS);
  let s = ii % SEGS;
  if (bl.strength <= 0.0 || bl.age > VISIBLE_S) { return bolt_dead(); }
  var pa: vec3<f32>;
  var pb: vec3<f32>;
  var br = 0u;
  var ta = 0.0;
  var tb = 0.0;
  var path_a = 0.0;
  var path_b = 0.0;
  var wsig = bl.sigma;
  var ga = 1.0;
  var gb = 1.0;
  if (s < MAIN_SEGS) {
    pa = main_point(bl, s);
    pb = main_point(bl, s + 1u);
    ta = f32(s) / f32(MAIN_SEGS);
    tb = f32(s + 1u) / f32(MAIN_SEGS);
    path_a = ta;
    path_b = tb;
    // a short bolt starts in the cloud, so it fades in from nothing instead of ending in mid-air
    ga = smoothstep(0.0, 0.14, ta);
    gb = smoothstep(0.0, 0.14, tb);
  } else {
    let q = s - MAIN_SEGS;
    br = q / BR_SEGS + 1u;
    if (br > bl.branches) { return bolt_dead(); }
    let sj = q % BR_SEGS;
    let jo = branch_j(bl, br - 1u);
    let o = main_point(bl, jo);
    let e = branch_end(bl, br - 1u, o);
    pa = branch_point(bl, br - 1u, o, e, sj);
    pb = branch_point(bl, br - 1u, o, e, sj + 1u);
    ta = f32(sj) / f32(BR_SEGS);
    tb = f32(sj + 1u) / f32(BR_SEGS);
    let to = f32(jo) / f32(MAIN_SEGS);
    path_a = to + 0.15 * ta;
    path_b = to + 0.15 * tb;
    wsig = bl.sigma * (0.62 - 0.30 * 0.5 * (ta + tb));
    ga = 0.62 * (1.0 - 0.7 * ta);
    gb = 0.62 * (1.0 - 0.7 * tb);
  }
  let gain = (1.4 + 2.6 * bl.strength) * flash_gain() * select(1.0, 0.16, bl.ghost);
  let ia = bolt_light(bl, br, path_a) * ga * gain;
  let ib = bolt_light(bl, br, path_b) * gb * gain;
  let tint = mix(palette(0.05, bl.line), palette(0.6, bl.line), 0.35);
  return bolt_quad(pa, pb, ia, ib, wsig, tint, vi, mirror);
}

@vertex fn vs_bolt(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> BoltOut {
  return bolt_segment(vi, ii, false);
}
@vertex fn vs_bolt_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> BoltOut {
  return bolt_segment(vi, ii, true);
}

// ---- faint discharges between strikes: tiny flickering sparks of light standing on the spectrum line, more of
// them and brighter where the level is high, so a sustained passage is never an empty stage
fn corona_segment(vi: u32, ii: u32, mirror: bool) -> BoltOut {
  let slot = ii / CORONA_SEGS;
  let s = ii % CORONA_SEGS;
  let tq = floor(u.t * 9.0 + f32(slot) * 0.41);
  let ser = slot * 7919u + u32(tq);
  let band = (f32(slot) + rnd(slot * 131u + 5u, 0u)) / f32(CORONAS);
  let lev = hist_m(u32(clamp(band, 0.0, 1.0) * f32(NX - 1u) + 0.5), 0u);
  let gate = clamp(u.ctl.w, 0.0, 1.0);
  if (rnd(ser, 500u) > lev * lev * 0.9 * gate) { return bolt_dead(); }
  let x = (band - 0.5) * u.ctl.y;
  let z = (rnd(ser, 501u) - 0.5) * 1.2;
  let h = (0.30 + 0.90 * lev) * (0.6 + 0.8 * rnd(ser, 502u));
  let lean = (rnd(ser, 503u) - 0.5) * 0.9;
  let ta = f32(s) / f32(CORONA_SEGS);
  let tb = f32(s + 1u) / f32(CORONA_SEGS);
  let oa = frac_off(ser, s, 3u, 0.65) * (0.22 * h);
  let ob = frac_off(ser, s + 1u, 3u, 0.65) * (0.22 * h);
  let pa = vec3<f32>(x + lean * h * ta + oa.x, 0.03 + h * ta, z + 0.6 * oa.y);
  let pb = vec3<f32>(x + lean * h * tb + ob.x, 0.03 + h * tb, z + 0.6 * ob.y);
  let g = (0.07 + 0.36 * lev) * flash_gain();
  let tint = palette(0.1 + 0.6 * lev, band);
  return bolt_quad(pa, pb, g * (1.0 - 0.6 * ta), g * (1.0 - 0.6 * tb), 0.012, tint, vi, mirror);
}

@vertex fn vs_corona(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> BoltOut {
  return corona_segment(vi, ii, false);
}
@vertex fn vs_corona_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> BoltOut {
  return corona_segment(vi, ii, true);
}

// The capsule's Gaussian profile across, integrated along: a long line has the same brightness all the way,
// and the ends fall off with the error function.
fn line_spot(loc: vec2<f32>, len: f32, sigma: f32) -> f32 {
  let k = 0.70710678 / sigma;
  return exp(-loc.y * loc.y / (2.0 * sigma * sigma)) * 0.5 * (erf_approx((len - loc.x) * k) + erf_approx(loc.x * k));
}

@fragment fn fs_bolt(in: BoltOut) -> @location(0) vec4<f32> {
  let s = clamp(in.loc.x / max(in.len, 1e-3), 0.0, 1.0);
  let inten = mix(in.inten.x, in.inten.y, s);
  let core = line_spot(in.loc, in.len, in.sig);
  let halo = line_spot(in.loc, in.len, 3.4 * in.sig);
  // a hard stroke is white-hot, a fading one takes the colour of the theme
  let hot = clamp(0.30 * inten, 0.0, 1.0);
  let col = mix(in.tint, vec3<f32>(1.0), hot) * core + in.tint * halo * 0.30;
  return vec4<f32>(col * inten * u.exposure, 1.0);
}

// ---- the floor
struct FloorOut { @builtin(position) pos: vec4<f32>, @location(0) wp: vec3<f32> };
@vertex fn vs_floor(@builtin(vertex_index) vi: u32) -> FloorOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-FLOOR_EXTENT, FLOOR_EXTENT, c.x), 0.0, mix(-FLOOR_EXTENT, FLOOR_EXTENT, c.y));
  return FloorOut(u.vp * vec4<f32>(w, 1.0), w);
}

@fragment fn fs_floor(in: FloorOut) -> @location(0) vec4<f32> {
  let dist = length(in.wp - u.cam.xyz);
  let fade = 1.0 - smoothstep(18.0, 120.0, dist);
  let aa = line_aa(max(fwidth(in.wp.x), fwidth(in.wp.z)));
  let g = max(gridline(in.wp.x), gridline(in.wp.z)) * aa;
  var c = vec3<f32>(0.007, 0.008, 0.015) + palette(0.35, 0.5) * g * 0.06;

  // under every live strike: a pool of light that comes up in a few frames and dies away in about a second, and
  // for a low strike a thunder ring that runs out across the glass. Flash scales all of it.
  var lit = vec3<f32>(0.0);
  for (var k = 0u; k < MAX_BOLTS; k++) {
    let a = lt.bolts[4u * k];
    let b = lt.bolts[4u * k + 1u];
    let cc = lt.bolts[4u * k + 2u];
    let d4 = lt.bolts[4u * k + 3u];
    if (a.w <= 0.0 || cc.y > 0.5) { continue; }          // unused, or a ghost (no flash)
    let d = in.wp.xz - a.xy;
    let d2 = dot(d, d);
    let tint = mix(palette(0.15, b.x), vec3<f32>(1.0), 0.30);
    let age = max(a.z - LEADER_S, 0.0);                  // the floor lights when the leader reaches it
    let sg = d4.y;
    if (d2 < 16.0 * sg * sg) {
      let env = a.w * smoothstep(0.0, 0.04, age) * exp(-age / 0.28);
      lit += tint * (env * exp(-d2 / (2.0 * sg * sg)));
    }
    if (cc.w > 0.0) {
      let r = sqrt(d2);
      let ring_r = 1.0 + 7.5 * pow(age, 0.85);
      let wd = 0.25 + 0.55 * age;
      let x = (r - ring_r) / wd;
      if (abs(x) < 3.5) {
        lit += tint * (cc.w * smoothstep(0.0, 0.02, age) * exp(-age * 1.5) * exp(-x * x) * 0.95);
      }
    }
  }
  c += lit * (0.90 * u.flash) * (0.7 + 0.8 * g);

  // the spectrum line the strikes land on (bass left, treble right), and the light it throws on the glass
  let half_w = 0.5 * u.ctl.y;
  if (abs(in.wp.z) < 1.6 && abs(in.wp.x) < half_w + 1.0) {
    let t = clamp(in.wp.x / u.ctl.y + 0.5, 0.0, 1.0);
    let l = hist_m(u32(t * f32(NX - 1u) + 0.5), 0u);
    let col = palette(l * 0.85, t);
    let bar = exp(-in.wp.z * in.wp.z / (2.0 * 0.04 * 0.04));
    let glow = exp(-in.wp.z * in.wp.z / (2.0 * 0.45 * 0.45)) * l * l;
    let edge = 1.0 - smoothstep(half_w, half_w + 0.8, abs(in.wp.x));
    c += col * (bar * (0.08 + 1.5 * l) + 0.18 * glow) * edge;
  }
  return vec4<f32>(mix(BG, c, fade) * u.exposure, 1.0);
}

// ---- the splash of sparks at the strike point (the particle system's)
fn spark_colour(p: Particle) -> vec3<f32> {
  let age = 1.0 - p.life / p.max_life;
  let base = mix(palette(0.2, p.look.x), vec3<f32>(1.0), 0.45);
  let hot = mix(vec3<f32>(1.0), base, smoothstep(0.0, 0.18, age));
  let fade = clamp(p.life / (0.40 * p.max_life), 0.0, 1.0);
  return hot * p.look.z * fade * (1.0 + 1.2 * p.look.w) * (0.30 + 0.70 * u.flash);
}

@vertex fn vs_spark(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  let p = parts[ii];
  if (p.life <= 0.0) { return sprite_dead(); }
  return sprite_make(p.pos, p.vel, p.look.y, vi, spark_colour(p), false);
}

@vertex fn vs_spark_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  let p = parts[ii];
  if (p.life <= 0.0) { return sprite_dead(); }
  let v = normalize(u.cam.xyz - p.pos);
  let fres = 0.10 + 0.90 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 3.0);
  return sprite_make(p.pos, p.vel, p.look.y, vi, spark_colour(p) * (fres * exp(-0.35 * p.pos.y) * 0.9), true);
}
