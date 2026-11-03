struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
    @location(3) local: vec2<f32>,
    @location(4) rect_size: vec2<f32>,
    @location(5) @interpolate(flat) radius: f32,
    @location(6) @interpolate(flat) border: f32,
};

@group(0) @binding(0) var glyph_atlas: texture_2d<f32>;
@group(0) @binding(1) var glyph_sampler: sampler;

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) kind: u32,
    @location(4) local: vec2<f32>,
    @location(5) rect_size: vec2<f32>,
    @location(6) radius: f32,
    @location(7) border: f32,
) -> VertexOut {
    var output: VertexOut;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = uv;
    output.color = color;
    output.kind = kind;
    output.local = local;
    output.rect_size = rect_size;
    output.radius = radius;
    output.border = border;
    return output;
}

fn rounded_box_distance(point: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(point) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0)))
        + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    if input.kind == 0u {
        let half_size = input.rect_size * 0.5;
        let point = (input.local - vec2<f32>(0.5)) * input.rect_size;
        let distance = rounded_box_distance(point, half_size, input.radius);
        let aa = max(fwidth(distance), 0.5);
        let outer = 1.0 - smoothstep(-aa, aa, distance);
        if input.border > 0.0 {
            let inner = 1.0 - smoothstep(-aa, aa, distance + input.border);
            let ring = clamp(outer - inner, 0.0, 1.0);
            return vec4<f32>(input.color.rgb, input.color.a * ring);
        }
        return vec4<f32>(input.color.rgb, input.color.a * outer);
    }
    let coverage = textureSample(glyph_atlas, glyph_sampler, input.uv).r;
    return vec4<f32>(input.color.rgb, input.color.a * coverage);
}
