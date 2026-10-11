struct P { dir: vec2<f32>, texel: vec2<f32>, a: vec4<f32> };

struct FsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_full(@builtin(vertex_index) vi: u32) -> FsOut {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  let xy = p[vi];
  return FsOut(vec4<f32>(xy, 0.0, 1.0), vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5));
}

// ---- single-texture passes (bright / blur / down)
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<uniform> p: P;

@fragment fn fs_bright(in: FsOut) -> @location(0) vec4<f32> {
  let c = textureSample(tex, smp, in.uv).rgb;
  let m = max(c.r, max(c.g, c.b));
  let k = max(m - 0.85, 0.0) / max(m, 1e-4);
  return vec4<f32>(c * k, 1.0);
}
@fragment fn fs_blur(in: FsOut) -> @location(0) vec4<f32> {
  var w = array<f32, 5>(0.2270270, 0.1945946, 0.1216216, 0.0540541, 0.0162162);
  var acc = textureSample(tex, smp, in.uv).rgb * w[0];
  for (var i = 1; i < 5; i++) {
    let o = p.dir * p.texel * f32(i) * 1.6;
    acc += textureSample(tex, smp, in.uv + o).rgb * w[i];
    acc += textureSample(tex, smp, in.uv - o).rgb * w[i];
  }
  return vec4<f32>(acc, 1.0);
}
@fragment fn fs_down(in: FsOut) -> @location(0) vec4<f32> {
  let t = p.texel;
  var c = textureSample(tex, smp, in.uv + vec2<f32>(-t.x, -t.y)).rgb;
  c += textureSample(tex, smp, in.uv + vec2<f32>(t.x, -t.y)).rgb;
  c += textureSample(tex, smp, in.uv + vec2<f32>(-t.x, t.y)).rgb;
  c += textureSample(tex, smp, in.uv + vec2<f32>(t.x, t.y)).rgb;
  return vec4<f32>(c * 0.25, 1.0);
}

// ---- composite: scene + bloom -> tonemap -> display
@group(0) @binding(0) var scene_t: texture_2d<f32>;
@group(0) @binding(1) var bloom1_t: texture_2d<f32>;
@group(0) @binding(2) var bloom2_t: texture_2d<f32>;
@group(0) @binding(3) var csmp: sampler;
@group(0) @binding(4) var<uniform> cp: P;

fn aces(x: vec3<f32>) -> vec3<f32> {
  return clamp((x * (2.51 * x + vec3<f32>(0.03))) / (x * (2.43 * x + vec3<f32>(0.59)) + vec3<f32>(0.14)),
               vec3<f32>(0.0), vec3<f32>(1.0));
}
@fragment fn fs_comp(in: FsOut) -> @location(0) vec4<f32> {
  var c = textureSample(scene_t, csmp, in.uv).rgb;
  let b1 = textureSample(bloom1_t, csmp, in.uv).rgb;
  let b2 = textureSample(bloom2_t, csmp, in.uv).rgb;
  c += (b1 * 0.7 + b2 * 1.1) * cp.a.x;
  let q = in.uv - vec2<f32>(0.5);
  c *= 1.0 - 0.55 * dot(q, q);                    // vignette
  c = aces(c * 1.05);
  return vec4<f32>(pow(c, vec3<f32>(1.0 / 2.2)), 1.0);
}
