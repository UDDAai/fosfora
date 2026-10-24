// OrganicLiquid — vertical half of the 9-tap Gaussian. Reads the horizontal blur.

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let pixel = 1.0 / res;
    let uv = frag_coord.xy * pixel;
    let v = pixel.y;

    var sum = vec4f(0.0);
    sum += input0(fract(vec2f(uv.x, uv.y - 4.0 * v))) * 0.05;
    sum += input0(fract(vec2f(uv.x, uv.y - 3.0 * v))) * 0.09;
    sum += input0(fract(vec2f(uv.x, uv.y - 2.0 * v))) * 0.12;
    sum += input0(fract(vec2f(uv.x, uv.y - 1.0 * v))) * 0.15;
    sum += input0(fract(uv)) * 0.16;
    sum += input0(fract(vec2f(uv.x, uv.y + 1.0 * v))) * 0.15;
    sum += input0(fract(vec2f(uv.x, uv.y + 2.0 * v))) * 0.12;
    sum += input0(fract(vec2f(uv.x, uv.y + 3.0 * v))) * 0.09;
    sum += input0(fract(vec2f(uv.x, uv.y + 4.0 * v))) * 0.05;
    return vec4f(sum.xyz / 0.98, 1.0);
}
