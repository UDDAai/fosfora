// OrganicLiquid — horizontal half of the 9-tap Gaussian. Reads the new state.

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let pixel = 1.0 / res;
    let uv = frag_coord.xy * pixel;
    let h = pixel.x;

    var sum = vec4f(0.0);
    sum += input0(fract(vec2f(uv.x - 4.0 * h, uv.y))) * 0.05;
    sum += input0(fract(vec2f(uv.x - 3.0 * h, uv.y))) * 0.09;
    sum += input0(fract(vec2f(uv.x - 2.0 * h, uv.y))) * 0.12;
    sum += input0(fract(vec2f(uv.x - 1.0 * h, uv.y))) * 0.15;
    sum += input0(fract(uv)) * 0.16;
    sum += input0(fract(vec2f(uv.x + 1.0 * h, uv.y))) * 0.15;
    sum += input0(fract(vec2f(uv.x + 2.0 * h, uv.y))) * 0.12;
    sum += input0(fract(vec2f(uv.x + 3.0 * h, uv.y))) * 0.09;
    sum += input0(fract(vec2f(uv.x + 4.0 * h, uv.y))) * 0.05;
    return vec4f(sum.xyz / 0.98, 1.0);
}
