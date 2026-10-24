// Mycelium — shader.gallery (mycelium)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: abyssal

const SG_C0: vec3f = vec3f(0.039000, 0.518000, 0.553000);
const SG_C1: vec3f = vec3f(0.122000, 0.737000, 0.667000);
const SG_C2: vec3f = vec3f(0.616000, 0.918000, 0.776000);
const SG_C3: vec3f = vec3f(0.063000, 0.286000, 0.451000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// mycelium (Strand) - a hyphal colony spreading across dark substrate, seen
// from above under a black light. No cells anywhere: every thread is an open
// strand. The colony fans out from an origin off the lower-left corner as a
// family of leading hyphae (implicit radial lines in a slowly warped domain),
// and as the fan widens each new generation of child hyphae peels off its
// parent line at an acute angle and runs on outward, so branching keeps the
// spacing roughly even from the near corner to the far one. Short lateral
// twigs sprout off the runners as explicit polyline segments measured by
// distance-to-segment in a 3x3 cell neighbourhood. The line fields give
// thin uniform-width threads with dark ground between them.
// Growth is real: a ragged front of swollen bright tips sweeps outward
// across the frame, twigs extend behind it, and pulses of bioluminescence
// stream out along every thread. When one wave has crossed the frame the
// next seeds at the origin with a fresh warp and branching, and the old
// network dims into the soil beneath it, so the piece loops without ever
// un-growing. Under it all a combed haze of fine fibres and a mottled,
// gritty ground.


// tweakable params (see meta.json; the runtime feeds defaults)

const TAU: f32 = 6.28318530718;
const ORIGIN: vec2f = vec2f(-1.15, -0.78);   // colony origin, off the lower-left corner
const RMIN: f32 = 0.30;                   // radius where the near corner starts
const RMAX: f32 = 2.45;                   // radius past the far corner

fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}

