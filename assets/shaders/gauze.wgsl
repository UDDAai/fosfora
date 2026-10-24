// Gauze — shader.gallery (gauze)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: fuchsia

const SG_C0: vec3f = vec3f(0.890000, 0.220000, 0.620000);
const SG_C1: vec3f = vec3f(0.976000, 0.392000, 0.741000);
const SG_C2: vec3f = vec3f(0.659000, 0.286000, 0.890000);
const SG_C3: vec3f = vec3f(0.518000, 0.110000, 0.404000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// gauze (Strand) - a node network seen through a shallow depth of field. Glow
// nodes are scattered through depth and linked by hairline threads; only the
// nodes near the focal plane resolve to sharp points, while those nearer or
// farther swell into soft out-of-focus bokeh discs - big and faint, the energy
// spread thin. The focal plane racks slowly back and forth so the web breathes
// in and out of focus, different strands sharpening as others bloom. Open
// threads only; the field between them is left to the soft glow.
// (ASCII-only comments: the headless poster compiler is fussy about apostrophes.)


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.035, 0.035, 0.043);
const NODE_CSS: f32 = 2.1;
const LINE_CSS: f32 = 1.4;

fn hash2(p_in: vec2f) -> vec2f {
  var p = p_in;

  p = vec2f(dot(p, vec2f(127.1, 311.7)), dot(p, vec2f(269.5, 183.3)));
  return fract(sin(p) * 43758.5453);
}
fn wheelW(s: f32, c: f32) -> f32 {
 let d = abs(s - c);
  return max(0.0, 1.0 - min(d, 4.0 - d)); }
fn wheelCol(k: f32, c0: vec3f, c1: vec3f, c2: vec3f, c3: vec3f) -> vec3f {

  let s = fract(k) * 4.0;
  let a = wheelW(s, 0.0);
  let b = wheelW(s, 1.0);
  let cc = wheelW(s, 2.0);
  let dd = wheelW(s, 3.0);
  return (c0 * a + c1 * b + c2 * cc + c3 * dd) / max(a + b + cc + dd, 0.001);
}

// node screen position for a cell: jittered anchor circled on a slow hashed orbit
fn nodePos(cid: vec2f, spacing: f32, t: f32) -> vec2f {

  let h = hash2(cid);
  let h2 = hash2(cid + 19.0);
  let anchor = (cid + 0.25 + 0.5 * h) * spacing;
  let per = mix(14.0, 36.0, h2.x);
  let ang = t * (6.2831853 / per) + h2.y * 6.2831853;
  return anchor + vec2f(cos(ang), sin(ang)) * spacing * 0.18;
}
fn nodeDepth(cid: vec2f) -> f32 {
 return hash2(cid + 7.0).x; }   // 0..1 depth of the node
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let pr = 1.0;
  let t = u.time;

  let refScale = min(res.x, res.y) / (max(pr, 1.0) * 400.0);
  let spacing = max(param(0u), 18.0) * refScale * pr;
  let nodeR = NODE_CSS * pr * max(param(5u), 0.1);
  let lw = LINE_CSS * pr;
  let blurMax = spacing * 0.55 * max(param(2u), 0.0);

  // focal plane racks slowly back and forth through depth (phase-continuous)
  let focus = clamp(param(1u) + 0.42 * sin(t * param(3u) * 0.5), 0.02, 0.98);

  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  let baseId = floor(fc / spacing);
  var acc = vec3f(0.0);

  // --- bokeh discs over a 5x5 neighbourhood (discs can swell well past a cell).
  for (var j: i32 = 0; j < 5; j += 1) {
    for (var i: i32 = 0; i < 5; i += 1) {
      let cid = baseId + vec2f(f32(i) - 2.0, f32(j) - 2.0);
      let np = nodePos(cid, spacing, t);
      let z = nodeDepth(cid);
      var coc = nodeR + abs(z - focus) * blurMax;        // circle of confusion
      let dist = distance(fc, np);
      // light spread as the disc swells: sharp node = bright point, blurred = softer
      // and dimmer disc, but kept visible (gentle linear falloff, not 1/area).
      let den = 1.0 + coc / nodeR;
      let disc = (1.0 - smoothstep(coc * 0.5, coc, dist)) / den;
      let rq = (dist - coc * 0.82) / max(coc * 0.18, 1.0);   // (x*x, not pow: neg base -> NaN in ANGLE)
      let rim = exp(-rq * rq) * 0.5 / den;                    // bokeh rim
      let hue = (cid.x * 0.11 + cid.y * 0.07) * param(6u) + z * 0.3 + t * 0.01;
      acc += wheelCol(hue, c0, c1, c2, c3) * (disc + rim) * 3.2;
    }
  }

  // --- threads: proximity links over the inner 3x3, blurred by the pair depth.
  let thresh = spacing * 1.1 * max(param(4u), 0.1);
  for (var a: i32 = 0; a < 9; a += 1) {
    for (var b: i32 = 0; b < 9; b += 1) {
      if (b <= a) { continue; }
      let ca = baseId + vec2f(f32(a/3) - 1.0, f32(i32(sg_mod(f32(a),3.0))) - 1.0);
      let cb = baseId + vec2f(f32(b/3) - 1.0, f32(i32(sg_mod(f32(b),3.0))) - 1.0);
      let pa = nodePos(ca, spacing, t);
      let pb = nodePos(cb, spacing, t);
      let d = distance(pa, pb);
      if (d > thresh) { continue; }
      let za = nodeDepth(ca);
      let zb = nodeDepth(cb);
      var coc = 0.5 * (abs(za - focus) + abs(zb - focus)) * blurMax;
      let ba = pb - pa;
      let h = clamp(dot(fc - pa, ba) / max(dot(ba, ba), 1e-4), 0.0, 1.0);
      let sd = length((fc - pa) - ba * h);
      let w = lw + coc * 0.5;
      let close = 1.0 - d / thresh;
      let stroke = (1.0 - smoothstep(0.0, w, sd)) / max(1.0 + coc / (nodeR * 2.0), 1.0);
      var halo = (w / (sd + w * 1.4)); halo = halo * halo * 0.35;
      let cc = wheelCol((ca.x * 0.11 + ca.y * 0.07) * param(6u) + t * 0.01, c0, c1, c2, c3);
      acc += cc * (stroke + halo) * close * 1.35;
    }
  }

  acc = vec3f(1.0) - exp(-acc * 1.1);
  return vec4f(BG + acc, 1.0);
}
