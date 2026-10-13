// A procedural night sky, shared by scenes that show one: stars, a nebula band, a ringed planet, a small
// planet and a moon, all tinted from the theme. Everything is a function of the view direction, so a
// scene can draw it as a backdrop and mirror it in a glass floor. It breathes with the bass: onset
// envelopes flare the stars and planet atmospheres, hits send a shockwave across the sky, and the
// sustained low end swells the haze and nebula.
//
// It runs for every pixel of the backdrop and of the mirror, so it is built to be cheap: each part
// bounds its own work (stars skip the horizon, planets are a single dot product away from the disc,
// the nebula only exists inside its band) and theme colours are only looked up where something shows.
const SKY_TAU: f32 = 6.2831853;
const SKY_HIT_ORIGIN: vec3<f32> = vec3<f32>(0.0, 0.19803, -0.98020);   // where a bass hit's shockwave starts

// The planets: direction, cosine of the angle beyond which nothing of the planet shows, and tangent of
// its angular radius.
const GIANT_C: vec3<f32> = vec3<f32>(-0.55391, 0.10575, -0.82583);
const GIANT_B: f32 = 0.992686;
const GIANT_T: f32 = 0.054053;
const SMALL_C: vec3<f32> = vec3<f32>(0.47125, 0.13035, -0.87231);
const SMALL_B: f32 = 0.998367;
const SMALL_T: f32 = 0.026006;
const MOON_C: vec3<f32> = vec3<f32>(0.10020, 0.16032, -0.98197);
const MOON_B: f32 = 0.999591;
const MOON_T: f32 = 0.013001;
const HIGH_C: vec3<f32> = vec3<f32>(0.15996, 0.54986, -0.81980);   // out of the default frame, mirrored in the near floor
const HIGH_B: f32 = 0.982881;
const HIGH_T: f32 = 0.085205;

// Integer hash of a lattice point to three values in 0..1.
fn sky_hash(p: vec3<f32>) -> vec3<f32> {
  var v = bitcast<vec3<u32>>(vec3<i32>(p));
  v = v * 1664525u + vec3<u32>(1013904223u);
  v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
  v ^= v >> vec3<u32>(16u);
  v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
  return vec3<f32>(v >> vec3<u32>(8u)) * (1.0 / 16777216.0);
}
fn sky_h1(p: vec3<f32>) -> f32 { return fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453); }
fn sky_noise(p: vec3<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let w = f * f * (3.0 - 2.0 * f);
  return mix(mix(mix(sky_h1(i), sky_h1(i + vec3<f32>(1.0, 0.0, 0.0)), w.x),
                 mix(sky_h1(i + vec3<f32>(0.0, 1.0, 0.0)), sky_h1(i + vec3<f32>(1.0, 1.0, 0.0)), w.x), w.y),
             mix(mix(sky_h1(i + vec3<f32>(0.0, 0.0, 1.0)), sky_h1(i + vec3<f32>(1.0, 0.0, 1.0)), w.x),
                 mix(sky_h1(i + vec3<f32>(0.0, 1.0, 1.0)), sky_h1(i + vec3<f32>(1.0, 1.0, 1.0)), w.x), w.y), w.z);
}
// Cloud density from a few warped sines: much cheaper than lattice noise, and smooth enough for gas.
fn sky_cloud(p: vec3<f32>) -> f32 {
  var q = p + 0.6 * sin(p.yzx * 1.7 + vec3<f32>(1.3, 0.2, 2.6));
  q += 0.4 * sin(q.zxy * 2.9 + vec3<f32>(4.1, 1.1, 0.4));
  return 0.5 + 0.5 * sin(q.x * 2.3 + q.z * 0.7) * sin(q.y * 2.9 - q.x * 0.5 + 1.3);
}

// Bass hits as shockwaves: rings of angular distance from the sky's vanishing point, widening as they
// travel and dying with age. The CPU packs live rings at the front of the list.
fn sky_wave(d: vec3<f32>) -> f32 {
  if (u.rings[0].y <= 0.0) { return 0.0; }
  var w = 0.0;
  var ang = -1.0;
  for (var k = 0u; k < 4u; k++) {
    let rg = u.rings[k];
    if (rg.y <= 0.0) { break; }
    if (rg.z < 0.5) {
      if (ang < 0.0) { ang = acos(clamp(dot(d, SKY_HIT_ORIGIN), -1.0, 1.0)); }
      let dr = (ang - (0.10 + rg.x * 0.55)) / (0.09 + 0.07 * rg.x);
      if (abs(dr) < 3.0) { w += exp(-dr * dr) * rg.y * exp(-rg.x * 1.3); }
    }
  }
  return w;
}

