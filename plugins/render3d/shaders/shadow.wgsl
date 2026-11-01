// Vertex-only depth pipeline for one directional light's shadow map.
struct ShadowGlobals {
    light_view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> globals: ShadowGlobals;
@group(0) @binding(1) var<storage, read> joint_matrices: array<mat4x4<f32>>;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints0: vec4<u32>,
    @location(4) weights0: vec4<f32>,
    @location(5) joints1: vec4<u32>,
    @location(6) weights1: vec4<f32>,
    @location(7) model0: vec4<f32>,
    @location(8) model1: vec4<f32>,
    @location(9) model2: vec4<f32>,
    @location(10) model3: vec4<f32>,
    @location(11) tint: vec4<f32>,
    @location(12) bone_offset: u32,
}

@vertex
fn vs_main(in: VertexIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(in.model0, in.model1, in.model2, in.model3);
    var local_position = in.position;
    let weight_sum = dot(in.weights0, vec4<f32>(1.0))
        + dot(in.weights1, vec4<f32>(1.0));
    if weight_sum > 0.000001 {
        var skin = mat4x4<f32>(
            vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0)
        );
        if in.weights0.x > 0.0 { skin += joint_matrices[in.bone_offset + in.joints0.x] * in.weights0.x; }
        if in.weights0.y > 0.0 { skin += joint_matrices[in.bone_offset + in.joints0.y] * in.weights0.y; }
        if in.weights0.z > 0.0 { skin += joint_matrices[in.bone_offset + in.joints0.z] * in.weights0.z; }
        if in.weights0.w > 0.0 { skin += joint_matrices[in.bone_offset + in.joints0.w] * in.weights0.w; }
        if in.weights1.x > 0.0 { skin += joint_matrices[in.bone_offset + in.joints1.x] * in.weights1.x; }
        if in.weights1.y > 0.0 { skin += joint_matrices[in.bone_offset + in.joints1.y] * in.weights1.y; }
        if in.weights1.z > 0.0 { skin += joint_matrices[in.bone_offset + in.joints1.z] * in.weights1.z; }
        if in.weights1.w > 0.0 { skin += joint_matrices[in.bone_offset + in.joints1.w] * in.weights1.w; }
        local_position = (skin * vec4<f32>(in.position, 1.0)).xyz;
    }
    return globals.light_view_proj * model * vec4<f32>(local_position, 1.0);
}
