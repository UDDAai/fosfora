// Chamber — shader.gallery (chamber)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: prism

const SG_C0: vec3f = vec3f(0.376000, 0.647000, 0.949000);
const SG_C1: vec3f = vec3f(0.345000, 0.847000, 0.612000);
const SG_C2: vec3f = vec3f(0.965000, 0.792000, 0.318000);
const SG_C3: vec3f = vec3f(0.949000, 0.408000, 0.471000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// chamber (Kaleido) — the three-mirror tube. The triangular mirror chamber of a
// real kaleidoscope tiles the whole plane with reflected copies of one triangle,
// so this fills the frame edge-to-edge with a repeating jewelled wallpaper
// rather than a single radial star. Motes of coloured glass drift through the
// fundamental triangle; the lattice reflects them into an endless honeycomb that
// slides and turns. Distinct from the radial pieces — this is the wallpaper view.


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.034, 0.034, 0.043);
const SQ3: f32 = 1.7320508;

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
  var p = (fc - ctr) / mn;
  let t = u.time;

  // turn the whole lattice slowly
  let ca = cos(t * param(2u));
  let sa = sin(t * param(2u));
  p = mat2x2<f32>(ca, -sa, sa, ca) * p;

  // RADIAL mirror-fold (the Teleido split): reflect the plane into 12 wedges
  // around the centre, so the triangular tiling radiates from the middle with
  // mirror seams and fills the frame to the corners — a hexagonal kaleidoscope
  // rather than a flat parallel wallpaper.
  let rr = length(p);
  var aa = atan2(p.y, p.x);
  let segf = 6.2831853 / 12.0;
  aa = abs(sg_mod(aa, segf) - segf * 0.5);
  p  = vec2f(cos(aa), sin(aa)) * rr;

  let scale = max(param(0u), 0.5);
  let g = p * scale;

  // --- fold the plane into one equilateral-triangle cell (reflective tiling) ---
  // express g in the rhombic lattice basis e1=(1,0), e2=(0.5, SQ3/2)
  var u = g.x - g.y / SQ3;
  var v = g.y * 2.0 / SQ3;
  u = fract(u);
  v = fract(v);
  // the rhombus splits into two mirrored triangles; fold the far one back so the
  // content is identical (mirrored) in every triangle — a true reflective tiling
  if (u + v > 1.0) { u = 1.0 - u; v = 1.0 - v; }

  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  var col = BG;

  // barycentric weights of the fundamental triangle: w0 at the origin vertex,
  // wu at (1,0), wv at (0,1). Because the content depends ONLY on these (same in
  // every triangle), the tiling is a seamless mirror honeycomb.
  let w0 = 1.0 - u - v;
  let wu = u;
  let wv = v;
  let br = 0.5 + 0.5 * sin(t * 0.5 * param(1u));

  // each triangle is a little stained-glass pane: a smooth three-colour blend
  // across its vertices, so every pane carries all the hues and edges match
  let glass = c0 * w0 + c1 * wu + c2 * wv;

  // distance to the nearest triangle edge (0 on an edge, 1/3 at the centroid)
  let edge = min(min(w0, wu), wv);
  let interior = smoothstep(0.0, 0.30, edge);     // dark at the leading, lit inside
  col += glass * (0.16 + 0.7 * interior) * (0.7 + 0.45 * br) * param(3u);

  // GLOWING MIRROR NET: bright lines along every triangle edge make the
  // kaleidoscope chamber legible as a triangular lattice, not a dot grid
  let net = smoothstep(0.05, 0.0, edge);
  col += palBlend(c0, c1, c2, c3, 0.6 + t * 0.05) * net * 0.5 * param(3u);

  // a bright jewel at each triangle centroid, breathing
  let cd = length(vec2f(u - 1.0 / 3.0, v - 1.0 / 3.0));
  let pip = smoothstep(0.26, 0.0, cd);
  col += palBlend(c0, c1, c2, c3, 0.15 + t * 0.04) * pip * pip * (0.45 + 0.5 * br) * param(3u);

  // gentle centre-weighting that still keeps the corners lit (length(p) is the
  // true radius — both the rotation and the radial fold preserve it)
  let vign = 1.0 - 0.4 * smoothstep(0.5, 1.4, length(p));
  col *= mix(0.95, 1.12, smoothstep(1.0, 0.0, length(p))) * vign;

  return vec4f(col, 1.0);
}
