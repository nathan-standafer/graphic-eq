// Ferrofluid Orb, the height pass: for every texel of the longitude / latitude map, the sphere's displacement
// in that direction and the gradient of it on the sphere. xyz = the gradient (a world-space vector tangent to
// the sphere, pointing uphill), w = the height, both in fractions of the radius. Storing the analytic gradient
// instead of differencing a texture later gives smooth normals: bilinear filtering of a height map has a kink
// at every texel edge, and a sharp strip light reflected in the surface shows each one.
@group(0) @binding(0) var<uniform> o: OrbU;

@vertex fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(p[vi], 0.0, 1.0);
}

@fragment fn fs_height(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
  let dir = map_dir(frag.xy / o.b.xy);
  let w = o.b.z;
  let cos_w = cos(w);
  var h = 0.0;
  var g = vec3<f32>(0.0);

  // The spikes: cones with concave flanks, h = A (1 - alpha / w)^2 for the angle alpha from the site. The
  // square leaves a sharp tip and a base that runs into the surface smoothly, like a real Rosensweig spike.
  // Most sites are farther than w from a texel, and a dot product rejects those.
  for (var i = 0u; i < 192u; i++) {
    let s = o.sites[i];
    let c = dot(dir, s.xyz);
    if (c > cos_w && s.w > 0.0005) {
      let alpha = acos(min(c, 1.0));
      let k = 1.0 - alpha / w;
      h += s.w * k * k;
      let sa = sin(alpha);
      if (sa > 1e-4) {
        g += (s.xyz - c * dir) * (2.0 * s.w * k / (w * sa));
      }
    }
  }

  // The lobes: broad bumps, A exp(-kappa (1 - cos alpha)), which wrap around the sphere without a seam.
  let kap = o.b.w;
  for (var i = 0u; i < 11u; i++) {
    let s = o.lobes[i];
    let c = dot(dir, s.xyz);
    let e = s.w * exp(-kap * (1.0 - c));
    h += e;
    g += (s.xyz - c * dir) * (kap * e);
  }
  return vec4<f32>(g, h);
}
