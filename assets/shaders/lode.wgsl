// Lode — shader.gallery (lode)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: midnight

const SG_C0: vec3f = vec3f(0.231000, 0.510000, 0.965000);
const SG_C1: vec3f = vec3f(0.659000, 0.333000, 0.969000);
const SG_C2: vec3f = vec3f(0.133000, 0.827000, 0.933000);
const SG_C3: vec3f = vec3f(0.957000, 0.247000, 0.369000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// lode (Delve) — a mine shaft sunk through ore-bearing rock, seen straight down
// the bore. Near-black stone walls (low FBM in angle / log-depth space, banded
// into concentric receding rings of rock) salted with tiny hash-scattered
// mineral glints, a few glowing ore veins that snake radially as they descend,
// and one warm waist of light: a tight stationary lamp band at a fixed screen
// radius that rock and glints flare through as they pour past it. The wall pours
// perpetually down the scale axis toward a true-black focal depth at the centre;
// the self-similar log wrap is hidden by the lamp's falloff and the core dark.
//
// Uniforms provided by the runtime:
//   u.time        seconds, monotonically increasing
//   u.resolution  drawing-buffer size in device pixels
//   vec2f(0.0)       pointer in device pixels (0,0 when absent)
//   1.0  devicePixelRatio used for the buffer
//   u_palette[4]  four glow colours, themeable (linear-ish 0..1 rgb)


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.018, 0.017, 0.024); // near-black shaft base
const PI: f32 = 3.14159265;
const TWO_PI: f32 = 6.28318531;
const CELLS_A: f32 = 20.0;  // angular cells around the bore
const CELLS_D: f32 = 3.0;   // log-depth cells per scale octave
const WRAP: f32 = 6.0;   // log-depth octaves before self-similar wrap

// ---- hashes (no textures) ----
fn hash11(p_in: f32) -> f32 {
  var p = p_in;

  p = fract(p * 0.2317);
  p *= p + 23.19;
  p *= p + p;
  return fract(p);
}
fn hash21(p: vec2f) -> f32 {

  var p3 = fract(vec3f(p.xyx) * 0.2317);
  p3 += dot(p3, p3.yzx + 23.19);
  return fract((p3.x + p3.y) * p3.z);
}

