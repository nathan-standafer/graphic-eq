// The fountain: the newest spectrum row is a line of emitters along the floor. Each continuous spawn
// picks an emitter with probability growing with its level and launches a particle upward at a speed set
// by that level, so every jet's apex follows its bin. Bass is broad, heavy arcs; treble is fine spray.
// Onsets add bursts. Particles arc under gravity, drift on a gentle wind and bounce on the floor.
const G: f32 = 14.0;
const V_MAX: f32 = 14.0;       // launch speed at full level: an apex 7 units up
const BOUNCE: f32 = 0.5;

fn dead_particle() -> Particle {
  return Particle(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 1.0, vec4<f32>(0.0));
}

fn life_for(speed: f32, r: f32) -> f32 {
  return clamp(2.2 * speed / G + 0.9 + 0.5 * r, 0.8, 4.0);
}

// A jet particle from the emitter line.
fn spawn_jet(s: u32) -> Particle {
  var xi = 0u;
  var l = 0.0;
  var ok = false;
  for (var k = 0u; k < 8u; k++) {
    let a = min(u32(rnd(s, 2u * k) * f32(NX - 1u) + 0.5), NX - 1u);
    let la = hist_m(a, 0u);
    if (rnd(s, 2u * k + 1u) < la * la) { xi = a; l = la; ok = true; break; }
  }
  if (!ok) { return dead_particle(); }
  let t = f32(xi) / f32(NX - 1u);                         // 0 bass .. 1 treble
  let x01 = (f32(xi) + rnd(s, 20u) - 0.5) / f32(NX - 1u);
  let speed = V_MAX * pow(l, 0.8) * (0.92 + 0.16 * rnd(s, 21u));
  let spray = mix(0.05, 0.28, t);                         // broad arcs hold their line; spray scatters
  let th = 6.2831853 * rnd(s, 22u);
  let rho = spray * speed * sqrt(rnd(s, 23u));
  var p: Particle;
  p.pos = vec3<f32>((x01 - 0.5) * u.ctl.y, 0.03, 0.25 * (rnd(s, 24u) - 0.5));
  p.vel = vec3<f32>(rho * cos(th) + (x01 - 0.5) * speed * 0.10, speed, 0.6 * rho * sin(th));
  p.max_life = life_for(speed, rnd(s, 25u));
  p.life = p.max_life;
  let size = mix(0.046, 0.017, pow(t, 0.7)) * (0.8 + 0.4 * rnd(s, 26u));
  p.look = vec4<f32>(t, size, l * 0.85, 0.0);
  return p;
}

// A burst from an onset: a cone of particles from the point on the line where the hit sounded.
fn spawn_burst(s: u32, k: u32) -> Particle {
  let bd = pu.bursts[k];
  let band = u32(bd.z + 0.5);
  let st = clamp(bd.y, 0.0, 1.5);
  let w = select(select(0.07, 0.05, band == 2u), 0.10, band == 0u);
  let x01 = clamp(bd.x + (rnd(s, 0u) - 0.5) * w, 0.0, 1.0);
  let elev = select(select(0.7, 0.96, band == 2u), 0.5, band == 0u) * rnd(s, 1u);   // angle from vertical, radians
  let th = 6.2831853 * rnd(s, 2u);
  let speed = V_MAX * (0.55 + 0.45 * rnd(s, 3u)) * (0.75 + 0.25 * min(st, 1.2));
  var p: Particle;
  p.pos = vec3<f32>((x01 - 0.5) * u.ctl.y, 0.03, 0.4 * (rnd(s, 4u) - 0.5));
  p.vel = vec3<f32>(sin(elev) * cos(th), cos(elev), 0.6 * sin(elev) * sin(th)) * speed;
  p.max_life = life_for(speed, rnd(s, 5u));
  p.life = p.max_life;
  let size = select(select(0.026, 0.016, band == 2u), 0.042, band == 0u) * (0.8 + 0.4 * rnd(s, 6u));
  let role = select(select(0.8, 1.0, band == 2u), 0.6, band == 0u);
  p.look = vec4<f32>(x01, size, role, 0.0);
  return p;
}

fn spawn_particle(s: u32, burst: u32, n: u32) -> Particle {
  if (burst >= 6u) { return spawn_jet(s); }
  return spawn_burst(s, burst);
}

fn integrate(p: Particle, dt: f32) -> Particle {
  var q = p;
  q.vel.y -= G * dt;
  q.vel += curl_wind(q.pos * 0.35, pu.step.y * 0.3) * (1.4 * dt);
  q.vel *= exp(-0.2 * dt);
  q.pos += q.vel * dt;
  q.life -= dt;
  q.look.w *= exp(-14.0 * dt);                             // splash flash from the last bounce
  if (q.pos.y < 0.0 && q.vel.y < 0.0) {
    q.pos.y = -q.pos.y * BOUNCE;
    q.vel = vec3<f32>(q.vel.x * 0.75, -q.vel.y * BOUNCE, q.vel.z * 0.75);
    q.look.w = 1.0;
    if (q.vel.y < 1.2) { q.life = min(q.life, 0.2); }      // settled: let it fade
  }
  return q;
}
