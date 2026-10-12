// The draw side: each particle is a streak (a Gaussian dot dragged along its screen-space velocity, which
// is what makes fast particles read as motion at 120 Hz). A scene's vertex shader computes the colour and
// calls `sprite_make`; every scene shares the fragment shader.
@group(1) @binding(0) var<storage, read> parts: array<Particle>;
@group(1) @binding(1) var<uniform> pu: PartU;

struct SpriteOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) loc: vec2<f32>,                     // px along the streak from the head, and across it
  @location(1) @interpolate(flat) len: f32,        // streak length in px
  @location(2) @interpolate(flat) sig: f32,        // Gaussian sigma in px
  @location(3) @interpolate(flat) col: vec3<f32>,  // HDR colour, premultiplied by life and brightness
};

const TRAIL_S: f32 = 0.045;                        // seconds of velocity drawn behind the head

fn sprite_dead() -> SpriteOut {
  var o: SpriteOut;
  o.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);           // outside the screen: the quad is clipped away
  o.loc = vec2<f32>(0.0);
  o.len = 0.0;
  o.sig = 1.0;
  o.col = vec3<f32>(0.0);
  return o;
}

// One corner of a particle's quad. `size` is the world radius; `mirror` reflects the particle in the
// floor (y = 0) for the glass reflection.
fn sprite_make(pos: vec3<f32>, vel: vec3<f32>, size: f32, vi: u32, col: vec3<f32>, mirror: bool) -> SpriteOut {
  var wp = pos;
  var wv = vel;
  if (mirror) { wp.y = -wp.y; wv.y = -wv.y; }
  let c = u.vp * vec4<f32>(wp, 1.0);
  if (c.w < 0.2) { return sprite_dead(); }
  var ct = u.vp * vec4<f32>(wp - wv * TRAIL_S, 1.0);
  if (ct.w < 0.2) { ct = c; }
  let half = u.res.xy * 0.5;
  let a = c.xy / c.w * half;
  let b = ct.xy / ct.w * half;
  let focal = length(vec3<f32>(u.vp[0][1], u.vp[1][1], u.vp[2][1]));
  let r_true = size * focal * half.y / c.w;        // the world radius in pixels
  let sig = max(0.5 * r_true, 0.6);
  let kk = min(1.0, (0.5 * r_true) * (0.5 * r_true) / (sig * sig));   // a dot thinner than a pixel keeps its energy by dimming
  let d = b - a;
  let len = length(d);
  var dir = vec2<f32>(1.0, 0.0);
  if (len > 1e-3) { dir = d / len; }
  let nrm = vec2<f32>(-dir.y, dir.x);
  let reach = 4.0 * sig + 1.5;
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let k = q[vi];
  let along = select(-reach, len + reach, k.x > 0.5);
  let across = k.y * reach;
  let p = a + dir * along + nrm * across;
  var o: SpriteOut;
  o.pos = vec4<f32>(p / half, c.z / c.w, 1.0);
  o.loc = vec2<f32>(along, across);
  o.len = len;
  o.sig = sig;
  o.col = col * kk;
  return o;
}

@fragment fn fs_sprite(in: SpriteOut) -> @location(0) vec4<f32> {
  let dx = max(max(-in.loc.x, in.loc.x - in.len), 0.0);
  let d2 = dx * dx + in.loc.y * in.loc.y;
  let g = exp(-d2 / (2.0 * in.sig * in.sig));
  let hs = 1.8 * in.sig;
  let halo = exp(-d2 / (2.0 * hs * hs)) * 0.12;
  let tail = 1.0 - 0.8 * clamp(in.loc.x / max(in.len, 1e-3), 0.0, 1.0);   // the tail end of the streak is dimmer
  return vec4<f32>(in.col * (g + halo) * tail * u.exposure, 1.0);
}
