// Scales — shader.gallery (scales)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: abyssal

const SG_C0: vec3f = vec3f(0.039000, 0.518000, 0.553000);
const SG_C1: vec3f = vec3f(0.122000, 0.737000, 0.667000);
const SG_C2: vec3f = vec3f(0.616000, 0.918000, 0.776000);
const SG_C3: vec3f = vec3f(0.063000, 0.286000, 0.451000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// scales (Burnish) - the flank of a fish seen close: rows of rounded scales,
// each row laid a semi-pitch across from the one beneath it and overlapping
// it, so only the lower arc of every scale shows. There is no cell gather
// here: the scales sit on an offset-row lattice, and a pixel asks only the
// six scales that can physically reach it (two columns in the row in front,
// its own row, and the row behind), front to back, in painter order. The
// first one that covers the pixel is the scale we see; every scale in front
// of it that did not cover throws a soft contact shadow onto it from its
// edge. The lattice lives in a log-polar map of the body so scale size grows
// smoothly toward the head and the rows bow gently, and the whole sheet is
// wrapped onto a horizontal body cylinder so the rows foreshorten toward the
// back and the belly. Each scale is a shallow tilted dome with growth rings,
// a raised free edge that catches a rim light, and an iridescent sheen that
// shifts between palette poles with the viewing angle. A slow flex runs down
// the body and tilts the scales as it passes, so a band of highlights travels
// along the flank while the light itself drifts across.


// tweakable params (see meta.json; the runtime feeds defaults)

const ROW: f32 = 0.5;   // row pitch in lattice units (column pitch is 1)

fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}

