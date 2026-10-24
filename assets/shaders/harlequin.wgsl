// Harlequin — shader.gallery (harlequin)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: amethyst

const SG_C0: vec3f = vec3f(0.494000, 0.318000, 0.831000);
const SG_C1: vec3f = vec3f(0.659000, 0.435000, 0.949000);
const SG_C2: vec3f = vec3f(0.380000, 0.580000, 0.949000);
const SG_C3: vec3f = vec3f(0.290000, 0.180000, 0.561000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// harlequin (Kaleido) — a cut-crystal mandala of hard, flat facets. Where the
// other Kaleido pieces blur, this one keeps knife-sharp mirror seams: the folded
// field is quantised into angular/radial facets, each flat-shaded a single jewel
// hue with a bright bevel at its edges, like a faceted gemstone catching light.
// Inner and outer rings counter-rotate, so the diamond pattern shears and
// reshuffles as the stone turns.


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.030, 0.030, 0.039);

fn hash21(p: vec2f) -> f32 {
 return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453); }
fn wheelW(s: f32, c: f32) -> f32 {
 let d = abs(s - c);
  return max(0.0, 1.0 - min(d, 4.0 - d)); }
fn palBlend(c0: vec3f, c1: vec3f, c2: vec3f, c3: vec3f, h: f32) -> vec3f {

  let s = fract(h) * 4.0;
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
  let mn = min(res.x, res.y);
  let p = (fc - ctr) / mn;
  let t = u.time;

  let r = length(p) * 2.0;
  var a = atan2(p.y, p.x);

  // RIGID counter-rotation by radial zone: each concentric band turns as one
  // piece (alternating sense) so the diamonds stay crisp instead of shearing into
  // spirals — the kaleidoscope-turn click, kept hard-edged.
  let RINGS = max(floor(param(1u)), 2.0);
  let zone = floor(r * RINGS * 0.5);
  let zdir =select(-1.0, 1.0, sg_mod(zone, 2.0) < 0.5);
  a += t * param(2u) * zdir;

  let N = max(floor(param(0u)), 3.0);
  let seg = 6.2831853 / N;
  // continuous pane coordinate around the full circle (each unit = one diamond),
  // mirror-folded so opposite sides reflect
  let paneU = a / seg;
  let fold = abs(fract(paneU) - 0.5) * 2.0;   // 0..1, mirror within each pane

  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  // ARGYLE diamonds: a polar lattice of ring x pane cells, alternate rings offset
  // half a pane so the cells lock into a diamond harlequin. Each diamond a bold
  // jewel hue from the full palette wheel; the interstices are dark, and thin
  // contrast threads cross the lattice like argyle overstitch.
  let ringF = r * RINGS;
  let ri = floor(ringF);
  let rf = fract(ringF);
  let off = 0.5 * sg_mod(ri, 2.0);
  let pf = fract(paneU + off);            // 0..1 within the pane (staggered)
  let pi = floor(paneU + off);

  // diamond cell: L1 distance from the cell centre — diamond edge at dd = 0.5
  let dd = abs(rf - 0.5) + abs(pf - 0.5);
  let bev = clamp(param(3u), 0.1, 3.0);
  let edgeW = 0.16 / bev;
  let plate = smoothstep(0.5, 0.5 - edgeW, dd);   // flat diamond, crisp edge

  let h = hash21(vec2f(pi, ri) * 1.31 + 4.0);
  let facet = palBlend(c0, c1, c2, c3, h + t * 0.02);   // full-wheel jewel per diamond
  let shimmer = 0.7 + 0.3 * sin(ri * 1.1 - t * 0.7 + h * 6.28);

  var col = BG;
  col += facet * plate * (0.7 + 0.7 * shimmer) * 1.25;

  // bright thread riding the diamond edge (the harlequin overstitch glint)
  let thread = smoothstep(0.5 + edgeW, 0.5, dd) * smoothstep(0.5 - edgeW * 2.0, 0.5, dd);
  col += palBlend(c0, c1, c2, c3, h + 0.5) * thread * 0.6;

  // a small contrast pip at each diamond centre, the jester's dot
  let pip = smoothstep(0.13, 0.0, dd);
  col += palBlend(c0, c1, c2, c3, h + 2.0) * pip * 0.5;

  // a hard central crystal table
  col += palBlend(c0, c1, c2, c3, 0.2 + t * 0.04) * smoothstep(0.12, 0.0, r) * 0.6;

  // keep the argyle running into the corners (corner radius ~1.9); only soften
  // the very edge so the diamonds fill the frame instead of a black disc
  col *= 1.0 - 0.4 * smoothstep(1.55, 2.1, r);

  return vec4f(col, 1.0);
}
