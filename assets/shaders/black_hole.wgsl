// Black Hole
// XorDev (x.com/XorDev), MilkDrop port by MilkDrop2077.
// Ported from XorPlayer's "XorDev - MilkDrop2077 - Black hole.milk"
// (BSD-3-Clause, copyright 2025 milkdrop2077).
//
// param(0) = bass. 0.5 is MilkDrop's resting bass_att of 1.
// param(1) = spin. 0.5 is the original rate.
// param(2) = integral of spin, written by the engine (`"rates"` in the .pfx).

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = u.resolution;
    let uv = frag_coord.xy / res;
    let aspect = res.x / max(res.y, 1.0);
    let p = (uv - 0.5) * 1.8 * vec2f(aspect, 1.0);

    // 0.5 on the slider is the original resting level (bass_att = 1).
    let bass_att = mix(0.35, 1.65, param(0u));
    // Integral of the spin slider. At the default 0.5 this matches `time * 0.5`,
    // and the inner oscillator below is twice that, matching the original `+ time`.
    let travel = param(2u);
    let inner_t = travel * 2.0;

    let d = vec2f(-1.0, 1.0);
    let q = 5.0 * p - d;
    let s = 0.5 + 5.0 / max(dot(q, q), 1e-4);
    let fold = d / s;
    // HLSL mul(p, float2x2(1, 1, fold)) — row vector on the left.
    let c = vec2f(p.x + p.y * fold.x, p.x + p.y * fold.y);

    var v = c;
    let cs = cos(log(max(length(v), 1e-4)) + travel + vec4f(0.0, 33.0, 11.0, 0.0));
    // HLSL mul(v, float2x2(cs)) * 5.
    v = vec2f(v.x * cs.x + v.y * cs.z, v.x * cs.y + v.y * cs.w) * 5.0;

    // The milk file starts this loop at 0 and divides by i. The published fix
    // runs i = 1..9 so the first sample is finite.
    var o = vec4f(0.0);
    for (var i = 1; i < 10; i++) {
        let fi = f32(i);
        o += sin(vec4f(v.x, v.y, v.y, v.x)) + 1.0;
        v += 0.9 * sin(vec2f(v.y, v.x) * fi + inner_t) / fi + 0.5;
    }

    let hue = exp(c.x * vec4f(0.6, -0.4 + bass_att, 1.4, 0.0));
    let ring = length(sin(v / 0.3) * 0.2 + c * vec2f(1.0, 2.0)) - 1.0;
    let denom_a = 0.1 + 0.1 * ring * ring;
    let denom_b = 1.0 + bass_att * 5.0 * exp(0.3 * c.y - dot(c, c));
    let denom_c = 0.03 + abs(length(p) - 0.7);
    let shaped = hue / max(o, vec4f(1e-4)) / denom_a / denom_b / denom_c * 0.2;
    let rgb = (1.0 - exp(-shaped)).xyz;
    return vec4f(max(rgb, vec3f(0.0)), 1.0);
}
