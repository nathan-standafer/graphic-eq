// Sparks over the 3D columns. Each lit column sheds embers from the top of its strip, in proportion to
// its level; when a column's peak cap jumps five or more LED segments the CPU asks for a burst of sparks
// there, thrown upward in a fan that arcs under gravity and skitters on the glossy floor: the bigger the
// jump, the more, bigger, faster and longer-lived the sparks. Embers drift and cool.
const PLINTH: f32 = 0.16;       // the geometry of the columns (see columns.wgsl)
const COL_MAX_H: f32 = 6.0;
const CORE_W: f32 = 0.56;

fn dead_particle() -> Particle {
  return Particle(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 1.0, vec4<f32>(0.0));
}

fn column_x(i: u32) -> f32 { return (f32(i) - 5.0) * u.ctl.y; }
fn cdf(i: u32) -> f32 { return pu.extra[i / 4u][i % 4u]; }

// A column chosen with probability proportional to its weight (the CPU sends the running sums).
fn pick_column(r: f32) -> u32 {
  let want = r * cdf(10u);
  for (var i = 0u; i < 10u; i++) {
    if (want <= cdf(i)) { return i; }
  }
  return 10u;
}

fn spawn_ember(s: u32) -> Particle {
  let i = pick_column(rnd(s, 0u));
  let l = lvl(i);
  var p: Particle;
  p.pos = vec3<f32>(column_x(i) + (rnd(s, 1u) - 0.5) * CORE_W, PLINTH + l * COL_MAX_H + 0.03, (rnd(s, 2u) - 0.5) * CORE_W);
  p.vel = vec3<f32>(rnd(s, 3u) - 0.5, 0.5 + 1.6 * rnd(s, 4u), rnd(s, 5u) - 0.5);
  p.max_life = 1.3 + 1.7 * rnd(s, 6u);
  p.life = p.max_life;
  p.look = vec4<f32>(f32(i) / 10.0, 0.026 + 0.026 * rnd(s, 7u), l, 0.0);
  return p;
}

fn spawn_spark(s: u32, k: u32) -> Particle {
  let bd = pu.bursts[k];
  let i = min(u32(bd.x + 0.5), 10u);
  let st = clamp(bd.y, 0.0, 1.0);                          // 0 for the smallest push that throws sparks, 1 for the biggest
  let top = PLINTH + max(pk(i), lvl(i)) * COL_MAX_H;       // the peak cap, or the strip's top when caps are off
  let elev = 0.9 * sqrt(rnd(s, 0u));                       // angle from vertical
  let th = 6.2831853 * rnd(s, 1u);
  let speed = (2.5 + 3.5 * rnd(s, 2u)) * (0.85 + 0.45 * st);
  var p: Particle;
  p.pos = vec3<f32>(column_x(i) + (rnd(s, 3u) - 0.5) * CORE_W, top, (rnd(s, 4u) - 0.5) * CORE_W);
  p.vel = vec3<f32>(sin(elev) * cos(th), cos(elev), sin(elev) * sin(th)) * speed;
  p.max_life = (0.9 + 1.1 * rnd(s, 5u)) * (0.9 + 0.4 * st);
  p.life = p.max_life;
  p.look = vec4<f32>(f32(i) / 10.0, (0.020 + 0.022 * rnd(s, 6u)) * (1.15 + 1.05 * st), 1.0, 1.0);
  return p;
}

fn spawn_particle(s: u32, burst: u32, n: u32) -> Particle {
  if (burst >= 6u) { return spawn_ember(s); }
  return spawn_spark(s, burst);
}

fn integrate(p: Particle, dt: f32) -> Particle {
  var q = p;
  let spark = q.look.w > 0.5;
  if (spark) {
    q.vel.y -= 7.5 * dt;
    q.vel *= exp(-0.35 * dt);
    q.vel += curl_wind(q.pos * 0.5, pu.step.y * 0.4) * (0.3 * dt);
  } else {
    q.vel.y -= 0.9 * dt;                                    // embers rise, hang and sink
    q.vel *= exp(-1.1 * dt);
    q.vel += curl_wind(q.pos * 0.6, pu.step.y * 0.4) * (1.6 * dt);
  }
  q.pos += q.vel * dt;
  q.life -= dt;
  if (q.pos.y < 0.0 && q.vel.y < 0.0) {
    if (spark) {
      q.pos.y = -q.pos.y * 0.4;
      q.vel = vec3<f32>(q.vel.x * 0.7, -q.vel.y * 0.4, q.vel.z * 0.7);
    } else {
      q.life = 0.0;
    }
  }
  return q;
}
