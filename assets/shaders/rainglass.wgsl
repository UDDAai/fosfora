// Rainglass — shader.gallery (rainglass)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: peacock

const SG_C0: vec3f = vec3f(0.071000, 0.612000, 0.694000);
const SG_C1: vec3f = vec3f(0.176000, 0.451000, 0.804000);
const SG_C2: vec3f = vec3f(0.561000, 0.357000, 0.812000);
const SG_C3: vec3f = vec3f(0.965000, 0.737000, 0.286000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// rainglass (Veil) - rain on a window at night. Behind the glass is a lit
// field the eye never sees sharply: a dark ground, a horizon of street lights
// and scattered bokeh discs, built analytically from small additive light
// sources so the same field can be drawn at any defocus. The pane is fogged,
// so the field is drawn very soft and lifted by a grey haze. On the glass sit
// three kinds of water. Running drops fall in tall lattice columns, each one
// wiggling as it goes, stalling and darting the way a runner does on real
// glass, and leaving a trail of small beads above it plus a wiped streak
// where the fog has been cleared and the lights behind come back into focus.
// Static beads sit on a finer lattice, swelling slowly as new drops form and
// vanishing where a runner has just passed through. Every drop is a lens: it
// shows the lit field sharp and inverted through its own bulge, with a dark
// refractive rim and one tight highlight from the light above the window.


// tweakable params (see meta.json; the runtime feeds defaults)

var<private> P0: vec3f;
var<private> P1: vec3f;
var<private> P2: vec3f;
var<private> P3: vec3f;
fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}
fn hash23(p: vec2f) -> vec3f {

  let a = hash21(p);
  let b = hash21(p + vec2f(17.3, 9.1));
  var c = hash21(p + vec2f(41.7, 23.9));
  return vec3f(a, b, c);
}
fn luma(c: vec3f) -> f32 {
 return dot(c, vec3f(0.299, 0.587, 0.114)); }

// a palette pick by a random key, with a little brightness spread
fn lightCol(k: f32) -> vec3f {

  var x = floor(fract(k) * 4.0);
  var c = P0;
  c = mix(c, P1, step(0.5, x));
  c = mix(c, P2, step(1.5, x));
  c = mix(c, P3, step(2.5, x));
  // lights are near white at the core, so lift toward white a touch
  return mix(c, vec3f(1.0), 0.10);
}

// one defocused point light seen through the glass: a disc that grows and
// softens with defocus, with the slightly bright rim a lens iris leaves.
// dist in frame units; base is the sharp point size; soft is 0 (in focus)
// to 1 (fogged)
fn bokeh(dist: f32, base: f32, soft: f32) -> f32 {

  let r = base * (1.0 + 7.0 * soft);
  let w = base * (0.6 + 6.0 * soft);
  var disc = 1.0 - smoothstep(r - w, r + w, dist);
  disc *= 0.75 + 0.45 * (1.0 - smoothstep(0.0, r + w, dist));
  let rim = smoothstep(r - w * 2.5, r, dist) * (1.0 - smoothstep(r, r + w, dist));
  // energy conservation: the bigger the disc, the dimmer it is
  let gain = 1.0 / (1.0 + 4.5 * soft);
  return (disc + rim * 0.5 * soft) * gain;
}