// The view direction on a cube around the viewer: face coordinates (u, v) in -1..1, the face (0..4) and
// 1 + u² + v², which is how much a face unit is stretched compared to the angle at the face's centre.
fn sky_face(d: vec3<f32>) -> vec4<f32> {
  let a = abs(d);
  var uv = d.xz;
  var m = a.y;
  var face = 4.0;
  if (a.z >= a.x && a.z >= a.y) {
    uv = d.xy; m = a.z; face = select(0.0, 1.0, d.z < 0.0);
  } else if (a.x >= a.y) {
    uv = d.zy; m = a.x; face = select(2.0, 3.0, d.x < 0.0);
  }
  uv /= m;
  return vec4<f32>(uv, face, 1.0 + dot(uv, uv));
}

// One layer of stars on a grid over each cube face, one star per occupied cell and kept clear of the
// cell edges so no lookup of neighbours is needed. Cells shrink toward a face's corners, so the chance
// of a star falls with them and the stars stay evenly spread over the sky. `pa` is the angular size of
// a pixel: stars never get thinner than that, and give up brightness instead. `dom` and `acc` tint some.
fn sky_stars(f: vec4<f32>, pa: f32, n: f32, dens: f32, size: f32, gain: f32, seed: f32,
             pulse: f32, wave: f32, dom: vec3<f32>, acc: vec3<f32>) -> vec3<f32> {
  let p = f.xy * n;
  let cell = floor(p);
  let h = sky_hash(vec3<f32>(cell, f.z * 41.0 + seed));
  let s = f.w;
  if (h.x > dens * inverseSqrt(s * s * s)) { return vec3<f32>(0.0); }
  let g = fract(h.zxy * 57.31 + h.yzx * 31.17 + 0.37);          // more random numbers from the same hash
  let b = g.x * g.x * g.x;                                       // many dim stars, few bright ones
  let to_cells = n * s;                                          // cells per radian here
  let base = min(size * (0.6 + 1.6 * b) * to_cells, 0.2);
  let a = max(base, 0.9 * pa * to_cells);
  let r = length(p - (cell + 0.25 + 0.5 * h.yz));               // in cells
  if (r > select(3.0 * a, 0.22, b > 0.45)) { return vec3<f32>(0.0); }   // most of a cell is far from its star
  let k = (base * base) / (a * a);                               // energy kept when thinner than a pixel
  let core = exp(-(r * r) / (a * a) * 1.6) * k;
  let halo = exp(-r / (a * 4.0)) * (1.0 - smoothstep(0.10, 0.22, r)) * 0.025 * b * sqrt(k);
  let flare = (0.78 + 0.22 * sin(u.t * (1.4 + 3.0 * g.y) + 40.0 * g.z)) * (1.0 + pulse * (1.2 + 3.5 * b) + wave * (2.0 + 5.0 * b));
  let tint = select(select(vec3<f32>(1.0), dom, g.y < 0.25), acc, g.y > 0.72);
  return mix(vec3<f32>(1.0), tint, 0.55) * (core + halo) * (0.12 + b) * gain * flare;
}

