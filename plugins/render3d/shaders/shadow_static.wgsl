// Depth-only colorless fragment pipeline; MASK coverage matches the main color pass.
struct ShadowGlobals {
    light_view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> globals: ShadowGlobals;
@group(0) @binding(1) var<storage, read> joint_matrices: array<mat4x4<f32>>;

struct Material {
    base_color: vec4<f32>,
    surface: vec4<f32>,
    emissive: vec4<f32>,
    render: vec4<f32>,
    texture_uv_sets: vec4<u32>,
    texture_uv_sets_extra: vec4<u32>,
}
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(2) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(9) model0: vec4<f32>,
    @location(10) model1: vec4<f32>,
    @location(11) model2: vec4<f32>,
    @location(12) model3: vec4<f32>,
    @location(13) tint: vec4<f32>,
    @location(14) bone_offset: u32,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv0: vec2<f32>,
    @location(1) uv1: vec2<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(in.model0, in.model1, in.model2, in.model3);
    var out: VertexOut;
    out.clip = globals.light_view_proj * model * vec4<f32>(in.position, 1.0);
    out.uv0 = in.uv0;
    out.uv1 = in.uv1;
    return out;
}

@fragment
fn fs_shadow(in: VertexOut) {
    // MASK materials must discard the same texels in the shadow map as in the color pass.
    if material.render.x > 0.5 && material.render.x < 1.5 {
        let uv = select(in.uv0, in.uv1, material.texture_uv_sets.x == 1u);
        let alpha = material.base_color.a * textureSample(base_texture, base_sampler, uv).a;
        if alpha < material.render.y {
            discard;
        }
    }
}