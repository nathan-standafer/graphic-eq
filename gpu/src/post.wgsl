// Post: layered bloom (threshold + 13-tap downsample chain, tent-filtered upsample chain with
// independent per-level weights), scene crossfade, and the final composite.
struct P { dir: vec2<f32>, texel: vec2<f32>, a: vec4<f32> };

struct FsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_full(@builtin(vertex_index) vi: u32) -> FsOut {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  let xy = p[vi];
  return FsOut(vec4<f32>(xy, 0.0, 1.0), vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5));
}

fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }

// ---- single-texture passes: downsample chain
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<uniform> p: P;

// a.x = threshold, a.y = knee: only the part of a pixel above the threshold feeds the bloom
fn prefilter(c: vec3<f32>) -> vec3<f32> {
  let br = max(c.r, max(c.g, c.b));
  let knee = max(p.a.y, 1e-4);
  var rq = clamp(br - p.a.x + knee, 0.0, 2.0 * knee);
  rq = rq * rq * 0.25 / knee;
  return c * max(rq, br - p.a.x) / max(br, 1e-4);
}
fn tap(uv: vec2<f32>, o: vec2<f32>) -> vec3<f32> {
  return textureSampleLevel(tex, smp, uv + o * p.texel, 0.0).rgb;
}
// A stray NaN or Inf in the scene would otherwise spread through the whole pyramid.
fn sane(c: vec3<f32>) -> vec3<f32> {
  let bad = (bitcast<vec3<u32>>(c) & vec3<u32>(0x7fffffffu)) >= vec3<u32>(0x7f800000u);
  return clamp(select(c, vec3<f32>(0.0), bad), vec3<f32>(0.0), vec3<f32>(60000.0));
}
fn tap_f(uv: vec2<f32>, o: vec2<f32>) -> vec3<f32> { return prefilter(sane(tap(uv, o))); }

