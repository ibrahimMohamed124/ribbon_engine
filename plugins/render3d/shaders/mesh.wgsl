// Native-surface PBR shader: directional light, one shadowed spotlight, and up to
// eight unshadowed point lights. Shadows use 3x3 PCF with normal-dependent bias.
struct Globals {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    spot_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    camera_pos: vec4<f32>,
    spot_position_range: vec4<f32>,
    spot_direction_outer: vec4<f32>,
    spot_color_intensity: vec4<f32>,
    spot_params: vec4<f32>,
    point_positions_range: array<vec4<f32>, 8>,
    point_colors_intensity: array<vec4<f32>, 8>,
    point_count: vec4<f32>,
    fog_color_density: vec4<f32>,
    fog_params: vec4<f32>,
}

struct Material {
    base_color: vec4<f32>,
    // x = metallic, y = perceptual roughness.
    surface: vec4<f32>,
    emissive: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var directional_shadow_map: texture_depth_2d;
@group(0) @binding(2) var directional_shadow_sampler: sampler_comparison;
@group(0) @binding(3) var spot_shadow_map: texture_depth_2d;
@group(0) @binding(4) var spot_shadow_sampler: sampler_comparison;
@group(0) @binding(7) var<storage, read> joint_matrices: array<mat4x4<f32>>;
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;
@group(1) @binding(3) var metallic_roughness_texture: texture_2d<f32>;
@group(1) @binding(4) var normal_texture: texture_2d<f32>;
@group(1) @binding(5) var occlusion_texture: texture_2d<f32>;
@group(1) @binding(6) var emissive_texture: texture_2d<f32>;

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

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec3<f32>,
    @location(3) world_position: vec3<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(in.model0, in.model1, in.model2, in.model3);
    var local_position = in.position;
    var local_normal = in.normal;
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
        let s0 = skin[0].xyz;
        let s1 = skin[1].xyz;
        let s2 = skin[2].xyz;
        local_normal = cross(s1, s2) * in.normal.x
            + cross(s2, s0) * in.normal.y
            + cross(s0, s1) * in.normal.z;
    }
    let world_position = (model * vec4<f32>(local_position, 1.0)).xyz;
    let c0 = model[0].xyz;
    let c1 = model[1].xyz;
    let c2 = model[2].xyz;
    let transformed_normal = cross(c1, c2) * local_normal.x
        + cross(c2, c0) * local_normal.y
        + cross(c0, c1) * local_normal.z;

    var out: VertexOut;
    out.clip = globals.view_proj * vec4<f32>(world_position, 1.0);
    out.normal = normalize(transformed_normal);
    out.uv = in.uv;
    out.tint = in.tint.rgb;
    out.world_position = world_position;
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

fn brdf_light(
    n: vec3<f32>,
    v: vec3<f32>,
    albedo: vec3<f32>,
    metallic: f32,
    roughness: f32,
    l: vec3<f32>,
) -> vec3<f32> {
    let n_dot_l = max(dot(n, l), 0.0);
    if n_dot_l <= 0.0 {
        return vec3<f32>(0.0);
    }

    let h = normalize(v + l);
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
    return (diffuse + specular) * n_dot_l;
}

fn directional_pcf_shadow(shadow_position: vec4<f32>, n_dot_l: f32) -> f32 {
    if shadow_position.w <= 0.0 {
        return 1.0;
    }

    let ndc = shadow_position.xyz / shadow_position.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0 {
        return 1.0;
    }

    let bias = max(0.00035, 0.0018 * (1.0 - n_dot_l));
    let texel = 1.0 / vec2<f32>(textureDimensions(directional_shadow_map));
    var visibility = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            visibility += textureSampleCompare(
                directional_shadow_map,
                directional_shadow_sampler,
                uv + vec2<f32>(f32(x), f32(y)) * texel,
                ndc.z - bias
            );
        }
    }
    return visibility / 9.0;
}

