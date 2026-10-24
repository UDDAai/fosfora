// Quintet — shader.gallery (quintet)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: ember

const SG_C0: vec3f = vec3f(0.984000, 0.451000, 0.133000);
const SG_C1: vec3f = vec3f(0.957000, 0.247000, 0.369000);
const SG_C2: vec3f = vec3f(0.965000, 0.620000, 0.043000);
const SG_C3: vec3f = vec3f(0.561000, 0.149000, 0.305000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// quintet (Veil) — a five-voice sibling of Triad. Where Triad crosses three
// standing plane-wave trains at 120 degrees into a hexagonal honeycomb, Quintet
// crosses FIVE trains at 72 degrees. Five-fold symmetry cannot tile the plane,
// so the summed antinodes settle into a decagonal QUASICRYSTAL: ten-pointed
// star rosettes nested at every scale, a pattern that never quite repeats.
// Hash-scheduled strikes flood all five trains; they ring down at five separate
// rates, so the crisp stars melt through partial five-, four- and two-fold
// phases as voices drop out, before sinking to a faint residual lattice.
//
// Uniforms provided by the runtime:
//   u.time        seconds, monotonically increasing
//   u.resolution  drawing-buffer size in device pixels
//   vec2f(0.0)       pointer in device pixels (0,0 when absent)
//   1.0  devicePixelRatio used for the buffer
//   u_palette[4]  four glow colours, themeable (0..1 rgb)


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.035, 0.035, 0.043); // near-black bronze-dark base
const TAU: f32 = 6.2831853;
const NSTR: f32 = 12.0; // strikes we look back over (constant loop bound)

// small deterministic hash -> 0..1
fn hash11(n: f32) -> f32 {
 return fract(sin(n * 91.3458) * 47453.5453); }

// hash a strike index to a single shared 2D lattice shift (0..1 turns per axis).
// One shift translates the WHOLE quasicrystal rigidly so the five antinode grids
// stay co-registered into one coherent decagonal lattice rather than beating.
fn strikeShift(k: f32) -> vec2f {

  return vec2f(hash11(k + 1.7), hash11(k + 33.1));
}