// the lit field behind the pane at frame point p, drawn at defocus soft
fn litField(p: vec2f, soft: f32, t: f32, dens: f32) -> vec3f {

  // ground: night dark, a low band of glow where the street is
  let deep = vec3f(0.016, 0.018, 0.026) + mix(vec3f(luma(P3)), P3, 0.5) * 0.035;
  let glow = mix(vec3f(luma(P0)), P0, 0.6) * 0.20;
  let street = exp(-abs(p.y - 0.34) * 3.0);
  var col = deep + glow * street * 0.7 + P1 * 0.03 * smoothstep(0.7, 0.0, p.y);

  // horizon of street lights: a 1D lattice along x just above the street
  {
    let sx = p.x * 9.0;
    let ix = floor(sx);
    for (var k: i32 = -1; k <= 1; k += 1) {
      let i = ix + f32(k);
      var n = hash23(vec2f(i, 3.0));
      if (n.z > 0.45 + 0.5 * dens) { continue; }
      var c = vec2f((i + 0.5 + (n.x - 0.5) * 0.7) / 9.0, 0.36 + (n.y - 0.5) * 0.05);
      var tw = 0.85 + 0.15 * sin(t * 1.7 + n.x * 20.0);
      col += lightCol(n.y * 3.7 + 0.1) * bokeh(length(p - c), 0.006, soft) * 2.4 * tw;
    }
  }
  // scattered lights across the whole field, two lattice layers
  for (var ly: i32 = 0; ly < 2; ly += 1) {
    let sc =select(7.0, 3.5, ly == 0);
    let o =select(vec2f(0.73, 0.52), vec2f(0.31, 0.17), ly == 0);
    let ymax =select(0.62, 0.9, ly == 0);
    let g = p * sc + o;
    let ig = floor(g);
    for (var y: i32 = -1; y <= 1; y += 1) {
      for (var x: i32 = -1; x <= 1; x += 1) {
        var id = ig + vec2f(f32(x), f32(y));
        var n = hash23(id + f32(ly) * 11.0);
        // fewer lights high up in the dark; more near the street
        var c = (id + 0.5 + (n.xy - 0.5) * 0.8 - o) / sc;
        let keep = 0.25 * dens * (1.0 + 1.5 * smoothstep(ymax, 0.2, c.y));
        if (n.z > keep) { continue; }
        let base = 0.004 + 0.006 * fract(n.x * 7.0);
        var tw = 0.8 + 0.2 * sin(t * (0.9 + n.y) + n.x * 30.0);
        col += lightCol(n.z * 9.3 + n.x) * bokeh(length(p - c), base, soft) * 1.9 * tw;
      }
    }
  }
  return col;
}

// ---- the water --------------------------------------------------------------

// fraction of a runner cycle spent creeping down before it lets go and darts.
// Lower means more runners are mid-dart at any instant, so more of them show a
// smear. The climb amplitude below is solved from this, so the two stay in step.
const RISE: f32 = 0.78;

// the dwell-then-dart curve a runner follows across its cycle. Pulled out of
// runners so the runner own speed can be sampled from it by finite difference.
fn dwell(ph: f32) -> f32 {

  return clamp(ph / RISE, 0.0, 1.0) * smoothstep(1.0, RISE, ph);
}

