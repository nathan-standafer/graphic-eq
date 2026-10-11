// Spectrum Tunnel. The wall is the history texture wrapped around a tube: bass at the bottom,
// treble at the top, left channel on the left wall (instance 0), right channel on the right
// (instance 1). Loud bins protrude inward. Newest rows are nearest the camera and recede.
const R: f32 = 2.3;               // tube radius (vertical); the horizontal radius is R * ellipse
const DEPTH: f32 = 0.40;          // how far a full-scale bin protrudes inward (fraction of radius)
const ROW_SP: f32 = 0.16;         // distance between history rows
const Z_END: f32 = 30.72;         // NZ * ROW_SP
const RING_V: f32 = 12.5;         // ring speed down the tunnel (the history scrolls at 9.6 /s)
const RING_Z0: f32 = 0.0;

fn ellipse() -> f32 { return u.ctl.y; }

// Height of a wall bin, lightly smoothed across neighbouring bins so the wall reads as a surface
// rather than a bar chart. The two walls meet at the bottom and top seams, so near them the
// channels blend toward their mean to avoid a step in the radius.
fn tun_h(xi: u32, zr: u32, side: u32) -> f32 {
  let xm = max(xi, 1u) - 1u;
  let xp = min(xi + 1u, NX - 1u);
  let v = 0.25 * hist_h2(xm, zr) + 0.5 * hist_h2(xi, zr) + 0.25 * hist_h2(xp, zr);
  let own = select(v.x, v.y, side == 1u);
  let e = f32(min(xi, NX - 1u - xi));
  return mix(0.5 * (v.x + v.y), own, smoothstep(0.0, 5.0, e));
}

// Rows run from -PAD (behind the camera) to NZ-1 (oldest). Rows before the newest repeat it, so
// the tube continues past the viewer and its open end is never in view.
const PAD: i32 = 36;

fn tun_pos(xi: u32, zi: i32, side: u32) -> vec3<f32> {
  let h = tun_h(xi, u32(max(zi, 0)), side);
  let s = f32(xi) / f32(NX - 1u);
  // right wall: bottom (-90 deg) up the right side to the top (+90 deg); left wall: round the other way
  let th = -PI * 0.5 + select(-PI, PI, side == 1u) * s;
  let zz = (f32(zi) + u.res.z) * ROW_SP;
  // sustained bass makes the whole tube breathe; mids send a slow swell down it
  let breath = 1.0 + u.ctl.x * (0.045 * u.sus.x + 0.025 * u.sus.y * sin(zz * 0.9 - u.t * 2.2));
  let r = (1.0 - DEPTH * h) * breath;
  return vec3<f32>(cos(th) * R * ellipse() * r, sin(th) * R * r, -zz);
}

struct TunOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) wp: vec3<f32>,
  @location(1) n: vec3<f32>,
  @location(2) cell: vec2<f32>,
  @location(3) h: f32,
  @location(4) @interpolate(flat) side: u32,
};

@vertex fn vs_tun(@builtin(vertex_index) vi: u32, @builtin(instance_index) side: u32) -> TunOut {
  var cs = array<vec2<u32>, 6>(
    vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(0u, 1u),
    vec2<u32>(1u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u));
  let cell = vi / 6u;
  let o = cs[vi % 6u];
  let xi = cell % (NX - 1u) + o.x;
  let zi = i32(cell / (NX - 1u) + o.y) - PAD;
  let p = tun_pos(xi, zi, side);
  // smooth normal from the neighbouring vertices, pointing toward the tube axis
  let px0 = tun_pos(max(xi, 1u) - 1u, zi, side);
  let px1 = tun_pos(min(xi + 1u, NX - 1u), zi, side);
  let pz0 = tun_pos(xi, max(zi - 1, -PAD), side);
  let pz1 = tun_pos(xi, min(zi + 1, i32(NZ) - 1), side);
  let cr = cross(px1 - px0, pz1 - pz0);
  var n = cr / max(length(cr), 1e-9);
  if (dot(n, vec3<f32>(-p.x, -p.y, 0.0)) < 0.0) { n = -n; }
  var out: TunOut;
  out.pos = u.vp * vec4<f32>(p, 1.0);
  out.wp = p;
  out.n = n;
  out.cell = vec2<f32>(f32(xi), f32(zi));
  out.h = tun_h(xi, u32(max(zi, 0)), side);
  out.side = side;
  return out;
}

