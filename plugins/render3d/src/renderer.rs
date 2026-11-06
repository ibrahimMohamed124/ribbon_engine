//! wgpu pipeline that draws instanced, textured meshes with a depth buffer.

use crate::components::{FogSettings, PointLight, SpotLight, SunLight};
use crate::mesh;
use crate::profiling::{GpuCompletedSample, GpuTimestampProfiler, GpuTimingSnapshot};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3, Vec4};
use ribbon_plugin_gltf::AlphaMode;
use ribbon_render::profiling::RenderPassTiming;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Instant;
use wgpu::util::DeviceExt;

#[path = "particles.rs"]
mod particles;

/// Depth format used by the mesh pass.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Default maximum upstream distance, in world units, for directional shadow casters.
/// This is intentionally finite so a distant off-screen object cannot collapse cascade depth
/// precision. Applications with larger worlds should set an explicit value on `MeshRenderer`.
pub const DEFAULT_DIRECTIONAL_SHADOW_CASTER_EXTRUSION_DISTANCE: f32 = 100.0;
/// Hard safety limit applied by the runtime setting to keep depth ranges and matrices finite.
pub const MAX_DIRECTIONAL_SHADOW_CASTER_EXTRUSION_DISTANCE: f32 = 10_000.0;

/// Small CPU-side AABB used for conservative renderer visibility and shadow-caster fitting.
/// The transform helper enumerates all eight corners, so affine rotation, shear, negative scale,
/// and non-uniform scale remain conservative.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Aabb3 {
    min: Vec3,
    max: Vec3,
}

impl Aabb3 {
    fn is_valid(self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.min.cmple(self.max).all()
    }

    fn from_min_max(min: Vec3, max: Vec3) -> Option<Self> {
        if !min.is_finite() || !max.is_finite() || min.x > max.x || min.y > max.y || min.z > max.z {
            return None;
        }
        Some(Self { min, max })
    }

    fn from_points(points: impl IntoIterator<Item = Vec3>) -> Option<Self> {
        let mut bounds: Option<Self> = None;
        for point in points {
            if !point.is_finite() {
                return None;
            }
            if let Some(bounds) = &mut bounds {
                bounds.include_point(point);
            } else {
                bounds = Some(Self {
                    min: point,
                    max: point,
                });
            }
        }
        bounds
    }

    fn include_point(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    fn include_bounds(&mut self, other: Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    fn corners(self) -> [Vec3; 8] {
        [
            Vec3::new(self.min.x, self.min.y, self.min.z),
            Vec3::new(self.max.x, self.min.y, self.min.z),
            Vec3::new(self.min.x, self.max.y, self.min.z),
            Vec3::new(self.max.x, self.max.y, self.min.z),
            Vec3::new(self.min.x, self.min.y, self.max.z),
            Vec3::new(self.max.x, self.min.y, self.max.z),
            Vec3::new(self.min.x, self.max.y, self.max.z),
            Vec3::new(self.max.x, self.max.y, self.max.z),
        ]
    }

    fn transformed(self, matrix: Mat4) -> Option<Self> {
        if !self.is_valid()
            || !matrix.to_cols_array().iter().all(|value| value.is_finite())
            || matrix.x_axis.w.abs() > 1.0e-5
            || matrix.y_axis.w.abs() > 1.0e-5
            || matrix.z_axis.w.abs() > 1.0e-5
            || (matrix.w_axis.w - 1.0).abs() > 1.0e-5
        {
            return None;
        }
        Self::from_points(
            self.corners()
                .into_iter()
                .map(|corner| matrix.transform_point3(corner)),
        )
    }

    fn scaled_from_origin(self, scale: f32) -> Option<Self> {
        if !scale.is_finite() || scale < 1.0 {
            return None;
        }
        let a = self.min * scale;
        let b = self.max * scale;
        Self::from_min_max(a.min(b), a.max(b))
    }
}

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

/// Generation-checked handle to a standalone uploaded texture retained by the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId {
    slot: usize,
    generation: u32,
}

/// Generation-checked handle to a standalone uploaded sampler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SamplerId {
    slot: usize,
    generation: u32,
}

/// Addressing mode for one axis of a standalone GPU sampler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuSamplerAddressMode {
    ClampToEdge,
    Repeat,
    MirrorRepeat,
}

/// Filtering mode for a standalone GPU sampler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuSamplerFilter {
    Nearest,
    Linear,
}