// cyclic triangular weight for a palette entry centred at c on a 0..4 wheel
fn wheelW(s: f32, c: f32) -> f32 {

  let d = abs(s - c);
  return max(0.0, 1.0 - min(d, 4.0 - d));
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let pr = 1.0;
  let ctr = res * 0.5;
  let t = u.time;

  // guard params so the full slider range is safe
  let refScale = min(u.resolution.x, u.resolution.y) / (max(1.0, 1.0) * 400.0);
  let cell = max(param(0u), 8.0) * refScale * pr;     // antinode cell scale in px
  let gap = max(param(1u), 1.0);                    // mean seconds between strikes
  let ring = max(param(2u), 0.5);                     // ring-down time constant

  // spatial frequency of each plane-wave train (radians per px)
  let kf = TAU / cell;

  // five unit directions 72 degrees apart, rotated 6 degrees off the axes so the
  // decagon rows are not pixel-column aligned. Five-fold cannot tessellate, so
  // the summed field is a genuine quasicrystal (Penrose-class), not a grid.
  const A0: f32 = 0.10471976; // base 6 deg in radians
  const STEP: f32 = 1.25663706; // 72 deg in radians
  let d0 = vec2f(cos(A0 + 0.0 * STEP), sin(A0 + 0.0 * STEP));
  let d1 = vec2f(cos(A0 + 1.0 * STEP), sin(A0 + 1.0 * STEP));
  let d2 = vec2f(cos(A0 + 2.0 * STEP), sin(A0 + 2.0 * STEP));
  let d3 = vec2f(cos(A0 + 3.0 * STEP), sin(A0 + 3.0 * STEP));
  let d4 = vec2f(cos(A0 + 4.0 * STEP), sin(A0 + 4.0 * STEP));

  // SKEW gives each voice a slow, independent PHASE drift (not a frequency
  // detune — detuning would stretch the lattice and break the five-fold
  // isotropy into directional banding). Equal frequencies keep the quasicrystal
  // perfectly decagonal while the wandering phases make it breathe and morph
  // through its inflation symmetry; at zero the pattern locks static.
  let drift = param(3u) * 9.0;
  let dph0 = sin(t * 0.037 + 0.0) * drift;
  let dph1 = sin(t * 0.041 + 1.3) * drift;
  let dph2 = sin(t * 0.034 + 2.6) * drift;
  let dph3 = sin(t * 0.043 + 3.9) * drift;
  let dph4 = sin(t * 0.039 + 5.2) * drift;

  // --- accumulate ring-down amplitude per train over recent strikes ---
  // A strike at index k happens at a hash-jittered time near k*gap. Each strike
  // floods all five trains; they decay at five separated rates so the stars melt
  // mid-decay through lower-order symmetric phases as voices fade unevenly.
  let now = t / gap;
  let base = floor(now);

  // per-train running amplitude (a0..a4), plus a single shared lattice shift the
  // whole quasicrystal currently rides (accumulated as a circular mean per axis
  // so crossfading strikes reseat the lattice smoothly without snapping)
  var a0 = 0.0;
  var a1 = 0.0;
  var a2 = 0.0;
  var a3 = 0.0;
  var a4 = 0.0;
  var shR = vec2f(0.0);   // amplitude-weighted shared shift (real, per axis)
  var shI = vec2f(0.0);   // amplitude-weighted shared shift (imag, per axis)
  var hotAmp = 0.0;      // recency of the most recent strike, for c3 tint

  for (var i: f32 = 0.0; i < NSTR; i += 1.0) {
    let k = base - i + 1.0;            // strike index (most recent first-ish)
    let jit = (hash11(k + 5.0) - 0.5) * 0.7; // hash-jitter the strike time
    let tk = (k + 0.5 + jit) * gap;     // absolute strike time in seconds
    let dt = t - tk;                    // time since this strike
    if (dt < 0.0) { continue; }                 // strike is in the future

    // five clearly separated decay rates: fastest voices die first, leaving the
    // slow voices ringing. As trains drop the five-fold rosette decomposes into
    // simpler few-train interference before the residual restores the quasicrystal.
    let att = smoothstep(0.0, 0.18, dt);  // sharp attack ~0.18s (crossfade-in)
    let e0 = exp(-dt * (0.40 / ring)) * att;
    let e1 = exp(-dt * (0.78 / ring)) * att;
    let e2 = exp(-dt * (1.25 / ring)) * att;
    let e3 = exp(-dt * (1.85 / ring)) * att;
    let e4 = exp(-dt * (2.60 / ring)) * att;

    a0 += e0; a1 += e1; a2 += e2; a3 += e3; a4 += e4;

    let sh = strikeShift(k);
    let wt = e0 + e1 + e2 + e3 + e4;      // total strike amplitude (shift weight)
    shR += wt * cos(sh * TAU);
    shI += wt * sin(sh * TAU);

    hotAmp = max(hotAmp, exp(-dt * 3.5 / ring) * att);
  }

  // faint residual lattice so the pan stays readable at rest. Small enough that a
  // few surviving trains can dominate mid-decay and show partial symmetry; the
  // residual only restores the balanced quasicrystal once all trains have died.
  let residual = 0.040;
  a0 += residual; a1 += residual; a2 += residual; a3 += residual; a4 += residual;

  // resolved shared lattice shift (radians) from the weighted unit-vector mean.
  let shift = atan2(shI, shR + 1e-4);

  // overall ring-down energy: the pan glows brightest just after a strike and
  // sinks toward the dim residual between blows, so the whole frame breathes
  let norm = a0 + a1 + a2 + a3 + a4;
  let energy = clamp(norm * 0.34, 0.0, 1.0);

  // palette fallback (headless contexts can leave the array zeroed)
  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  // each train owns a colour spread around the 0..4 palette wheel, so a phase
  // where one voice rings alone tints toward that voice. Precompute the five.
  let tc0 = (c0*wheelW(0.0,0.0) + c1*wheelW(0.0,1.0) + c2*wheelW(0.0,2.0) + c3*wheelW(0.0,3.0));
  let tc1 = (c0*wheelW(0.8,0.0) + c1*wheelW(0.8,1.0) + c2*wheelW(0.8,2.0) + c3*wheelW(0.8,3.0));
  let tc2 = (c0*wheelW(1.6,0.0) + c1*wheelW(1.6,1.0) + c2*wheelW(1.6,2.0) + c3*wheelW(1.6,3.0));
  let tc3 = (c0*wheelW(2.4,0.0) + c1*wheelW(2.4,1.0) + c2*wheelW(2.4,2.0) + c3*wheelW(2.4,3.0));
  let tc4 = (c0*wheelW(3.2,0.0) + c1*wheelW(3.2,1.0) + c2*wheelW(3.2,2.0) + c3*wheelW(3.2,3.0));

  // ---- supersampled quasicrystal field --------------------------------------
  // The decagonal pattern is the SUM of the five raw cosines (not each squared):
  // antinode stars sit where all five voices crest constructively at once, the
  // self-similar ten-fold rosette centres of a Penrose-class quasicrystal. We
  // average over a rotated 4-tap sub-pixel kernel to anti-alias the fine fringes
  // into a luminous mesh instead of a moire grille.
  let c = fc - ctr;

  let o0 = vec2f( 0.140,  0.347);
  let o1 = vec2f( 0.347, -0.140);
  let o2 = vec2f(-0.140, -0.347);
  let o3 = vec2f(-0.347,  0.140);

  // accumulated per-train crest weight (for hue) and the normalised sum field
  var bb0 = 0.0;
  var bb1 = 0.0;
  var bb2 = 0.0;
  var bb3 = 0.0;
  var bb4 = 0.0;
  var fAccum = 0.0;

  for (var s: f32 = 0.0; s < 4.0; s += 1.0) {
    let off = select(select(select(o3, o2, (s < 2.5)), o1, (s < 1.5)), o0, (s < 0.5));
    let p = c + off;

    // ONE shared shift projected onto each train so the lattice translates
    // rigidly; the per-voice phase drift (dph*) morphs it without detuning.
    let w0 = cos(dot(p, d0) * kf + dot(shift, d0) + dph0);
    let w1 = cos(dot(p, d1) * kf + dot(shift, d1) + dph1);
    let w2 = cos(dot(p, d2) * kf + dot(shift, d2) + dph2);
    let w3 = cos(dot(p, d3) * kf + dot(shift, d3) + dph3);
    let w4 = cos(dot(p, d4) * kf + dot(shift, d4) + dph4);

    // amplitude-weighted SUM: peaks where the loud voices crest together. Divided
    // by the running amplitude so the field stays in -1..1 as voices decay.
    let sumW = a0*w0 + a1*w1 + a2*w2 + a3*w3 + a4*w4;
    fAccum += sumW / max(norm, 0.001);

    // crest weighting per voice (rectified) so hue follows the constructive trains
    bb0 += a0 * max(w0, 0.0); bb1 += a1 * max(w1, 0.0); bb2 += a2 * max(w2, 0.0);
    bb3 += a3 * max(w3, 0.0); bb4 += a4 * max(w4, 0.0);
  }
  bb0 *= 0.25; bb1 *= 0.25; bb2 *= 0.25; bb3 *= 0.25; bb4 *= 0.25;

  // field is the anti-aliased mean of the five cosines, remapped to 0..1. The
  // QUASICRYSTAL reads when this field is shown as direct tone: bright plateaus
  // are constructive PENTAGONS, dark hollows the ten-pointed STARS between them,
  // interlocking into self-similar rosettes a hexagon or grid can never make.
  let field = fAccum * 0.25 * 0.5 + 0.5;       // 0..1, anti-aliased

  // CONTOUR-LINE rendering. Glowing the antinode cells gives a soft dot field
  // (the gallery has too many). Instead we draw the field's iso-contours: nested
  // rings around every extremum that interlock into the pentagon and ten-pointed
  // star TILING of the quasicrystal — luminous thread, not beads. Many thin
  // levels keep the lines crisp and the five-fold read unmistakable.
  let NLEV = 9.0;                                  // contour levels across 0..1
  let lev = field * NLEV;
  let fr = fract(lev);
  let dist = min(fr, 1.0 - fr);                    // 0 exactly on a contour line
  var lines = 1.0 - smoothstep(0.0, 0.22, dist);   // glowing iso-line thread
  lines = pow(lines, 0.7);                           // fatten and lift the thread
  // a faint plateau glow on the bright (constructive) side so the pentagons read
  // as lit tiles behind the thread instead of pure black
  let plateau = smoothstep(0.58, 0.95, field);
  let peakSide = smoothstep(0.46, 0.54, field);    // 0 = hollow, 1 = antinode peak
  let arms = lines;                                // alias for compose below
  let cellGlow = plateau;
  let web = lines;

  // hue blends the five trains by their LOCAL instantaneous contribution, gamma-
  // boosted so a momentarily dominant voice claims its colour clearly.
  let g0 = bb0*bb0;
  let g1 = bb1*bb1;
  let g2 = bb2*bb2;
  let g3 = bb3*bb3;
  let g4 = bb4*bb4;
  let gn = g0 + g1 + g2 + g3 + g4 + 1e-4;
  var hue = (tc0*g0 + tc1*g1 + tc2*g2 + tc3*g3 + tc4*g4) / gn;

  // colour 3 appears only as a hot tint in the first moments after a strike
  hue = mix(hue, c3, clamp(hotAmp * 0.6, 0.0, 0.65));

  // the contour thread cycles continuously through the palette by FIELD LEVEL, so
  // each nested ring shimmers a different hue and the inflation of the tiling is
  // legible as a colour gradient; the per-voice blend warms the constructive side.
  let sLev = fract(field * 1.5) * 4.0;
  let lw0 = wheelW(sLev, 0.0);
  let lw1 = wheelW(sLev, 1.0);
  let lw2 = wheelW(sLev, 2.0);
  let lw3 = wheelW(sLev, 3.0);
  let levHue = (c0*lw0 + c1*lw1 + c2*lw2 + c3*lw3) / max(lw0+lw1+lw2+lw3, 0.001);
  let cellHue = mix(levHue, hue, 0.4 * peakSide);

  // compose: dark base + the glowing decagonal contour thread, with a faint lit
  // plateau on the constructive tiles. Brightness tracks ring-down energy.
  let drive = mix(0.22, 1.0, energy);
  var col = BG;
  col += cellHue * arms * 1.05 * drive;      // glowing pentagon/star thread
  col += hue * cellGlow * 0.30 * drive;      // faint lit constructive tiles

  // bloom on the brightest thread for a soft struck-metal shimmer
  let bloom = smoothstep(0.5, 1.0, lines * drive);
  col += cellHue * bloom * bloom * 0.40 * energy;

  // radial vignette: keep the rim dark, the struck centre luminous
  let vign = 1.0 - smoothstep(0.18, 1.02, length((fc - ctr) / res));
  col *= mix(0.32, 1.0, vign);

  // keep vec2f(0.0) referenced so the no-pointer face is the only face
  col += 0.0 * (vec2f(0.0).x);

  return vec4f(col, 1.0);
}