fn spotlight_pcf_shadow(shadow_position: vec4<f32>, n_dot_l: f32) -> f32 {
    if shadow_position.w <= 0.0 {
        return 1.0;
    }

    let ndc = shadow_position.xyz / shadow_position.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0 {
        return 1.0;
    }

    // Perspective depth has much finer precision close to the light and compresses
    // distant depth values. Use a separate, smaller bias than the directional map or
    // the bias can erase actual spotlight shadows on the floor and far wall.
    let bias = max(0.000025, 0.00018 * (1.0 - n_dot_l));
    let texel = 1.0 / vec2<f32>(textureDimensions(spot_shadow_map));
    var visibility = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            visibility += textureSampleCompare(
                spot_shadow_map,
                spot_shadow_sampler,
                uv + vec2<f32>(f32(x), f32(y)) * texel,
                ndc.z - bias
            );
        }
    }
    return visibility / 9.0;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let base_n = normalize(in.normal);
    let normal_sample = textureSample(normal_texture, base_sampler, in.uv).xyz * 2.0 - vec3<f32>(1.0);
    let dp1 = dpdx(in.world_position);
    let dp2 = dpdy(in.world_position);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let uv_determinant = duv1.x * duv2.y - duv1.y * duv2.x;

    // Degenerate or missing UVs can occur in third-party glTF files. Start from a
    // stable orthonormal frame and replace it only when the UV derivatives are valid.
    let tangent_axis = select(
        vec3<f32>(1.0, 0.0, 0.0),
        vec3<f32>(0.0, 1.0, 0.0),
        abs(base_n.y) < 0.99
    );
    var tangent = normalize(cross(tangent_axis, base_n));
    var bitangent = normalize(cross(base_n, tangent));
    if abs(uv_determinant) > 0.000001 {
        let derived_tangent = (dp1 * duv2.y - dp2 * duv1.y) / uv_determinant;
        let projected_tangent = derived_tangent - base_n * dot(base_n, derived_tangent);
        if dot(projected_tangent, projected_tangent) > 0.000001 {
            tangent = normalize(projected_tangent);
            let derived_bitangent = (-dp1 * duv2.x + dp2 * duv1.x) / uv_determinant;
            let handedness = select(
                -1.0,
                1.0,
                dot(cross(base_n, tangent), derived_bitangent) >= 0.0
            );
            bitangent = normalize(cross(base_n, tangent)) * handedness;
        }
    }

    let n = normalize(mat3x3<f32>(tangent, bitangent, base_n) * normal_sample);
    let v = normalize(globals.camera_pos.xyz - in.world_position);
    let albedo = max(
        in.tint * material.base_color.rgb * textureSample(base_texture, base_sampler, in.uv).rgb,
        vec3<f32>(0.0)
    );
    let surface_sample = textureSample(metallic_roughness_texture, base_sampler, in.uv);
    let metallic = clamp(material.surface.x * surface_sample.b, 0.0, 1.0);
    let roughness = clamp(material.surface.y * surface_sample.g, 0.04, 1.0);

    let sun_l = normalize(-globals.sun_dir.xyz);
    let sun_n_dot_l = max(dot(n, sun_l), 0.0);
    // Re-project the perspective-correct interpolated world position in the fragment
    // shader. Interpolating light clip-space coordinates as ordinary varyings makes
    // them perspective-correct against the camera projection, which is not the light
    // projection and can push every comparison outside the real shadow map.
    let directional_shadow_position = globals.light_view_proj * vec4<f32>(in.world_position, 1.0);
    let sun_visibility = directional_pcf_shadow(directional_shadow_position, sun_n_dot_l);
    var radiance = brdf_light(n, v, albedo, metallic, roughness, sun_l)
        * globals.sun_color.rgb * sun_visibility;

    if globals.spot_params.z > 0.5 {
        let to_surface = in.world_position - globals.spot_position_range.xyz;
        let distance = length(to_surface);
        let spot_ray = normalize(to_surface);
        let cone_cos = dot(spot_ray, normalize(globals.spot_direction_outer.xyz));
        let inner = max(globals.spot_params.x, globals.spot_direction_outer.w + 0.0001);
        let cone = smoothstep(globals.spot_direction_outer.w, inner, cone_cos);
        let range = max(globals.spot_position_range.w, 0.001);
        let range_falloff = pow(clamp(1.0 - distance / range, 0.0, 1.0), 2.0);
        let spot_l = -spot_ray;
        var spot_visibility = 1.0;

        if globals.spot_params.y > 0.5 {
            // Re-project world position using the spotlight matrix in the fragment
            // shader so shadow coordinates are derived from the perspective-correct
            // interpolated world position, not interpolated light clip coordinates.
            let shadow_clip = globals.spot_view_proj * vec4<f32>(in.world_position, 1.0);
            spot_visibility = spotlight_pcf_shadow(shadow_clip, max(dot(n, spot_l), 0.0));
        }

        radiance += brdf_light(n, v, albedo, metallic, roughness, spot_l)
            * globals.spot_color_intensity.rgb * globals.spot_color_intensity.w
            * cone * range_falloff * spot_visibility / max(distance * distance, 1.0);
    }

    let count = u32(clamp(globals.point_count.x, 0.0, 8.0));
    for (var i = 0u; i < count; i = i + 1u) {
        let light_position_range = globals.point_positions_range[i];
        let delta = light_position_range.xyz - in.world_position;
        let distance = length(delta);
        let light_l = normalize(delta);
        let falloff = pow(
            clamp(1.0 - distance / max(light_position_range.w, 0.001), 0.0, 1.0),
            2.0
        );
        radiance += brdf_light(n, v, albedo, metallic, roughness, light_l)
            * globals.point_colors_intensity[i].rgb * globals.point_colors_intensity[i].w
            * falloff / max(distance * distance, 0.5);
    }

    let ao = textureSample(occlusion_texture, base_sampler, in.uv).r;
    let ambient = globals.ambient.rgb * albedo * (1.0 - metallic * 0.7)
        * mix(1.0, ao, clamp(material.surface.z, 0.0, 1.0));
    let emissive = material.emissive.rgb * textureSample(emissive_texture, base_sampler, in.uv).rgb;
    var color = ambient + radiance + emissive;
    let distance_to_camera = distance(globals.camera_pos.xyz, in.world_position);
    let height_factor = exp(-max(in.world_position.y - globals.fog_params.x, 0.0) * globals.fog_params.y);
    let fog_amount = clamp(
        1.0 - exp(-globals.fog_color_density.w * distance_to_camera * height_factor),
        0.0,
        1.0
    );
    color = mix(color, globals.fog_color_density.rgb, fog_amount);
    return vec4<f32>(color, material.base_color.a);
}