/// Renderer-independent sampler descriptor accepted by generic GPU warmup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuSamplerDesc {
    pub address_mode_u: GpuSamplerAddressMode,
    pub address_mode_v: GpuSamplerAddressMode,
    pub address_mode_w: GpuSamplerAddressMode,
    pub mag_filter: GpuSamplerFilter,
    pub min_filter: GpuSamplerFilter,
    pub mipmap_filter: GpuSamplerFilter,
    pub lod_min_clamp: f32,
    pub lod_max_clamp: f32,
}

impl Default for GpuSamplerDesc {
    fn default() -> Self {
        Self {
            address_mode_u: GpuSamplerAddressMode::Repeat,
            address_mode_v: GpuSamplerAddressMode::Repeat,
            address_mode_w: GpuSamplerAddressMode::Repeat,
            mag_filter: GpuSamplerFilter::Linear,
            min_filter: GpuSamplerFilter::Linear,
            mipmap_filter: GpuSamplerFilter::Linear,
            lod_min_clamp: 0.0,
            lod_max_clamp: 32.0,
        }
    }
}

impl GpuSamplerDesc {
    pub fn validate(self) -> Result<(), String> {
        if !self.lod_min_clamp.is_finite() || !self.lod_max_clamp.is_finite() {
            return Err("sampler LOD clamps must be finite".to_owned());
        }
        if self.lod_min_clamp < 0.0 || self.lod_max_clamp < self.lod_min_clamp {
            return Err("sampler LOD range must satisfy 0 <= min <= max".to_owned());
        }
        Ok(())
    }
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
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PbrMaterialMipmaps<'a> {
    pub(crate) base_color: Option<&'a [(u32, u32, Vec<u8>)]>,
    pub(crate) metallic_roughness: Option<&'a [(u32, u32, Vec<u8>)]>,
    pub(crate) normal: Option<&'a [(u32, u32, Vec<u8>)]>,
    pub(crate) occlusion: Option<&'a [(u32, u32, Vec<u8>)]>,
    pub(crate) emissive: Option<&'a [(u32, u32, Vec<u8>)]>,
}

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

/// Additive render-state options for PBR materials. The original `PbrMaterialDesc`
/// remains source-compatible; callers opt into glTF alpha and sidedness behavior here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PbrMaterialRenderOptions {
    /// Opaque, alpha-mask, or alpha-blend interpretation.
    pub alpha_mode: AlphaMode,
    /// Alpha-test threshold used by `AlphaMode::Mask`, clamped to 0..=1 on upload.
    pub alpha_cutoff: f32,
    /// Render both triangle orientations and flip back-face normals.
    pub double_sided: bool,
}