fn vnoise(p: vec2f) -> f32 {

  var i = floor(p);
  var f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  var a = hash21(i);
  var b = hash21(i + vec2f(1.0, 0.0));
  var c = hash21(i + vec2f(0.0, 1.0));
  var d = hash21(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

const M2: mat2x2<f32> = mat2x2<f32>(vec2f(0.80, 0.60), vec2f(-0.60, 0.80));

fn fbm(p_in: vec2f) -> f32 {
  var p = p_in;

  var a = 0.5;
  var s = 0.0;
  for (var i: i32 = 0; i < 3; i += 1) {
    s += a * vnoise(p);
    p = M2 * p * 2.03 + vec2f(11.7, 5.3);
    a *= 0.5;
  }
  return s * 1.14;
}

fn dirOf(a: f32) -> vec2f {
 return vec2f(cos(a), sin(a)); }

// where the growth front of generation g sits, in radius, for the line n of
// family f: a ragged margin, each hypha a little ahead or behind its fellows
fn frontOf(tg: f32, n: f32, f: f32, g: f32) -> f32 {

  let j = hash21(vec2f(n * 1.7 + f * 13.1, g * 7.3)) - 0.5;
  return RMIN - 0.20 + (RMAX - RMIN + 0.45) * smoothstep(0.0, 0.85, tg) + j * 0.35;
}

// per-line angular offset in spacing units: the bend that peels a child line
// off its parent near its start (half a spacing, decaying outward) plus a
// small fixed jitter that only takes hold once the bend has relaxed, so no
// line ever crosses its own index boundary
fn offsetOf(r: f32, rs: f32, sgn: f32, jit: f32) -> f32 {

  var b = sgn * 0.49 * exp(-max(r - rs, 0.0) / 0.14);
  return b + jit * (1.0 - abs(b) / 0.49);
}
// radius where child line n of family f gives up: many hyphae simply stop
fn lifeOf(rs: f32, n: f32, f: f32, g: f32) -> f32 {

  var h = hash21(vec2f(n * 3.1 + f * 7.7, g * 5.9 + 2.0));
  return rs + 0.25 + 2.4 * h * h;
}
// start radius of family f in generation g: where its lines branch off
fn startOf(f: f32, g: f32) -> f32 {

  if (f < 0.5) { return RMIN - 0.3; }
  return RMIN + 0.12 + f * 0.50 + (hash21(vec2f(g, f)) - 0.5) * 0.28;
}

// one generation of the colony at warped point pw. Accumulates core coverage
// of the nearest line, its brightness, the halo and the tip glow.
fn colony(pw: vec2f, g: f32, tg: f32, s0: f32, px: f32, core: ptr<function, f32>, bright: ptr<function, f32>, halo: ptr<function, f32>, tip: ptr<function, f32>) {

  var d = pw - ORIGIN;
  var r = length(d);
  var th = atan2(d.y, d.x);
  var alpha = 1.0 - smoothstep(1.0, 1.85, tg);
  if (alpha <= 0.0) { return; }
  var s = s0;
  for (var f: i32 = 0; f < 5; f += 1) {
    var ff = f32(f);
    var rsf = startOf(ff, g);
    let u0 = th / s;
    var n = floor(u0);
    var hn = hash21(vec2f(n, ff + g * 3.0));
    var sgn = select((select(-1.0, 1.0, hn < 0.5)), 0.0, (f == 0));
    var jit = (hash21(vec2f(n * 1.3, ff * 2.1 + g)) - 0.5) * 0.30;
    let F = fract(u0 + offsetOf(r, rsf, sgn, jit)) - 0.5;
    // perpendicular distance: |F| over the gradient magnitude of u
    let db = -sgn * 0.49 / 0.14 * exp(-max(r - rsf, 0.0) / 0.14);
    let gu = sqrt(1.0 / (s * s * r * r) + db * db);
    let dist = abs(F) / gu;
    // visibility: born at its start radius, alive up to the growth front or
    // until the hypha gives up on its own
    var front = min(frontOf(tg, n, ff, g), lifeOf(rsf, n, ff, g));
    let born = smoothstep(rsf - 0.02, rsf + 0.02, r);
    let alive = smoothstep(front + 0.008, front - 0.008, r);
    let vis = born * alive * alpha;
    // width: leaders thickest, each family finer, tapering to the *tip
    var w = 0.0021 * pow(0.80, ff) * (1.0 - 0.4 * smoothstep(front - 0.5, front, r));
    var c = smoothstep(w + px, w - px * 0.6, dist) * vis;
    // the body brightens toward its *tip where the cytoplasm is busiest
    let age = exp(-(front - r) * 1.1);
    let bb = 0.55 + 0.45 * age;
    *bright = mix(*bright, bb, step(*core, c) * step(0.001, c));
    *core  = max(*core, c);
    *halo += vis * exp(-dist * dist * 3500.0) * (0.3 + 0.7 * age) * pow(0.85, ff);
    // apex: swollen *bright *tip at the end of every living line
    let tipv = vec2f(dist * 1.6, (r - front) * 0.9);
    let td = dot(tipv, tipv);
    *tip += alpha * born * exp(-td * 30000.0) * pow(0.9, ff);
    s *= 0.5;
  }
}

// one twig segment; see the twig layer below
fn seg(q: vec2f, a: vec2f, b: vec2f, s0: f32, len: f32, w: f32, reach: f32, alpha: f32, minD: ptr<function, f32>, minW: ptr<function, f32>, minA: ptr<function, f32>, halo: ptr<function, f32>, tip: ptr<function, f32>) {

  let frac = clamp((reach - s0) / len, 0.0, 1.0);
  if (frac <= 0.0) { return; }
  let ab = (b - a) * frac;
  let aq = q - a;
  var h = clamp(dot(aq, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
  let dd = length(aq - ab * h);
  let ww = w * (1.0 - 0.45 * (s0 + len * frac * h));
  if (dd < *minD) { *minD = dd; *minW = ww; *minA = alpha; }
  *halo += alpha * exp(-dd * dd * 500.0);
  if (frac < 1.0) {
    let tp = q - (a + ab);
    *tip += alpha * exp(-dot(tp, tp) * 2500.0);
  }
}

// lateral twigs of generation g: short branching polylines rooted on the
// finest runner line of their cell and growing outward behind the front
fn twigs(pw: vec2f, g: f32, tg: f32, s0: f32, cell: f32, density: f32, minD: ptr<function, f32>, minW: ptr<function, f32>, minA: ptr<function, f32>, halo: ptr<function, f32>, tip: ptr<function, f32>) {

  var alpha = 1.0 - smoothstep(1.0, 1.85, tg);
  if (alpha <= 0.0) { return; }
  let q = pw / cell;
  let ci = floor(q);
  for (var y: i32 = -1; y <= 1; y += 1) {
    for (var x: i32 = -1; x <= 1; x += 1) {
      let id = ci + vec2f(f32(x), f32(y));
      let gs = id + vec2f(g * 5.3, g * 2.1);
      let h0 = hash21(gs);
      let h1 = hash21(gs + 11.1);
      let h2 = hash21(gs + 23.7);
      let h3 = hash21(gs + 37.3);
      if (h3 > density) { continue; }
      // find the finest family born at this cell and its nearest line, then
      // put the root exactly on that line, bend and jitter included
      let pc = (id + 0.5) * cell;
      var d = pc - ORIGIN;
      var r = length(d) + (h0 - 0.5) * 0.6 * cell;
      var th = atan2(d.y, d.x);
      var s = s0;
      var ff = 0.0;
      var rsf = startOf(0.0, g);
      for (var f: i32 = 1; f < 5; f += 1) {
        let rt = startOf(f32(f), g);
        if (r > rt + 0.05) { s *= 0.5; ff = f32(f); rsf = rt; }
      }
      var n = floor(th / s);
      var hn = hash21(vec2f(n, ff + g * 3.0));
      var sgn = select((select(-1.0, 1.0, hn < 0.5)), 0.0, (ff < 0.5));
      var jit = (hash21(vec2f(n * 1.3, ff * 2.1 + g)) - 0.5) * 0.30;
      let lineTh = (n + 0.5 - offsetOf(r, rsf, sgn, jit)) * s;
      var root = ORIGIN + dirOf(lineTh) * r;
      // keep the root within reach of the cell so nothing clips at borders
      root = clamp(root / cell, id - 0.35, id + 1.35);
      var front = min(frontOf(tg, n, ff, g), lifeOf(rsf, n, ff, g));
      // grows once the front has passed its root; none past a dead end
      let reach = smoothstep(0.0, 0.45, front - r) * 0.8;
      // outward lateral: off the runner at an acute angle, then wavering on
      let sb =select(-1.0, 1.0, h1 < 0.5);
      let a0 = lineTh + sb * (0.45 + 0.5 * h2);
      let a1 = a0 - sb * (0.2 + 0.5 * h0);
      let P1 = root + dirOf(a0) * 0.36;
      let P2 = P1 + dirOf(a1) * 0.30;
      let B1 = P1 + dirOf(a0 + sb * (0.5 + 0.4 * h3)) * 0.20;
      var w = 0.012 * (0.8 + 0.4 * h2);
      seg(q, root, P1, 0.00, 0.34, w, reach, alpha, minD, minW, minA, halo, tip);
      seg(q, P1, P2, 0.34, 0.28, w * 0.75, reach, alpha, minD, minW, minA, halo, tip);
      seg(q, P1, B1, 0.34, 0.18, w * 0.55, reach, alpha, minD, minW, minA, halo, tip);
    }
  }
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  let uv = fc / u.resolution.xy;
  let aspect = u.resolution.x / u.resolution.y;
  let p = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5);
  let spd = clamp(param(1u), 0.0, 3.0);
  let t = u.time * spd / 32.0 + 0.22;          // one wave per 32 s
  let tp = u.time * spd;
  let s0 = 0.17 / max(param(0u), 0.3);            // angular spacing of the leaders
  let glow = clamp(param(3u), 0.0, 2.0);
  let wander = clamp(param(4u), 0.0, 2.0);
  let dens = clamp(param(2u), 0.0, 1.0);
  let px = 1.0 / u.resolution.y;                // frame units per device px

  let gen = floor(t);
  let tgA = t - gen;          // the growing wave
  let tgB = tgA + 1.0;        // the previous wave, dimming

  // ---- ground: dark damp substrate with pulp mottle and grit
  let mott = fbm(p * 3.0 + 4.0);
  let grit = hash21(floor(fc / max(1.0, 1.0) * 0.5));
  var soil = mix(vec3f(0.030, 0.027, 0.025), c3 * 0.11 + c0 * 0.03, 0.75);
  soil *= 0.70 + 0.60 * mott;
  soil += (grit - 0.5) * 0.012;
  let colonyF = fbm(p * 0.9 + vec2f(1.0, 5.0) + tp * 0.006);
  let hazeCol = mix(c2, c0, smoothstep(0.35, 0.65, fbm(p * 0.6 + 13.0)));
  soil += hazeCol * 0.04 * glow * smoothstep(0.35, 0.8, colonyF);

  // ---- fine fuzz: the felt of finest hyphae the colony lays down, streaks
  // aligned with the radial flow, very faint
  let dp = p - ORIGIN;
  let rp = length(dp);
  let thp = atan2(dp.y, dp.x);
  let fz1 = vnoise(vec2f(thp * 90.0, rp * 5.0) + 7.0);
  let fz2 = vnoise(vec2f(thp * 170.0, rp * 9.0) + 19.0);
  var fmat = smoothstep(0.60, 0.92, fz1) * 0.6 + smoothstep(0.66, 0.94, fz2) * 0.4;
  fmat *= 0.4 + 0.6 * smoothstep(0.30, 0.75, colonyF);
  let matCol = mix(c2, c1, 0.4) * 0.14 * glow + vec3f(0.04);
  var col = soil + matCol * fmat;

  // ---- the two waves, each in its own warped domain
  let warpA = vec2f(fbm(p * 1.4 + vec2f(3.1, 7.7) + gen * 4.7),
                    fbm(p * 1.4 + vec2f(9.2, 1.4) + gen * 2.9)) - 0.5;
  let warpB = vec2f(fbm(p * 1.4 + vec2f(3.1, 7.7) + (gen - 1.0) * 4.7),
                    fbm(p * 1.4 + vec2f(9.2, 1.4) + (gen - 1.0) * 2.9)) - 0.5;
  // a fine wobble so the threads are never ruler-straight
  let wob = vec2f(vnoise(p * 9.0 + 41.0), vnoise(p * 9.0 + 77.0)) - 0.5;
  let pwA = p + (warpA * 0.30 + wob * 0.014) * wander;
  let pwB = p + (warpB * 0.30 + wob * 0.014) * wander;

  // previous wave first: it sits beneath
  var coreB = 0.0;
  var brB = 0.0;
  var haloB = 0.0;
  var tipB = 0.0;
  colony(pwB, gen - 1.0, tgB, s0, px, &coreB, &brB, &haloB, &tipB);
  var coreA = 0.0;
  var brA = 0.0;
  var haloA = 0.0;
  var tipA = 0.0;
  colony(pwA, gen, tgA, s0, px, &coreA, &brA, &haloA, &tipA);

  // twigs on both waves
  let cell = 0.13;
  var tD = 10.0;
  var tW = 0.02;
  var tA = 0.0;
  var tHalo = 0.0;
  var tTip = 0.0;
  twigs(pwB, gen - 1.0, tgB, s0, cell, dens, &tD, &tW, &tA, &tHalo, &tTip);
  twigs(pwA, gen, tgA, s0, cell, dens, &tD, &tW, &tA, &tHalo, &tTip);
  let pxC = px / cell;
  let twigCore = smoothstep(tW + pxC, tW - pxC * 0.5, tD) * tA;

  // pulses of light streaming outward along every thread
  let rA = length(pwA - ORIGIN);
  var pulse = 0.5 + 0.5 * cos(rA * 26.0 - tp * 1.6);
  pulse = pulse * pulse; pulse = pulse * pulse;

  // ---- colour. Halo of bioluminescence, then the pale hyphal bodies
  let haloCol = mix(c2, c1, smoothstep(0.3, 0.7, colonyF));
  let haloAll = haloB * 0.5 + haloA + tHalo * 0.6;
  col += haloCol * 0.22 * glow * min(haloAll, 2.5);

  // old wave: dimmer, cooler, sunk into the ground
  let bodyB = mix(vec3f(0.60, 0.60, 0.57), c2, 0.45) * (0.6 + 0.3 * brB);
  col = mix(col, bodyB, coreB * 0.9);
  col = mix(col, bodyB * 0.9, twigCore * 0.8);
  // new wave: pale translucent body lit from within by the pulse
  var bodyA = mix(vec3f(0.90, 0.88, 0.82), c2, 0.28) * (0.55 + 0.45 * brA);
  bodyA *= 0.75 + 0.55 * pulse * glow;
  bodyA += c0 * 0.25 * pulse * glow;
  col = mix(col, bodyA, coreA);
  col = mix(col, bodyA, twigCore * step(0.001, tA) * 0.9);

  // apex glow: swollen bright tips along the advancing margin
  let tipCol = mix(c1, vec3f(1.0), 0.45);
  col += tipCol * min(tipA + tipB * 0.3 + tTip * 0.5, 1.4) * (0.4 + 0.5 * glow);

  return vec4f(col, 1.0);
}
