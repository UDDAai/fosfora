// Stone — shader.gallery (stone)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: verdigris

const SG_C0: vec3f = vec3f(0.235000, 0.620000, 0.561000);
const SG_C1: vec3f = vec3f(0.380000, 0.769000, 0.671000);
const SG_C2: vec3f = vec3f(0.831000, 0.737000, 0.451000);
const SG_C3: vec3f = vec3f(0.137000, 0.404000, 0.404000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// stone — agate / malachite mineral cross-section. The same FBM field that
// drives marble is read as dense iso-bands, but here the bands stay nearly
// concentric and SETTLED: thin luminous mineral seams separate filled bands of
// deep palette colour that cycle hue band-by-band like layered geology, a slab
// of banded agate lit from within. Motion is a very slow breathing drift of the
// band phase and the warp offsets, unbounded, so the slab perpetually grows and
// re-layers with no wrap, no reset. Filled bands keep the frame from going
// black; the bright fortification seams carry the luminous-filament identity.


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.035, 0.035, 0.043); // house near-black
const NBASE: f32 = 22.0;  // base count of mineral bands across the field range

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
  let b = hash21(i + vec2f(1.0, 0.0));
  let c = hash21(i + vec2f(0.0, 1.0));
  let d = hash21(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

const M2: mat2x2<f32> = mat2x2<f32>(vec2f(0.80, 0.60), vec2f(-0.60, 0.80)); // rotate between octaves

fn fbm(p_in: vec2f) -> f32 {
  var p = p_in;

  var a = 0.5;
  var s = 0.0;
  for (var i: i32 = 0; i < 5; i += 1) {
    s += a * vnoise(p);
    p = M2 * p * 2.03 + vec2f(11.7, 5.3);
    a *= 0.5;
  }
  return s * 1.032;
}

// cyclic 4-stop palette loop a->b->c->d->a, u any real (fract taken). No dynamic
// array indexing — plain ternary selects keep it GLSL ES 1.00 safe.
fn palLoop(u: f32, a: vec3f, b: vec3f, c: vec3f, d: vec3f) -> vec3f {

  let x = fract(u) * 4.0;
  let seg = floor(x);
  var f = smoothstep(0.0, 1.0, fract(x));
  let lo =select(select(select(d, c, seg < 2.5), b, seg < 1.5), a, seg < 0.5);
  let hi =select(select(select(a, d, seg < 2.5), c, seg < 1.5), b, seg < 0.5);
  return mix(lo, hi, f);
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  let pr = max(1.0, 0.25);
  let refScale = min(u.resolution.x, u.resolution.y) / (max(1.0, 1.0) * 400.0);
  let scalePx = max(param(2u), 40.0) * refScale * pr;
  let p = fc / scalePx;
  let t = u.time * clamp(param(0u), 0.0, 4.0);
  let warp = max(param(3u), 0.0);
  let NB = NBASE * (0.4 + 1.6 * clamp(param(1u), 0.05, 3.0));

  // ---- gentle, settled domain warp: one slow stage of drifting centred noise
  // (a quarter of the marble churn). The slab flexes; it does not roil.
  let q = vec2f(fbm(p + vec2f( 0.05 * t, -0.04 * t)),
                fbm(p + vec2f(4.3, 1.9) + vec2f(-0.045 * t, 0.05 * t))) - 0.5;
  let pw = p + warp * 1.1 * q;
  let h = fbm(pw + vec2f(0.03 * t, -0.02 * t));

  // ---- dense iso-bands of the field = the agate fortification layers. Phase
  // scrolls slowly with time so the slab keeps growing inward (continuous).
  let s = h * NB + 0.12 * t;
  let band = fract(s);
  let idx = floor(s);
  let edge = min(band, 1.0 - band);     // 0 at a seam, 0.5 mid-band

  // bright thin fortification seam between bands (the luminous filament)
  var seam = 1.0 - smoothstep(0.0, 0.085, edge);
  seam *= seam;
  // soft bleed of the seam into the band body
  let halo = exp(-(edge * edge) / 0.02);

  // shimmer so seams read as mineral, not flat contours
  let amp = 0.66 + 0.34 * vnoise(pw * 2.6 + vec2f(idx * 0.61, -idx * 0.29));

  // filled band body: hue cycles band-by-band so the layers read as geology;
  // a slow large-scale term keeps adjacent slabs from collapsing to one hue.
  let hue = idx * 0.118 + 0.30 * h;
  let layer = palLoop(hue, c0, c1, c2, c3);
  // the seam leans toward the next layer up, like a mineral vein of its own ink
  let vein = palLoop(hue + 0.5, c0, c1, c2, c3);

  // ---- compose: deep coloured slab (never black), luminous seams on top
  var col = BG;
  col += layer * 0.155 * (0.6 + 0.4 * (1.0 - 2.0 * edge)); // filled mineral band body (deep colour, never black)
  col += vein  * halo * 0.34 * amp;                          // bleed around seam
  col += vein  * seam * 0.85 * amp;                          // luminous seam
  col += vec3f(0.14) * seam * seam * amp;                     // white-hot seam core

  // gentle vignette to seat the slab
  let vq = fc / max(u.resolution.xy, vec2f(1.0));
  col *= 1.0 - 0.32 * smoothstep(0.35, 1.05, length(vq - 0.5) * 1.42);

  return vec4f(col, 1.0);
}
