// Vertex-only depth pipeline for one directional light's shadow map.
struct ShadowGlobals {
    light_view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> globals: ShadowGlobals;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) model0: vec4<f32>,
    @location(4) model1: vec4<f32>,
    @location(5) model2: vec4<f32>,
    @location(6) model3: vec4<f32>,
    @location(7) tint: vec4<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(in.model0, in.model1, in.model2, in.model3);
    return globals.light_view_proj * model * vec4<f32>(in.position, 1.0);
}
