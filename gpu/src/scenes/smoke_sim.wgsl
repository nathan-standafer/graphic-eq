// Smoke Jets: a 2D stable-fluids solver in fragment passes, plus the shading pass that turns the dye
// into the light field the screen pass colours. Every pass writes one full-screen triangle into a
// texture of its own size. Textures: velocity (rg16f), pressure / curl / divergence (r16f), dye
// (rgba16f: x density, y heat, z band position x density, w accent) and the lit result.
//
// The domain is open at the top (the pressure is 0 above it, so smoke flows out and air can come in) and
// closed by free-slip walls at the floor and the sides. The nozzles stand half a spacing from the side
// walls, so a wall is a mirror image at exactly the spacing of the other neighbours and the outer plumes
// behave like the inner ones.
@group(0) @binding(0) var<uniform> s: SimU;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var t0: texture_2d<f32>;
@group(0) @binding(3) var t1: texture_2d<f32>;
@group(0) @binding(4) var t2: texture_2d<f32>;

@vertex fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(p[vi], 0.0, 1.0);
}

fn grid_size() -> vec2<i32> { return vec2<i32>(s.vel.xy); }

// velocity of a cell: the walls reflect the component through them (free slip), the top is open
fn vel_at(p: vec2<i32>) -> vec2<f32> {
  let sz = grid_size();
  var v = textureLoad(t0, clamp(p, vec2<i32>(0), sz - vec2<i32>(1)), 0).xy;
  if (p.y >= sz.y) { v.y = -v.y; }
  if (p.x < 0 || p.x >= sz.x) { v.x = -v.x; }
  return v;
}

// ---- vorticity
@fragment fn fs_curl(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let p = vec2<i32>(fp.xy);
  let l = vel_at(p + vec2<i32>(-1, 0));
  let r = vel_at(p + vec2<i32>(1, 0));
  let up = vel_at(p + vec2<i32>(0, -1));
  let dn = vel_at(p + vec2<i32>(0, 1));
  return vec4<f32>(0.5 * ((r.y - l.y) - (up.x - dn.x)), 0.0, 0.0, 1.0);
}

fn curl_at(p: vec2<i32>) -> f32 {
  return textureLoad(t1, clamp(p, vec2<i32>(0), grid_size() - vec2<i32>(1)), 0).x;
}

fn hash12(p: vec2<f32>, seed: f32) -> f32 {
  var q = fract(vec3<f32>(p.x, p.y, seed) * vec3<f32>(0.1031, 0.1030, 0.0973));
  q += dot(q, q.yzx + vec3<f32>(33.33));
  return fract((q.x + q.y) * q.z);
}

// ---- forces: vorticity confinement, buoyancy of the hot smoke, and the nozzles
@fragment fn fs_force(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let p = vec2<i32>(fp.xy);
  let dt = s.step.x;
  var v = textureLoad(t0, p, 0).xy;

  let c = clamp(curl_at(p), -s.extra.w, s.extra.w);
  let gx = 0.5 * (abs(curl_at(p + vec2<i32>(1, 0))) - abs(curl_at(p + vec2<i32>(-1, 0))));
  let gy = 0.5 * (abs(curl_at(p + vec2<i32>(0, -1))) - abs(curl_at(p + vec2<i32>(0, 1))));   // y up
  let n = vec2<f32>(gx, gy) / (length(vec2<f32>(gx, gy)) + 1.0e-4);
  v += s.force.x * c * vec2<f32>(n.y, -n.x) * dt;

  let uv = (vec2<f32>(p) + 0.5) * s.vel.zw;
  let heat = textureSampleLevel(t2, smp, uv, 0.0).y;
  v.y += s.force.y * heat * dt;

  // nozzles: near the floor the air is pulled toward the jet's velocity
  let hy = s.vel.y - 0.5 - f32(p.y);      // cells above the floor
  if (hy < 14.0) {
    let wy = exp(-hy * hy / 18.0);
    let grip = 1.0 - exp(-s.extra.y * dt);
    for (var i = 0u; i < 11u; i++) {
      let nz = s.noz[i];
      let dx = (f32(p.x) + 0.5 - nz.x) / nz.y;
      if (nz.z > 0.5 && abs(dx) < 3.6) {
        let w = exp(-0.5 * dx * dx) * wy * grip;
        // a slow side-to-side wobble, different for every jet (slow for bass, quicker for treble), and a little noise
        let fi = f32(i);
        let ph = vec2<f32>(fract(sin(fi * 91.7 + 1.3) * 437.58), fract(sin(fi * 47.3 + 5.1) * 253.17)) * 6.2832;
        let wob = (sin(s.misc.x * (0.7 + 0.16 * fi) + ph.x) + 0.6 * sin(s.misc.x * (1.9 + 0.23 * fi) + ph.y)) * s.force.w;
        let jit = (hash12(vec2<f32>(p), s.misc.x + s.misc.y) - 0.5) * 0.04;
        v = mix(v, vec2<f32>(nz.z * (wob + jit), nz.z), w);
      }
    }
  }
  return vec4<f32>(v, 0.0, 1.0);
}

