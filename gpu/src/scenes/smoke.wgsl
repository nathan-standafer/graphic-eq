// Smoke Jets' screen pass: colours the light field the shading pass left (one bilinear read), then
// draws the floor strip and the lamps the jets blow from. Colour happens here, not in the simulation
// passes, so two looks of the scene can crossfade with their own palettes.
@group(1) @binding(0) var<uniform> s: SimU;
@group(1) @binding(1) var smp: sampler;
@group(1) @binding(2) var lit: texture_2d<f32>;

@vertex fn vs_smoke(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(p[vi], 0.0, 1.0);
}

@fragment fn fs_smoke(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = fp.xy / s.scr.xy;
  let l = textureSampleLevel(lit, smp, uv, 0.0);
  let tint = palette(0.6 * l.w, l.w);
  let accent = palette(1.0, l.w);
  var c = tint * l.x + mix(tint, vec3<f32>(1.0), 0.25) * l.y + accent * l.z * (0.12 + 0.25 * u.flash);

  // the floor strip, and a lamp in every nozzle that burns with its band
  let fh = s.scr.w;
  let hy = s.scr.y - fp.y;                 // px above the bottom edge
  if (hy < fh + 28.0) {
    let on_floor = 1.0 - smoothstep(fh - 1.0, fh + 0.5, hy);
    var lamp = vec3<f32>(0.0);
    for (var i = 0u; i < 11u; i++) {
      let nz = s.noz[i];
      let t = s.inj[i].w;
      let dx = fp.x - nz.x * s.scr.z;
      let ws = max(0.35 * nz.y * s.scr.z, 5.0);
      let level = nz.w;
      let col = palette(0.6 * t, t);
      let core = exp(-0.5 * dx * dx / (ws * ws));
      let up = max(hy - fh, 0.0);
      // a bright pad on the floor, and a short flame standing over it
      lamp += col * core * (on_floor * (0.18 + 3.2 * level) + level * exp(-up / 7.0) * 1.2 * (1.0 - on_floor));
    }
    var strip = vec3<f32>(0.004, 0.005, 0.011) + palette(0.0, 0.5) * 0.010;
    strip += palette(0.0, 0.5) * 0.10 * (1.0 - smoothstep(0.0, 1.4, abs(hy - fh)));   // the edge catches light
    c = mix(c, strip, on_floor) + lamp;
  }
  return vec4<f32>(c * u.exposure, 1.0);
}