impl Default for PbrMaterialRenderOptions {
    fn default() -> Self {
        Self {
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}

/// glTF texture-coordinate set selected independently by each material texture. Only UV0 and
/// UV1 are supported by the current vertex formats; higher indices safely fall back to UV0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MaterialTextureCoordinates {
    pub base_color: u32,
    pub metallic_roughness: u32,
    pub normal: u32,
    pub occlusion: u32,
    pub emissive: u32,
}

/// Pack material texture selectors into the two `vec4<u32>` fields shared by all mesh shaders.
/// Only set 1 is represented explicitly; set 0 and unsupported sets greater than 1 use UV0.
pub(crate) fn encode_material_texture_coordinates(
    coordinates: MaterialTextureCoordinates,
) -> ([u32; 4], [u32; 4]) {
    (
        [
            if coordinates.base_color == 1 { 1 } else { 0 },
            if coordinates.metallic_roughness == 1 {
                1
            } else {
                0
            },
            if coordinates.normal == 1 { 1 } else { 0 },
            if coordinates.occlusion == 1 { 1 } else { 0 },
        ],
        [if coordinates.emissive == 1 { 1 } else { 0 }, 0, 0, 0],
    )
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

/// One skinned instance. Its palette is immutable for the duration of a render snapshot.
#[derive(Clone, Debug)]
pub struct SkinnedDraw {
    pub draw: Draw,
    /// Model-local joint matrices in glTF skin joint order.
    pub joint_matrices: Arc<[Mat4]>,
}

#[derive(Clone, Debug)]
struct PreparedDraw {
    draw: Draw,
    palette: Option<Arc<[Mat4]>>,
    palette_order_hash: u64,
    bone_offset: u32,
}

/// Borrowed resources for one mesh-render submission. Grouping these keeps the internal
/// renderer entry points small as additional per-frame inputs (for example skinning palettes)
/// are introduced, without hiding their ownership or extending resource lifetimes.
pub(crate) struct RenderTarget<'a, 'scene> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    /// Color attachment; multisampled when `MeshRenderer::sample_count() > 1`.
    pub view: &'a wgpu::TextureView,
    /// Single-sample resolve destination for MSAA. Must be `None` for sample count one.
    pub resolve_view: Option<&'a wgpu::TextureView>,
    pub size: (u32, u32),
    pub scene: &'a Scene<'scene>,
}

/// Everything one render call needs besides the GPU handles.
pub struct Scene<'a> {
    /// Combined projection and view matrix.
    pub view_proj: Mat4,
    /// Camera world position used by view-dependent BRDF terms.
    pub camera_position: Vec3,
    /// Camera world rotation used to orient camera-facing particle billboards.
    pub camera_rotation: Quat,
    /// Directional light and ambient term.
    pub sun: &'a SunLight,
    /// World-space spot lights. The renderer ranks valid records deterministically and currently
    /// renders at most one shadow-casting light; a multi-caster atlas remains future work.
    pub spot_lights: &'a [SpotLight],
    /// Point lights ranked by estimated contribution; the strongest eight are consumed.
    pub point_lights: &'a [PointLight],
    /// Exponential height/distance fog.
    pub fog: FogSettings,
    /// Objects to draw.
    pub draws: &'a [Draw],
    /// Extracted particle batches drawn after mesh instances.
    pub particles: &'a [ribbon_render_contracts::ParticleBatch],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawInstance {
    model: [[f32; 4]; 4],
    tint: [f32; 4],
    bone_offset: u32,
    _padding: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_view_proj: [[[f32; 4]; 4]; shadows::DIRECTIONAL_CASCADE_COUNT],
    cascade_splits: [f32; 4],
    /// x: camera near depth, y: cascade blend fraction, z: debug mode, w: selected depth layer.
    cascade_params: [f32; 4],
    camera_forward: [f32; 4],
    spot_view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    camera_pos: [f32; 4],
    spot_position_range: [f32; 4],
    spot_direction_outer: [f32; 4],
    spot_color_intensity: [f32; 4],
    spot_params: [f32; 4],
    point_positions_range: [[f32; 4]; MAX_POINT_LIGHTS],
    point_colors_intensity: [[f32; 4]; MAX_POINT_LIGHTS],
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
    /// x: alpha mode (0 opaque, 1 mask, 2 blend), y: alpha cutoff.
    render: [f32; 4],
    /// UV set indices: base color, metallic-roughness, normal, occlusion.
    texture_uv_sets: [u32; 4],
    /// x: emissive UV set index; remaining values reserved.
    texture_uv_sets_extra: [u32; 4],
}

struct DepthTarget {
    view: wgpu::TextureView,
    size: (u32, u32),
    sample_count: u32,
}

struct DirectionalShadowTarget {
    // Keep the layered texture alive alongside its sampling and per-layer render views.
    _texture: wgpu::Texture,
    sample_view: wgpu::TextureView,
    layer_views: [wgpu::TextureView; shadows::DIRECTIONAL_CASCADE_COUNT],
}

struct ShadowTarget {
    // Keep the texture alive alongside its view.
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MeshVertexLayout {
    Static,
    Skinned,
}

/// Runtime policy for directional shadow caster depth coverage.
///
/// Insert this resource into the application `World` to control how far upstream from the
/// camera-frustum receiver region opaque/masked shadow casters may extend a cascade's light-space
/// depth interval. The distance is measured in world units along the directional-light ray.
/// A value of zero disables off-frustum caster extension while preserving receiver-only fitting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalShadowCasterSettings {
    /// Maximum upstream caster-to-receiver extrusion distance in world units.
    pub max_extrusion_distance: f32,
}

impl Default for DirectionalShadowCasterSettings {
    fn default() -> Self {
        Self {
            max_extrusion_distance: DEFAULT_DIRECTIONAL_SHADOW_CASTER_EXTRUSION_DISTANCE,
        }
    }
}

impl DirectionalShadowCasterSettings {
    /// Return settings with a finite, bounded extrusion distance suitable for GPU fitting.
    pub fn sanitized(self) -> Self {
        Self {
            max_extrusion_distance: if self.max_extrusion_distance.is_finite() {
                self.max_extrusion_distance
                    .clamp(0.0, MAX_DIRECTIONAL_SHADOW_CASTER_EXTRUSION_DISTANCE)
            } else {
                DEFAULT_DIRECTIONAL_SHADOW_CASTER_EXTRUSION_DISTANCE
            },
        }
    }
}

