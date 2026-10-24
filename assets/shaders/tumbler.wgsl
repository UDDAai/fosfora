// Tumbler — shader.gallery (tumbler)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: peacock

const SG_C0: vec3f = vec3f(0.071000, 0.612000, 0.694000);
const SG_C1: vec3f = vec3f(0.176000, 0.451000, 0.804000);
const SG_C2: vec3f = vec3f(0.561000, 0.357000, 0.812000);
const SG_C3: vec3f = vec3f(0.965000, 0.737000, 0.286000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// tumbler (Kaleido) — the feel of turning the tube: angular glass shards in the
// object cell that each tumble on their own axis while the mirror-fold reflects
// them. As the shards rotate and slide the reflected figure keeps re-locking
// into new symmetric arrangements — the characteristic "click and reshuffle" of
// a kaleidoscope being turned. Hard-edged chips of colour on near-black.


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.032, 0.032, 0.041);

fn hash21(p: vec2f) -> f32 {
 return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453); }
fn wheelW(s: f32, c: f32) -> f32 {
 var d = abs(s - c);
  return max(0.0, 1.0 - min(d, 4.0 - d)); }
fn palBlend(c0: vec3f, c1: vec3f, c2: vec3f, c3: vec3f, h: f32) -> vec3f {

  let s = fract(h) * 4.0;
  let w0 = wheelW(s, 0.0);
  let w1 = wheelW(s, 1.0);
  let w2 = wheelW(s, 2.0);
  let w3 = wheelW(s, 3.0);
  return (c0 * w0 + c1 * w1 + c2 * w2 + c3 * w3) / max(w0 + w1 + w2 + w3, 0.001);
}

// signed distance to a square chip (rotated, half-size hs)
fn sdBox(p: vec2f, hs: f32) -> f32 {
 var d = abs(p) - hs;
  return length(max(d, vec2f(0.0))) + min(max(d.x, d.y), 0.0); }
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let ctr = res * 0.5;
  let mn = min(res.x, res.y);
  let p = (fc - ctr) / mn;
  let t = u.time;

  let r = length(p);
  var a = atan2(p.y, p.x);
  let N = max(floor(param(0u)), 2.0);
  let seg = 6.2831853 / N;
  a += t * param(3u);
  a  = sg_mod(a, seg);
  a  = abs(a - seg * 0.5);
  let q = vec2f(cos(a), sin(a)) * r;

  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  // shard lattice in the folded space; each cell holds a chip tumbling on its own axis
  let cs = mix(5.5, 2.4, clamp((param(2u) - 0.4) / 2.1, 0.0, 1.0)); // cells across
  var g = q * cs;
  g += vec2f(sin(t * 0.1) * 0.3, t * 0.16);   // slow slide through the cell

  let id = floor(g);
  var csum = vec3f(0.0);
  var wsum = 0.0;
  for (var oy: i32 = -1; oy <= 1; oy += 1) {
    for (var ox: i32 = -1; ox <= 1; ox += 1) {
      let cid = id + vec2f(f32(ox), f32(oy));
      let h = hash21(cid);
      // chip centre wobbles; chip tumbles (rotates) at a per-chip rate
      let cc = cid + 0.5 + 0.18 * vec2f(sin(t * 0.4 + h * 6.3), cos(t * 0.33 + h * 4.1));
      let ang = (h - 0.5) * 6.2831853 + t * param(1u) * (0.6 + h) * (select(-1.0, 1.0, h < 0.5));
      let ca = cos(ang);
      let sa = sin(ang);
      let lp = mat2x2<f32>(ca, -sa, sa, ca) * (g - cc);
      let hs = 0.22 + 0.16 * h;
      let d = sdBox(lp, hs);
      let chip = smoothstep(0.06, 0.0, d);        // hard-ish chip body
      let edge = smoothstep(0.05, 0.0, abs(d));   // bright cut edge
      let w = chip;
      csum += palBlend(c0, c1, c2, c3, h * 1.8 + t * 0.02) * (chip + edge * 0.7);
      wsum += w;
    }
  }
  let shards = csum / max(wsum, 0.001);
  let cover = min(wsum, 1.0);

  var col = BG;
  col += shards * cover * 1.3;

  // luminous core, dark rim
  col *= smoothstep(1.2, 0.05, r);
  col += palBlend(c0, c1, c2, c3, 0.3 + t * 0.05) * exp(-r * r * 70.0) * 0.22;

  return vec4f(col, 1.0);
}
