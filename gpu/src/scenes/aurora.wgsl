// Aurora, ribbon pass. Each ribbon is a vertical sheet standing at its own distance in front of the camera,
// folded in depth by a pair of slow waves. Screen x is frequency on every ribbon (bass at the left); the
// nearest sheet shows the newest spectrum and each farther one shows older history, so a hit appears near
// and then echoes back into the distance. A column's height follows its bin's level.
//
// The light is a bright, sharp lower edge with rays rising from it and fading out, brighter where a fold
// turns the sheet edge-on to the viewer (the pleats). Ribbons are drawn farthest first and each dims what
// is behind it a little, so overlapping curtains keep their shapes instead of adding up to white.
// The output is premultiplied: rgb = weights of the palette roles, a = how much it dims what lies behind.

struct Rib {
  dist: f32, base: f32, height: f32, delay: f32,
  fold: f32, f1: f32, p1: f32, f2: f32,
  p2: f32, gain: f32, soft: f32, rayf: f32,
  seed: f32, arc: f32, lag: f32, rate: f32,
};
fn rib_get(k: u32) -> Rib {
  let a = au.rib[4u * k];
  let b = au.rib[4u * k + 1u];
  let c = au.rib[4u * k + 2u];
  let d = au.rib[4u * k + 3u];
  return Rib(a.x, a.y, a.z, a.w, b.x, b.y, b.z, b.w, c.x, c.y, c.z, c.w, d.x, d.y, d.z, d.w);
}

// A bin's level `pos` rows back (fractional: interpolated in time; row 0 is the newest).
fn hist_at(bin: u32, pos: f32) -> f32 {
  let i0 = floor(pos);
  let r = u32(i0);
  return mix(hist_m(bin, r), hist_m(bin, r + 1u), pos - i0);
}
// The level at `x` (0..1 along the spectrum), smoothed over a few bins so the sheet reads as a surface.
fn col_level(x: f32, pos: f32) -> f32 {
  let bf = clamp(x, 0.0, 1.0) * f32(NX - 1u);
  let b0 = i32(floor(bf));
  let fr = bf - f32(b0);
  var s = 0.0;
  var w = 0.0;
  for (var k = -3; k <= 4; k++) {
    let bi = u32(clamp(b0 + k, 0, i32(NX) - 1));
    let wk = max(0.0, 1.0 - abs(f32(k) - fr) / 3.0);
    s += hist_at(bi, pos) * wk;
    w += wk;
  }
  return s / w;
}

struct RibOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) v: f32,          // height over the base in units of the column's own height
  @location(1) s: f32,          // lateral coordinate on the sheet (1 = the screen edge at that distance)
  @location(2) bright: f32,     // the column's brightness: level, pleat, lateral fade, signal gate
  @location(3) xpos: f32,       // 0..1 along the spectrum
  @location(4) @interpolate(flat) rid: u32,
};

@vertex fn vs_rib(@builtin(vertex_index) vi: u32, @builtin(instance_index) k: u32) -> RibOut {
  var cs = array<vec2<u32>, 6>(
    vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(0u, 1u),
    vec2<u32>(1u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u));
  let cell = vi / 6u;
  let o = cs[vi % 6u];
  let rb = rib_get(k);
  let s = (f32(cell + o.x) / f32(NCOL - 1u) * 2.0 - 1.0) * XS;
  let xpos = clamp(0.5 + 0.5 * s, 0.0, 1.0);
  let d = rb.dist;
  let th = au.a.y;

  // the sheet's fold in depth, and how edge-on it turns to the viewer there
  let fa = rb.fold * au.c.x;
  let a1 = rb.f1 * s + rb.p1 + au.a.w * rb.rate;
  let a2 = rb.f2 * s + rb.p2 + au.a.w * rb.rate * 1.31;
  let dz = d * fa * (0.65 * sin(a1) + 0.35 * sin(a2));
  let dzds = d * fa * (0.65 * rb.f1 * cos(a1) + 0.35 * rb.f2 * cos(a2));
  let slope = dzds / (d * th);
  let pleat = min(pow(1.0 + slope * slope, 0.75), 2.2);

  // this column's level, a little behind the newest row so the time interpolation is smooth
  let pos = rb.delay + 1.0 - u.res.z;
  let h = clamp(col_level(xpos, pos) * (0.80 + 0.55 * xpos), 0.0, 1.0);   // the spectrum slopes down toward the treble
  let hcol = 0.10 + 0.90 * pow(h, 0.85);                                   // a low, dim glow when nothing plays
  let arc = rb.arc * (0.7 * sin(s * 1.6 + rb.p1 * 0.5 + au.a.w * rb.rate * 0.4) + 0.3 * sin(s * 3.9 + rb.p2 + au.a.w * rb.rate * 0.7));
  let base = rb.base + au.c.z + arc;
  let ht = rb.height * hcol * (0.85 + 0.30 * au.b.w);
  let vv = select(V_LO, V_HI, o.y == 1u);
  let p = vec3<f32>(s * d * th, EYE_Y + d * (base + ht * vv), -(d + dz));

  let side = 1.0 - smoothstep(1.12, 1.38, abs(s));
  var out: RibOut;
  out.pos = u.vp * vec4<f32>(p, 1.0);
  out.v = vv;
  out.s = s;
  out.bright = (0.16 + 0.84 * pow(h, 1.3)) * pleat * side * au.b.z;
  out.xpos = xpos;
  out.rid = k;
  return out;
}