/// Optional visualization modes for diagnosing cascaded directional shadows.
///
/// These modes only affect the diagnostic color output; they do not change directional or
/// spotlight shadow generation. The default is `Disabled`, preserving normal PBR rendering.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectionalShadowDebugMode {
    /// Normal PBR shading and shadowing.
    #[default]
    Disabled = 0,
    /// Paint visible geometry by the selected directional cascade.
    CascadeIndex = 1,
    /// Visualize the blend weight used while transitioning to the next cascade.
    BlendWeight = 2,
    /// Replace the scene with a grayscale preview of one directional depth-array layer.
    DepthLayer = 3,
    /// Render the final directional shadow visibility after sampling and blending cascades.
    ShadowVisibility = 4,
    /// Render current-cascade visibility, next-cascade visibility, and their final mix in RGB.
    ShadowVisibilityComponents = 5,
}

/// Runtime controls for the directional-shadow diagnostic renderer.
///
/// `Render3dPlugin` installs this as a world resource and reads it every frame. Applications can
/// change it at runtime with `world.resource_mut::<DirectionalShadowDebugOptions>()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectionalShadowDebugOptions {
    /// Visualization mode; defaults to normal rendering.
    pub mode: DirectionalShadowDebugMode,
    /// Depth-array layer shown by `DepthLayer`, clamped to the supported range `0..=3`.
    pub depth_layer: u32,
}

