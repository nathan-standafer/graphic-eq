// Lightning Storm (compute): the splash of sparks a strike throws off the glass. The CPU (`lightning.rs`) sends
// each strike's line position, strength, seed and spark count in `pu.bursts[k]` and its point on the floor in
// `pu.extra[k]`. Sparks leave in a low crown, arc under gravity and bounce once or twice before they burn out.
//
// look = (line 0..1, world radius, brightness, splash flash from the last bounce)
const G: f32 = 14.0;
const BOUNCE: f32 = 0.42;

fn dead_particle() -> Particle {
  return Particle(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 1.0, vec4<f32>(0.0));
}

fn spawn_spark(s: u32, k: u32) -> Particle {
  let bd = pu.bursts[k];
  let ex = pu.extra[k];
  let st = clamp(bd.y, 0.0, 1.0);
  let elev = 0.20 + 1.15 * rnd(s, 1u);                    // above horizontal: mostly a low crown, a few steep
  let az = 6.2831853 * rnd(s, 2u);
  let speed = (2.4 + 6.6 * st) * (0.30 + 0.70 * rnd(s, 3u));
  var p: Particle;
  p.pos = vec3<f32>(ex.x + 0.12 * cos(az), 0.03, ex.z + 0.12 * sin(az));
  p.vel = vec3<f32>(cos(elev) * cos(az), sin(elev), 0.7 * cos(elev) * sin(az)) * speed;
  p.max_life = (0.40 + 0.75 * rnd(s, 4u)) * (0.6 + 0.4 * st);
  p.life = p.max_life;
  p.look = vec4<f32>(bd.x, (0.016 + 0.014 * st) * (0.8 + 0.4 * rnd(s, 5u)), 0.55 + 0.6 * st, 0.0);
  return p;
}

fn spawn_particle(s: u32, burst: u32, n: u32) -> Particle {
  if (burst >= 6u) { return dead_particle(); }
  return spawn_spark(s, burst);
}

fn integrate(p: Particle, dt: f32) -> Particle {
  var q = p;
  q.vel.y -= G * dt;
  q.vel *= exp(-0.6 * dt);
  q.pos += q.vel * dt;
  q.life -= dt;
  q.look.w *= exp(-14.0 * dt);
  if (q.pos.y < 0.0 && q.vel.y < 0.0) {
    q.pos.y = abs(q.pos.y);                               // height is never negative
    q.vel = vec3<f32>(q.vel.x * 0.7, -q.vel.y * BOUNCE, q.vel.z * 0.7);
    q.look.w = 1.0;
    if (q.vel.y < 1.0) { q.life = min(q.life, 0.15); }
  }
  return q;
}