// First level: threshold, then a Karis average over the five 4-tap groups so isolated bright
// pixels and thin moving lines do not flicker into the bloom.
@fragment fn fs_down0(in: FsOut) -> @location(0) vec4<f32> {
  let uv = in.uv;
  let a = tap_f(uv, vec2<f32>(-2.0, -2.0)); let b = tap_f(uv, vec2<f32>(0.0, -2.0)); let c = tap_f(uv, vec2<f32>(2.0, -2.0));
  let d = tap_f(uv, vec2<f32>(-2.0, 0.0));  let e = tap_f(uv, vec2<f32>(0.0, 0.0));  let f = tap_f(uv, vec2<f32>(2.0, 0.0));
  let g = tap_f(uv, vec2<f32>(-2.0, 2.0));  let h = tap_f(uv, vec2<f32>(0.0, 2.0));  let i = tap_f(uv, vec2<f32>(2.0, 2.0));
  let j = tap_f(uv, vec2<f32>(-1.0, -1.0)); let k = tap_f(uv, vec2<f32>(1.0, -1.0));
  let l = tap_f(uv, vec2<f32>(-1.0, 1.0));  let m = tap_f(uv, vec2<f32>(1.0, 1.0));
  let g0 = (a + b + d + e) * 0.25;
  let g1 = (b + c + e + f) * 0.25;
  let g2 = (d + e + g + h) * 0.25;
  let g3 = (e + f + h + i) * 0.25;
  let g4 = (j + k + l + m) * 0.25;
  let w0 = 0.125 / (1.0 + luma(g0));
  let w1 = 0.125 / (1.0 + luma(g1));
  let w2 = 0.125 / (1.0 + luma(g2));
  let w3 = 0.125 / (1.0 + luma(g3));
  let w4 = 0.5 / (1.0 + luma(g4));
  let col = (g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4) / (w0 + w1 + w2 + w3 + w4);
  return vec4<f32>(col, 1.0);
}
@fragment fn fs_down(in: FsOut) -> @location(0) vec4<f32> {
  let uv = in.uv;
  let a = tap(uv, vec2<f32>(-2.0, -2.0)); let b = tap(uv, vec2<f32>(0.0, -2.0)); let c = tap(uv, vec2<f32>(2.0, -2.0));
  let d = tap(uv, vec2<f32>(-2.0, 0.0));  let e = tap(uv, vec2<f32>(0.0, 0.0));  let f = tap(uv, vec2<f32>(2.0, 0.0));
  let g = tap(uv, vec2<f32>(-2.0, 2.0));  let h = tap(uv, vec2<f32>(0.0, 2.0));  let i = tap(uv, vec2<f32>(2.0, 2.0));
  let j = tap(uv, vec2<f32>(-1.0, -1.0)); let k = tap(uv, vec2<f32>(1.0, -1.0));
  let l = tap(uv, vec2<f32>(-1.0, 1.0));  let m = tap(uv, vec2<f32>(1.0, 1.0));
  let col = (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + e * 0.125 + (j + k + l + m) * 0.125;
  return vec4<f32>(col, 1.0);
}

// ---- two-texture passes: upsample/accumulate, crossfade, composite
@group(0) @binding(0) var tex0: texture_2d<f32>;
@group(0) @binding(1) var tex1: texture_2d<f32>;
@group(0) @binding(2) var smp2: sampler;
@group(0) @binding(3) var<uniform> q: P;

fn s1(uv: vec2<f32>, o: vec2<f32>) -> vec3<f32> {
  return textureSampleLevel(tex1, smp2, uv + o * q.texel, 0.0).rgb;
}
// up[i] = down[i] * weight + tent(up[i+1])  (a.x = weight of this level, a.y = 1 if there is a level below)
@fragment fn fs_up(in: FsOut) -> @location(0) vec4<f32> {
  let uv = in.uv;
  var low = s1(uv, vec2<f32>(0.0, 0.0)) * 4.0;
  low += (s1(uv, vec2<f32>(1.0, 0.0)) + s1(uv, vec2<f32>(-1.0, 0.0)) + s1(uv, vec2<f32>(0.0, 1.0)) + s1(uv, vec2<f32>(0.0, -1.0))) * 2.0;
  low += s1(uv, vec2<f32>(1.0, 1.0)) + s1(uv, vec2<f32>(-1.0, 1.0)) + s1(uv, vec2<f32>(1.0, -1.0)) + s1(uv, vec2<f32>(-1.0, -1.0));
  let same = textureSampleLevel(tex0, smp2, uv, 0.0).rgb;
  return vec4<f32>(same * q.a.x + low * (q.a.y / 16.0), 1.0);
}

// linear HDR crossfade between two scene outputs (a.x = weight of the second)
@fragment fn fs_xfade(in: FsOut) -> @location(0) vec4<f32> {
  let a = textureSampleLevel(tex0, smp2, in.uv, 0.0).rgb;
  let b = textureSampleLevel(tex1, smp2, in.uv, 0.0).rgb;
  return vec4<f32>(mix(a, b, q.a.x), 1.0);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
  return clamp((x * (2.51 * x + vec3<f32>(0.03))) / (x * (2.43 * x + vec3<f32>(0.59)) + vec3<f32>(0.14)),
               vec3<f32>(0.0), vec3<f32>(1.0));
}
fn ign(p: vec2<f32>) -> f32 {   // interleaved gradient noise
  return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}
// a.x = bloom master, a.y = vignette, a.z = time (dither seed)
@fragment fn fs_comp(in: FsOut) -> @location(0) vec4<f32> {
  var c = textureSampleLevel(tex0, smp2, in.uv, 0.0).rgb;
  if (q.a.w > 1.5) {   // debug: red = NaN, green = Inf, blue = magnitude/16
    let b = bitcast<vec3<u32>>(c) & vec3<u32>(0x7fffffffu);
    let nan = any(b > vec3<u32>(0x7f800000u));
    let inf = any(b == vec3<u32>(0x7f800000u));
    return vec4<f32>(select(0.0, 1.0, nan), select(0.0, 1.0, inf), clamp(max(c.r, max(c.g, c.b)) / 16.0, 0.0, 1.0), 1.0);
  }
  if (q.a.w > 0.5) { return vec4<f32>(pow(aces(c), vec3<f32>(1.0 / 2.2)), 1.0); }   // debug: scene only
  c += textureSampleLevel(tex1, smp2, in.uv, 0.0).rgb * q.a.x;
  if (q.a.w < -0.5) { return vec4<f32>(pow(aces(textureSampleLevel(tex1, smp2, in.uv, 0.0).rgb), vec3<f32>(1.0 / 2.2)), 1.0); }   // debug: bloom only
  let d = in.uv - vec2<f32>(0.5);
  c *= 1.0 - q.a.y * dot(d, d);                    // vignette
  c = aces(c * 1.05);
  c = pow(c, vec3<f32>(1.0 / 2.2));
  let n = ign(in.pos.xy + vec2<f32>(fract(q.a.z * 0.37) * 64.0)) + ign(in.pos.yx * 1.31 + vec2<f32>(7.0)) - 1.0;
  return vec4<f32>(c + vec3<f32>(n / 255.0), 1.0);   // triangular dither: no banding in dark gradients
}
