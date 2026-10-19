// Instanced metallic-roughness mesh shader using a Cook-Torrance BRDF.
// The mesh pass renders linear HDR values, samples a directional shadow map, and
// then runs the separate fullscreen tone-mapping pass before presenting the frame.

struct Globals {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    camera_pos: vec4<f32>,
}

struct Material {
    base_color: vec4<f32>,
    // x = metallic, y = perceptual roughness.
    surface: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;
@group(1) @binding(3) var metallic_roughness_texture: texture_2d<f32>;

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

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec3<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) shadow_position: vec4<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(in.model0, in.model1, in.model2, in.model3);
    let world_position = (model * vec4<f32>(in.position, 1.0)).xyz;
    let c0 = model[0].xyz;
    let c1 = model[1].xyz;
    let c2 = model[2].xyz;
    // Cofactor matrix is proportional to inverse-transpose; normalization removes scale.
    let transformed_normal = cross(c1, c2) * in.normal.x
        + cross(c2, c0) * in.normal.y
        + cross(c0, c1) * in.normal.z;
    var out: VertexOut;
    out.clip = globals.view_proj * vec4<f32>(world_position, 1.0);
    out.normal = normalize(transformed_normal);
    out.uv = in.uv;
    out.tint = in.tint.rgb;
    out.world_position = world_position;
    out.shadow_position = globals.light_view_proj * vec4<f32>(world_position, 1.0);
    return out;
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - clamp(cos_theta, 0.0, 1.0), 5.0);
}

fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let alpha = roughness * roughness;
    let alpha2 = alpha * alpha;
    let denominator = n_dot_h * n_dot_h * (alpha2 - 1.0) + 1.0;
    return alpha2 / max(3.14159265 * denominator * denominator, 0.000001);
}

fn geometry_schlick_ggx(n_dot_x: f32, roughness: f32) -> f32 {
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    return n_dot_x / max(n_dot_x * (1.0 - k) + k, 0.000001);
}

fn shadow_visibility(shadow_position: vec4<f32>, n_dot_l: f32) -> f32 {
    if shadow_position.w <= 0.0 {
        return 1.0;
    }
    let ndc = shadow_position.xyz / shadow_position.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0 {
        return 1.0;
    }
    let bias = max(0.00025, 0.0012 * (1.0 - n_dot_l));
    return textureSampleCompare(shadow_map, shadow_sampler, uv, ndc.z - bias);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let v = normalize(globals.camera_pos.xyz - in.world_position);
    let l = normalize(-globals.sun_dir.xyz);
    let h = normalize(v + l);
    let albedo = max(
        in.tint * material.base_color.rgb * textureSample(base_texture, base_sampler, in.uv).rgb,
        vec3<f32>(0.0),
    );
    let surface_sample = textureSample(metallic_roughness_texture, base_sampler, in.uv);
    let metallic = clamp(material.surface.x * surface_sample.b, 0.0, 1.0);
    let roughness = clamp(material.surface.y * surface_sample.g, 0.04, 1.0);
    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_v = max(dot(n, v), 0.001);
    let n_dot_h = max(dot(n, h), 0.0);
    let h_dot_v = max(dot(h, v), 0.0);

    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let fresnel = fresnel_schlick(h_dot_v, f0);
    let distribution = distribution_ggx(n_dot_h, roughness);
    let geometry = geometry_schlick_ggx(n_dot_v, roughness)
        * geometry_schlick_ggx(n_dot_l, roughness);
    let specular = (distribution * geometry * fresnel)
        / max(4.0 * n_dot_v * max(n_dot_l, 0.001), 0.001);
    let diffuse_weight = (vec3<f32>(1.0) - fresnel) * (1.0 - metallic);
    let diffuse = diffuse_weight * albedo / 3.14159265;
    let visibility = shadow_visibility(in.shadow_position, n_dot_l);
    let direct = (diffuse + specular) * globals.sun_color.rgb * n_dot_l * visibility;
    let ambient = globals.ambient.rgb * albedo * (1.0 - metallic * 0.7);
    return vec4<f32>(ambient + direct, material.base_color.a);
}
