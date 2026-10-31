//! wgpu pipeline that draws instanced, textured meshes with a depth buffer.

use crate::components::{FogSettings, PointLight, SpotLight, SunLight};
use crate::mesh::{self, Vertex};
use crate::profiling::{GpuCompletedSample, GpuTimestampProfiler, GpuTimingSnapshot};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use ribbon_render::profiling::RenderPassTiming;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};
use std::time::Instant;
use wgpu::util::DeviceExt;

/// Depth format used by the mesh pass.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Generation-checked handle to a mesh stored in a [`MeshRenderer`].
///
/// Releasing and reusing a slot cannot make an old handle refer to the replacement mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId {
    slot: usize,
    generation: u32,
}

/// Generation-checked handle to a material stored in a [`MeshRenderer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialId {
    slot: usize,
    generation: u32,
}

/// RGBA8 pixels in sRGB encoding, row-major from the top-left.
#[derive(Clone, Copy, Debug)]
pub struct TextureData<'a> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA8 pixels.
    pub rgba8: &'a [u8],
}

/// Parameters and optional texture maps for a metallic-roughness material.
#[derive(Clone, Copy, Debug)]
pub struct PbrMaterialDesc<'a> {
    /// Linear RGBA base-color factor.
    pub base_color: [f32; 4],
    /// Metallic factor in the inclusive range 0..=1.
    pub metallic: f32,
    /// Roughness factor, clamped by the renderer to 0.04..=1.
    pub roughness: f32,
    /// Optional sRGB base-color texture.
    pub base_color_texture: Option<TextureData<'a>>,
    /// Optional linear glTF metallic-roughness texture (G=roughness, B=metallic).
    pub metallic_roughness_texture: Option<TextureData<'a>>,
    /// Optional tangent-space normal texture (linear RGB).
    pub normal_texture: Option<TextureData<'a>>,
    /// Optional red-channel ambient occlusion texture (linear).
    pub occlusion_texture: Option<TextureData<'a>>,
    /// Multiplier for the occlusion texture (0..=1).
    pub occlusion_strength: f32,
    /// Optional emissive texture, sampled in sRGB.
    pub emissive_texture: Option<TextureData<'a>>,
    /// Linear emissive color factor.
    pub emissive_factor: [f32; 3],
}

/// One object to draw.
#[derive(Clone, Copy, Debug)]
pub struct Draw {
    /// Geometry to draw.
    pub mesh: MeshId,
    /// Surface material.
    pub material: MaterialId,
    /// Object-to-world matrix.
    pub model: Mat4,
    /// Linear RGB multiplier applied on top of the material.
    pub tint: [f32; 3],
}

/// Everything one render call needs besides the GPU handles.
pub struct Scene<'a> {
    /// Combined projection and view matrix.
    pub view_proj: Mat4,
    /// Camera world position used by view-dependent BRDF terms.
    pub camera_position: Vec3,
    /// Directional light and ambient term.
    pub sun: &'a SunLight,
    /// World-space spot lights. The renderer uses at most one shadow-casting light.
    pub spot_lights: &'a [SpotLight],
    /// Non-shadowed point lights; only the first eight are consumed.
    pub point_lights: &'a [PointLight],
    /// Exponential height/distance fog.
    pub fog: FogSettings,
    /// Objects to draw.
    pub draws: &'a [Draw],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawInstance {
    model: [[f32; 4]; 4],
    tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_view_proj: [[f32; 4]; 4],
    spot_view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    camera_pos: [f32; 4],
    spot_position_range: [f32; 4],
    spot_direction_outer: [f32; 4],
    spot_color_intensity: [f32; 4],
    spot_params: [f32; 4],
    point_positions_range: [[f32; 4]; 8],
    point_colors_intensity: [[f32; 4]; 8],
    point_count: [f32; 4],
    fog_color_density: [f32; 4],
    fog_params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowGlobals {
    light_view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    /// x: metallic, y: perceptual roughness, z: AO strength.
    surface: [f32; 4],
    emissive: [f32; 4],
}

struct DepthTarget {
    view: wgpu::TextureView,
    size: (u32, u32),
}