impl DirectionalShadowDebugOptions {
    fn sanitized(self) -> Self {
        Self {
            depth_layer: self
                .depth_layer
                .min(shadows::DIRECTIONAL_CASCADE_COUNT.saturating_sub(1) as u32),
            ..self
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowDepthDebugUniform {
    /// x = selected depth-array layer; yzw are reserved and must remain zero.
    selected_layer: [u32; 4],
    /// xy = output dimensions used to letterbox the square shadow map without distortion.
    target_size: [u32; 4],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    vertex_layout: MeshVertexLayout,
    indices: wgpu::Buffer,
    index_count: u32,
    bounds_center: Vec3,
    bounds_radius: f32,
    /// Valid local-space geometry bounds. Kept separately from the legacy sphere used by culling.
    local_bounds: Option<Aabb3>,
    /// Sorted (joint index, local influence AABB) pairs precomputed at skinned-mesh upload.
    joint_influence_bounds: Vec<(u16, Aabb3)>,
    /// Vertices for which the production shader's `weight_sum <= 1e-6` path skips skinning.
    unweighted_bounds: Option<Aabb3>,
    /// Maximum accepted positive weight sum. Values up to 1.001 are permitted by model upload.
    max_skin_weight_sum: f32,
    min_skin_weight_sum: f32,
    skin_bounds_valid: bool,
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
    alpha_mode: AlphaMode,
    double_sided: bool,
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

struct TextureSlot {
    generation: u32,
    texture: Option<Arc<GpuTexture>>,
}

struct SamplerSlot {
    generation: u32,
    sampler: Option<wgpu::Sampler>,
}

/// Estimated live resource usage for one [`MeshRenderer`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshRendererResourceStats {
    /// Number of currently live meshes, including the built-in cube.
    pub live_meshes: usize,
    /// Number of currently live materials, including the built-in white material.
    pub live_materials: usize,
    /// Unique cached textures retained only by standalone texture leases rather than materials.
    pub live_standalone_textures: usize,
    /// Estimated bytes for standalone-only textures, included in total residency estimates.
    pub standalone_texture_bytes_estimate: u64,
    /// Number of live standalone sampler objects. Driver memory is not reported for samplers.
    pub live_samplers: usize,
    /// Number of camera-visible mesh instances prepared for the main color pass.
    pub visible_mesh_instances: usize,
    /// Number of grouped indexed draw calls issued by the main color pass.
    pub visible_draw_calls: usize,
    /// Number of particle instances drawn in the latest main pass.
    pub particle_instances_drawn: usize,
    /// Number of particle batches drawn in the latest main pass.
    pub particle_batches_drawn: usize,
    /// Number of grouped indexed draw calls issued across directional cascades and the selected spotlight pass.
    pub shadow_draw_calls: usize,
    /// Skinned draws retained for the current frame's color/shadow submissions.
    pub skinned_draws_after_palette_budget: usize,
    /// Skinned draws rejected because the frame-local joint palette capacity was exhausted.
    pub skinned_draws_rejected_palette_capacity: usize,
    /// Number of joint matrices uploaded to the frame-local storage buffer.
    pub skin_palette_matrices_staged: usize,
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
            .saturating_add(self.standalone_texture_bytes_estimate)
            .saturating_add(self.buffer_bytes_estimate)
            .saturating_add(self.depth_target_bytes_estimate)
    }
}

/// Owns the pipeline, uploaded meshes and materials, and the depth target.
pub struct MeshRenderer {
    /// Negotiated sample count shared by the main color pipelines and depth attachment.
    sample_count: u32,
    /// Opaque and masked, back-face culled pipeline.
    pipeline: wgpu::RenderPipeline,
    /// Opaque and masked pipeline for glTF double-sided materials.
    double_sided_pipeline: wgpu::RenderPipeline,
    /// Alpha-blended pipeline; depth test remains enabled and depth writes are disabled.
    blend_pipeline: wgpu::RenderPipeline,
    /// Alpha-blended double-sided pipeline.
    blend_double_sided_pipeline: wgpu::RenderPipeline,
    /// Compact-vertex opaque/masked pipeline.
    static_pipeline: wgpu::RenderPipeline,
    /// Compact-vertex double-sided opaque/masked pipeline.
    static_double_sided_pipeline: wgpu::RenderPipeline,
    /// Compact-vertex alpha-blended pipeline.
    static_blend_pipeline: wgpu::RenderPipeline,
    /// Compact-vertex alpha-blended double-sided pipeline.
    static_blend_double_sided_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    /// Directional shadow variant for glTF double-sided materials.
    shadow_double_sided_pipeline: wgpu::RenderPipeline,
    /// Static-vertex directional shadow pipeline.
    static_shadow_pipeline: wgpu::RenderPipeline,
    /// Static-vertex double-sided directional shadow pipeline.
    static_shadow_double_sided_pipeline: wgpu::RenderPipeline,
    spot_shadow_pipeline: wgpu::RenderPipeline,
    /// Static-vertex spotlight shadow pipeline.
    static_spot_shadow_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    directional_shadow_globals: [wgpu::Buffer; shadows::DIRECTIONAL_CASCADE_COUNT],
    directional_shadow_globals_groups: [wgpu::BindGroup; shadows::DIRECTIONAL_CASCADE_COUNT],
    /// Fixed-capacity per-frame storage palette shared by color and shadow vertex stages.
    skin_matrices: wgpu::Buffer,
    spot_shadow_globals: wgpu::Buffer,
    spot_shadow_globals_group: wgpu::BindGroup,
    shadow_target: DirectionalShadowTarget,
    spot_shadow_target: ShadowTarget,
    _shadow_sampler: wgpu::Sampler,
    /// Full-screen inspector for one layer of the directional depth texture array.
    shadow_depth_debug_pipeline: wgpu::RenderPipeline,
    shadow_depth_debug_params: wgpu::Buffer,
    shadow_depth_debug_group: wgpu::BindGroup,
    directional_shadow_debug: DirectionalShadowDebugOptions,
    /// Maximum upstream caster distance from the receiver depth interval, in world units.
    /// Zero disables off-frustum caster extension while retaining receiver-only fitting.
    directional_shadow_caster_extrusion_distance: f32,
    /// Reused projected caster bounds; capacity is retained between frames.
    directional_shadow_caster_bounds_scratch: Vec<Aabb3>,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    meshes: Vec<MeshSlot>,
    free_mesh_slots: Vec<usize>,
    materials: Vec<MaterialSlot>,
    free_material_slots: Vec<usize>,
    // Standalone texture leases keep the Arc alive independently of any material bind group.
    textures: Vec<TextureSlot>,
    free_texture_slots: Vec<usize>,
    samplers: Vec<SamplerSlot>,
    free_sampler_slots: Vec<usize>,
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
    last_particle_instances_drawn: usize,
    last_particle_batches_drawn: usize,
    particles: particles::ParticleGpu,
    last_shadow_draw_calls: usize,
    last_skinned_draws_after_palette_budget: usize,
    last_skinned_draws_rejected_palette_capacity: usize,
    last_skin_palette_matrices_staged: usize,
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
    /// Actual sample count used by the color pipelines and matching depth target.
    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// Set optional directional-shadow diagnostic output. Passing the default options restores
    /// normal PBR rendering. The selected depth-array layer is clamped to the four valid layers.
    pub fn set_directional_shadow_debug(&mut self, options: DirectionalShadowDebugOptions) {
        self.directional_shadow_debug = options.sanitized();
    }

    /// Return the currently active directional-shadow diagnostic settings.
    pub fn directional_shadow_debug(&self) -> DirectionalShadowDebugOptions {
        self.directional_shadow_debug
    }

    /// Configure the maximum upstream caster distance used by directional CSM depth fitting.
    /// The value is measured in world units along the sun-ray direction. Zero disables caster
    /// extension; non-finite values restore the safe default and finite values are clamped to the
    /// supported range `0..=MAX_DIRECTIONAL_SHADOW_CASTER_EXTRUSION_DISTANCE`.
    pub fn set_directional_shadow_caster_extrusion_distance(&mut self, distance: f32) {
        self.directional_shadow_caster_extrusion_distance = DirectionalShadowCasterSettings {
            max_extrusion_distance: distance,
        }
        .sanitized()
        .max_extrusion_distance;
    }

    /// Return the configured maximum upstream directional-shadow caster distance in world units.
    pub fn directional_shadow_caster_extrusion_distance(&self) -> f32 {
        self.directional_shadow_caster_extrusion_distance
    }

    /// Main scene depth view used by the screen-space ambient-occlusion post pass.
    /// The view is valid until the renderer is resized or its device is lost.
    pub(crate) fn depth_view(&self) -> Option<&wgpu::TextureView> {
        self.depth.as_ref().map(|depth| &depth.view)
    }

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
/// Shader/uniform capacity for point lights. Keep synchronized with `Globals` and mesh.wgsl.
pub const MAX_POINT_LIGHTS: usize = 8;
const INSTANCE_BUFFER_SHRINK_THRESHOLD: usize = 256;
const INSTANCE_BUFFER_SHRINK_AFTER_FRAMES: u32 = 120;
/// Per-frame storage palette capacity (mat4x4<f32> per joint); overflow is rejected, never indexed.
pub const MAX_SKIN_MATRICES_PER_FRAME: usize = 65_536;

// `SkinnedVertex` stores all eight joint indices before all eight weights. The shader's
// locations are grouped by influence set: locations 5/6 are JOINTS_0/1 and 7/8 are
// WEIGHTS_0/1. Keep these offsets explicit because their shader order differs from the
// struct's byte order; `vertex_attr_array!` would otherwise interpret JOINTS_1 as WEIGHTS_0.
const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 9] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 12,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 24,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 32,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 48,
        shader_location: 4,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint16x4,
        offset: 56,
        shader_location: 5,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint16x4,
        offset: 64,
        shader_location: 6,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 72,
        shader_location: 7,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 88,
        shader_location: 8,
    },
];
const STATIC_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 12,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 24,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 32,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 48,
        shader_location: 4,
    },
];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    9 => Float32x4, 10 => Float32x4, 11 => Float32x4, 12 => Float32x4, 13 => Float32x4, 14 => Uint32
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

