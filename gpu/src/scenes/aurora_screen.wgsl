// Aurora, screen pass (full resolution, joins the scene's single-sample pass). It shows the half-resolution
// ribbon target over a night sky, behind a low treeline, and mirrors it in a still lake. The ribbon target
// holds palette-role weights; the colours come from the theme here.
//
// Needs sky.wgsl in front (sky_dir, sky_face, sky_stars, sky_cloud) and aurora_common.wgsl.
@group(1) @binding(1) var aur: texture_2d<f32>;
@group(1) @binding(2) var aur_smp: sampler;

@vertex fn vs_aur(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(p[vi], 0.0, 1.0);
}

// The theme's colour for a pixel of the ribbon target. The brightest light burns toward white.
fn aur_light(wv: vec3<f32>, x: f32) -> vec3<f32> {
  let tot = wv.x + wv.y + wv.z;
  // where several ribbons pile up the colour compresses instead of adding up to white
  let c = (wv.x * palette(0.0, x) + wv.y * palette(0.6, x) + wv.z * palette(1.0, x)) / (1.0 + 0.45 * tot);
  return c + vec3<f32>(0.07) * clamp(tot - 1.2, 0.0, 4.0);
}

// The night sky above the horizon: a faint glow along it, a pale band of the Milky Way, and stars.
fn aur_sky(d: vec3<f32>, pa: f32) -> vec3<f32> {
  let e = max(d.y, 0.0);
  let dom = palette(0.0, 0.5);
  var c = BG + vec3<f32>(0.0035, 0.0075, 0.0150) * exp(-e * 7.0) + dom * (0.004 * exp(-e * 14.0));
  let off = (dot(d, vec3<f32>(-0.38, 0.925, 0.0)) - 0.16) / 0.18;
  let band = exp(-off * off);
  if (band > 0.12) {
    let f = sky_cloud(d * 3.2 + vec3<f32>(0.0, 0.0, u.t * 0.004));
    let neb = pow(clamp((f - 0.40) * 2.0, 0.0, 1.0), 1.5);
    c += vec3<f32>(0.50, 0.62, 0.85) * neb * band * smoothstep(0.0, 0.08, d.y) * 0.032;
  }
  if (d.y > 0.004) {
    let face = sky_face(d);
    let acc = palette(1.0, 0.5);
    var stars = sky_stars(face, pa, 11.0, 0.30, 0.0015, 2.2, 3.0, 0.0, 0.0, dom, acc);
    stars += sky_stars(face, pa, 28.0, 0.35 + 0.25 * band, 0.0012, 0.9, 2.0, 0.0, 0.0, dom, acc);
    stars += sky_stars(face, pa, 70.0, 0.34 + 0.2 * band, 0.0007, 0.38, 1.0, 0.0, 0.0, dom, acc);
    c += stars * smoothstep(0.004, 0.08, d.y);
  }
  return c;
}

// Height (px) of the treeline over the horizon at column x: low hills with clumps of pines on them.
fn tree_h(x: f32, k: f32) -> f32 {
  let xs = x / k;
  let hills = 17.0 + 11.0 * sin(xs * 0.0023 + 1.7) + 7.0 * sin(xs * 0.0061 + 0.4) + 3.0 * sin(xs * 0.0149 + 2.9);
  let cell = floor(xs / 12.0);
  let tri = 1.0 - abs(2.0 * fract(xs / 12.0) - 1.0);
  let clump = smoothstep(0.40, 0.62, vnoise(xs * 0.0042 + 3.0));
  return max(hills + clump * tri * (22.0 + 48.0 * hash11(cell)), 3.0) * k;
}

@fragment fn fs_aur(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let res = u.res.xy;
  let uv = fp.xy / res;
  let yh = au.a.x;
  let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
  let dir = sky_dir(ndc);
  let pa = length(fwidth(dir)) * 0.58;
  let k = res.y / 1440.0;
  let dpx = (uv.y - yh) * res.y;          // pixels below the horizon (negative above it)
  let th = tree_h(fp.x, k);
  var col = vec3<f32>(0.0);

  if (dpx <= 0.0) {
    let m = textureSampleLevel(aur, aur_smp, uv, 0.0);
    col = aur_sky(dir, pa) * (1.0 - m.a) + aur_light(m.rgb, uv.x);
    col = mix(col, vec3<f32>(0.0004, 0.0006, 0.0012), smoothstep(-1.0, 1.0, th + dpx));   // the treeline, a black cutout
  } else {
    // the lake mirrors the sky about the horizon; ripples stretch and wobble the image more toward the viewer
    let dn = (uv.y - yh) / max(1.0 - yh, 0.05);
    let amp = (0.0003 + 0.0011 * au.c.y) * (0.3 + 1.7 * dn);
    let wob = sin(dpx * 0.085 * (1.0 / max(k, 0.3)) + au.b.y * 0.9 + 1.5 * sin(uv.x * 7.0 + au.b.y * 0.13)) + 0.4 * sin(dpx * 0.21 * (1.0 / max(k, 0.3)) - au.b.y * 1.4 + uv.x * 19.0);
    let my = yh - (uv.y - yh);
    let mu = vec2<f32>(clamp(uv.x + wob * amp, 0.0, 1.0), clamp(my + wob * amp * 0.6, 0.0, 1.0));
    let stretch = (0.006 + 0.020 * dn);
    let m = 0.25 * textureSampleLevel(aur, aur_smp, mu, 0.0)
          + 0.40 * textureSampleLevel(aur, aur_smp, vec2<f32>(mu.x, clamp(mu.y - stretch, 0.0, 1.0)), 0.0)
          + 0.35 * textureSampleLevel(aur, aur_smp, vec2<f32>(mu.x, clamp(mu.y - 2.0 * stretch, 0.0, 1.0)), 0.0);
    let refl = aur_light(m.rgb, uv.x);
    let fres = mix(0.55, 0.14, smoothstep(0.0, 0.8, dn));
    let dom = palette(0.0, uv.x);
    col = vec3<f32>(0.0030, 0.0045, 0.0085) * (1.0 - 0.6 * dn) + dom * (0.006 * exp(-dn * 16.0)) + refl * fres;
    // the trees stand in the water too
    col *= 1.0 - 0.92 * smoothstep(-2.0, 2.0, th * 0.9 - dpx);
  }
  return vec4<f32>(col * u.exposure, 1.0);
}