@fragment fn fs_rib(in: RibOut) -> @location(0) vec4<f32> {
  let rb = rib_get(in.rid);
  let v = in.v;
  let w = max(fwidth(v), 1e-4);
  let foot = smoothstep(-w, w, v);                     // the base edge, anti-aliased analytically
  let soft = rb.soft;
  let near = f32(in.rid) / max(au.a.z - 1.0, 1.0);     // 0 farthest .. 1 nearest

  // rays: vertical striations that lean a little with height, shorter or longer from ray to ray. Each finer
  // octave fades out as it nears a pixel in size (the target is half resolution), so nothing shimmers.
  let sx = in.s * rb.rayf + rb.seed * 7.0 + au.b.x * (0.55 + 0.9 * rb.rate) + max(v, 0.0) * 5.0 * sin(in.s * 1.7 + rb.seed);
  let fw = fwidth(sx);
  if (foot <= 0.0 || in.bright <= 0.0) { return vec4<f32>(0.0); }
  let o2 = 0.3 * (1.0 - smoothstep(0.25, 0.55, fw * 2.3));
  let o3 = 0.2 * (1.0 - smoothstep(0.25, 0.55, fw * 5.1));
  let o1 = 0.5 + (0.3 - o2) + (0.2 - o3);            // what the faded octaves give up goes to the coarse one
  let rays = o1 * vnoise(sx) + o2 * vnoise(sx * 2.3 + 17.0) + o3 * vnoise(sx * 5.1 + 41.0);
  let streak = smoothstep(0.20, 0.80, rays);
  let contrast = clamp((0.90 - 0.55 * soft) * (1.0 + 0.7 * u.fx.z * u.flash), 0.0, 1.0);
  let rayv = mix(1.0, streak, contrast);
  let len = 0.55 + 0.50 * vnoise(sx * 0.37 + 5.0 + au.b.x * 0.05);
  let fall = clamp(1.0 - max(v, 0.0) / len, 0.0, 1.0);
  let body = pow(fall, 2.1) * rayv;

  // the sharp, bright lower edge
  let edge = exp(-max(v, 0.0) / (0.030 + 0.070 * soft)) * (1.3 - 0.95 * soft) * (0.55 + 0.45 * rayv);

  let light = foot * (0.30 * body + edge) * in.bright * rb.gain;

  // colour: dominant at the edge, supporting through the body, the accent only up in the thin tops
  var t = clamp(pow(max(v, 0.0), 2.2) * 0.95, 0.0, 1.0);
  t = mix(t, 0.5 + 0.1 * t, 0.5 * soft);
  var wv = role_w(t) * light;

  // small highlights run up the rays where a hit landed, a little later on each ribbon behind the nearest
  var hl = 0.0;
  for (var j = 0u; j < 8u; j++) {
    let rg = u.rings[j];
    if (rg.y <= 0.0) { break; }
    let age = rg.x - rb.lag;
    if (age <= 0.0) { continue; }
    let wd = mix(0.011, 0.005, clamp(rg.z * 0.5, 0.0, 1.0)) * (1.0 + 0.6 * age);
    let dx = (in.xpos - rg.w) / wd;
    if (abs(dx) > 3.5) { continue; }
    let head = 1.15 * (1.0 - exp(-age * 5.0));
    let dv = v - head;
    let prof = select(exp(-dv * dv / 0.0016), exp(dv / 0.14), dv <= 0.0);
    hl += exp(-dx * dx) * prof * exp(-age * 3.4) * smoothstep(0.0, 0.03, age) * rg.y * (0.35 + 0.65 * streak);
  }
  let flare = hl * u.flash * 1.1 * foot * rb.gain * (1.0 - 0.5 * soft) * au.b.z * smoothstep(0.0, 0.05, v + 0.05);
  wv += vec3<f32>(0.35, 0.60, 0.55) * flare;

  let occl = clamp((light + flare) * 0.45 * (0.35 + 0.65 * near), 0.0, 0.6);
  return vec4<f32>(wv, occl);
}