struct ShadowTarget {
    // Keep the texture alive alongside its view.
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    bounds_center: Vec3,
    bounds_radius: f32,
    estimated_bytes: u64,
}

struct MeshSlot {
    generation: u32,
    mesh: Option<GpuMesh>,
}

struct GpuTexture {
    // The view does not replace ownership of the underlying texture.
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    estimated_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TextureCacheKey {
    width: u32,
    height: u32,
    srgb: bool,
    fingerprint: [u64; 2],
}

struct GpuMaterial {
    bind_group: wgpu::BindGroup,
    _uniform: wgpu::Buffer,
    _base_texture: Arc<GpuTexture>,
    _metallic_roughness_texture: Arc<GpuTexture>,
    _normal_texture: Arc<GpuTexture>,
    _occlusion_texture: Arc<GpuTexture>,
    _emissive_texture: Arc<GpuTexture>,
    uniform_bytes_estimate: u64,
}

struct MaterialSlot {
    generation: u32,
    material: Option<GpuMaterial>,
}

/// Estimated live resource usage for one [`MeshRenderer`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshRendererResourceStats {
    /// Number of currently live meshes, including the built-in cube.
    pub live_meshes: usize,
    /// Number of currently live materials, including the built-in white material.
    pub live_materials: usize,
    /// Number of camera-visible mesh instances prepared for the main color pass.
    pub visible_mesh_instances: usize,
    /// Number of grouped indexed draw calls issued by the main color pass.
    pub visible_draw_calls: usize,
    /// Number of grouped indexed draw calls issued by the directional shadow pass.
    pub shadow_draw_calls: usize,
    /// Estimated live vertex/index buffer bytes.
    pub mesh_bytes_estimate: u64,
    /// Estimated material texture and uniform bytes. Texture allocations are counted once even
    /// when multiple materials share identical image content and color-space interpretation.
    pub material_bytes_estimate: u64,
    /// Number of unique live GPU textures referenced by material slots.
    pub live_material_textures: usize,
    /// Estimated bytes across unique live material textures, including mip levels.
    pub material_texture_bytes_estimate: u64,
    /// Estimated material uniform-buffer bytes; included in `material_bytes_estimate`.
    pub material_uniform_bytes_estimate: u64,
    /// Cumulative requests satisfied from the renderer's content-addressed texture cache.
    pub material_texture_cache_hits: u64,
    /// Cumulative texture allocations caused by a content-cache miss.
    pub material_texture_cache_misses: u64,
    /// Estimated uniform and instance buffer bytes.
    pub buffer_bytes_estimate: u64,
    /// Estimated shadow-map and depth-target bytes.
    pub depth_target_bytes_estimate: u64,
    /// Capacity retained by both the visible and shadow instance buffers.
    pub instance_capacity: usize,
    /// Count of visible/shadow instance-buffer capacity increases during this renderer lifetime.
    pub instance_buffer_grows_cumulative: u64,
    /// Count of visible/shadow instance-buffer capacity decreases during this renderer lifetime.
    pub instance_buffer_shrinks_cumulative: u64,
    /// Peak total tracked allocation estimate since renderer construction.
    pub peak_bytes_estimate: u64,
}

impl MeshRendererResourceStats {
    /// Sum all tracked allocations. Pipeline/bind-group overhead is excluded.
    pub fn total_bytes_estimate(self) -> u64 {
        self.mesh_bytes_estimate
            .saturating_add(self.material_bytes_estimate)
            .saturating_add(self.buffer_bytes_estimate)
            .saturating_add(self.depth_target_bytes_estimate)
    }
}

