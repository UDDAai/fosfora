// Damask — shader.gallery (damask)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: wine

const SG_C0: vec3f = vec3f(0.659000, 0.149000, 0.286000);
const SG_C1: vec3f = vec3f(0.831000, 0.247000, 0.357000);
const SG_C2: vec3f = vec3f(0.541000, 0.180000, 0.451000);
const SG_C3: vec3f = vec3f(0.380000, 0.090000, 0.196000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// damask (Burnish) - a brocade wallpaper of mirror-symmetric foliate medallions
// on a half-drop repeat. Each cell folds a noise field through an N-fold dihedral
// kaleidoscope into an ornate, perfectly symmetric motif; the ground carries the
// pattern tone-on-tone like real damask, and gold filigree outlines the ornament
// on a deep jewel field. Lit evenly across the whole frame - no dark midriff.
// Comments short/ASCII on purpose (headless-gl poster compiler is fussy).
//
// Uniforms: u.time, u.resolution, vec2f(0.0), 1.0, u_palette[4]



fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;
 p = fract(p * vec2f(123.34, 345.45)); p += dot(p, p + 34.345);
  return fract(p.x * p.y); }
fn vnoise(p: vec2f) -> f32 {

  var i = floor(p);
  var f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  var a = hash21(i);
  var b = hash21(i + vec2f(1.0, 0.0));
  let c = hash21(i + vec2f(0.0, 1.0));
  var d = hash21(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
fn fbm(p_in: vec2f) -> f32 {
  var p = p_in;

  var v = 0.0;
  var amp = 0.5;
  for (var i: i32 = 0; i < 5; i += 1) { v += amp * vnoise(p); p = p * 2.02 + vec2f(5.2, 1.3); amp *= 0.5; }
  return v;
}
fn wheelW(s: f32, c: f32) -> f32 {
 var d = abs(s - c);
  return max(0.0, 1.0 - min(d, 4.0 - d)); }
fn wheelCol(k: f32, c0: vec3f, c1: vec3f, c2: vec3f, c3: vec3f) -> vec3f {

  let s = fract(k) * 4.0;
  var a = wheelW(s, 0.0);
  var b = wheelW(s, 1.0);
  let cc = wheelW(s, 2.0);
  let dd = wheelW(s, 3.0);
  return (c0 * a + c1 * b + c2 * cc + c3 * dd) / max(a + b + cc + dd, 0.001);
}

// one medallion, centred at the origin of gg (cell units), seeded by cid.
// Returns vec3f(mass, line, boss). The figure can grow past its own cell (fill>1)
// so neighbouring medallions OVERLAP rather than crop at the cell edge.
fn medallion(gg: vec2f, cid: vec2f, fill: f32, det: f32, Nf: f32, mt: f32) -> vec3f {

  let gs = gg / fill;
  let r = length(gs);
  if (r > 0.5) { return vec3f(0.0); }             // outside this figure entirely
  let ang = atan2(gs.y, gs.x);
  let sector = 6.2831853 / Nf;
  var fa = sg_mod(ang, sector);
  fa = abs(fa - 0.5 * sector);
  let fg = vec2f(cos(fa), sin(fa)) * r;
  let d1 = fbm(fg * det + cid * 0.3 + 2.1 + mt);
  let d2 = fbm(fg * det * 1.9 + 8.4 - mt);
  let orn = mix(d1, d2, 0.35);
  let radMask = smoothstep(0.5, 0.12, r);
  let mass = smoothstep(0.5, 0.62, orn) * radMask;
  let line = smoothstep(0.035, 0.0, abs(orn - 0.56)) * radMask;
  let boss = smoothstep(0.09, 0.0, r);
  return vec3f(mass, line, boss);
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let pr = 1.0;
  let ctr = res * 0.5;
  let t = u.time;

  let refScale = min(res.x, res.y) / (max(pr, 1.0) * 400.0);
  let cell = max(param(0u), 40.0) * refScale * pr;
  // rotate the whole wallpaper, then add a slow back-and-forth sway so the cloth
  // reads as a gently drifting drape rather than a frozen print.
  let ra = radians(param(6u));
  let cr = cos(ra);
  let sr = sin(ra);
  let rel = mat2x2<f32>(cr, -sr, sr, cr) * (fc - ctr);
  var uv = rel / cell;
  uv += param(4u) * vec2f(sin(t * 0.16), cos(t * 0.13)) * 0.35;

  // element spacing on a half-drop repeat: tightening grows each medallion past
  // its cell so neighbours OVERLAP instead of cropping at the edge. Gather the
  // figure from a 3x3 block of cells (alternate columns half-dropped) and union
  // them, so a fragment can be covered by more than one overlapping medallion.
  let fill = clamp(1.0 - param(9u) * 0.6, 0.35, 1.8);
  let det = 6.0 + param(2u) * 10.0;
  let Nf = max(floor(param(1u) + 0.5), 2.0);
  let mt = t * param(5u) * 0.14;

  let baseCell = floor(uv);
  var mass = 0.0;
  var line = 0.0;
  var boss = 0.0;
  var rNear = 100.0;
  var cellId = baseCell;
  for (var dx: i32 = -1; dx <= 1; dx += 1) {
    for (var dy: i32 = -1; dy <= 1; dy += 1) {
      let cid = baseCell + vec2f(f32(dx), f32(dy));
      let odd = sg_mod(cid.x, 2.0);
      let center = vec2f(cid.x + 0.5, cid.y + 0.5 - odd * 0.5);  // half-drop
      let gg = uv - center;
      let m = medallion(gg, cid, fill, det, Nf, mt);
      mass = max(mass, m.x);
      line = max(line, m.y);
      boss = max(boss, m.z);
      let rr = length(gg);
      if (rr < rNear) { rNear = rr; cellId = cid; }
    }
  }
  let r = rNear / fill;                                     // radial coord (shim)
  let ink = clamp(max(line, boss * 0.9) + mass * 0.25, 0.0, 1.0);

  // --- palette ---
  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  // deep jewel ground, hue drifting gently across the wall (even, no midriff dark)
  let groundHue = 0.08 + 0.10 * sin(uv.x * 0.25) + 0.10 * cos(uv.y * 0.22);
  var jewel = wheelCol(groundHue, c0, c1, c2, c3);
  jewel = mix(vec3f(dot(jewel, vec3f(0.33))), jewel, 1.05) * 0.42 * param(7u); // rich but deep
  // tone-on-tone: the motif body shows as a slightly lifted shade of the ground
  let ground = jewel * (1.0 + 0.55 * mass);

  // gold filigree, faintly themed and shimmering along the line
  let shim = 0.6 + 0.4 * sin(t * (0.4 + param(3u)) + (cellId.x + cellId.y) * 1.7 + r * 18.0);
  var gold = mix(vec3f(1.0, 0.82, 0.42), vec3f(1.0, 0.93, 0.7), shim);
  gold = mix(gold, gold * (0.7 + 0.7 * wheelCol(0.1, c0, c1, c2, c3)), 0.18);
  gold *= param(8u);                                   // gold filigree intensity

  var col = mix(ground, gold, ink);
  col += gold * line * 0.4 * shim;                  // seam glow for bloom

  return vec4f(col, 1.0);
}
