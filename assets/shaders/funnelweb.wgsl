// Funnelweb — shader.gallery (funnelweb)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: peacock

const SG_C0: vec3f = vec3f(0.071000, 0.612000, 0.694000);
const SG_C1: vec3f = vec3f(0.176000, 0.451000, 0.804000);
const SG_C2: vec3f = vec3f(0.561000, 0.357000, 0.812000);
const SG_C3: vec3f = vec3f(0.965000, 0.737000, 0.286000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// funnelweb (Strand) - a funnel-weaver web wound into a vortex. Two counter-
// winding families of logarithmic-spiral threads cross into a swirling web that
// coils down toward an off-centre throat, faint radial spokes tying the arms
// together. The threads crowd as they near the throat and the whole figure
// drifts slowly inward, so the web seems to spin away down the funnel while the
// depths sink to black. Open strands; never a filled cell.
// (ASCII-only comments: the headless poster compiler is fussy about apostrophes.)


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.035, 0.035, 0.043);

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
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let pr = 1.0;
  let t = u.time;

  let refScale = min(res.x, res.y) / (max(pr, 1.0) * 400.0);
  let lw = max(param(4u), 0.3) * refScale * pr;
  let nS = max(floor(param(0u) + 0.5), 4.0);
  let da = 6.2831853 / nS;
  let pitch = max(param(1u), 0.2);

  // vanishing point off the centre (rule of thirds) so the tunnel leans
  let vp = res * vec2f(0.62, 0.43);
  let d = fc - vp;
  let r = max(length(d), 2.0);
  let ang = atan2(d.y, d.x);
  let lr = log(r);                                   // depth coordinate (perspective)

  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  var acc = vec3f(0.0);
  let TWO = 6.2831853;
  let wind = param(2u);                                // turns the threads wind in

  // depth fog: the throat (small r) sinks toward black, the mouth stays lit
  let mouth = clamp(r / (min(res.x, res.y) * 0.55), 0.0, 1.0);
  let fog = mix(1.0 - clamp(param(5u), 0.0, 1.0), 1.0, mouth);

  // --- two counter-winding logarithmic-spiral thread families. Each is a set of
  // lines of constant (depth -/+ wind*angle); where the two cross they weave a
  // swirling vortex web that spins down into the off-centre throat instead of the
  // flat concentric rings. The whole figure drifts inward over time.
  let drift = t * param(3u) * 0.5;
  let fA = lr / pitch - wind * ang / TWO - drift;    // clockwise arms
  let fB = lr / pitch + wind * ang / TWO - drift;    // anticlockwise arms
  let dA = abs(fract(fA + 0.5) - 0.5) * r * pitch;   // local px to nearest arm
  let dB = abs(fract(fB + 0.5) - 0.5) * r * pitch;
  let armA = smoothstep(lw * 1.4, 0.0, dA);
  let armB = smoothstep(lw * 1.4, 0.0, dB);
  let iA = floor(fA + 0.5);
  let iB = floor(fB + 0.5);
  let colA = wheelCol(iA * 0.13 * param(6u) + lr * 0.05 + t * 0.006, c0, c1, c2, c3);
  let colB = wheelCol(iB * 0.13 * param(6u) + 0.5 + lr * 0.05 + t * 0.008, c0, c1, c2, c3);
  acc += colA * armA * 1.1 * fog;
  acc += colB * armB * 1.1 * fog;

  // --- faint radial spokes (also swept by the swirl) tie the spiral arms together
  // so it reads as one web rather than two loose spirals.
  let aS = ang + wind * lr * 0.5;
  let phi = aS - floor(aS / da + 0.5) * da;
  let spoke = smoothstep(lw * 1.3, 0.0, abs(r * sin(phi)));
  acc += wheelCol(floor(aS / da + 0.5) * 0.07 * param(6u) + 0.2, c0, c1, c2, c3) * spoke * 0.5 * fog;

  // bright glow knot at the throat where the vortex winds shut (off-centre)
  acc += colA * exp(-pow(r / (min(res.x,res.y) * 0.07), 2.0)) * 0.3 * (1.0 - clamp(param(5u),0.0,1.0));

  acc = vec3f(1.0) - exp(-acc * 1.7);
  return vec4f(BG + acc, 1.0);
}
