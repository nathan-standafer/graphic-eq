// Ripple Pond's offscreen passes: one step of the damped wave equation on the height field, the slope pass that
// turns the field into what the water shader reads, and the box filter that builds that texture's mip chain.
// Every pass draws one full-screen triangle into a texture of its own size.
//
// State (Rg32Float, read only with textureLoad): x = height, y = its velocity. The equation is
//   v' = v e^(-gamma dt) + dt c^2 laplacian(h),   h' = h + dt v'
// (semi-implicit Euler, stable for c dt < 1/sqrt(2) cells), with a little restoring leak on h so a displacement
// that waves have left behind does not stay, a sponge that raises the damping toward the pond's edge so rings leave
// instead of coming back, and a clamp on both so a dense passage cannot blow the field up.
@group(0) @binding(0) var<uniform> r: RpU;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var t0: texture_2d<f32>;

@vertex fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(p[vi], 0.0, 1.0);
}

fn st(p: vec2<i32>) -> vec2<f32> {
  return textureLoad(t0, clamp(p, vec2<i32>(0), vec2<i32>(r.grid.xy) - vec2<i32>(1)), 0).xy;
}

@fragment fn fs_step(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let p = vec2<i32>(fp.xy);
  let s0 = st(p);
  let lap = st(p + vec2<i32>(-1, 0)).x + st(p + vec2<i32>(1, 0)).x + st(p + vec2<i32>(0, -1)).x + st(p + vec2<i32>(0, 1)).x - 4.0 * s0.x;
  let dt = r.step.x;
  let q = vec2<f32>(p) + vec2<f32>(0.5);
  let d = min(min(q.x, r.grid.x - q.x), min(q.y, r.grid.y - q.y));
  let sp = clamp(1.0 - d / r.damp.z, 0.0, 1.0);
  let gamma = r.damp.x + r.damp.y * sp * sp;
  var v = s0.y * exp(-gamma * dt) + dt * r.step.y * lap;
  var h = (s0.x + dt * v) * exp(-r.look.x * dt);
  // a drop: a Ricker wavelet of displacement (zero mean, so it leaves a clean ring and no lasting hump), added in
  // pieces over the frame's steps
  for (var i = 0u; i < u32(r.step.w); i++) {
    let im = r.imp[i];
    let dq = (q - im.xy) / im.z;
    let q2 = 0.5 * dot(dq, dq);
    if (q2 < 12.0) { h += im.w * r.step.z * (1.0 - q2) * exp(-q2); }
  }
  h = clamp(h, -r.damp.w, r.damp.w);
  v = clamp(v, -r.look.y, r.look.y);
  return vec4<f32>(h, v, 0.0, 1.0);
}

// What the water reads: the slope of the height per world unit in x and z, and the height. Central differences
// of the exact grid values, so a bilinear read of this texture is smooth where one of the height itself has a
// kink at every texel edge.
@fragment fn fs_slope(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let p = vec2<i32>(fp.xy);
  let per_unit = r.grid.x / r.pond.z;
  let gx = (st(p + vec2<i32>(1, 0)).x - st(p + vec2<i32>(-1, 0)).x) * 0.5 * per_unit;
  let gz = (st(p + vec2<i32>(0, 1)).x - st(p + vec2<i32>(0, -1)).x) * 0.5 * per_unit;
  return vec4<f32>(gx, gz, st(p).x, 0.0);
}

// The next mip: a bilinear read at the centre of each 2x2 block is its mean.
@fragment fn fs_mip(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let dst = vec2<f32>(textureDimensions(t0)) * 0.5;
  return textureSampleLevel(t0, smp, fp.xy / dst, 0.0);
}