/// Owns the pipeline, uploaded meshes and materials, and the depth target.
pub struct MeshRenderer {
    pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    spot_shadow_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    shadow_globals: wgpu::Buffer,
    shadow_globals_group: wgpu::BindGroup,
    spot_shadow_globals: wgpu::Buffer,
    spot_shadow_globals_group: wgpu::BindGroup,
    shadow_target: ShadowTarget,
    spot_shadow_target: ShadowTarget,
    _shadow_sampler: wgpu::Sampler,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    meshes: Vec<MeshSlot>,
    free_mesh_slots: Vec<usize>,
    materials: Vec<MaterialSlot>,
    free_material_slots: Vec<usize>,
    /// Weak content-addressed texture entries: live materials own shared textures, and released
    /// materials allow unreferenced GPU textures to be dropped immediately.
    texture_cache: HashMap<TextureCacheKey, Weak<GpuTexture>>,
    texture_cache_hits: u64,
    texture_cache_misses: u64,
    /// Optional adapter-supported GPU timer for shadow and mesh render passes.
    gpu_timing: Option<GpuTimestampProfiler>,
    peak_resource_bytes_estimate: u64,
    last_visible_mesh_instances: usize,
    last_visible_draw_calls: usize,
    last_shadow_draw_calls: usize,
    /// Compact, frustum-visible instances used by the main color pass.
    instances: wgpu::Buffer,
    /// All instances, including off-screen casters, used by the shadow pass.
    shadow_instances: wgpu::Buffer,
    instance_capacity: usize,
    underused_instance_frames: u32,
    instance_buffer_grows_cumulative: u64,
    instance_buffer_shrinks_cumulative: u64,
    depth: Option<DepthTarget>,
}

impl MeshRenderer {
    /// Generation-checked handle for the permanent built-in cube mesh (slot zero).
    pub const CUBE: MeshId = MeshId {
        slot: 0,
        generation: 0,
    };
    /// Generation-checked handle for the permanent built-in white material (slot zero).
    pub const WHITE: MaterialId = MaterialId {
        slot: 0,
        generation: 0,
    };
}
const MIN_INSTANCE_CAPACITY: usize = 16;
const INSTANCE_BUFFER_SHRINK_THRESHOLD: usize = 256;
const INSTANCE_BUFFER_SHRINK_AFTER_FRAMES: u32 = 120;

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
];

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mesh instances"),
        size: (capacity * std::mem::size_of::<RawInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn record_cpu_phase(
    phases: &mut Vec<RenderPassTiming>,
    name: &'static str,
    started: Option<Instant>,
) {
    if let Some(started) = started {
        phases.push(RenderPassTiming {
            name: name.to_owned(),
            cpu_time_ns: started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
        });
    }
}

fn srgb_to_linear(value: u8) -> f32 {
    let s = f32::from(value) / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

/// Box-filter mip chain (level 0 first). Color is averaged in linear light.
pub fn build_mips(width: u32, height: u32, rgba8: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, rgba8.to_vec())];
    while let Some((w, h, previous)) = levels.last() {
        if *w == 1 && *h == 1 {
            break;
        }
        let (w, h) = (*w, *h);
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = Vec::with_capacity((nw * nh * 4) as usize);
        for y in 0..nh {
            for x in 0..nw {
                let mut color = [0.0_f32; 3];
                let mut alpha = 0.0_f32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(w - 1);
                    let sy = (y * 2 + dy).min(h - 1);
                    let at = ((sy * w + sx) * 4) as usize;
                    for (channel, sum) in color.iter_mut().enumerate() {
                        *sum += srgb_to_linear(previous[at + channel]);
                    }
                    alpha += f32::from(previous[at + 3]);
                }
                next.extend(color.map(|sum| linear_to_srgb(sum / 4.0)));
                next.push((alpha / 4.0).round() as u8);
            }
        }
        levels.push((nw, nh, next));
    }
    levels
}

/// Box-filter non-color data such as glTF metallic-roughness maps without sRGB conversion.
fn build_data_mips(width: u32, height: u32, rgba8: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, rgba8.to_vec())];
    while let Some((w, h, previous)) = levels.last() {
        if *w == 1 && *h == 1 {
            break;
        }
        let (w, h) = (*w, *h);
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = Vec::with_capacity((nw * nh * 4) as usize);
        for y in 0..nh {
            for x in 0..nw {
                for channel in 0..4 {
                    let mut sum = 0_u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        let at = ((sy * w + sx) * 4 + channel) as usize;
                        sum += u32::from(previous[at]);
                    }
                    next.push(((sum + 2) / 4) as u8);
                }
            }
        }
        levels.push((nw, nh, next));
    }
    levels
}

