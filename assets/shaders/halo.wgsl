// Halo — shader.gallery (halo)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: midnight

const SG_C0: vec3f = vec3f(0.231000, 0.510000, 0.965000);
const SG_C1: vec3f = vec3f(0.659000, 0.333000, 0.969000);
const SG_C2: vec3f = vec3f(0.133000, 0.827000, 0.933000);
const SG_C3: vec3f = vec3f(0.957000, 0.247000, 0.369000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// halo (Shoal) — total solar eclipse. A dark occulting moon hangs high in the
// frame; around its limb a bright corona blazes, its edge frayed by ridged FBM
// into the filamental streamers and prominences of a real eclipse. A thin warm
// chromosphere rings the limb, and a faint diamond-ring point flares at one
// side. Behind it all a slow-twinkling starfield fills the deep space, occulted
// to black behind the moon. The corona crawls slowly around the disc (noise
// drifted in the angular domain, periodic so seamless), the radius breathes a
// few percent, and the streamers shimmer with warped noise — all very slow,
// nothing that can reset.
//
// Uniforms provided by the runtime:
//   u.time        seconds, monotonically increasing
//   u.resolution  drawing-buffer size in device pixels
//   vec2f(0.0)       pointer in device pixels (0,0 when absent)
//   1.0  devicePixelRatio used for the buffer
//   u_palette[4]  four glow colours, themeable (linear-ish 0..1 rgb)


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.018, 0.020, 0.032); // deep space base
// disc centre lift above frame centre, as a FRACTION of disc radius so the
// eclipse hangs high at every radius and its full corona stays on-screen.
const DISC_LIFT_FRAC: f32 = 0.55;

// --- hash / value noise (no textures, all procedural) ---
fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(123.34, 345.45));
  p += dot(p, p + 34.345);
  return fract(p.x * p.y);
}

