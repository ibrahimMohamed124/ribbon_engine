// HDR tone mapping plus configurable color grading, FXAA, vignette, film grain and chromatic
// aberration. Surface targets using an sRGB format receive linear output; Rgba8Unorm offscreen
// targets request the explicit output transfer through params.flags.y.
@group(0) @binding(0) var hdr_scene: texture_2d<f32>;
@group(0) @binding(1) var hdr_sampler: sampler;
@group(0) @binding(2) var bloom_texture: texture_2d<f32>;
@group(0) @binding(3) var<storage, read> exposure_state: f32;

struct ToneMapParams {
    exposure_and_color: vec4<f32>, // manual exposure, contrast, saturation, FXAA enabled
    look: vec4<f32>,               // temperature, tint, vignette intensity, vignette radius
    effects: vec4<f32>,            // vignette softness, grain intensity, chromatic px, time
    flags: vec4<f32>,              // auto exposure enabled, explicit sRGB output, AO enabled, reserved
}

@group(0) @binding(4) var<uniform> params: ToneMapParams;
@group(0) @binding(5) var ao_texture: texture_2d<f32>;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex_index];
    var out: VertexOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return out;
}

fn clamp_uv(uv: vec2<f32>) -> vec2<f32> {
    return clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.299, 0.587, 0.114));
}

fn grade_color(input_color: vec3<f32>) -> vec3<f32> {
    let temperature = params.look.x;
    let tint = params.look.y;
    let warm_cool = vec3<f32>(1.0 + temperature * 0.12, 1.0, 1.0 - temperature * 0.12);
    let green_magenta = vec3<f32>(1.0 - tint * 0.035, 1.0 + tint * 0.07, 1.0 - tint * 0.035);
    var color = max(input_color * warm_cool * green_magenta, vec3<f32>(0.0));
    let gray = vec3<f32>(luminance(color));
    color = mix(gray, color, params.exposure_and_color.z);
    color = (color - vec3<f32>(0.5)) * params.exposure_and_color.y + vec3<f32>(0.5);
    return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn display_color(uv_in: vec2<f32>) -> vec3<f32> {
    let uv = clamp_uv(uv_in);
    var scene = textureSample(hdr_scene, hdr_sampler, uv).rgb;
    if params.flags.z > 0.5 {
        // Screen-space AO is applied to scene lighting; bloom remains unoccluded so bright
        // emissive highlights do not acquire dark halos around depth discontinuities.
        let visibility = textureSample(ao_texture, hdr_sampler, uv).r;
        scene *= visibility;
    }
    let bloom = textureSample(bloom_texture, hdr_sampler, uv).rgb * 0.42;
    let automatic_exposure = select(1.0, exposure_state, params.flags.x > 0.5);
    let hdr = max((scene * 1.15 + bloom) * params.exposure_and_color.x * automatic_exposure, vec3<f32>(0.0));
    let mapped = hdr / (vec3<f32>(1.0) + hdr);
    return grade_color(mapped);
}

// Apply channel separation after FXAA so the edge filter does not multiply the cost of
// chromatic-aberration taps across every neighborhood sample.
fn apply_chromatic_aberration(uv_in: vec2<f32>, center_color: vec3<f32>) -> vec3<f32> {
    let intensity = params.effects.z;
    if intensity <= 0.0001 {
        return center_color;
    }
    let uv = clamp_uv(uv_in);
    let dimensions = max(vec2<f32>(textureDimensions(hdr_scene, 0)), vec2<f32>(1.0));
    let radial = uv - vec2<f32>(0.5);
    let chromatic_offset = radial * (intensity / dimensions);
    let red = display_color(uv + chromatic_offset).r;
    let blue = display_color(uv - chromatic_offset).b;
    return vec3<f32>(red, center_color.g, blue);
}

fn hash_noise(pixel: vec2<f32>, frame: f32) -> f32 {
    let seed = pixel + vec2<f32>(frame * 17.0, frame * 29.0);
    return fract(sin(dot(seed, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn apply_fxaa(uv: vec2<f32>, center_color: vec3<f32>, inverse_size: vec2<f32>) -> vec3<f32> {
    let nw = luminance(display_color(uv + vec2<f32>(-1.0, -1.0) * inverse_size));
    let ne = luminance(display_color(uv + vec2<f32>( 1.0, -1.0) * inverse_size));
    let sw = luminance(display_color(uv + vec2<f32>(-1.0,  1.0) * inverse_size));
    let se = luminance(display_color(uv + vec2<f32>( 1.0,  1.0) * inverse_size));
    let middle = luminance(center_color);
    let luma_min = min(middle, min(min(nw, ne), min(sw, se)));
    let luma_max = max(middle, max(max(nw, ne), max(sw, se)));
    let edge_span = luma_max - luma_min;
    if edge_span < max(0.0312, luma_max * 0.125) {
        return center_color;
    }
    var direction = vec2<f32>(-((nw + ne) - (sw + se)), (nw + sw) - (ne + se));
    let reduce = max((nw + ne + sw + se) * 0.03125, 0.0078125);
    let reciprocal_min = 1.0 / (min(abs(direction.x), abs(direction.y)) + reduce);
    direction = clamp(direction * reciprocal_min, vec2<f32>(-8.0), vec2<f32>(8.0)) * inverse_size;
    let rgb_a = 0.5 * (
        display_color(uv + direction * (1.0 / 3.0 - 0.5)) +
        display_color(uv + direction * (2.0 / 3.0 - 0.5))
    );
    let rgb_b = rgb_a * 0.5 + 0.25 * (
        display_color(uv + direction * -0.5) +
        display_color(uv + direction * 0.5)
    );
    let result_luma = luminance(rgb_b);
    if result_luma < luma_min || result_luma > luma_max {
        return rgb_a;
    }
    return rgb_b;
}

fn linear_to_srgb(value: vec3<f32>) -> vec3<f32> {
    let low = value * 12.92;
    let high = 1.055 * pow(max(value, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(high, low, value <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let dimensions = max(vec2<f32>(textureDimensions(hdr_scene, 0)), vec2<f32>(1.0));
    let inverse_size = 1.0 / dimensions;
    var color = display_color(in.uv);
    if params.exposure_and_color.w > 0.5 {
        color = apply_fxaa(in.uv, color, inverse_size);
    }
    color = apply_chromatic_aberration(in.uv, color);

    let aspect_corrected = (in.uv - vec2<f32>(0.5)) * vec2<f32>(dimensions.x / dimensions.y, 1.0);
    let distance_from_center = length(aspect_corrected);
    let inner_radius = params.look.w * (1.0 - params.effects.x);
    let vignette = 1.0 - params.look.z * smoothstep(inner_radius, params.look.w, distance_from_center);
    color *= vignette;

    if params.effects.y > 0.0 {
        let pixel = floor(in.uv * dimensions);
        let frame = floor(params.effects.w * 60.0);
        let grain = hash_noise(pixel, frame) - 0.5;
        color += vec3<f32>(grain * params.effects.y);
    }

    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    if params.flags.y > 0.5 {
        color = linear_to_srgb(color);
    }
    return vec4<f32>(color, 1.0);
}