// A lit sphere of angular radius `atan(tr)` around direction `c`, with optional rings, over `bg`; the
// caller has already checked that `d` is near enough. `ring` is the outer ring radius in planet radii
// (0 for none), `rot` tilts the ring plane and the bands, `light` points toward the sun in the disc's
// frame (x right, y up, z toward the viewer). The atmosphere flares with `pulse`.
fn sky_planet(bg: vec3<f32>, d: vec3<f32>, pa: f32, c: vec3<f32>, tr: f32, ca: vec3<f32>, cb: vec3<f32>,
              bands: f32, ring: f32, rot: f32, light: vec3<f32>, rim: vec3<f32>, pulse: f32, seed: f32) -> vec3<f32> {
  let cd = dot(d, c);
  if (cd <= 0.0) { return bg; }
  let b1 = normalize(cross(c, vec3<f32>(0.0, 1.0, 0.0)));
  let b2 = cross(b1, c);
  let sun = normalize(light.x * b1 + light.y * b2 - light.z * c);
  let q = d / cd - c;                                           // offset in the plane facing the viewer
  let x = dot(q, b1) / tr;
  let y = dot(q, b2) / tr;
  let rr = x * x + y * y;
  let rho_d = sqrt(rr);
  let sr = sin(rot);
  let cr = cos(rot);
  let xr = cr * x + sr * y;
  let yr = -sr * x + cr * y;
  var col = bg;

  // the atmosphere's halo, outside the disc
  let halo = exp(-(rho_d - 1.0) * 6.5) * (1.0 - smoothstep(1.0, 2.2, rho_d)) * select(0.0, 1.0, rho_d > 0.97);
  col += rim * halo * (0.07 + 0.55 * pulse);

  // rings: an ellipse in the tilted plane; the far half passes behind the ball, the near half in front
  var ring_a = 0.0;
  var ring_c = vec3<f32>(0.0);
  if (ring > 0.0) {
    let rho = sqrt(xr * xr + (yr * yr) / 0.09);
    let span = smoothstep(ring * 0.58, ring * 0.58 + 0.05, rho) * (1.0 - smoothstep(ring - 0.08, ring, rho));
    let gq = (rho - ring * 0.8) / 0.03;
    let gap = 1.0 - 0.85 * exp(-gq * gq);
    let stripe = 0.65 + 0.35 * sin(rho * 31.0 + sky_noise(vec3<f32>(rho * 6.0, seed, 0.0)) * 5.0);
    ring_a = span * gap * stripe * 0.62;
    ring_c = mix(ca, cb, 0.5) * 0.9;
    if (yr > 0.0) { col = mix(col, ring_c, ring_a); }            // behind: the ball covers it below
  }

  // the ball
  let w = clamp(1.4 * pa / tr, 0.003, 0.25);
  let cover = 1.0 - smoothstep(1.0 - w, 1.0 + w, rho_d);
  if (cover > 0.0) {
    let z = sqrt(max(1.0 - rr, 0.0));
    let n = x * b1 + y * b2 - z * c;                            // surface normal, toward the viewer
    let lit = smoothstep(-0.12, 0.65, dot(n, sun));
    let band = 0.5 + 0.5 * sin(yr * 8.5 + 2.2 * sky_noise(vec3<f32>(xr * 1.6, yr * 2.4, seed)));
    var surf = mix(ca, cb, mix(0.5, band, bands));
    surf *= 0.8 + 0.4 * sky_noise(vec3<f32>(x * 7.0, y * 7.0, seed + 3.0));   // mottling, and craters on the moon
    let limb = pow(1.0 - z, 3.0);
    let ball = surf * (0.03 + 0.95 * lit) * (0.55 + 0.45 * z) * (1.0 + 0.5 * pulse)
             + rim * limb * (0.12 + 0.88 * lit) * (0.3 + 1.5 * pulse);
    col = mix(col, ball, cover);
  }
  if (ring > 0.0 && yr < 0.0) { col = mix(col, ring_c * (0.45 + 0.55 * smoothstep(-0.4, 0.5, light.y + 0.3)), ring_a); }
  return col;
}

// The whole sky in HDR (before exposure) for a view direction.
fn sky_color(d: vec3<f32>, pa: f32) -> vec3<f32> {
  let dom = palette(0.0, 0.5);
  let pulse = clamp(u.fx.x, 0.0, 1.5) * (0.7 + 1.2 * u.flash);   // bass onsets
  let swell = u.sus.x;                                           // sustained bass
  let wave = sky_wave(d);
  let e = max(d.y, 0.0);

  // haze along the horizon
  var c = BG + dom * (0.024 * exp(-e * 14.0)) * (1.0 + 0.8 * pulse + 0.4 * swell);
  c += mix(dom, vec3<f32>(1.0), 0.3) * wave * 0.09 * exp(-e * 4.0) * smoothstep(0.0, 0.1, d.y);

  // a nebula band crossing the view
  let off = (dot(d, vec3<f32>(0.26, -0.965, 0.0)) + 0.10) / 0.22;
  let band = exp(-off * off);
  if (band > 0.03) {
    let p = d * 3.0 + vec3<f32>(0.0, 0.0, u.t * 0.01);
    let f = 0.6 * sky_cloud(p) + 0.4 * sky_cloud(p * 2.3 + 5.0);
    let neb = pow(clamp((f - 0.42) * 2.2, 0.0, 1.0), 1.5);
    c += mix(palette(0.6, 0.5), dom, f) * neb * band * smoothstep(0.0, 0.08, d.y) * 0.055 * (1.0 + 0.9 * pulse + 0.5 * swell + wave);
  }

  // stars thin out toward the horizon (haze) and crowd into the band
  if (d.y > 0.005) {
    let face = sky_face(d);
    let acc = palette(1.0, 0.5);
    var stars = sky_stars(face, pa, 11.0, 0.30, 0.0015, 2.2, 3.0, pulse, wave, dom, acc);
    stars += sky_stars(face, pa, 28.0, 0.35 + 0.25 * band, 0.0012, 0.9, 2.0, pulse, wave, dom, acc);
    stars += sky_stars(face, pa, 70.0, 0.34 + 0.2 * band, 0.0007, 0.38, 1.0, pulse, wave, dom, acc);
    c += stars * smoothstep(0.005, 0.10, d.y);
  }

  // the planets, each only worked out near its disc
  if (dot(d, GIANT_C) > GIANT_B) {
    let sup = palette(0.6, 0.5);
    c = sky_planet(c, d, pa, GIANT_C, GIANT_T, mix(dom, vec3<f32>(1.0), 0.15) * 0.7, mix(sup, vec3<f32>(0.4), 0.2) * 0.8,
                   1.0, 2.25, -0.32, vec3<f32>(0.75, 0.45, 0.55), mix(palette(1.0, 0.5), vec3<f32>(1.0), 0.25), pulse, 1.0);
  }
  if (dot(d, SMALL_C) > SMALL_B) {
    let acc = palette(1.0, 0.5);
    c = sky_planet(c, d, pa, SMALL_C, SMALL_T, mix(acc, vec3<f32>(1.0), 0.1) * 0.75, mix(acc, palette(0.6, 0.5), 0.5) * 0.8,
                   0.6, 0.0, 0.5, vec3<f32>(-0.8, 0.35, 0.5), mix(acc, vec3<f32>(1.0), 0.25), pulse, 4.0);
  }
  if (dot(d, MOON_C) > MOON_B) {
    c = sky_planet(c, d, pa, MOON_C, MOON_T, vec3<f32>(0.75), mix(vec3<f32>(0.45), dom, 0.3),
                   0.0, 0.0, 0.0, vec3<f32>(0.7, 0.5, 0.5), mix(palette(1.0, 0.5), vec3<f32>(1.0), 0.25), pulse * 0.5, 9.0);
  }
  if (dot(d, HIGH_C) > HIGH_B) {
    c = sky_planet(c, d, pa, HIGH_C, HIGH_T, vec3<f32>(0.78, 0.8, 0.9), mix(vec3<f32>(0.4, 0.42, 0.5), palette(0.6, 0.5), 0.35),
                   0.0, 0.0, 0.0, vec3<f32>(-0.75, 0.4, 0.55), mix(palette(1.0, 0.5), vec3<f32>(1.0), 0.25), pulse * 0.7, 6.0);
  }
  return c;
}