// running drops in tall lattice columns. p is in frame units. Returns the
// drop mask, the wiped-trail mask, and the lens offset (xy) in .zw
fn runners(p: vec2f, t: f32, scale: f32, seed: f32, amount: f32, smear: f32) -> vec4f {

  let cellAsp = vec2f(1.0, 2.6);                 // cells are tall and narrow
  var st = p * scale / cellAsp;
  st.y += t * 0.36;                              // the lattice scrolls down
  st.x += seed;
  var id = floor(st);
  var n = hash23(id + seed * 7.0);
  var f = st - id;
  var on = step(n.z, amount);

  // the runner: an x that wiggles as it descends, a y that stalls and darts
  var x = (n.x - 0.5) * 0.5;
  let wy = st.y * 7.0;
  x += sin(wy + sin(wy * 0.7 + n.y * 6.28)) * (0.5 - abs(x)) * 0.35 * (n.y - 0.5) * 2.0;
  let cyc = 0.42 * (0.7 + 0.3 * n.x);          // cycles per unit t, per runner
  let ph = fract(t * cyc + n.z);
  // Dwell then dart, and it must NEVER read as a drop crawling up the glass.
  // The lattice scrolls down at 0.36 per unit t; the local y climbs against it,
  // so the climb rate has to stay strictly under 0.36 at every instant. The
  // rise is therefore LINEAR (a smoothstep rise peaks at 1.5x its own average,
  // which is what let fast runners out-climb the scroll and drift upward), and
  // the amplitude is solved from RISE and this runner own cycle rate so the
  // climb is always 0.8 of the scroll. Net: every runner creeps down through the dwell,
  // then the smooth fall lets it dart down the cell.
  let amp = RISE * 0.36 * 0.8 / cyc;
  let saw = dwell(ph);
  var y = -0.5 * amp + amp * saw;
  let dp = vec2f(x, y);

  // how fast this runner is going down the glass right now: the lattice carries
  // it down at 0.36 while its own dwell climbs against that. Creeping through
  // the dwell gives about 0.07, the dart gives well over 2, so the two states
  // are far enough apart to drive a per-drop smear.
  let e = 0.004;
  let dsaw = (dwell(fract(ph + e)) - dwell(fract(ph - e))) / (2.0 * e);
  let spd = max(0.36 - amp * dsaw * cyc, 0.0);

  // main drop: a teardrop, rounder below, drawn out above
  var d = (f - 0.5 - dp) * cellAsp;
  d.y *= 1.0 + 0.5 * smoothstep(0.0, 0.5, d.y) * (0.5 + 0.5 * on);
  // squared weighting so most runners are small and a big one is occasional
  var rad = 0.11 + 0.13 * n.y * n.y;
  // per-drop smear: the faster it runs, the further its tail is drawn out
  // behind it (upward, since it runs down), so a darting drop reads as a
  // streak while a stalled one stays round
  let tail = 1.0 + smear * (0.30 + spd * 1.6);
  d.y /= mix(1.0, tail, step(0.0, d.y));
  let dl = length(d);
  let drop = (1.0 - smoothstep(rad * 0.85, rad, dl)) * on;
  var off = d / rad * drop;

  // trail of small beads above the runner, along its wiggle
  let ty = fract(st.y * 6.0 + n.z * 3.0) - 0.5;
  let above = smoothstep(0.0, 0.05, f.y - 0.5 - y - rad * 0.5);
  let td = vec2f((f.x - 0.5 - x) * cellAsp.x, ty / 6.0 * cellAsp.y);
  let trad = rad * (0.30 + 0.25 * hash21(id + floor(st.y * 6.0 + n.z * 3.0)));
  let tl = length(td);
  var bead = (1.0 - smoothstep(trad * 0.8, trad, tl)) * above * on;
  bead *= smoothstep(1.0, 0.4, f.y - 0.5 - y);   // beads dry up far behind
  off += td / trad * bead * 0.6;

  // the wiped streak the runner has cleared behind it
  let dx = (f.x - 0.5 - x) * cellAsp.x;
  let reach = 1.1 * (1.0 + 0.6 * smear * spd);       // a fast runner wets further back
  let age = clamp((f.y - 0.5 - y) / reach, 0.0, 1.0); // 0 at the drop, 1 far above
  let ww = rad * (0.9 - 0.45 * age);                    // the streak narrows as it dries
  let we = smoothstep(ww * 0.5, ww, abs(dx));
  let fade = above * on * (1.0 - age * age) * smoothstep(1.0, 0.8, f.y);
  let wipe = (1.0 - we) * fade;
  // the wet channel bends light a little at its two edges
  off += vec2f(sign(dx) * we * (1.0 - we) * 0.6, 0.0) * fade;
  return vec4f(max(drop, bead), wipe, off);
}

