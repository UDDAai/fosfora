// Chalk — shader.gallery (chalk)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: sherbet

const SG_C0: vec3f = vec3f(0.984000, 0.706000, 0.557000);
const SG_C1: vec3f = vec3f(0.980000, 0.612000, 0.694000);
const SG_C2: vec3f = vec3f(0.690000, 0.851000, 0.871000);
const SG_C3: vec3f = vec3f(0.831000, 0.929000, 0.682000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// chalk (Trace) - a slate chalkboard mid-lesson. The board is the ground: a
// dark slate with cloudy mottle, a fine tooth of pits at css-pixel scale, old
// wash streaks and a soft window light from the upper left. Three working
// areas across the board each carry a set of hand-drawn geometric marks, a
// circle with a radius or a square, a triangle, a hatch bundle, a spiral or
// wave and a couple of small crosses, all
// drawn stroke by stroke with a wobbling hand, uneven pressure and a dusty
// broken chalk edge where the chalk only catches the high points of the
// slate. Each area runs its own cycle: the marks are drawn, held, then wiped
// by an eraser pass whose front travels across them, and the wipe leaves the
// classic eraser ghost, a streaky smear of dust and the pale shadow of the
// old marks, which fades as the next set is chalked up in a new place.
// Chalk is white, plus pastel sticks tinted by the palette; the slate itself
// takes a whisper of the deepest palette colour so the board re-themes.


// tweakable params (see meta.json; the runtime feeds defaults)

const TAU: f32 = 6.28318530718;
const PERIOD: f32 = 24.0;   // seconds per draw / hold / wipe cycle at speed 1

var<private> c0: vec3f;
var<private> c1: vec3f;
var<private> c2: vec3f;
var<private> c3: vec3f;
fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}

fn vnoise(p: vec2f) -> f32 {

  var i = floor(p);
  let f = fract(p);
  var u = f * f * (3.0 - 2.0 * f);
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
  for (var i: i32 = 0; i < 4; i += 1) {
    s += a * vnoise(p);
    p = M2 * p * 2.03 + vec2f(11.7, 5.3);
    a *= 0.5;
  }
  return s * 1.07;
}

fn luma(c: vec3f) -> f32 {
 return dot(c, vec3f(0.299, 0.587, 0.114)); }
fn rot(a: f32) -> mat2x2<f32> {
 var c = cos(a);
  var s = sin(a);
  return mat2x2<f32>(c, -s, s, c); }

// a pastel chalk stick in colour c: desaturate a little, then lift toward white
fn chalkOf(c: vec3f) -> vec3f {

  var pastel = mix(vec3f(luma(c)), c, 0.85);
  pastel = mix(pastel, vec3f(1.0), 0.35);
  return clamp(pastel * 1.05, vec3f(0.0), vec3f(1.0));
}