// The fog the floor and far terrain fade into: the sky at the horizon, so they meet it seamlessly.
fn sky_fog() -> vec3<f32> {
  let pulse = clamp(u.fx.x, 0.0, 1.5) * (0.7 + 1.2 * u.flash);
  return BG + palette(0.0, 0.5) * (0.024 * (1.0 + 0.8 * pulse + 0.4 * u.sus.x));
}

// A view ray for each pixel, recovered from the view-projection matrix (rows 0, 1 and 3 hold the
// camera's right, up and forward axes scaled by the projection).
fn sky_dir(ndc: vec2<f32>) -> vec3<f32> {
  let r0 = vec3<f32>(u.vp[0][0], u.vp[1][0], u.vp[2][0]);
  let r1 = vec3<f32>(u.vp[0][1], u.vp[1][1], u.vp[2][1]);
  let r3 = vec3<f32>(u.vp[0][3], u.vp[1][3], u.vp[2][3]);
  return normalize(normalize(r3) + r0 * (ndc.x / dot(r0, r0)) + r1 * (ndc.y / dot(r1, r1)));
}
struct SkyOut { @builtin(position) pos: vec4<f32>, @location(0) ndc: vec2<f32> };
// A quad over the part of the screen the floor does not cover: the sky, and the strip of fog below the
// horizon where the floor is beyond the far plane (100 units). The horizon is a straight line in clip
// space: the rays with no elevation satisfy a + b x + c y = 0.
@vertex fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
  let r0 = vec3<f32>(u.vp[0][0], u.vp[1][0], u.vp[2][0]);
  let r1 = vec3<f32>(u.vp[0][1], u.vp[1][1], u.vp[2][1]);
  let r3 = vec3<f32>(u.vp[0][3], u.vp[1][3], u.vp[2][3]);
  let a = normalize(r3).y + 0.03 + 0.0276 * u.cam.y;            // reaches below the horizon as far as the floor is clipped
  let b = r0.y / dot(r0, r0);
  let c = r1.y / dot(r1, r1);
  var yl = -3.0;                                                // looking straight up or down, or rolled over: everything
  var yr = -3.0;
  if (c > 0.01 * length(r1) / dot(r1, r1)) {
    yl = -(a - b) / c;
    yr = -(a + b) / c;
  }
  let top = max(1.0, max(yl, yr)) + 0.5;
  var q = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, yl), vec2<f32>(1.0, yr), vec2<f32>(-1.0, top),
    vec2<f32>(1.0, yr), vec2<f32>(1.0, top), vec2<f32>(-1.0, top));
  return SkyOut(vec4<f32>(q[vi], 1.0, 1.0), q[vi]);
}
@fragment fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
  let d = sky_dir(in.ndc);
  let pa = length(fwidth(d)) * 0.58;
  // below the horizon is only the fog the far floor fades into
  if (d.y <= 0.0) { return vec4<f32>(sky_fog() * u.exposure, 1.0); }
  return vec4<f32>(sky_color(d, pa) * u.exposure, 1.0);
}