// static beads on a fine lattice, swelling as they form. returns mask, offset
fn beads(p: vec2f, t: f32, scale: f32, seed: f32, amount: f32) -> vec3f {

  let st = p * scale + seed;
  var id = floor(st);
  var n = hash23(id + seed * 3.1);
  var f = st - id - 0.5;
  var on = step(n.z, amount);
  var c = (n.xy - 0.5) * 0.5;
  let grow = 0.55 + 0.45 * sin(t * 0.25 + n.x * 6.28 + n.y * 3.0);
  var rad = (0.10 + 0.20 * fract(n.z * 5.0)) * grow;
  let d = f - c;
  let m = (1.0 - smoothstep(rad * 0.8, rad, length(d))) * on;
  return vec3f(m, d / rad * m);
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    P0 = SG_C0; P1 = SG_C1; P2 = SG_C2; P3 = SG_C3;
  if (dot(P0,P0)+dot(P1,P1)+dot(P2,P2)+dot(P3,P3) < 1e-5) {
    P0 = vec3f(0.231,0.510,0.965); P1 = vec3f(0.659,0.333,0.969);
    P2 = vec3f(0.133,0.827,0.933); P3 = vec3f(0.957,0.247,0.369);
  }
  let aspect = res.x / res.y;
  let uv = fc / res;
  let p = vec2f(uv.x * aspect, uv.y);       // frame units, height 1
  let t = u.time * clamp(param(1u), 0.0, 3.0);
  let rain = clamp(param(0u), 0.0, 1.5);
  let fog = clamp(param(3u), 0.0, 1.0);
  let lens = clamp(param(4u), 0.0, 2.0);
  let dens = clamp(param(5u), 0.2, 2.0);

  // ---- water on the glass: two runner lattices, two bead lattices
  let smear = clamp(param(2u), 0.0, 2.0);
  let r1 = runners(p, t,        5.0, 0.0, 0.62 * rain, smear);
  let r2 = runners(p, t * 0.8,  9.0, 3.7, 0.55 * rain, smear);
  let wipe = max(r1.y, r2.y);
  var b1 = beads(p, t, 20.0, 1.3, 0.62 * rain);
  var b2 = beads(p, t, 34.0, 5.9, 0.44 * rain);
  // a runner passing through takes the beads with it
  let clear = 1.0 - wipe * 0.9 - max(r1.x, r2.x);
  b1.x *= clamp(clear, 0.0, 1.0);
  b2.x *= clamp(clear, 0.0, 1.0);

  let drop = max(max(r1.x, r2.x), max(b1.x, b2.x));
  let off = r1.zw + r2.zw * 0.8 + b1.yz * 0.55 + b2.yz * 0.35;

  // ---- defocus: fog blurs the pane, wiped streaks clear it, drops focus it
  var soft = fog * (1.0 - 0.55 * wipe);
  soft = mix(soft, 0.16, drop);

  // lens: the drop shows the field inverted through its bulge, so the offset
  // points back across the drop centre and reaches well past the drop
  let look = p - off * (0.42 * lens + 0.02);

  var col = litField(look, soft, t, dens);

  // ---- the fog haze lifts the blacks and greys the colour; wiped away where
  // the runners have cleared it
  let haze = fog * (1.0 - 0.6 * wipe) * (1.0 - drop);
  let hazeCol = mix(vec3f(luma(P2)), P2, 0.12) * 0.075;
  col = mix(col, vec3f(luma(col)), 0.30 * haze);
  col += hazeCol * haze;
  // condensation grain on the fogged glass
  let grain = hash21(floor(fc / max(1.0, 0.5) * 0.5));
  col *= 1.0 - 0.06 * haze * grain;

  // ---- drop shading: dark refractive rim, sky glint, a tight highlight
  let ol = length(off);
  let N = normalize(vec3f(off * 0.9, max(1.0 - ol * ol * 0.6, 0.15)));
  let L = normalize(vec3f(-0.35, 0.75, 0.55));
  let H = normalize(L + vec3f(0.0, 0.0, 1.0));
  let ndh = max(dot(N, H), 0.0);
  var s8 = ndh * ndh; s8 *= s8; s8 *= s8;
  var s64 = s8 * s8; s64 *= s64; s64 *= s64;
  let rim = smoothstep(0.68, 1.0, ol) * drop;
  col *= (1.0 + 0.6 * drop) * (1.0 - 0.5 * rim);
  // the pane glow above the window reflects in the upper face of each drop
  col += (P2 * 0.4 + 0.1) * 0.10 * drop * smoothstep(-0.2, 0.9, off.y) * (1.0 - rim);
  col += (P2 * 0.5 + 0.5) * s64 * 0.9 * drop;
  col += P1 * 0.08 * s8 * drop;
  // beads catch the street glow on their lower edge
  col += P0 * 0.06 * drop * smoothstep(0.2, 0.9, -off.y) * (1.0 - rim);

  return vec4f(col, 1.0);
}
