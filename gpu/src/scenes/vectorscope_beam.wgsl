// The beam: every pair of consecutive samples is one instanced quad that deposits a Gaussian spot swept
// along the segment, with its closed-form integral so the line is analytically anti-aliased and bright
// where the beam dwells and faint where it races. The result is added to the persistence buffer, whose
// red channel is the fast flash and green the slow afterglow.
//
// Each segment runs three instances: the X/Y scope, and the left and right traces on either side.
@group(1) @binding(0) var wave: texture_2d<f32>;   // ring of (left, right, left sweep x, right sweep x)
@group(1) @binding(1) var<uniform> sc: ScopeU;

struct BeamOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) loc: vec2<f32>,                      // distance along the segment from its start, and across it (px)
  @location(1) @interpolate(flat) len: f32,
  @location(2) @interpolate(flat) kind: u32,
};

fn texel(i: u32) -> vec4<f32> {
  return textureLoad(wave, vec2<i32>(i32(i % sc.seg.z), 0), 0);
}

// Pixel position of a sample for one of the three views, and whether the beam is on there.
fn beam_point(t: vec4<f32>, kind: u32) -> vec3<f32> {
  if (kind == 0u) {
    let g = sc.area.z * sc.area.w * 0.70710678;
    return vec3<f32>(sc.area.x + (t.y - t.x) * g, sc.area.y - (t.x + t.y) * g, 1.0);   // X = (R - L)/sqrt2, Y = (L + R)/sqrt2
  }
  let box = select(sc.left, sc.right, kind == 2u);
  let x = select(t.z, t.w, kind == 2u);
  let a = select(t.x, t.y, kind == 2u);
  let on = select(0.0, 1.0, x >= 0.0 && box.z > box.x);
  let half = 0.5 * (box.w - box.y);
  return vec3<f32>(mix(box.x, box.z, max(x, 0.0)), box.y + half - a * sc.trace.x * half, on);
}

@vertex fn vs_beam(@builtin(vertex_index) vi: u32, @builtin(instance_index) inst: u32) -> BeamOut {
  let seg = inst / 3u;
  let kind = inst % 3u;
  let ta = texel(sc.seg.x + seg);
  let tb = texel(sc.seg.x + seg + 1u);
  let pa = beam_point(ta, kind);
  let pb = beam_point(tb, kind);
  // a trace segment only exists while the beam is sweeping forward; a restart or a blanked stretch has none
  var live = pa.z * pb.z;
  if (kind != 0u) {
    live *= select(0.0, 1.0, select(tb.z, tb.w, kind == 2u) >= select(ta.z, ta.w, kind == 2u));
  }
  var out: BeamOut;
  out.kind = kind;
  if (live < 0.5) {
    out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);        // outside the screen: the quad is clipped away
    out.loc = vec2<f32>(0.0);
    out.len = 0.0;
    return out;
  }
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let d = pb.xy - pa.xy;
  let len = length(d);
  let dir = select(vec2<f32>(1.0, 0.0), d / max(len, 1e-6), len > 1e-4);
  let nrm = vec2<f32>(-dir.y, dir.x);
  let reach = 3.5 * sc.beam.y;                      // far enough to catch the halo
  let along = select(-reach, len + reach, c.x > 0.5);
  let across = c.y * reach;
  let p = pa.xy + dir * along + nrm * across;
  out.pos = vec4<f32>(p.x / u.res.x * 2.0 - 1.0, 1.0 - p.y / u.res.y * 2.0, 0.0, 1.0);
  out.loc = vec2<f32>(along, across);
  out.len = len;
  return out;
}

// A spot of standard deviation `sigma` swept along a segment of length `len`, as the mean over the sweep
// (a stationary spot has peak 1): exp(-y²/2σ²) · σ√(2π)/len · ½ [erf((len-x)/σ√2) + erf(x/σ√2)].
fn swept_spot(loc: vec2<f32>, len: f32, sigma: f32) -> f32 {
  let across = exp(-loc.y * loc.y / (2.0 * sigma * sigma));
  if (len < 0.3 * sigma) {                           // nearly stationary: the mid-point spot is within a hair
    let x = loc.x - 0.5 * len;
    return across * exp(-x * x / (2.0 * sigma * sigma));
  }
  let k = 0.70710678 / sigma;
  return across * (sigma * 2.5066283 / len) * 0.5 * (erf_approx((len - loc.x) * k) + erf_approx(loc.x * k));
}

@fragment fn fs_beam(in: BeamOut) -> @location(0) vec4<f32> {
  let core = swept_spot(in.loc, in.len, sc.beam.x);
  let halo = swept_spot(in.loc, in.len, sc.beam.y);
  // the beam current follows the signal gate, so silence leaves a faint dot rather than a burning one
  let gate = mix(sc.trace.z, 1.0, clamp(u.ctl.w, 0.0, 1.0));
  let level = gate * select(1.0, sc.trace.y, in.kind != 0u);
  let fast = sc.beam.z * (core + 0.05 * halo);
  let slow = sc.beam.w * (0.30 * core + 0.55 * halo);
  return vec4<f32>(fast * level, slow * level, 0.0, 0.0);
}

// ---- fading: the persistence buffer is multiplied by a per-channel constant (the blend constant)
@vertex fn vs_fade(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(p[vi], 0.0, 1.0);
}
@fragment fn fs_fade() -> @location(0) vec4<f32> {
  return vec4<f32>(0.0);
}
