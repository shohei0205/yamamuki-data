struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) old_uv: vec2<f32>,
    @location(1) new_uv: vec2<f32>,
    @location(2) amount: f32,
}

@vertex
fn vertex(
    @location(0) position: vec2<f32>,
    @location(1) old_uv: vec2<f32>,
    @location(2) new_uv: vec2<f32>,
    @location(3) amount: f32,
) -> Vertex {
    return Vertex(vec4<f32>(position, 0.0, 1.0), old_uv, new_uv, amount);
}

@group(0) @binding(0) var old_tile: texture_2d<f32>;
@group(0) @binding(1) var new_tile: texture_2d<f32>;
@group(1) @binding(0) var linear_sampler: sampler;

fn gamma(color: vec4<f32>) -> vec4<f32> {
    let rgb = select(
        1.055 * pow(color.rgb, vec3<f32>(1.0 / 2.4)) - 0.055,
        color.rgb * 12.92,
        color.rgb < vec3<f32>(0.0031308),
    );
    return vec4<f32>(rgb, color.a);
}

@fragment
fn gamma_color(in: Vertex) -> @location(0) vec4<f32> {
    let old = gamma(textureSample(old_tile, linear_sampler, in.old_uv));
    let next = gamma(textureSample(new_tile, linear_sampler, in.new_uv));
    return mix(old, next, in.amount);
}

@fragment
fn linear_color(in: Vertex) -> @location(0) vec4<f32> {
    let old = textureSample(old_tile, linear_sampler, in.old_uv);
    let next = textureSample(new_tile, linear_sampler, in.new_uv);
    return mix(old, next, in.amount);
}

fn shade(in: Vertex) -> f32 {
    let old = textureSample(old_tile, linear_sampler, in.old_uv).rg;
    let next = textureSample(new_tile, linear_sampler, in.new_uv).rg;
    return mix(old.g - old.r, next.g - next.r, in.amount);
}

@fragment
fn darken(in: Vertex) -> @location(0) vec4<f32> {
    return vec4<f32>(vec3<f32>(max(-shade(in), 0.0)), 0.0);
}

@fragment
fn brighten(in: Vertex) -> @location(0) vec4<f32> {
    return vec4<f32>(vec3<f32>(max(shade(in), 0.0)), 0.0);
}