const SRGB_MIP_LUT_STEPS: usize = 16_384;

struct SrgbMipLookup {
    to_linear: [f32; 256],
    to_srgb: Box<[u8]>,
}

fn srgb_mip_lookup() -> &'static SrgbMipLookup {
    static LOOKUP: OnceLock<SrgbMipLookup> = OnceLock::new();
    LOOKUP.get_or_init(|| SrgbMipLookup {
        to_linear: std::array::from_fn(|value| srgb_to_linear(value as u8)),
        // Quantizing linear values to 1/16384 keeps the encoded result within a fraction of
        // one 8-bit sRGB step while avoiding hundreds of thousands of powf calls per mip chain.
        to_srgb: (0..=SRGB_MIP_LUT_STEPS)
            .map(|value| linear_to_srgb(value as f32 / SRGB_MIP_LUT_STEPS as f32))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    })
}

/// Box-filter mip chain (level 0 first). Color is averaged in linear light.
///
/// The conversion tables are initialized once and then reused by all texture uploads. This
/// keeps expensive transfer-function exponentials out of the render-thread hot path while
/// preserving linear-light filtering and 8-bit output precision.
pub fn build_mips(width: u32, height: u32, rgba8: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let lookup = srgb_mip_lookup();
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
                let mut alpha = 0_u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(w - 1);
                    let sy = (y * 2 + dy).min(h - 1);
                    let at = ((sy * w + sx) * 4) as usize;
                    for (channel, sum) in color.iter_mut().enumerate() {
                        *sum += lookup.to_linear[previous[at + channel] as usize];
                    }
                    alpha += u32::from(previous[at + 3]);
                }
                for linear_sum in color {
                    let linear_average = (linear_sum * 0.25).clamp(0.0, 1.0);
                    let lookup_index =
                        (linear_average * SRGB_MIP_LUT_STEPS as f32).round() as usize;
                    next.push(lookup.to_srgb[lookup_index.min(SRGB_MIP_LUT_STEPS)]);
                }
                next.push(((alpha + 2) / 4) as u8);
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
    // `Hasher::write` processes the slice in bulk. Hashing every byte as a separate `Hash`
    // invocation caused a million tiny hasher calls for a 512x512 RGBA texture.
    second.write(source.rgba8);

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
    prepared_mips: Option<&[(u32, u32, Vec<u8>)]>,
) -> (wgpu::Texture, wgpu::TextureView) {
    // glTF texture mip levels are prepared by the AssetServer decode worker. Standalone callers
    // retain the compatibility path that builds their mip chain here on the render thread.
    let generated_mips = prepared_mips.is_none().then(|| {
        if srgb {
            build_mips(source.width, source.height, source.rgba8)
        } else {
            build_data_mips(source.width, source.height, source.rgba8)
        }
    });
    let mipmaps = prepared_mips.unwrap_or_else(|| {
        generated_mips
            .as_deref()
            .and_then(|levels| levels.get(1..))
            .unwrap_or(&[])
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: source.width,
            height: source.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: (mipmaps.len() + 1) as u32,
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
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        source.rgba8,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(source.width * 4),
            rows_per_image: Some(source.height),
        },
        wgpu::Extent3d {
            width: source.width,
            height: source.height,
            depth_or_array_layers: 1,
        },
    );
    for (index, (width, height, pixels)) in mipmaps.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: index as u32 + 1,
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

fn pack_point_lights(
    lights: &[PointLight],
) -> ([[f32; 4]; MAX_POINT_LIGHTS], [[f32; 4]; MAX_POINT_LIGHTS]) {
    let mut positions_range = [[0.0; 4]; MAX_POINT_LIGHTS];
    let mut colors_intensity = [[0.0; 4]; MAX_POINT_LIGHTS];
    for (index, light) in lights.iter().take(MAX_POINT_LIGHTS).enumerate() {
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

/// Estimate point-light contribution near the active camera. This deliberately mirrors the
/// shader's range-limited inverse-square behavior while preferring lights with useful range.
/// Invalid/non-positive lights are rejected before they reach the uniform buffer.
fn point_light_importance(light: PointLight, camera_position: Vec3) -> f32 {
    if !light.position.is_finite()
        || !light.color.is_finite()
        || !light.intensity.is_finite()
        || !light.range.is_finite()
        || !camera_position.is_finite()
        || light.intensity <= 0.0
        || light.range <= 0.0
    {
        return 0.0;
    }

    let distance_squared = light.position.distance_squared(camera_position);
    if !distance_squared.is_finite() {
        return 0.0;
    }
    let color = light.color.max(Vec3::ZERO);
    let luminance = color.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    let range_squared = light.range * light.range;
    let denominator = distance_squared + (range_squared * 0.04).max(1.0e-4);
    let score = light.intensity * luminance * range_squared / denominator;
    if score.is_finite() {
        score
    } else {
        f32::MAX
    }
}

/// Compare light values lexicographically so equal-importance lights are independent of ECS
/// query/insertion order. Identical records are interchangeable in the GPU uniform buffer.
fn compare_point_light_identity(a: &PointLight, b: &PointLight) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let a_values = [
        a.position.x,
        a.position.y,
        a.position.z,
        a.color.x,
        a.color.y,
        a.color.z,
        a.intensity,
        a.range,
    ];
    let b_values = [
        b.position.x,
        b.position.y,
        b.position.z,
        b.color.x,
        b.color.y,
        b.color.z,
        b.intensity,
        b.range,
    ];
    for (a, b) in a_values.into_iter().zip(b_values) {
        let order = a.total_cmp(&b);
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

/// Select the strongest bounded set of point lights in deterministic order.
fn select_point_lights(lights: &[PointLight], camera_position: Vec3) -> Vec<PointLight> {
    let mut ranked: Vec<(f32, PointLight)> = lights
        .iter()
        .copied()
        .filter_map(|light| {
            let score = point_light_importance(light, camera_position);
            (score > 0.0).then_some((score, light))
        })
        .collect();
    ranked.sort_by(|(score_a, light_a), (score_b, light_b)| {
        score_b
            .total_cmp(score_a)
            .then_with(|| compare_point_light_identity(light_a, light_b))
    });
    ranked
        .into_iter()
        .take(MAX_POINT_LIGHTS)
        .map(|(_, light)| light)
        .collect()
}

/// Estimate a spotlight's contribution near the active camera.
///
/// This is a bounded selection heuristic, not a substitute for evaluating every visible
/// fragment. It combines range-limited inverse-square falloff, linear color luminance, intensity,
/// and the cone's solid-angle coverage. Invalid or non-contributing lights are rejected before
/// they reach the uniforms or shadow pass.
fn spot_light_importance(light: SpotLight, camera_position: Vec3) -> f32 {
    let direction_length_squared = light.direction.length_squared();
    if !light.position.is_finite()
        || !light.direction.is_finite()
        || !direction_length_squared.is_finite()
        || direction_length_squared <= 1.0e-12
        || !light.color.is_finite()
        || !light.intensity.is_finite()
        || !light.range.is_finite()
        || !light.inner_cone_angle.is_finite()
        || !light.outer_cone_angle.is_finite()
        || !camera_position.is_finite()
        || light.intensity <= 0.0
        || light.range <= 0.0
        || light.outer_cone_angle <= 0.0
        || light.inner_cone_angle < 0.0
        || light.inner_cone_angle > light.outer_cone_angle
    {
        return 0.0;
    }

    let distance_squared = light.position.distance_squared(camera_position);
    if !distance_squared.is_finite() {
        return 0.0;
    }

    let color = light.color.max(Vec3::ZERO);
    let luminance = color.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    if !luminance.is_finite() || luminance <= 0.0 {
        return 0.0;
    }

    let outer_angle = light.outer_cone_angle.clamp(0.01, 1.55);
    let cone_solid_angle_factor = (1.0 - outer_angle.cos()).max(0.0);
    let range_squared = light.range * light.range;
    let denominator = distance_squared + (range_squared * 0.04).max(1.0e-4);
    let score = light.intensity * luminance * range_squared * cone_solid_angle_factor / denominator;
    if score.is_finite() {
        score
    } else {
        f32::MAX
    }
}

/// Break equal scores by light values, not by incidental ECS extraction order. Identical light
/// records are interchangeable in the single-shadow-map renderer.
fn compare_spot_light_identity(a: &SpotLight, b: &SpotLight) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let a_values = [
        a.position.x,
        a.position.y,
        a.position.z,
        a.direction.x,
        a.direction.y,
        a.direction.z,
        a.color.x,
        a.color.y,
        a.color.z,
        a.intensity,
        a.range,
        a.inner_cone_angle,
        a.outer_cone_angle,
    ];
    let b_values = [
        b.position.x,
        b.position.y,
        b.position.z,
        b.direction.x,
        b.direction.y,
        b.direction.z,
        b.color.x,
        b.color.y,
        b.color.z,
        b.intensity,
        b.range,
        b.inner_cone_angle,
        b.outer_cone_angle,
    ];

    for (a_value, b_value) in a_values.into_iter().zip(b_values) {
        let order = a_value.total_cmp(&b_value);
        if order != Ordering::Equal {
            return order;
        }
    }
    a.casts_shadows.cmp(&b.casts_shadows)
}

/// Select the most important valid spotlight and, independently, the most important valid
/// shadow caster. The current renderer still supports one spotlight shadow map; selection is
/// deterministic and camera-aware until the bounded multi-caster atlas is implemented.
fn select_spot_lights(
    lights: &[SpotLight],
    camera_position: Vec3,
) -> (Option<SpotLight>, Option<SpotLight>) {
    let mut ranked: Vec<(f32, SpotLight)> = lights
        .iter()
        .copied()
        .filter_map(|light| {
            let score = spot_light_importance(light, camera_position);
            (score > 0.0).then_some((score, light))
        })
        .collect();
    ranked.sort_by(|(score_a, light_a), (score_b, light_b)| {
        score_b
            .total_cmp(score_a)
            .then_with(|| compare_spot_light_identity(light_a, light_b))
    });

    let shadow = ranked
        .iter()
        .find(|(_, light)| light.casts_shadows)
        .map(|(_, light)| *light);
    let active = shadow.or_else(|| ranked.first().map(|(_, light)| *light));
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