fn vnoise(p: vec2f) -> f32 {

  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  var a = hash21(i);
  let b = hash21(i + vec2f(1.0, 0.0));
  var c = hash21(i + vec2f(0.0, 1.0));
  let d = hash21(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn luma(c: vec3f) -> f32 {
 return dot(c, vec3f(0.299, 0.587, 0.114)); }

// body surface: frame point p (y in -1..1) -> point on the flank cylinder.
// returns (x along the body, s arc length round the girth) and the angle.
fn flank(p: vec2f, t: f32, flex: f32) -> vec3f {

  let rb = 1.12;                              // body radius in frame units
  let th = asin(clamp(p.y / rb, -0.999, 0.999));
  var s = rb * th;
  // the swimming flex: a slow wave running down the body that lifts and
  // drops the rows a little as it passes
  s += 0.05 * flex * sin(p.x * 2.2 - t * 0.9);
  return vec3f(p.x, s, th);
}

// lattice coords of a flank point: log-polar about a pole off the right of
// the frame, so scale size grows toward the left (the head) and rows bow
fn lattice(fp: vec2f, dens: f32) -> vec2f {

  let z = fp - vec2f(3.4, 0.0);
  let r = length(z);
  var a = atan2(z.y, -z.x);                    // 0 along the body axis
  // one lattice unit is one scale pitch; the same factor on both axes keeps
  // the map conformal, and it is chosen so dens columns fit the frame width
  // at the centre of the body
  let k = dens * 1.06;
  return vec2f(log(r) * k, a * k);
}

// centre, radius and identity of the scale in column i of row k
fn scaleAt(i: f32, k: f32, c: ptr<function, vec2f>, rad: ptr<function, f32>, id: ptr<function, f32>) {

  let ox = 0.5 * sg_mod(k, 2.0);
  *id  = hash21(vec2f(i, k) + 0.37);
  let jx = hash21(vec2f(i, k) * 1.7 + 9.1) - 0.5;
  let jy = hash21(vec2f(i, k) * 2.3 + 4.7) - 0.5;
  *c   = vec2f(i + ox + 0.5 + jx * 0.06, (k + 0.5) * ROW + jy * 0.03);
  *rad = clamp(param(1u), 0.5, 0.75) * (1.0 + (*id - 0.5) * 0.05);
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  let aspect = res.x / res.y;
  let uv = fc / res;
  let p = (uv - 0.5) * vec2f(aspect, 1.0) * 2.0;
  let t = u.time * clamp(param(4u), 0.0, 3.0);
  let flex = clamp(param(3u), 0.0, 1.5);
  let gloss = clamp(param(2u), 0.0, 2.0);
  let dens = max(param(0u), 3.0);

  // ---- body and lattice, plus a finite-difference pixel footprint for AA
  let fk = flank(p, t, flex);
  let q = lattice(fk.xy, dens);
  let pdx = p + vec2f(2.0 / res.y, 0.0);
  let qdx = lattice(flank(pdx, t, flex).xy, dens);
  let px = length(qdx - q);                   // lattice units per device px
  let aa = px * 0.9;

  // ---- painter walk: row in front, own row, row behind; two columns each.
  // The first scale that covers the pixel wins; the ones passed before it
  // (which are in front) leave their edge distance for the contact shadow.
  let j = floor(q.y / ROW);
  var wc = vec2f(0.0);
  var wr = 1.0;
  var wid = 0.0;
  var found = 0.0;
  var dfront = 10.0;                          // distance to the nearest front edge
  var dwin = 10.0;                          // signed distance to the winner edge
  for (var kk: i32 = 1; kk >= -1; kk -= 1) {
    let k = j + f32(kk);
    let ox = 0.5 * sg_mod(k, 2.0);
    let ic = floor(q.x - ox - 0.5);
    // the two columns that bracket the pixel in this row
    var ca: vec2f;
    var cb: vec2f;
    var ra: f32;
    var rb: f32;
    var ia: f32;
    var ib: f32;
    scaleAt(ic, k, &ca, &ra, &ia);
    scaleAt(ic + 1.0, k, &cb, &rb, &ib);
    // scales are a touch wider than tall, so the metric squeezes y a little
    let da = length((q - ca) * vec2f(1.0, 1.08)) - ra;
    let db = length((q - cb) * vec2f(1.0, 1.08)) - rb;
    let ranka = hash21(vec2f(ic, k) * 3.1 + 1.3);
    let rankb = hash21(vec2f(ic + 1.0, k) * 3.1 + 1.3);
    if (found < 0.5) {
      let ina = da < aa;
      let inb = db < aa;
      if (ina && inb) {
        // both reach: the higher rank lies on top, the other is behind it
        if (ranka > rankb) { wc = ca; wr = ra; wid = ia; dwin = da; }
        else               { wc = cb; wr = rb; wid = ib; dwin = db; }
        found = 1.0;
      } else if (ina) {
        wc = ca; wr = ra; wid = ia; dwin = da; found = 1.0;
        if (rankb > ranka) { dfront = min(dfront, db); }
      } else if (inb) {
        wc = cb; wr = rb; wid = ib; dwin = db; found = 1.0;
        if (ranka > rankb) { dfront = min(dfront, da); }
      } else {
        dfront = min(dfront, min(da, db));
      }
    }
  }

  // ---- the winning scale in its own frame
  let d = (q - wc) * vec2f(1.0, 1.08);
  let dl = length(d);
  let nd = d / wr;                            // -1..1 across the scale
  let rr = clamp(dl / wr, 0.0, 1.0);
  let cov = 1.0 - smoothstep(-aa, aa, dwin);   // AA at the free edge

  // per-scale tilt: the free edge lifts a little off the body, and the flex
  // wave passing along the body rocks each scale in turn
  let wave = sin(wc.x * 0.55 + wc.y * 0.25 - t * 1.4);
  let tilt = 0.10 + 0.32 * flex * wave;
  // growth rings: fine concentric circuli across the exposed field
  let ring = sin(rr * rr * 46.0 + wid * 6.28) * (1.0 - rr * 0.4);
  // radial striations, a few per scale
  let radi = sin(atan2(nd.y, nd.x) * 9.0 + wid * 6.28);
  // shallow dome plus the tilt, in the tangent frame of the body
  let slope = nd * 0.38 + normalize(nd + 1e-5) * ring * 0.045
              + normalize(nd + 1e-5) * radi * 0.012 * rr;
  let nl = normalize(vec3f(slope.x, slope.y - tilt, 1.0));

  // body cylinder frame: tangent round the girth and outward normal
  let th = fk.z;
  let T = vec3f(0.0, cos(th), -sin(th));
  let Nb = vec3f(0.0, sin(th),  cos(th));
  let N = normalize(nl.x * vec3f(1.0, 0.0, 0.0) + nl.y * T + nl.z * Nb);

  // ---- light: a key from upper left that drifts across, plus the viewer
  let L = normalize(vec3f(-0.55 + 0.35 * sin(t * 0.37), 0.72, 0.62));
  let V = vec3f(0.0, 0.0, 1.0);
  let H = normalize(L + V);
  let ndl = dot(N, L);
  let diff = clamp(ndl * 0.7 + 0.3, 0.0, 1.0);
  let ndh = max(dot(N, H), 0.0);
  var s4 = ndh * ndh; s4 *= s4;
  let s16 = s4 * s4;
  let s64 = s16 * s16;
  let ndv = max(dot(N, V), 0.0);
  var fres = 1.0 - ndv; fres = fres * fres;

  // ---- material: dark dorsal skin countershading to a paler belly, the
  // palette laid on as iridescence rather than paint
  let body = smoothstep(-1.0, 1.0, -p.y);      // 0 at the back, 1 at the belly
  let deep = mix(vec3f(luma(c3)), c3, 0.55) * 0.20 + c0 * 0.06;
  let skin = mix(vec3f(luma(c0)), c0, 0.55) * 0.42;
  let pale = mix(vec3f(luma(c2)), c2, 0.30) * 0.62;
  var albedo = mix(deep, skin, smoothstep(0.0, 0.55, body));
  albedo = mix(albedo, pale, smoothstep(0.55, 1.0, body));
  // scale to scale variation: a touch lighter or darker, a hint of hue shift
  albedo *= 0.84 + 0.30 * wid;
  let hsel = hash21(wc * 5.3);
  var hueC = mix(c1, c2, smoothstep(0.3, 0.6, hsel));
  hueC = mix(hueC, c0, smoothstep(0.6, 0.9, hsel));
  albedo = mix(albedo, hueC * 0.55, 0.22);

  // iridescence: the sheen colour walks c1 -> c2 -> c0 with the view angle and
  // the position along the body, so neighbouring scales flash different hues
  let ph = ndv * 5.0 + fk.x * 1.6 + wave * 0.6 + t * 0.15;
  var irid = mix(c1, c2, 0.5 + 0.5 * sin(ph));
  irid = mix(irid, c0, 0.5 + 0.5 * sin(ph * 0.61 + 2.0));
  let sheen = (0.10 + 0.35 * fres) * s4 * gloss + (0.05 + 0.16 * fres) * diff * gloss
              + 0.35 * s16 * gloss;

  // free-edge rim: the lifted edge of the scale is a bright thread
  var edge = 1.0 - smoothstep(0.0, 0.16 + aa * 2.0, -dwin);
  edge *= smoothstep(-0.02, 0.0, -dwin);
  let rimLit = 0.35 + 0.65 * clamp(dot(normalize(d + 1e-5), L.xy) * 1.4 + 0.5, 0.0, 1.0);
  let rim = mix(vec3f(1.0), c2, 0.45) * edge * rimLit * (0.60 + 0.5 * ndv) * gloss;

  // contact shadow from the scales in front: dark under their edges, and a
  // deeper pocket right where this scale slides beneath them
  let shad = exp(-max(dfront, 0.0) / 0.075);
  let pocket = exp(-max(dfront, 0.0) / 0.018);
  let ao = 1.0 - 0.60 * shad - 0.25 * pocket;
  // the flank turning away under the belly and along the back
  let turn = 0.55 + 0.45 * Nb.z;

  var col = albedo * (0.22 + 0.82 * diff) * ao * turn;
  col += albedo * ring * 0.05 * diff;
  col += irid * sheen * ao * 1.3;
  col += irid * fres * 0.08 * gloss;
  col += mix(vec3f(1.0), c2, 0.3) * s64 * 0.45 * gloss * ao;
  col += rim * (0.6 + 0.4 * ao);

  // slivers between scales (should be rare): the dark underskin
  col = mix(deep * 0.6, col, cov);

  // fine skin noise so the surface never reads as vector-clean
  col *= 0.96 + 0.08 * vnoise(fc / max(1.0, 0.5) * 0.7);

  return vec4f(col, 1.0);
}