// distance to a segment a-b, and the along-parameter 0..1 written to u
fn sdSeg(p: vec2f, a: vec2f, b: vec2f, u: ptr<function, f32>) -> f32 {

  let ab = b - a;
  let ap = p - a;
  *u = clamp(dot(ap, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
  return length(ap - ab * (*u));
}

// ---- the chalk stroke. d is the distance to the ideal path in frame units,
// w the stroke half-width, grain the shared slate-tooth field 0..1, press the
// hand pressure along the stroke. Chalk sits on the high points of the slate:
// dense at the core, ragged at the edge, a few stray specks just outside.
fn chalk(d: f32, w: f32, grain: f32, press: f32) -> f32 {

  let core = 1.0 - smoothstep(w * 0.25, w * 1.15, d);
  return smoothstep(0.62, 0.80, grain * 0.55 + 0.05 + core * (0.25 + 0.42 * press));
}

// hand pressure along a stroke: heavier at the start, lifts at the end,
// with a slow rock in between
fn pressure(u: f32, seed: f32) -> f32 {

  let rock = 0.5 + 0.5 * sin(u * 11.0 + seed) * sin(u * 4.7 + seed * 2.1);
  let taper = smoothstep(1.0, 0.86, u) * smoothstep(0.0, 0.04, u);
  return (0.55 + 0.45 * rock) * (0.35 + 0.65 * taper);
}

// draw-progress mask: the part of the stroke already laid down, with a soft
// leading edge so the chalk tip is never a hard cut
fn drawn(u: f32, prog: f32) -> f32 {

  return smoothstep(prog + 0.015, prog - 0.015, u);
}

// ---- one working area. p is the hand-wobbled frame coordinate, gc the area
// centre, seed selects the set, prog 0..1 is how far the drawing has got.
// Composites the chalk marks into col and accumulates their smear halo into
// ghost (used for the eraser ghost of the same set).
fn marks(p: vec2f, gc: vec2f, seed: f32, prog: f32, w: f32, grain: f32, amount: f32, col: ptr<function, vec3f>, ghost: ptr<function, f32>) {
var u: f32;
var d: f32;
  let haloW = w * 5.0;

  // colour picks for the four marks: white plus palette pastels
  let white = vec3f(0.93, 0.92, 0.88);
  let cmix = clamp(param(4u), 0.0, 1.0);
  let pick = hash21(vec2f(seed, 3.3));
  let tintA =select(c2, c0, pick < 0.5);
  let tintB =select(c3, c1, pick < 0.5);
  let chalkA = mix(white, chalkOf(tintA), cmix);
  let chalkB = mix(white, chalkOf(tintB), cmix);

  // -- mark 0: a circle with one radius, drawn counter-clockwise from a0,
  // or (some sets) a square drawn side by side, in white
  let p0 = clamp(prog * 4.0, 0.0, 1.0);
  {
    var h = vec2f(hash21(vec2f(seed, 1.1)), hash21(vec2f(seed, 1.7)));
    var c = gc + (h - 0.5) * vec2f(0.6, 1.0);
    var r = 0.20 + 0.26 * hash21(vec2f(seed, 2.3));
    var a0 = h.x * TAU;
    var cov = 0.0;
    var gh = 0.0;
    if (hash21(vec2f(seed, 2.9)) < 0.62) {
      var q = p - c;
      var ang = fract((atan2(q.y, q.x) - a0) / TAU);
      d = abs(length(q) - r);
      // the circle takes 70% of this mark, the radius the rest
      let pc = clamp(p0 / 0.7, 0.0, 1.0);
      cov = chalk(d, w, grain, pressure(ang, seed)) * drawn(ang, pc);
      gh = (1.0 - smoothstep(0.0, haloW, d)) * drawn(ang, pc);
      let pr = clamp((p0 - 0.7) / 0.3, 0.0, 1.0);
      let ar = a0 + 0.9;
      let tip = c + r * vec2f(cos(ar), sin(ar));
      d = sdSeg(p, c, tip, &u);
      cov = max(cov, chalk(d, w * 0.9, grain, pressure(u, seed + 1.0)) * drawn(u, pr));
      gh = max(gh, (1.0 - smoothstep(0.0, haloW, d)) * drawn(u, pr));
    } else {
      // a square, slightly turned, four sides in order with a diagonal
      var ta = (h.y - 0.5) * 0.5;
      var ex = vec2f(cos(ta), sin(ta)) * r * 0.9;
      var ey = vec2f(-ex.y, ex.x);
      var v0 = c - ex - ey;
      var v1 = c + ex - ey;
      var v2 = c + ex + ey;
      let v3 = c - ex + ey;
      for (var i: i32 = 0; i < 5; i += 1) {
        var k = f32(i);
        var a = v0;
        var b = v1;
        if (i == 1) { a = v1; b = v2; }
        if (i == 2) { a = v2; b = v3; }
        if (i == 3) { a = v3; b = v0; }
        if (i == 4) { a = v0; b = v2; }
        d = sdSeg(p, a, b, &u);
        var pk = clamp(p0 * 5.0 - k, 0.0, 1.0);
        cov = max(cov, chalk(d, w, grain, pressure(u, seed + 1.0 + k)) * drawn(u, pk));
        gh = max(gh, (1.0 - smoothstep(0.0, haloW, d)) * drawn(u, pk));
      }
    }
    *col = mix(*col, white, cov * amount);
    *ghost = max(*ghost, gh);
  }

  // -- mark 1: a triangle, three sides in order, in the first pastel
  let p1 = clamp(prog * 4.0 - 1.0, 0.0, 1.0);
  {
    var h = vec2f(hash21(vec2f(seed, 4.1)), hash21(vec2f(seed, 4.7)));
    var c = gc + (h - 0.5) * vec2f(0.75, 1.1) + vec2f(0.0, -0.05);
    let R = 0.22 + 0.16 * hash21(vec2f(seed, 5.3));
    var a0 = h.y * TAU;
    var v0 = c + R * vec2f(cos(a0), sin(a0));
    var v1 = c + R * 1.15 * vec2f(cos(a0 + 2.0), sin(a0 + 2.0));
    var v2 = c + R * 0.9  * vec2f(cos(a0 + 4.3), sin(a0 + 4.3));
    var cov = 0.0;
    var gh = 0.0;
    d = sdSeg(p, v0, v1, &u);
    cov = max(cov, chalk(d, w, grain, pressure(u, seed + 2.0)) * drawn(u, clamp(p1 * 3.0, 0.0, 1.0)));
    gh = max(gh, (1.0 - smoothstep(0.0, haloW, d)) * drawn(u, clamp(p1 * 3.0, 0.0, 1.0)));
    d = sdSeg(p, v1, v2, &u);
    cov = max(cov, chalk(d, w, grain, pressure(u, seed + 3.0)) * drawn(u, clamp(p1 * 3.0 - 1.0, 0.0, 1.0)));
    gh = max(gh, (1.0 - smoothstep(0.0, haloW, d)) * drawn(u, clamp(p1 * 3.0 - 1.0, 0.0, 1.0)));
    d = sdSeg(p, v2, v0, &u);
    cov = max(cov, chalk(d, w, grain, pressure(u, seed + 4.0)) * drawn(u, clamp(p1 * 3.0 - 2.0, 0.0, 1.0)));
    gh = max(gh, (1.0 - smoothstep(0.0, haloW, d)) * drawn(u, clamp(p1 * 3.0 - 2.0, 0.0, 1.0)));
    *col = mix(*col, chalkA, cov * amount);
    *ghost = max(*ghost, gh);
  }

  // -- mark 2: a hatch bundle of four parallel strokes, white
  let p2 = clamp(prog * 4.0 - 2.0, 0.0, 1.0);
  {
    var h = vec2f(hash21(vec2f(seed, 6.1)), hash21(vec2f(seed, 6.7)));
    var c = gc + (h - 0.5) * vec2f(0.85, 1.2);
    var ang = h.x * 3.1 + 0.4;
    let dir = vec2f(cos(ang), sin(ang));
    let perp = vec2f(-dir.y, dir.x);
    let L = 0.18 + 0.12 * h.y;
    let sp = 0.055;
    var cov = 0.0;
    var gh = 0.0;
    for (var i: i32 = 0; i < 4; i += 1) {
      var k = f32(i);
      let o = c + perp * (k - 1.5) * sp;
      // every other stroke is pulled the other way, as a hand does
      let flip =select(-1.0, 1.0, sg_mod(k, 2.0) < 0.5);
      var a = o - dir * L * flip * (0.95 + 0.1 * hash21(vec2f(seed, 7.0 + k)));
      var b = o + dir * L * flip * (0.95 + 0.1 * hash21(vec2f(seed, 8.0 + k)));
      d = sdSeg(p, a, b, &u);
      var pk = clamp(p2 * 4.0 - k, 0.0, 1.0);
      cov = max(cov, chalk(d, w * 0.85, grain, pressure(u, seed + 5.0 + k)) * drawn(u, pk));
      gh = max(gh, (1.0 - smoothstep(0.0, haloW, d)) * drawn(u, pk));
    }
    *col = mix(*col, white, cov * amount);
    *ghost = max(*ghost, gh);
  }

  // -- mark 3: a spiral or a wave, in the second pastel
  let p3 = clamp(prog * 4.0 - 3.0, 0.0, 1.0);
  {
    var h = vec2f(hash21(vec2f(seed, 9.1)), hash21(vec2f(seed, 9.7)));
    var c = gc + (h - 0.5) * vec2f(0.8, 1.1) + vec2f(0.0, 0.05);
var cov: f32;
var gh: f32;
    if (hash21(vec2f(seed, 10.3)) < 0.5) {
      // archimedean spiral, 2.5 turns, wound outward from the centre
      var b = 0.040 + 0.016 * h.x;
      let turns = 2.2;
      var q = p - c;
      var r = length(q);
      let th = atan2(q.y, q.x);
      // the nearest winding to this point
      var k = floor((r / b - th) / TAU + 0.5);
      let thk = th + k * TAU;
      let rk = b * thk;
      var uu = thk / (turns * TAU);
      let inside = step(0.0, thk) * step(thk, turns * TAU);
      d = mix(999.0, abs(r - rk), inside);
      // the winding just outside, for points between two arms
      let th2 = thk + TAU;
      let in2 = step(0.0, th2) * step(th2, turns * TAU);
      let d2 = mix(999.0, abs(r - b * th2), in2);
      let u2 = th2 / (turns * TAU);
      if (d2 < d) { d = d2; uu = u2; }
      cov = chalk(d, w, grain, pressure(uu, seed + 9.0)) * drawn(uu, p3);
      gh = (1.0 - smoothstep(0.0, haloW, d)) * drawn(uu, p3);
    } else {
      // a wave: y = A sin(kx) across a span, distance corrected by slope
      var ang = (h.x - 0.5) * 0.8;
      var q = rot(-ang) * (p - c);
      let A = 0.06 + 0.04 * h.y;
      let kx = 14.0 + 6.0 * h.x;
      let span = 0.28 + 0.1 * h.y;
      let s = q.x;
      let sl = A * kx * cos(kx * s);
      d = abs(q.y - A * sin(kx * s)) / sqrt(1.0 + sl * sl);
      d += max(abs(s) - span, 0.0) * 4.0;
      var uu = clamp((s + span) / (2.0 * span), 0.0, 1.0);
      cov = chalk(d, w, grain, pressure(uu, seed + 9.0)) * drawn(uu, p3);
      gh = (1.0 - smoothstep(0.0, haloW, d)) * drawn(uu, p3);
    }
    *col = mix(*col, chalkB, cov * amount);
    *ghost = max(*ghost, gh);
  }

  // -- mark 4: two small crosses, the quick annotation ticks, white, laid
  // down with the last mark
  {
    var cov = 0.0;
    var gh = 0.0;
    for (var i: i32 = 0; i < 2; i += 1) {
      var k = f32(i);
      var h = vec2f(hash21(vec2f(seed, 12.1 + k)), hash21(vec2f(seed, 12.7 + k)));
      var c = gc + (h - 0.5) * vec2f(1.1, 1.3);
      let sz = 0.035 + 0.02 * h.x;
      var ta = h.y * 1.5;
      var ex = vec2f(cos(ta), sin(ta)) * sz;
      var ey = vec2f(-ex.y, ex.x);
      var pk = clamp(p3 * 2.0 - k, 0.0, 1.0);
      d = sdSeg(p, c - ex - ey, c + ex + ey, &u);
      cov = max(cov, chalk(d, w * 0.8, grain, pressure(u, seed + 11.0 + k)) * drawn(u, clamp(pk * 2.0, 0.0, 1.0)));
      gh = max(gh, (1.0 - smoothstep(0.0, haloW, d)) * drawn(u, clamp(pk * 2.0, 0.0, 1.0)));
      d = sdSeg(p, c - ex + ey, c + ex - ey, &u);
      cov = max(cov, chalk(d, w * 0.8, grain, pressure(u, seed + 13.0 + k)) * drawn(u, clamp(pk * 2.0 - 1.0, 0.0, 1.0)));
      gh = max(gh, (1.0 - smoothstep(0.0, haloW, d)) * drawn(u, clamp(pk * 2.0 - 1.0, 0.0, 1.0)));
    }
    *col = mix(*col, white, cov * amount);
    *ghost = max(*ghost, gh);
  }
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    c0 = SG_C0; c1 = SG_C1; c2 = SG_C2; c3 = SG_C3;
  let pr = max(1.0, 0.5);
  let uv = fc / res;
  let aspect = res.x / res.y;
  let t = u.time * clamp(param(0u), 0.0, 4.0);
  let scl = 1.0 / max(param(1u), 0.3);
  let dustA = clamp(param(3u), 0.0, 2.0);

  // css-pixel coords for surface properties; frame coords two units tall
  let q = fc / pr;
  let p = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5) * 2.0 * scl;
  let w = 0.011 * max(param(2u), 0.2) * scl;

  // ---- the slate. Deep grey-green with a whisper of the darkest palette pole,
  // cloudy mottle from the stone, fine tooth pits, faint old wash streaks and
  // a window light falling from the upper left.
  let lum0 = luma(c0);
  let lum1 = luma(c1);
  let lum2 = luma(c2);
  let lum3 = luma(c3);
  var deep = c0;
  var dl = lum0;
  if (lum1 < dl) { deep = c1; dl = lum1; }
  if (lum2 < dl) { deep = c2; dl = lum2; }
  if (lum3 < dl) { deep = c3; dl = lum3; }
  var tint = mix(vec3f(luma(deep)), deep, 0.45);
  tint = tint / max(luma(tint), 0.05) * 0.12;
  var slate = mix(vec3f(0.105, 0.128, 0.118), tint, 0.40);

  let mottle = fbm(q * 0.009 + 3.0) - 0.5;
  let cloud = fbm(p * 1.3 + 11.0) - 0.5;
  let tooth = vnoise(q * 0.55 + 7.0) - 0.5;
  // old wash streaks: long horizontal smears from past cleanings
  var wash = vnoise(vec2f(q.x * 0.004, q.y * 0.06) + 5.0);
  wash = smoothstep(0.45, 0.85, wash) * (0.5 + 0.5 * vnoise(vec2f(q.x * 0.02, q.y * 0.3) + 9.0));
  let light = 1.0 + 0.28 * (0.6 - uv.x * 0.5 - (1.0 - uv.y) * 0.45);
  slate *= light * (1.0 + mottle * 0.5 + cloud * 0.25 + tooth * 0.22);
  let residue = smoothstep(0.35, 0.75, fbm(q * 0.006 + vec2f(21.0, 8.0)));
  slate += vec3f(0.85, 0.85, 0.82) * (wash * 0.035 + residue * 0.030) * dustA;

  // ---- shared slate-tooth field: where the chalk catches, at css-pixel scale
  let grain = 0.36 * vnoise(q * 0.22 + 1.0) + 0.36 * vnoise(q * 0.75 + 2.0)
              + 0.28 * vnoise(q * 2.1 + 3.0);

  // ---- the hand: a slow low-frequency wobble so no path is ruler-straight
  var wob = vec2f(vnoise(p * 2.6 + 4.0), vnoise(p * 2.6 + 17.0)) - 0.5;
  wob += (vec2f(vnoise(p * 9.0 + 6.0), vnoise(p * 9.0 + 23.0)) - 0.5) * 0.35;

  var col = slate;
  let dustCol = vec3f(0.88, 0.88, 0.85);
  var dust = 0.0;

  // ---- three working areas across the board, each on its own cycle
  for (var g: i32 = 0; g < 3; g += 1) {
    let k = f32(g);
    let cyc = t / PERIOD + k * 0.3333 + 0.1;
    let ph = fract(cyc);
    let n = floor(cyc);
    let seedNow = n * 17.3 + k * 5.1 + 2.7;
    let seedPrev = (n - 1.0) * 17.3 + k * 5.1 + 2.7;

    // area centre: thirds across, a little up or down per set
    let gx = (k - 1.0) * aspect * 0.64 * scl;
    // areas alternate high and low across the board so the whole height is used
    let gy = (sg_mod(k, 2.0) - 0.5) * 0.36 * scl;
    let gcNow = vec2f(gx, gy + (hash21(vec2f(seedNow, 0.3)) - 0.5) * 0.3 * scl);
    let gcPrev = vec2f(gx, -gy + (hash21(vec2f(seedPrev, 0.3)) - 0.5) * 0.3 * scl);

    // life of a set: drawn 0..0.36, held, wiped 0.78..0.9, then ghost only
    let prog = clamp(ph / 0.36, 0.0, 1.0);
    let eph = clamp((ph - 0.78) / 0.12, 0.0, 1.0);

    // the eraser pass for this area: a front travelling along dir, its edge
    // bowed like a hand sweep. Beyond the pass the marks are gone.
    let eang = (hash21(vec2f(seedNow, 0.9)) - 0.5) * 0.7;
    let dir = vec2f(cos(eang), sin(eang));
    let perp = vec2f(-dir.y, dir.x);
    let rel = p - gcNow;
    let s = dot(rel, dir) + 0.10 * sin(dot(rel, perp) * 4.0 + seedNow);
    let R = 1.05 * scl;
    let front = mix(-R - 0.15, R + 0.15, eph);
    let erased = smoothstep(front + 0.04, front - 0.04, s) * step(0.001, eph);

    // per-area hand wobble: the shared field turned so areas do not match
    let wk = rot(k * 2.1) * wob;

    // current set: chalk where not yet wiped, smear where it has been
    var colNow = col;
    var ghNow = 0.0;
    marks(p + wk * 0.022 * scl, gcNow, seedNow, prog, w, grain, 1.0, &colNow, &ghNow);
    col = mix(col, colNow, 1.0 - erased);

    // the eraser streaks: dust dragged along the pass direction
    var streak = vnoise(vec2f(dot(rel, dir) * 2.5, dot(rel, perp) * 38.0) + seedNow);
    streak = 0.45 + 0.9 * streak;
    // ghost of the current set inside the wiped zone, and the broad swath
    // of dust the felt leaves behind it
    let swath = 1.0 - smoothstep(R * 0.55, R * 1.05, length(rel * vec2f(1.0, 0.72)));
    dust = max(dust, erased * (ghNow * 0.42 + swath * 0.10) * streak);

    // previous set: fully wiped, fading through the first part of this cycle
    let fade = 1.0 - smoothstep(0.05, 0.75, ph);
    var dummy = col;
    var ghPrev = 0.0;
    marks(p + wk * 0.022 * scl, gcPrev, seedPrev, 1.0, w, grain, 0.0, &dummy, &ghPrev);
    let relP = p - gcPrev;
    let swathP = 1.0 - smoothstep(R * 0.55, R * 1.05, length(relP * vec2f(1.0, 0.72)));
    let streakP = 0.45 + 0.9 * vnoise(vec2f(dot(relP, dir) * 2.5, dot(relP, perp) * 38.0) + seedPrev);
    let dPrev = fade * (ghPrev * 0.34 + swathP * 0.09) * streakP;
    // the ghost is laid under the fresh chalk, so blend it before the marks
    dust = max(dust, dPrev);
  }

  // dust goes under the chalk: lighten the slate, then re-lay the marks on top
  // (col already carries the marks; lift only where dust exceeds the chalk)
  var dustAmt = clamp(dust * dustA, 0.0, 0.6);
  // chalk tooth in the dust too: the smear sits in the pits as well
  dustAmt *= 0.75 + 0.5 * grain;
  let lifted = mix(slate, dustCol, dustAmt);
  // where the chalk is brighter than the dust keep the chalk; where the slate
  // shows through the marks, show the dust
  col = max(col, lifted);

  // a faint airborne haze of chalk dust near the bottom, where it settles
  col += vec3f(0.02, 0.02, 0.018) * dustA * (1.0 - uv.y) * (0.6 + 0.4 * vnoise(q * 0.05 + 2.0));

  return vec4f(col, 1.0);
}
