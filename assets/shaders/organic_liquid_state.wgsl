// OrganicLiquid — state step of Flexi's expansive reaction-diffusion.
// https://www.shadertoy.com/view/4dcGW2
//
// feedback() = previous chemical field (Buffer A).
// input0     = previous full blur (Buffer C). The blur passes run after this
//              one, so the reaction sees last frame's diffusion, as on Shadertoy.
// param(0)   = flow. 0.5 matches the original per-frame step.

fn ol_noise(p: vec2f) -> vec2f {
    return vec2f(
        fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453),
        fract(sin(dot(p, vec2f(269.5, 183.3))) * 43758.5453),
    );
}

fn ol_finite(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) != 0x7f800000u;
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let pixel = 1.0 / res;
    let uv = frag_coord.xy * pixel;
    let noise = ol_noise(frag_coord.xy + vec2f(42.0, 56.0) * u.time);

    // Every feedback() call stays in uniform control flow. textureSample cannot
    // sit under a per-pixel branch.
    let here = feedback(uv);
    let corner_a = feedback(vec2f(0.17, 0.31));
    let corner_b = feedback(vec2f(0.73, 0.59));

    let step_px = pixel * 4.0;
    let dx = (input0(fract(uv + vec2f(step_px.x, 0.0))) - input0(fract(uv - vec2f(step_px.x, 0.0)))) * 0.5;
    let dy = (input0(fract(uv + vec2f(0.0, step_px.y))) - input0(fract(uv - vec2f(0.0, step_px.y)))) * 0.5;
    let uv_red = uv + vec2f(dx.x, dy.x) * pixel * 8.0;
    let jitter = (noise - 0.5) * pixel;
    let prev = feedback(fract(uv_red)).x;
    let prev_j = feedback(fract(uv_red + jitter)).x;
    let blur_j = input0(fract(uv_red + jitter)).x;

    // 0.5 on the slider is the original coefficient. 0 holds the field still.
    let flow = max(param(0u), 0.0) * 2.0;
    var new_red = prev;
    new_red += ((noise.x - 0.5) * 0.0025 - 0.002 + u.bass * 0.003) * flow;
    new_red -= (blur_j - prev_j) * 0.047 * flow;
    if (!ol_finite(new_red)) {
        new_red = 0.0;
    }
    new_red = clamp(new_red, 0.0, 1.0);

    // A cleared buffer is black. One frame of noise is the original 10-frame seed.
    let cold = here.x + corner_a.x + corner_b.x;
    let red = select(new_red, noise.x, cold < 1e-4);
    return vec4f(red, 0.0, 0.0, 1.0);
}