@fragment fn fs_tun(in: TunOut) -> @location(0) vec4<f32> {
  let n = in.n / max(length(in.n), 1e-4);
  let to_cam = u.cam.xyz - in.wp;
  let dist = length(to_cam);
  let v = to_cam / dist;
  let age = clamp(in.cell.y / f32(NZ - 1u), 0.0, 1.0);         // 0 newest .. 1 oldest
  let xpos = in.cell.x / f32(NX - 1u);                         // 0 bass .. 1 treble
  // the extruded pad past the newest row fades out, so the first thing the eye meets is live data
  let pad_fade = smoothstep(-30.0, -2.0, in.cell.y);
  let fade = pow(1.0 - age, 1.5) * exp(-dist * 0.04) * pad_fade;
  let h = clamp(in.h, 0.0, 1.0);                               // MSAA can extrapolate an attribute slightly outside its range
  let lit = smoothstep(0.22, 0.95, h);                         // only the loud part of the spectrum glows

  let dom = palette(0.0, xpos);
  let sup = palette(0.6, xpos);
  let acc = palette(1.0, xpos);
  // colour follows level from dominant through supporting; the accent is kept for the very loudest
  let heat = palette(0.62 * smoothstep(0.0, 0.9, h) + 0.38 * smoothstep(0.88, 1.0, h), xpos);

  // structure: a rib per two history rows, a seam per four bins; lines fade out when sub-pixel
  let fw = max(fwidth(in.cell.y), fwidth(in.cell.x));
  let aa = 1.0 - smoothstep(0.30, 0.75, fw);
  let rib = line(in.cell.y * 0.5) * aa;
  let spine = line(in.cell.x * 0.25) * aa;

  // a lamp just ahead of the camera lights the near wall and catches the ridges
  let lamp = u.cam.xyz + vec3<f32>(0.0, 0.2, -3.0);
  let ld = lamp - in.wp;
  let l = normalize(ld);
  let att = (1.0 / (1.0 + dot(ld, ld) * 0.035)) * (0.7 + 0.6 * u.sus.w);   // the lamp follows the slow energy trend
  let diff = max(dot(n, l), 0.0);
  let hv = l + v;
  let spec = pow(max(dot(n, hv / max(length(hv), 1e-4)), 0.0), 36.0);
  let fres = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 4.0);

  // dark glossy panel, tinted by the theme, with the lamp raking across it
  var c = mix(vec3<f32>(0.005, 0.007, 0.014), dom * 0.026, 0.4 + 0.6 * lit) * (0.35 + 4.0 * diff * att);
  c += heat * (0.008 + 0.13 * lit * lit);                                   // the panel glows where the spectrum is loud
  c += heat * rib * (0.06 + 1.10 * lit);                                    // ribs
  c += heat * spine * (0.015 + 0.20 * lit);                                 // seams
  c += mix(sup, vec3<f32>(1.0), 0.4) * spec * att * (0.05 + 0.60 * lit);    // glints along the ridges
  c += sup * fres * 0.030 * (0.4 + u.sus.z);                                // sheen at grazing angles

  // high-frequency accents: a few treble cells flicker as small dots
  let tw = floor(u.t * 14.0);
  let hs = hash21(floor(in.cell) + vec2<f32>(tw * 7.3, tw * 1.7));
  let dd = length(fract(in.cell) - vec2<f32>(0.5));
  let dot_mask = 1.0 - smoothstep(0.10, 0.26, dd);
  let spark = step(0.985 - 0.02 * u.fx.z, hs) * smoothstep(0.5, 0.9, xpos) * smoothstep(0.2, 0.6, h) * dot_mask;
  c += mix(acc, vec3<f32>(1.0), 0.5) * spark * u.fx.z * 2.4 * u.flash;

  // onset rings: bright bands travelling down the tunnel, strongest where the hit's spectrum was
  var ring = vec3<f32>(0.0);
  let zdist = -in.wp.z;
  for (var k = 0u; k < 8u; k++) {
    let rg = u.rings[k];
    if (rg.y <= 0.0) { break; }                 // the CPU packs live rings at the front
    let w = (0.09 + 0.06 * rg.x) * clamp(dist * 0.2, 0.35, 1.2);   // thinner near the viewer, where perspective magnifies it
    let d = (zdist - (RING_Z0 + RING_V * rg.x)) / w;
    if (abs(d) > 4.0) { continue; }             // a ring is a thin band: most pixels skip all of this
    let prof = exp(-0.5 * d * d);
    let life = exp(-rg.x * 0.85) * smoothstep(0.0, 0.03, rg.x);
    let dx = (xpos - rg.w) / 0.32;
    let loc = 0.45 + 0.55 * exp(-dx * dx);
    var kc = mix(acc, vec3<f32>(1.0), 0.18);
    if (rg.z > 0.5 && rg.z < 1.5) { kc = mix(sup, vec3<f32>(1.0), 0.15); }
    if (rg.z > 1.5) { kc = mix(dom, vec3<f32>(1.0), 0.3); }
    ring += kc * prof * life * rg.y * loc;
  }
  c += ring * (0.5 + 1.0 * lit) * u.flash;

  c *= fade;
  return vec4<f32>(c * u.exposure, 1.0);
}

// ---- a soft light at the far end of the tunnel: the vanishing point
struct CapOut { @builtin(position) pos: vec4<f32>, @location(0) q: vec2<f32> };
@vertex fn vs_cap(@builtin(vertex_index) vi: u32) -> CapOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
    vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(c.x * R * ellipse() * 1.1, c.y * R * 1.1, -Z_END);
  return CapOut(u.vp * vec4<f32>(w, 1.0), c);
}
@fragment fn fs_cap(in: CapOut) -> @location(0) vec4<f32> {
  let r2 = dot(in.q, in.q);
  let g = exp(-r2 * 7.0);
  let col = mix(palette(0.0, 0.5), palette(0.6, 0.5), 0.4);
  return vec4<f32>(col * g * (0.10 + 0.22 * u.sus.w) * u.exposure, 1.0);
}