fn vnoise(p: vec2f) -> f32 {

  let i = floor(p);
  var f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  let a = hash21(i + vec2f(0.0, 0.0));
  let b = hash21(i + vec2f(1.0, 0.0));
  let c = hash21(i + vec2f(0.0, 1.0));
  var d = hash21(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Periodic value noise on an angle a (0..2pi): sample on a circle of radius rad in a
// noise plane so it is seamless in the angular domain, plus a free axis (radius/time).
fn pnoise1(a: f32, freq: f32, yaxis: f32) -> f32 {

  let q = vec2f(cos(a), sin(a)) * freq;
  return vnoise(q + vec2f(0.0, yaxis));
}

// ridged fbm over the periodic angular noise — feathery filamental tongues for the corona
fn ridgedFray(a: f32, yaxis: f32) -> f32 {

  var sum = 0.0;
  var amp = 0.55;
  var fr = 3.0;
  for (var o: i32 = 0; o < 4; o += 1) {
    var n = pnoise1(a, fr, yaxis * (1.0 + f32(o) * 0.37));
    n = 1.0 - abs(2.0 * n - 1.0);   // ridge
    n = n * (0.55 + 0.45 * n);
    sum += n * amp;
    amp *= 0.5;
    fr  *= 2.0;
  }
  return sum; // ~0..1 range
}

// one starfield layer: hashed cells, sparse points, slow twinkle. `thresh` gates how
// many cells carry a star (higher thresh = fewer stars).
fn starLayer(fc: vec2f, cell: f32, t: f32, seed: f32, thresh: f32) -> f32 {

  let g = fc / cell;
  let id = floor(g);
  let f = fract(g);
  let h3 = hash21(id + seed + 19.3);
  if (h3 > thresh) { return 0.0; }                 // only some cells host a star
  let h = hash21(id + seed);
  let h2 = hash21(id + seed + 7.1);
  let sc = vec2f(0.25 + 0.5 * h, 0.25 + 0.5 * h2);
  var d = length((f - sc) * vec2f(1.0, 1.0));
  let bright = 0.35 + 0.65 * h;
  let tw = 0.55 + 0.45 * sin(t * (0.4 + h * 1.8) + h2 * 6.283185307);
  // tiny crisp core + faint glow
  let core = smoothstep(0.06, 0.0, d);
  let glow = smoothstep(0.22, 0.0, d) * 0.25;
  return (core + glow) * bright * tw;
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let pr = 1.0;
  let t = u.time;

  // guard radius against a 0 param so nothing divides by zero / explodes
  let reqR = max(param(0u), 1.0) * pr;
  // Soft-contain the moon so its full corona stays on-screen across the radius range.
  let maxVisible = min(res.x, res.y) * 0.34;
  let baseR = maxVisible * (1.0 - exp(-reqR / maxVisible));

  // moon centre: user-positioned, with a barely-there slow horizontal drift.
  let offX = res.x * clamp(param(4u), 0.0, 1.0) + 0.018 * res.x * sin(t * 0.05);
  let ctr = vec2f(offX, res.y * clamp(param(5u), 0.0, 1.0));
  let d = fc - ctr;
  let r = length(d);
  let ang = atan2(d.y, d.x); // -pi..pi

  let wsp = param(2u);

  // --- breathing radius (a few percent, very slow, two incommensurate sines) ---
  let breathe = 1.0
    + 0.014 * sin(t * (0.18 + wsp * 0.6))
    + 0.010 * sin(t * (0.11 + wsp * 0.37) + 1.7);
  let discR = baseR * breathe;

  // --- corona streamers: ridged noise drifting in the angular domain (crawls) ---
  let crawl = t * wsp * 0.40;
  let fray = ridgedFray(ang + crawl, t * wsp * 0.22);
  // a coarser angular modulation carves the broad streamer "petals" of a real corona
  let petals = 0.55 + 0.45 * pnoise1(ang * 1.0 + crawl * 0.6, 2.0, t * wsp * 0.1);

  // limb of the moon (the eclipse edge)
  let aa = max(2.0 * pr, baseR * 0.010);
  let limbR = discR;
  let inMoon = 1.0 - smoothstep(limbR - aa, limbR + aa, r);

  // Palette fallback (headless contexts can leave the array zeroed).
  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  let coolSky = mix(c0, c2, 0.5);      // cool corona body
  let warmCore = mix(c3, c1, 0.35);     // warm chromosphere / inner glow
  let starCol = mix(vec3f(0.85, 0.90, 1.0), c2, 0.18);

  var col = BG;

  // ---- starfield (behind everything; occulted by the moon) -----------------
  let dens = clamp(param(3u), 0.0, 2.0);
  // higher density -> lower threshold (more cells populated) + a finer layer
  let th1 = mix(0.82, 0.30, clamp(dens, 0.0, 1.0));
  let th2 = mix(0.92, 0.55, clamp(dens, 0.0, 1.0));
  let stars = starLayer(fc, 46.0 * pr, t, 3.0,  th1)
              + starLayer(fc, 26.0 * pr, t, 91.0, th2) * 0.8;
  // faint nebular wash so deep space isn't a flat black field
  let neb = vnoise(fc / res.xy * 3.0 + vec2f(t * wsp * 0.03, 0.0));
  col += coolSky * 0.030 * neb;
  col += starCol * stars * 1.4;

  // ---- the corona: bright frayed light streaming OUTWARD from the limb ------
  // base outward falloff, reach scaled by param(1u); streamers from fray+petals.
  let reach = baseR * (0.9 + 1.4 * param(1u));
  let outward = exp(-max(r - limbR, 0.0) / max(reach, 1.0));
  // streamer modulation: brighter where fray/petals peak, feathery between
  let streamer = (0.35 + 0.65 * fray) * petals;
  let corona = outward * streamer * (1.0 - inMoon);
  // a hot, thin inner ring right at the limb (the brightest part of the corona)
  let innerRing = exp(-abs(r - limbR) / max(aa * 6.0, baseR * 0.045)) * (1.0 - inMoon);

  // ---- chromosphere: thin warm band hugging the limb ------------------------
  let chromo = exp(-abs(r - limbR) / max(baseR * 0.02, aa * 3.0)) * (1.0 - inMoon);

  // ---- diamond-ring / Baily's bead: one bright flare at a fixed limb angle --
  let beadAng = 2.3 + 0.15 * sin(t * 0.07);
  let angDiff = ang - beadAng;
  // keep periodic: use cos of the difference for a smooth angular bump
  let beadGate = pow(max(0.0, cos(angDiff)), 40.0);
  let bead = beadGate * exp(-abs(r - limbR) / max(baseR * 0.05, aa * 4.0)) * (1.0 - inMoon);

  // ---- composite the light --------------------------------------------------
  col += coolSky  * corona     * 0.9;
  col += mix(coolSky, vec3f(1.0), 0.4) * innerRing * 0.8;
  col += warmCore * chromo     * 0.7;
  col += mix(warmCore, vec3f(1.0), 0.6) * bead * 1.6;

  // ---- the moon: dark occulting disc ---------------------------------------
  // earthshine — a barely-there cool fill so the moon reads as a sphere, not a hole;
  // and it occults the stars/nebula behind it.
  let earthshine = (0.012 + 0.010 * (1.0 - clamp(r / max(limbR, 1.0), 0.0, 1.0)));
  col = mix(col, coolSky * earthshine, inMoon);

  // gentle vignette to settle the corners into deep space
  let vign = 1.0 - smoothstep(0.50, 1.18, length((fc - res * 0.5) / res));
  col *= mix(0.7, 1.0, vign);

  // soft luminance ceiling: keep the brightest corona/bead a luminous glow, never a
  // blown-out white clip. Roll values above ~0.7 toward a gentle shoulder.
  let lum = dot(col, vec3f(0.2126, 0.7152, 0.0722));
  let capped = lum - 0.30 * max(lum - 0.68, 0.0);
  col *= capped / max(lum, 1e-4);

  return vec4f(col, 1.0);
}