// value noise periodic in x with period px (keeps the angular seam invisible)
fn pnoise(p: vec2f, px: f32) -> f32 {

  var i = floor(p);
  var f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  let ix0 = sg_mod(i.x, px);
  let ix1 = sg_mod(i.x + 1.0, px);
  let a = hash21(vec2f(ix0, i.y));
  let b = hash21(vec2f(ix1, i.y));
  let c = hash21(vec2f(ix0, i.y + 1.0));
  var d = hash21(vec2f(ix1, i.y + 1.0));
  return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
// fbm with the angular axis kept periodic; px is the period at the base octave
fn fbmA(p: vec2f, px: f32) -> f32 {

  var s = 0.0;
  var amp = 0.5;
  var freq = 1.0;
  for (var i: i32 = 0; i < 4; i += 1) {
    s += amp * pnoise(vec2f(p.x * freq, p.y * freq), px * freq);
    freq *= 2.0;
    amp *= 0.5;
  }
  return s;
}

// 1D smooth noise for vein angle wander, periodic over period px
fn n1p(x: f32, px: f32) -> f32 {

  var i = floor(x);
  var f = fract(x);
  f = f * f * (3.0 - 2.0 * f);
  let i0 = sg_mod(i, px);
  let i1 = sg_mod(i + 1.0, px);
  return mix(hash11(i0), hash11(i1), f);
}

// cyclic triangular weight for a palette entry on a 0..4 wheel
fn wheelW(s: f32, c: f32) -> f32 {

  var d = abs(s - c);
  return max(0.0, 1.0 - min(d, 4.0 - d));
}
fn palMix(c0: vec3f, c1: vec3f, c2: vec3f, c3: vec3f, s01: f32) -> vec3f {

  let s = fract(s01) * 4.0;
  let w0 = wheelW(s, 0.0);
  let w1 = wheelW(s, 1.0);
  let w2 = wheelW(s, 2.0);
  let w3 = wheelW(s, 3.0);
  return (c0 * w0 + c1 * w1 + c2 * w2 + c3 * w3) / max(w0 + w1 + w2 + w3, 0.001);
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let ctr = res * 0.5;
  let hs = min(res.x, res.y) * 0.5;

  let t = u.time;

  // screen position relative to bore axis, normalised to half-size
  let q = (fc - ctr) / hs;
  var r = max(length(q), 1e-4);     // radius, 0 at the bore axis
  let ang = atan2(q.y, q.x);           // -PI..PI

  // --- log-polar mapping: depth = -log(r) so equal screen rings map to equal
  // depth steps; descent advances depth linearly, a never-arriving zoom ---
  let descent = t * param(0u);
  let depth = -log(r) + descent;    // grows toward centre; +descent pours it down

  // angle in cell units (continuous around the loop)
  let au = (ang / TWO_PI + 0.5) * CELLS_A; // 0..CELLS_A
  let du = depth * CELLS_D;                 // depth in cells

  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  var col = BG;

  // ---- rock wall: FBM grain in (angle, depth) space, BANDED into concentric
  // receding rings so the shaft wall actually reads as nested hollows. The
  // banding is a periodic function of log-depth so rings are born at the frame
  // edge and swallowed by the centre, every ring self-similar to the next. ----
  let grain = fbmA(vec2f(au, du * 0.55), CELLS_A);
  let grain2 = fbmA(vec2f(au * 2.0, du * 1.3 + 11.0), CELLS_A * 2.0);
  let grainV = clamp(0.7 * grain + 0.45 * grain2, 0.0, 1.0);

  // concentric ring banding: a wavy dark groove every CELLS_D depth-cells.
  // the ring crest is offset per-angle by the grain so grooves are not perfect
  // circles but ragged stone ledges receding down the bore.
  let ringPhase = du + 0.9 * grain;            // depth carries rings inward
  let ringWave = 0.5 - 0.5 * cos(ringPhase * TWO_PI); // 0 in groove, 1 on ledge
  let ledge = pow(ringWave, 1.6);          // sharpen the ledge crests
  // rock luminance: grainy stone face, darkened in the ring grooves
  var rock = grainV * (0.30 + 0.70 * ledge);
  rock = pow(rock, 1.35);                         // contrast the grain up

  // faint stone tint that cycles with depth phase (rings shift hue as they fall)
  let rockTint = palMix(c0, c1, c2, c3, depth * 0.18 + 0.5);
  // ambient (un-lit) wall: dim but visibly grainy and ringed
  col += rockTint * rock * 0.28;

  // faint cosmic-dust haze filling the void between veins (in-shader; replaces the
  // old gradient backdrop): a low-frequency palette-tinted nebula cloud, textured
  // by FBM so it reads as drifting dust rather than a flat wash. Shaped by the
  // core-dark + vignette at the end so the focal centre and frame edge still fall off.
  var dust = fbmA(vec2f(au * 0.6, du * 0.32 + 5.0), CELLS_A);
  dust = pow(clamp(dust, 0.0, 1.0), 1.4);
  let dustCol = palMix(c0, c1, c2, c3, depth * 0.12 + 0.3);
  col += dustCol * dust * 0.13;

  // ---- the stationary lamp band: fixed SCREEN radius, no motion ----
  // a TIGHT warm ring of illumination at r = param(2u); brightness events
  // happen here as the descending wall passes through, without any light moving.
  let lr = clamp(param(2u), 0.05, 0.95);
  let sigma = 0.085;                            // band half-width, frac of half-size
  let band = exp(-pow((r - lr) / sigma, 2.0)); // discrete warm waist
  let lampCol = mix(c3, c2, 0.30);              // warmest palette colour leads

  // lamp flares the rock wall as it passes through the waist (its grain/rings
  // light up where they cross the band, then dim again above and below)
  col += mix(rockTint, lampCol, 0.45) * rock * band * 1.55;
  // a faint warm wash of the band itself catching dust in the air of the shaft
  col += lampCol * band * (0.05 + 0.12 * grainV);

  // ---- mineral glints: TINY hash-scattered point sparkles in log-polar cells ----
  // iterate a small fixed jitter neighbourhood so points near cell borders are
  // not clipped; each cell may carry at most one tiny sparkle gated by density.
  var glintAcc = 0.0;
  var glintCol = vec3f(0.0);
  let cellF = vec2f(au, du);
  for (var dy: i32 = -1; dy <= 1; dy += 1) {
    for (var dx: i32 = -1; dx <= 1; dx += 1) {
      let cell = floor(cellF) + vec2f(f32(dx), f32(dy));
      // wrap the angular cell index so the seam matches (mod CELLS_A)
      let cidA = sg_mod(cell.x, CELLS_A);
      // wrap depth cell over the period so the field is periodic across WRAP
      let cidD = sg_mod(cell.y, WRAP * CELLS_D);
      let h = hash21(vec2f(cidA, cidD));
      // density threshold: only a fraction of cells carry a sparkle
      if (h < param(1u)) {
        // sub-cell position of the sparkle
        let jit = vec2f(hash21(vec2f(cidA + 3.1, cidD + 7.7)),
                        hash21(vec2f(cidA + 5.3, cidD + 1.9)));
        let pc = cell + jit;          // sparkle centre in cell space
        let d = cellF - pc;
        let dist2 = dot(d, d);
        // tiny point — small enough to read as dust, not bokeh, but with a
        // faint halo so the field's density is legible across the wall.
        let bright = 0.6 + 0.7 * hash11(h * 91.7);
        let core = exp(-dist2 / 0.0024);     // tight pinprick
        let halo = exp(-dist2 / 0.020);      // faint dust halo
        let spark = core + 0.35 * halo;
        // glints hash across all four palette colours
        let gc = palMix(c0, c1, c2, c3, hash11(h * 13.3));
        glintAcc += spark * bright;
        glintCol += gc * spark * bright;
      }
    }
  }
  // glints are present in shadow and flare bright through the lamp band —
  // raised base so the ore sparkle reads across the dim wall, not just at the lamp
  let glintLit = 1.6 + 2.4 * band;
  col += glintCol * glintLit;
  col += vec3f(glintAcc) * (0.50 + band * 0.35);   // hard glint highlight

  // ---- ore veins: continuous glowing curves whose centreline angle wanders
  // WITH log-depth, so each vein snakes radially as it descends rather than
  // standing as a fixed spoke. Sampled along the radius via depth->angle. ----
  var veins = 0.0;
  for (var v: i32 = 0; v < 3; v += 1) {
    let fv = f32(v);
    // the vein's angle is a function of depth: base offset + a depth-driven
    // wander. Because both fragment angle and depth are known, the centreline
    // angle at THIS depth tells us how far the fragment is from the vein.
    // de-symmetrised base angle (hash-spread, NOT an even 120-deg pinwheel) and
    // a strong depth-driven wander so each vein snakes as it descends.
    let seed = hash11(fv * 23.7 + 4.0);
    let wob = (n1p(depth * 1.3 + fv * 7.0, WRAP) - 0.5) * 4.2;
    let baseA = (seed * TWO_PI) - PI + wob;      // snakes with depth
    // angular distance from the fragment to the vein centre line (wrapped)
    var da = ang - baseA;
    da = atan2(sin(da), cos(da));                   // wrap to -PI..PI
    // thin glowing stroke: a tight bright spine with a faint halo.
    // width shrinks toward the centre so the stroke tapers down the bore.
    let w = (0.030 + 0.035 * r) + 0.012 * band;  // thin thread
    let stroke = 1.0 * exp(-(da * da) / (w * w * 0.25))
                 + 0.40 * exp(-(da * da) / (w * w));
    // veins live in the wall: fade at the dead-black centre and the frame edge
    let reach = smoothstep(0.04, 0.20, r) * (1.0 - smoothstep(0.88, 1.15, r));
    // veins are richer where the wall grain is rich (embedded in the rock)
    veins += stroke * reach * (0.5 + 0.5 * grainV);
  }
  let veinCol = mix(c3, lampCol, 0.2);            // warmest palette colour
  let veinLit = 0.55 + 1.8 * band;               // veins flare through the lamp too
  col += veinCol * veins * veinLit * param(3u) * 2.1;

  // ---- focal depth: true black always waits at the centre ----
  let coreDark = smoothstep(0.0, 0.14, r);
  col *= coreDark;

  // edge vignette to compose the framing and hide the outer wrap
  let vign = 1.0 - smoothstep(0.74, 1.18, r);
  col *= mix(0.50, 1.0, vign);

  // gentle tonemap to keep accents luminous without blowing out
  col = col / (1.0 + col * 0.44);

  return vec4f(col, 1.0);
}