// ---- projection
@fragment fn fs_div(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let p = vec2<i32>(fp.xy);
  let l = vel_at(p + vec2<i32>(-1, 0)).x;
  let r = vel_at(p + vec2<i32>(1, 0)).x;
  let up = vel_at(p + vec2<i32>(0, -1)).y;
  let dn = vel_at(p + vec2<i32>(0, 1)).y;
  return vec4<f32>(0.5 * ((r - l) + (up - dn)), 0.0, 0.0, 1.0);
}

// pressure of a neighbour: 0 above the open top, the cell's own through a wall
fn pres_at(p: vec2<i32>, own: f32) -> f32 {
  let sz = grid_size();
  if (p.y < 0) { return 0.0; }
  if (p.y >= sz.y || p.x < 0 || p.x >= sz.x) { return own; }
  return textureLoad(t0, p, 0).x;
}

fn jacobi(fp: vec4<f32>, keep: f32) -> vec4<f32> {
  let p = vec2<i32>(fp.xy);
  let own = textureLoad(t0, p, 0).x;
  let l = pres_at(p + vec2<i32>(-1, 0), own);
  let r = pres_at(p + vec2<i32>(1, 0), own);
  let up = pres_at(p + vec2<i32>(0, -1), own);
  let dn = pres_at(p + vec2<i32>(0, 1), own);
  let d = textureLoad(t1, p, 0).x;
  return vec4<f32>((l + r + up + dn) * keep * 0.25 - 0.25 * d, 0.0, 0.0, 1.0);
}
// the first iteration starts from the previous step's pressure, partly forgotten
@fragment fn fs_jacobi0(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> { return jacobi(fp, s.force.z); }
@fragment fn fs_jacobi(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> { return jacobi(fp, 1.0); }

@fragment fn fs_grad(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let p = vec2<i32>(fp.xy);
  let own = textureLoad(t0, p, 0).x;
  let l = pres_at(p + vec2<i32>(-1, 0), own);
  let r = pres_at(p + vec2<i32>(1, 0), own);
  let up = pres_at(p + vec2<i32>(0, -1), own);
  let dn = pres_at(p + vec2<i32>(0, 1), own);
  var v = textureLoad(t1, p, 0).xy - 0.5 * vec2<f32>(r - l, up - dn);
  v = clamp(v, vec2<f32>(-s.extra.x), vec2<f32>(s.extra.x));
  return vec4<f32>(v, 0.0, 1.0);
}

// ---- velocity advection: MacCormack, so vortices keep their strength (plain semi-Lagrangian bleeds it
// away a little every step, and a puff would never roll up into a ring that travels)
fn vel_texel(t: texture_2d<f32>, p: vec2<i32>) -> vec2<f32> {
  return textureLoad(t, clamp(p, vec2<i32>(0), grid_size() - vec2<i32>(1)), 0).xy;
}

// pass 1: t0 = velocity; the plain semi-Lagrangian result
@fragment fn fs_adv_vel_a(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = fp.xy * s.vel.zw;
  let v = textureLoad(t0, vec2<i32>(fp.xy), 0).xy;
  let back = uv - s.step.x * vec2<f32>(v.x, -v.y) * s.vel.zw;
  return vec4<f32>(textureSampleLevel(t0, smp, back, 0.0).xy, 0.0, 1.0);
}

// pass 2: t0 = velocity, t1 = the pass-1 result; the error-corrected velocity, held inside the range the
// trace-back read, with a little drag
@fragment fn fs_adv_vel_b(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let p = vec2<i32>(fp.xy);
  let uv = fp.xy * s.vel.zw;
  let old = textureLoad(t0, p, 0).xy;
  let step = s.step.x * vec2<f32>(old.x, -old.y) * s.vel.zw;
  let hat = textureLoad(t1, p, 0).xy;
  let tilde = textureSampleLevel(t1, smp, uv + step, 0.0).xy;
  let b = vec2<i32>(floor((uv - step) * s.vel.xy - vec2<f32>(0.5)));
  let a = vel_texel(t0, b);
  let c = vel_texel(t0, b + vec2<i32>(1, 0));
  let e = vel_texel(t0, b + vec2<i32>(0, 1));
  let f = vel_texel(t0, b + vec2<i32>(1, 1));
  let lo = min(min(a, c), min(e, f));
  let hi = max(max(a, c), max(e, f));
  var nv = clamp(hat + 0.5 * (old - tilde), lo, hi);
  // a little viscosity: the non-dissipative advection would otherwise keep every cell-sized ripple
  let near = 0.25 * (vel_texel(t1, p + vec2<i32>(1, 0)) + vel_texel(t1, p + vec2<i32>(-1, 0)) + vel_texel(t1, p + vec2<i32>(0, 1)) + vel_texel(t1, p + vec2<i32>(0, -1)));
  nv += s.diff.x * (near - hat);
  nv = nv / (1.0 + s.step.x * s.step.y);
  return vec4<f32>(nv, 0.0, 1.0);
}

// ---- dye: MacCormack advection (a forward step, a backward step to estimate the error, a correction held
// inside the range the trace-back read) keeps filaments sharp where plain semi-Lagrangian blurs them a
// little every one of the 120 steps a second. Nothing flows in from above the open top.
fn dye_fetch(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
  if (uv.y < 0.0) { return vec4<f32>(0.0); }
  return textureSampleLevel(t, smp, uv, 0.0);
}
fn dye_texel(t: texture_2d<f32>, p: vec2<i32>) -> vec4<f32> {
  return textureLoad(t, clamp(p, vec2<i32>(0), vec2<i32>(s.dye.xy) - vec2<i32>(1)), 0);
}

// pass 1: t0 = velocity, t1 = dye; writes the plain semi-Lagrangian result
@fragment fn fs_adv_dye_a(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = fp.xy * s.dye.zw;
  let v = textureSampleLevel(t0, smp, uv, 0.0).xy;
  return dye_fetch(t1, uv - s.step.x * vec2<f32>(v.x, -v.y) * s.vel.zw);
}

// pass 2: t0 = velocity, t1 = dye, t2 = the pass-1 result; corrects it, decays and feeds the nozzles
@fragment fn fs_adv_dye_b(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let dt = s.step.x;
  let p = vec2<i32>(fp.xy);
  let uv = fp.xy * s.dye.zw;
  let v = textureSampleLevel(t0, smp, uv, 0.0).xy;
  let step = dt * vec2<f32>(v.x, -v.y) * s.vel.zw;
  let back = uv - step;
  var d = vec4<f32>(0.0);
  if (back.y >= 0.0) {
    let hat = textureLoad(t2, p, 0);
    let tilde = dye_fetch(t2, uv + step);
    let old = textureLoad(t1, p, 0);
    let q = back * s.dye.xy - vec2<f32>(0.5);
    let b = vec2<i32>(floor(q));
    let a = dye_texel(t1, b);
    let c = dye_texel(t1, b + vec2<i32>(1, 0));
    let e = dye_texel(t1, b + vec2<i32>(0, 1));
    let f = dye_texel(t1, b + vec2<i32>(1, 1));
    let lo = min(min(a, c), min(e, f));
    let hi = max(max(a, c), max(e, f));
    d = clamp(hat + 0.5 * (old - tilde), lo, hi);
    // and a little diffusion, so the thinnest filaments blur out instead of turning into grain
    let near = 0.25 * (dye_texel(t2, p + vec2<i32>(1, 0)) + dye_texel(t2, p + vec2<i32>(-1, 0)) + dye_texel(t2, p + vec2<i32>(0, 1)) + dye_texel(t2, p + vec2<i32>(0, -1)));
    d += s.diff.y * (near - hat);
  }
  let keep = exp(-dt * s.step.z);
  let d0 = d.x;
  d.x = max(d.x * keep - dt * s.misc.w, 0.0);
  d.z = d.z * (d.x / max(d0, 1.0e-5));                    // the band position follows the density
  d.y = d.y * exp(-dt * s.step.w);
  d.w = d.w * exp(-dt * s.misc.z);

  let cell = fp.xy * (s.vel.x * s.dye.z);               // this texel's place in cells
  let hy = s.vel.y - cell.y;
  if (hy < 12.0) {
    let wy = exp(-hy * hy / 32.0);
    for (var i = 0u; i < 11u; i++) {
      let nz = s.noz[i];
      let dx = (cell.x - nz.x) / nz.y;
      if (abs(dx) < 3.6) {
        let ij = s.inj[i];
        let w = exp(-0.5 * dx * dx) * wy * dt;
        d.x += ij.x * w;
        d.y += ij.y * w;
        d.z += ij.x * ij.w * w;
        d.w += ij.z * w;
      }
    }
  }
  return vec4<f32>(min(d.x, 3.5), min(d.y, 2.5), min(d.z, 3.5), min(d.w, 2.0));
}

// ---- shading: the dye becomes brightness, emission, accent and band position (the screen pass colours them)
fn dens_at(uv: vec2<f32>) -> f32 { return textureSampleLevel(t0, smp, uv, 0.0).x; }

@fragment fn fs_shade(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let tx = s.dye.zw;
  let uv = fp.xy * tx;
  let c = textureSampleLevel(t0, smp, uv, 0.0);
  let alpha = 1.0 - exp(-s.look.x * c.x);

  // a bump normal from the density gradient (y down on screen), lit from below and in front
  let gx = dens_at(uv + vec2<f32>(tx.x, 0.0)) - dens_at(uv - vec2<f32>(tx.x, 0.0));
  let gy = dens_at(uv + vec2<f32>(0.0, tx.y)) - dens_at(uv - vec2<f32>(0.0, tx.y));
  let n = normalize(vec3<f32>(-gx * s.look.y, -gy * s.look.y, 1.0));
  let diffuse = clamp(dot(n, normalize(vec3<f32>(0.0, 0.62, 0.78))), 0.0, 1.0);

  // self-shadowing: the optical depth between this point and the lamps below it
  var occ = dens_at(uv + vec2<f32>(0.0, 5.0 * tx.y)) * 0.9;
  occ += dens_at(uv + vec2<f32>(0.0, 11.0 * tx.y)) * 1.2;
  occ += dens_at(uv + vec2<f32>(0.0, 20.0 * tx.y)) * 1.6;
  occ += dens_at(uv + vec2<f32>(0.0, 34.0 * tx.y)) * 2.2;
  let shadow = exp(-occ * 0.55);

  // light from the lamps standing on the floor: a column above each that widens and fades with height
  let ph = vec2<f32>(fp.x, s.dye.y - fp.y) / s.dye.y;      // x and height, in screen heights
  var light = 0.0;
  var tsum = 0.0;
  for (var i = 0u; i < 11u; i++) {
    let nz = s.noz[i];
    let w0 = 1.6 * nz.y / s.vel.y;
    let sg = w0 + 0.20 * ph.y;
    let dx = (ph.x - nz.x / s.vel.y) / sg;
    let a = nz.w * (w0 / sg) * exp(-0.5 * dx * dx) * exp(-ph.y * 2.6);
    light += a;
    tsum += a * s.inj[i].w;
  }
  let t_lamp = tsum / max(light, 1.0e-4);
  let t_smoke = c.z / max(c.x, 1.0e-3);

  let haze = light * s.look.w * (1.0 - alpha);
  let scatter = (light + 0.015) * (s.look.z + (1.0 - s.look.z) * diffuse) * shadow * alpha;
  let live = smoothstep(0.02, 0.3, c.x);
  return vec4<f32>(haze + 1.35 * scatter, 0.45 * c.y * live, c.w * live, mix(t_lamp, t_smoke, alpha));
}