fn texture_cache_key(source: TextureData<'_>, srgb: bool) -> TextureCacheKey {
    // Use two independently salted 64-bit fingerprints to make accidental content collisions
    // vanishingly unlikely without adding a hash dependency or retaining duplicate CPU pixel data.
    let mut first = DefaultHasher::new();
    0x5249_4242_4f4e_0001_u64.hash(&mut first);
    source.width.hash(&mut first);
    source.height.hash(&mut first);
    srgb.hash(&mut first);
    source.rgba8.hash(&mut first);

    let mut second = DefaultHasher::new();
    0x5249_4242_4f4e_0002_u64.hash(&mut second);
    source.width.hash(&mut second);
    source.height.hash(&mut second);
    srgb.hash(&mut second);
    source.rgba8.len().hash(&mut second);
    for byte in source.rgba8.iter().rev() {
        byte.hash(&mut second);
    }

    TextureCacheKey {
        width: source.width,
        height: source.height,
        srgb,
        fingerprint: [first.finish(), second.finish()],
    }
}

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: TextureData<'_>,
    label: &'static str,
    srgb: bool,
) -> (wgpu::Texture, wgpu::TextureView) {
    let levels = if srgb {
        build_mips(source.width, source.height, source.rgba8)
    } else {
        build_data_mips(source.width, source.height, source.rgba8)
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: source.width,
            height: source.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (width, height, pixels)) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(*height),
            },
            wgpu::Extent3d {
                width: *width,
                height: *height,
                depth_or_array_layers: 1,
            },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn frustum_planes(view_proj: Mat4) -> [Vec4; 6] {
    let columns = view_proj.to_cols_array_2d();
    let row0 = Vec4::new(columns[0][0], columns[1][0], columns[2][0], columns[3][0]);
    let row1 = Vec4::new(columns[0][1], columns[1][1], columns[2][1], columns[3][1]);
    let row2 = Vec4::new(columns[0][2], columns[1][2], columns[2][2], columns[3][2]);
    let row3 = Vec4::new(columns[0][3], columns[1][3], columns[2][3], columns[3][3]);
    let normalize_plane = |plane: Vec4| {
        let length = plane.truncate().length();
        if length > 1.0e-6 {
            plane / length
        } else {
            plane
        }
    };
    [
        normalize_plane(row3 + row0),
        normalize_plane(row3 - row0),
        normalize_plane(row3 + row1),
        normalize_plane(row3 - row1),
        normalize_plane(row2),
        normalize_plane(row3 - row2),
    ]
}

fn sphere_visible(planes: &[Vec4; 6], center: Vec3, radius: f32) -> bool {
    planes
        .iter()
        .all(|plane| plane.truncate().dot(center) + plane.w >= -radius)
}

fn pack_point_lights(lights: &[PointLight]) -> ([[f32; 4]; 8], [[f32; 4]; 8]) {
    let mut positions_range = [[0.0; 4]; 8];
    let mut colors_intensity = [[0.0; 4]; 8];
    for (index, light) in lights.iter().take(8).enumerate() {
        positions_range[index] = [
            light.position.x,
            light.position.y,
            light.position.z,
            light.range.max(0.0),
        ];
        colors_intensity[index] = [
            light.color.x,
            light.color.y,
            light.color.z,
            light.intensity.max(0.0),
        ];
    }
    (positions_range, colors_intensity)
}

fn select_spot_lights(lights: &[SpotLight]) -> (Option<SpotLight>, Option<SpotLight>) {
    let shadow = lights.iter().copied().find(|light| light.casts_shadows);
    let active = shadow.or_else(|| lights.first().copied());
    (active, shadow)
}

fn valid_texture<'a>(texture: Option<TextureData<'a>>, fallback: &'a [u8; 4]) -> TextureData<'a> {
    match texture {
        Some(texture)
            if texture.width > 0
                && texture.height > 0
                && texture.rgba8.len()
                    == (texture.width as usize) * (texture.height as usize) * 4 =>
        {
            texture
        }
        _ => TextureData {
            width: 1,
            height: 1,
            rgba8: fallback,
        },
    }
}

mod assets;
mod draw;
mod setup;
mod shadows;
#[cfg(test)]
mod tests;
