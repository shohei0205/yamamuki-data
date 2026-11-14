struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> Vertex {
    return Vertex(vec4<f32>(position, 0.0, 1.0), uv);
}

@group(0) @binding(0) var amounts: texture_2d<f32>;
@group(1) @binding(0) var linear_sampler: sampler;

@fragment
fn darken(in: Vertex) -> @location(0) vec4<f32> {
    let value = textureSample(amounts, linear_sampler, in.uv);
    return vec4<f32>(vec3<f32>(max(value.r - value.g, 0.0)), 0.0);
}

@fragment
fn brighten(in: Vertex) -> @location(0) vec4<f32> {
    let value = textureSample(amounts, linear_sampler, in.uv);
    return vec4<f32>(vec3<f32>(max(value.g - value.r, 0.0)), 0.0);
}
