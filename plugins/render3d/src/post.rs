//! HDR scene targets, adaptive exposure, and configurable fullscreen post-processing.

use crate::profiling::{GpuCompletedSample, GpuTimestampProfiler, GpuTimingSnapshot};
use bytemuck::{Pod, Zeroable};
use ribbon_render::profiling::RenderPassTiming;
use std::time::Instant;
use wgpu::util::DeviceExt;

/// Linear HDR color format used between the mesh renderer and presentation.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Anti-aliasing path selected for the main 3D render pass. MSAA uses multisampled color and
/// depth attachments resolved into the single-sample HDR scene target; FXAA runs after tone mapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AntiAliasingMode {
    /// No edge filtering; preserves the legacy single-sample path.
    #[default]
    None,
    /// 2x multisample anti-aliasing.
    Msaa2x,
    /// 4x multisample anti-aliasing.
    Msaa4x,
    /// 8x multisample anti-aliasing.
    Msaa8x,
    /// Fast approximate anti-aliasing after tone mapping.
    Fxaa,
}

impl AntiAliasingMode {
    pub const fn requested_samples(self) -> u32 {
        match self {
            Self::Msaa2x => 2,
            Self::Msaa4x => 4,
            Self::Msaa8x => 8,
            Self::None | Self::Fxaa => 1,
        }
    }

    pub const fn uses_fxaa(self) -> bool {
        matches!(self, Self::Fxaa)
    }
}

/// Renderer resource controlling anti-aliasing. The default deliberately preserves existing
/// output. Insert or configure this resource before the first GPU mesh/model upload so MSAA
/// sample count is selected before sample-dependent pipelines and generation-checked GPU handles
/// are created. Changing the MSAA request after renderer initialization does not rebuild those
/// pipelines; the renderer keeps its originally negotiated sample count and emits a diagnostic.
/// FXAA selection follows the current resource at runtime. Selecting FXAA after starting with MSAA
/// therefore adds FXAA on top of the latched MSAA path rather than replacing its sample count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AntiAliasingSettings {
    pub mode: AntiAliasingMode,
}

/// Sampling quality for the depth-based ambient-occlusion pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AmbientOcclusionQuality {
    /// Four screen-space directions with two samples per direction.
    Low,
    /// Six screen-space directions with three samples per direction.
    #[default]
    Medium,
    /// Eight screen-space directions with four samples per direction.
    High,
}

impl AmbientOcclusionQuality {
    const fn sample_counts(self) -> (u32, u32) {
        match self {
            Self::Low => (4, 2),
            Self::Medium => (6, 3),
            Self::High => (8, 4),
        }
    }
}

/// Screen-space ambient-occlusion settings. The pass reconstructs world-space geometry from
/// the main depth buffer; it is deliberately disabled by default so existing projects retain their
/// previous image and GPU cost until they opt in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbientOcclusionSettings {
    pub enabled: bool,
    pub quality: AmbientOcclusionQuality,
    /// Approximate world-space sampling radius.
    pub radius: f32,
    /// AO contribution strength in [0, 1].
    pub intensity: f32,
    /// World-space depth/normal bias used to suppress self-occlusion.
    pub bias: f32,
    /// Power curve applied to the visibility result.
    pub power: f32,
}

impl Default for AmbientOcclusionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            quality: AmbientOcclusionQuality::Medium,
            radius: 1.25,
            intensity: 0.85,
            bias: 0.025,
            power: 1.25,
        }
    }
}

impl AmbientOcclusionSettings {
    pub fn cinematic() -> Self {
        Self {
            enabled: true,
            quality: AmbientOcclusionQuality::High,
            radius: 1.4,
            intensity: 0.9,
            bias: 0.03,
            power: 1.2,
        }
    }

    fn sanitized(self) -> Self {
        let finite_or = |value: f32, fallback: f32| {
            if value.is_finite() {
                value
            } else {
                fallback
            }
        };
        Self {
            enabled: self.enabled,
            quality: self.quality,
            radius: finite_or(self.radius, 1.25).clamp(0.05, 8.0),
            intensity: finite_or(self.intensity, 0.85).clamp(0.0, 1.0),
            bias: finite_or(self.bias, 0.025).clamp(0.0001, 0.5),
            power: finite_or(self.power, 1.25).clamp(0.25, 4.0),
        }
    }
}

/// Select the highest supported sample count not greater than the requested quality. Sample
/// count one is the unconditional fallback. The caller supplies the intersection of color and
/// depth format capabilities, so a selected count is compatible with both attachments.
pub fn negotiate_msaa_sample_count(requested: u32, supported: &[u32]) -> u32 {
    if requested <= 1 {
        return 1;
    }
    [8, 4, 2]
        .into_iter()
        .find(|count| *count <= requested && supported.contains(count))
        .unwrap_or(1)
}

/// GPU-driven automatic exposure controls. The luminance reduction samples the HDR scene on the
/// GPU and adapts the exposure state over time; no pixels are read back to the CPU.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoExposureSettings {
    pub enabled: bool,
    /// Middle-grey target luminance, conventionally around 0.18.
    pub key_value: f32,
    pub min_exposure: f32,
    pub max_exposure: f32,
    /// Adaptation rate when the image gets darker and exposure must increase.
    pub speed_up: f32,
    /// Adaptation rate when the image gets brighter and exposure must decrease.
    pub speed_down: f32,
}

impl Default for AutoExposureSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            key_value: 0.18,
            min_exposure: 0.25,
            max_exposure: 8.0,
            speed_up: 2.5,
            speed_down: 1.5,
        }
    }
}

/// Display look controls applied after HDR tone mapping and before vignette/grain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorGradingSettings {
    /// Exposure compensation in photographic stops (EV).
    pub exposure_compensation_ev: f32,
    /// Contrast multiplier around middle grey; 1.0 is neutral.
    pub contrast: f32,
    /// Saturation multiplier; 1.0 is neutral and 0.0 is monochrome.
    pub saturation: f32,
    /// Warm/cool balance in [-1, 1]. Positive values are warmer.
    pub temperature: f32,
    /// Green/magenta balance in [-1, 1]. Positive values add green.
    pub tint: f32,
}

impl Default for ColorGradingSettings {
    fn default() -> Self {
        Self {
            exposure_compensation_ev: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
        }
    }
}

/// Radial edge darkening. Zero intensity disables the effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VignetteSettings {
    pub intensity: f32,
    /// Radius at which the vignette reaches full strength, in normalized screen space.
    pub radius: f32,
    /// Width of the smooth transition as a fraction of the radius.
    pub softness: f32,
}

impl Default for VignetteSettings {
    fn default() -> Self {
        Self {
            intensity: 0.0,
            radius: 0.78,
            softness: 0.42,
        }
    }
}

/// Animated monochrome sensor grain. Zero intensity disables the effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilmGrainSettings {
    /// Noise amplitude in display-referred linear color, normally 0.0 to 0.08.
    pub intensity: f32,
}

impl Default for FilmGrainSettings {
    fn default() -> Self {
        Self { intensity: 0.0 }
    }
}

/// Radial RGB channel separation measured in output pixels. Zero disables the effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChromaticAberrationSettings {
    pub intensity_pixels: f32,
}

impl Default for ChromaticAberrationSettings {
    fn default() -> Self {
        Self {
            intensity_pixels: 0.0,
        }
    }
}

/// All configurable display post-processing controls. Defaults preserve the existing image
/// (fixed 1.15 exposure, Reinhard tone mapping and bloom) while allowing each effect to be enabled
/// independently at runtime without rebuilding GPU pipelines.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PostProcessingSettings {
    pub auto_exposure: AutoExposureSettings,
    pub color_grading: ColorGradingSettings,
    pub vignette: VignetteSettings,
    pub film_grain: FilmGrainSettings,
    pub chromatic_aberration: ChromaticAberrationSettings,
}

impl PostProcessingSettings {
    /// A restrained example look that exercises all Phase 3B features.
    pub fn cinematic() -> Self {
        Self {
            auto_exposure: AutoExposureSettings {
                enabled: true,
                ..AutoExposureSettings::default()
            },
            color_grading: ColorGradingSettings {
                exposure_compensation_ev: -0.05,
                contrast: 1.06,
                saturation: 0.94,
                temperature: 0.08,
                tint: 0.015,
            },
            vignette: VignetteSettings {
                intensity: 0.18,
                radius: 0.80,
                softness: 0.45,
            },
            film_grain: FilmGrainSettings { intensity: 0.012 },
            chromatic_aberration: ChromaticAberrationSettings {
                intensity_pixels: 0.45,
            },
        }
    }

    fn sanitized(self) -> Self {
        let finite_or = |value: f32, fallback: f32| {
            if value.is_finite() {
                value
            } else {
                fallback
            }
        };
        let mut settings = self;
        settings.auto_exposure.key_value =
            finite_or(settings.auto_exposure.key_value, 0.18).clamp(0.01, 1.0);
        settings.auto_exposure.min_exposure =
            finite_or(settings.auto_exposure.min_exposure, 0.25).clamp(0.01, 32.0);
        settings.auto_exposure.max_exposure = finite_or(settings.auto_exposure.max_exposure, 8.0)
            .clamp(settings.auto_exposure.min_exposure, 32.0);
        settings.auto_exposure.speed_up =
            finite_or(settings.auto_exposure.speed_up, 2.5).clamp(0.01, 20.0);
        settings.auto_exposure.speed_down =
            finite_or(settings.auto_exposure.speed_down, 1.5).clamp(0.01, 20.0);
        settings.color_grading.exposure_compensation_ev =
            finite_or(settings.color_grading.exposure_compensation_ev, 0.0).clamp(-10.0, 10.0);
        settings.color_grading.contrast =
            finite_or(settings.color_grading.contrast, 1.0).clamp(0.0, 4.0);
        settings.color_grading.saturation =
            finite_or(settings.color_grading.saturation, 1.0).clamp(0.0, 3.0);
        settings.color_grading.temperature =
            finite_or(settings.color_grading.temperature, 0.0).clamp(-1.0, 1.0);
        settings.color_grading.tint = finite_or(settings.color_grading.tint, 0.0).clamp(-1.0, 1.0);
        settings.vignette.intensity = finite_or(settings.vignette.intensity, 0.0).clamp(0.0, 1.0);
        settings.vignette.radius = finite_or(settings.vignette.radius, 0.78).clamp(0.1, 1.5);
        settings.vignette.softness = finite_or(settings.vignette.softness, 0.42).clamp(0.01, 1.0);
        settings.film_grain.intensity =
            finite_or(settings.film_grain.intensity, 0.0).clamp(0.0, 0.15);
        settings.chromatic_aberration.intensity_pixels =
            finite_or(settings.chromatic_aberration.intensity_pixels, 0.0).clamp(0.0, 8.0);
        settings
    }
}

/// Standalone HDR color target for callers that manage their own GPU resource lifetime.
/// Internal window and offscreen renderer paths use [`ribbon_render::gpu::TransientTexturePool`]
/// so similarly sized targets can be reused across frames and resizes.
pub struct HdrTarget {
    _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
}

impl HdrTarget {
    /// Allocate an unmanaged single-sample HDR target.
    pub fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render3d HDR scene color"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
            size,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PostProcessRenderOptions<'a> {
    pub(crate) settings: &'a PostProcessingSettings,
    pub(crate) fxaa: bool,
    pub(crate) ao_settings: &'a AmbientOcclusionSettings,
    pub(crate) ao_frame: Option<AmbientOcclusionFrame<'a>>,
    pub(crate) profile_cpu: bool,
    pub(crate) submitted_frame: u64,
}

/// Per-frame camera/depth inputs for the GPU ambient-occlusion pass.
#[derive(Clone, Copy)]
pub(crate) struct AmbientOcclusionFrame<'a> {
    pub depth_view: &'a wgpu::TextureView,
    pub inverse_view_projection: [[f32; 4]; 4],
    pub camera_position: [f32; 3],
    pub fov_y_radians: f32,
    pub sample_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ToneMapUniform {
    exposure_and_color: [f32; 4],
    look: [f32; 4],
    effects: [f32; 4],
    flags: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ExposureComputeUniform {
    limits: [f32; 4],
    adaptation: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GtaoUniform {
    inverse_view_projection: [[f32; 4]; 4],
    camera_and_radius: [f32; 4],
    settings: [f32; 4],
    dimensions: [u32; 4],
    quality: [u32; 4],
}

const DEFAULT_AO_SETTINGS: AmbientOcclusionSettings = AmbientOcclusionSettings {
    enabled: false,
    quality: AmbientOcclusionQuality::Medium,
    radius: 1.25,
    intensity: 0.85,
    bias: 0.025,
    power: 1.25,
};

/// Fullscreen tone mapping and configurable display post-processing.
pub struct ToneMapper {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    bloom_pipeline: wgpu::RenderPipeline,
    bloom_bind_group: wgpu::BindGroup,
    exposure_pipeline: wgpu::ComputePipeline,
    exposure_bind_group: wgpu::BindGroup,
    _exposure_state_buffer: wgpu::Buffer,
    exposure_params_buffer: wgpu::Buffer,
    tone_map_uniform_buffer: wgpu::Buffer,
    ao_params_buffer: wgpu::Buffer,
    ao_single_pipeline: wgpu::ComputePipeline,
    ao_msaa_pipeline: wgpu::ComputePipeline,
    ao_single_layout: wgpu::BindGroupLayout,
    ao_msaa_layout: wgpu::BindGroupLayout,
    ao_bind_group: Option<wgpu::BindGroup>,
    ao_depth_signature: Option<(u32, u32, u32)>,
    _bloom_texture: wgpu::Texture,
    bloom_view: wgpu::TextureView,
    _ao_texture: wgpu::Texture,
    ao_view: wgpu::TextureView,
    _sampler: wgpu::Sampler,
    estimated_bytes: u64,
    gpu_timing: Option<GpuTimestampProfiler>,
    initial_fxaa: bool,
    output_srgb: bool,
    width: u32,
    height: u32,
    started_at: Instant,
    last_frame_at: Instant,
}

impl ToneMapper {
    /// Build a mapper for an HDR source view and the current surface format.
    pub fn new(
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        surface_format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Self {
        Self::new_internal(device, source, surface_format, size, false, false)
    }

    /// Build the window tone mapper with optional post-tone-map FXAA.
    pub fn new_with_fxaa(
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        surface_format: wgpu::TextureFormat,
        size: (u32, u32),
        fxaa: bool,
    ) -> Self {
        Self::new_internal(device, source, surface_format, size, fxaa, false)
    }

    /// Build a mapper for a linear UNORM output target. Unlike an sRGB surface, an
    /// `Rgba8Unorm` texture does not encode output values automatically, so this pass
    /// performs the linear-to-sRGB transfer after tone mapping.
    pub fn new_offscreen_unorm(
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        size: (u32, u32),
    ) -> Self {
        Self::new_internal(
            device,
            source,
            wgpu::TextureFormat::Rgba8Unorm,
            size,
            false,
            true,
        )
    }

    fn new_internal(
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        surface_format: wgpu::TextureFormat,
        size: (u32, u32),
        initial_fxaa: bool,
        output_srgb: bool,
    ) -> Self {
        let bloom_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render3d quarter-resolution bloom texture"),
            size: wgpu::Extent3d {
                width: (size.0 / 4).max(1),
                height: (size.1 / 4).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_view = bloom_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom extraction layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let tone_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tone mapper layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let exposure_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GPU auto-exposure compute layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tone mapper sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..wgpu::SamplerDescriptor::default()
        });
        let exposure_state_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU auto-exposure adapted state"),
            contents: bytemuck::bytes_of(&1.0_f32),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let exposure_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU auto-exposure parameters"),
            contents: bytemuck::bytes_of(&ExposureComputeUniform {
                limits: [0.25, 8.0, 0.18, 1.0 / 60.0],
                adaptation: [2.5, 1.5, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let tone_map_uniform_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tone mapping and color grading parameters"),
                contents: bytemuck::bytes_of(&ToneMapUniform {
                    exposure_and_color: [1.0, 1.0, 1.0, initial_fxaa as u8 as f32],
                    look: [0.0, 0.0, 0.0, 0.78],
                    effects: [0.42, 0.0, 0.0, 0.0],
                    flags: [0.0, output_srgb as u8 as f32, 0.0, 0.0],
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let ao_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render3d full-resolution ambient-occlusion visibility"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let ao_view = ao_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ao_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GTAO camera and sampling parameters"),
            contents: bytemuck::bytes_of(&GtaoUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let (ao_single_layout, ao_single_pipeline) =
            create_gtao_pipeline(device, false, include_str!("../shaders/gtao.wgsl"));
        let (ao_msaa_layout, ao_msaa_pipeline) =
            create_gtao_pipeline(device, true, include_str!("../shaders/gtao_msaa.wgsl"));
        let bloom_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bright pass bind group"),
            layout: &bloom_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tone mapper bind group"),
            layout: &tone_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&bloom_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: exposure_state_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: tone_map_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&ao_view),
                },
            ],
        });
        let exposure_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GPU auto-exposure bind group"),
            layout: &exposure_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: exposure_state_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: exposure_params_buffer.as_entire_binding(),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("HDR tone mapper and post-processing shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/tonemap_post.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tone mapper pipeline layout"),
            bind_group_layouts: &[&tone_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tone mapper pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let exposure_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GPU auto-exposure luminance adaptation shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/auto_exposure.wgsl").into()),
        });
        let exposure_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("GPU auto-exposure pipeline layout"),
                bind_group_layouts: &[&exposure_layout],
                push_constant_ranges: &[],
            });
        let exposure_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("GPU auto-exposure adaptation pipeline"),
            layout: Some(&exposure_pipeline_layout),
            module: &exposure_shader,
            entry_point: Some("cs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let bloom_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bright-pass bloom shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/bloom.wgsl").into()),
        });
        let bloom_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bright-pass bloom pipeline layout"),
                bind_group_layouts: &[&bloom_layout],
                push_constant_ranges: &[],
            });
        let bloom_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bright-pass bloom pipeline"),
            layout: Some(&bloom_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &bloom_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &bloom_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let bloom_bytes = u64::from((size.0 / 4).max(1))
            .saturating_mul(u64::from((size.1 / 4).max(1)))
            .saturating_mul(8);
        let ao_bytes = u64::from(size.0.max(1))
            .saturating_mul(u64::from(size.1.max(1)))
            .saturating_mul(4);
        let fixed_bytes = std::mem::size_of::<f32>() as u64
            + std::mem::size_of::<ExposureComputeUniform>() as u64
            + std::mem::size_of::<ToneMapUniform>() as u64
            + std::mem::size_of::<GtaoUniform>() as u64;
        Self {
            pipeline,
            bind_group,
            bloom_pipeline,
            bloom_bind_group,
            exposure_pipeline,
            exposure_bind_group,
            _exposure_state_buffer: exposure_state_buffer,
            exposure_params_buffer,
            tone_map_uniform_buffer,
            ao_params_buffer,
            ao_single_pipeline,
            ao_msaa_pipeline,
            ao_single_layout,
            ao_msaa_layout,
            ao_bind_group: None,
            ao_depth_signature: None,
            _bloom_texture: bloom_texture,
            bloom_view,
            _ao_texture: ao_texture,
            ao_view,
            _sampler: sampler,
            estimated_bytes: bloom_bytes
                .saturating_add(ao_bytes)
                .saturating_add(fixed_bytes),
            gpu_timing: GpuTimestampProfiler::new(device),
            initial_fxaa,
            output_srgb,
            width: size.0.max(1),
            height: size.1.max(1),
            started_at: Instant::now(),
            last_frame_at: Instant::now(),
        }
    }

    /// Estimated bytes for bloom/AO targets, post-process uniforms and timestamp readback buffers.
    /// Pipelines, bind groups, query-set storage and allocator overhead are not included.
    pub fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes.saturating_add(
            self.gpu_timing
                .as_ref()
                .map_or(0, |profiler| profiler.snapshot().buffer_bytes_estimate),
        )
    }

    /// Poll completed pass timings without waiting for unfinished GPU work.
    pub fn poll_gpu_timing(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if let Some(profiler) = self.gpu_timing.as_mut() {
            profiler.poll(device, queue);
        }
    }

    /// Latest asynchronous GPU timings for bloom extraction and tone/display mapping. The
    /// automatic-exposure compute pass is not included in the current GPU timestamp query layout;
    /// CPU command-encoding time is reported separately and is not a measure of compute execution time.
    pub fn gpu_timing_snapshot(&self) -> GpuTimingSnapshot {
        self.gpu_timing
            .as_ref()
            .map(GpuTimestampProfiler::snapshot)
            .unwrap_or_default()
    }

    pub(crate) fn drain_gpu_timing_samples(&mut self) -> Vec<GpuCompletedSample> {
        self.gpu_timing
            .as_mut()
            .map_or_else(Vec::new, GpuTimestampProfiler::drain_completed)
    }

    pub fn clear_gpu_timing_samples(&mut self) {
        if let Some(profiler) = self.gpu_timing.as_mut() {
            profiler.clear_completed();
        }
    }

    /// Draw the fullscreen tone-mapping triangle into the acquired window image.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
    ) {
        let fxaa = self.initial_fxaa;
        self.render_internal(
            device,
            queue,
            target,
            PostProcessRenderOptions {
                settings: &PostProcessingSettings::default(),
                fxaa,
                ao_settings: &DEFAULT_AO_SETTINGS,
                ao_frame: None,
                profile_cpu: false,
                submitted_frame: 0,
            },
        );
    }

    /// Draw with live post-processing settings. Updating the resource changes only uniform data;
    /// all pipelines and bind groups remain stable for the lifetime of this mapper.
    pub fn render_with_settings(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        settings: &PostProcessingSettings,
        fxaa: bool,
    ) {
        self.render_internal(
            device,
            queue,
            target,
            PostProcessRenderOptions {
                settings,
                fxaa,
                ao_settings: &DEFAULT_AO_SETTINGS,
                ao_frame: None,
                profile_cpu: false,
                submitted_frame: 0,
            },
        );
    }

    pub(crate) fn render_with_options(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        options: PostProcessRenderOptions<'_>,
    ) -> Vec<RenderPassTiming> {
        self.render_internal(device, queue, target, options)
    }

    fn render_internal(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        options: PostProcessRenderOptions<'_>,
    ) -> Vec<RenderPassTiming> {
        let settings = options.settings.sanitized();
        let fxaa = options.fxaa;
        let ao_settings = options.ao_settings.sanitized();
        let ao_frame = options.ao_frame;
        let ao_enabled = ao_settings.enabled && ao_frame.is_some();
        let profile_cpu = options.profile_cpu;
        let submitted_frame = options.submitted_frame;
        let now = Instant::now();
        let delta_time = now
            .saturating_duration_since(self.last_frame_at)
            .as_secs_f32()
            .clamp(1.0 / 1000.0, 0.1);
        let elapsed = now.saturating_duration_since(self.started_at).as_secs_f32();
        self.last_frame_at = now;

        let color = settings.color_grading;
        let vignette = settings.vignette;
        let effects = ToneMapUniform {
            exposure_and_color: [
                2.0_f32.powf(color.exposure_compensation_ev),
                color.contrast,
                color.saturation,
                fxaa as u8 as f32,
            ],
            look: [
                color.temperature,
                color.tint,
                vignette.intensity,
                vignette.radius,
            ],
            effects: [
                vignette.softness,
                settings.film_grain.intensity,
                settings.chromatic_aberration.intensity_pixels,
                elapsed,
            ],
            flags: [
                settings.auto_exposure.enabled as u8 as f32,
                self.output_srgb as u8 as f32,
                ao_enabled as u8 as f32,
                0.0,
            ],
        };
        queue.write_buffer(
            &self.tone_map_uniform_buffer,
            0,
            bytemuck::bytes_of(&effects),
        );

        if settings.auto_exposure.enabled {
            let exposure = settings.auto_exposure;
            let params = ExposureComputeUniform {
                limits: [
                    exposure.min_exposure,
                    exposure.max_exposure,
                    exposure.key_value,
                    delta_time,
                ],
                adaptation: [exposure.speed_up, exposure.speed_down, 0.0, 0.0],
            };
            queue.write_buffer(&self.exposure_params_buffer, 0, bytemuck::bytes_of(&params));
        }

        if let (true, Some(frame)) = (ao_enabled, ao_frame) {
            let signature = (self.width, self.height, frame.sample_count.max(1));
            if self.ao_depth_signature != Some(signature) || self.ao_bind_group.is_none() {
                let layout = if frame.sample_count > 1 {
                    &self.ao_msaa_layout
                } else {
                    &self.ao_single_layout
                };
                self.ao_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cached GTAO depth and visibility bind group"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(frame.depth_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&self.ao_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.ao_params_buffer.as_entire_binding(),
                        },
                    ],
                }));
                self.ao_depth_signature = Some(signature);
            }
            let (directions, steps) = ao_settings.quality.sample_counts();
            let uniform = GtaoUniform {
                inverse_view_projection: frame.inverse_view_projection,
                camera_and_radius: [
                    frame.camera_position[0],
                    frame.camera_position[1],
                    frame.camera_position[2],
                    ao_settings.radius,
                ],
                settings: [
                    ao_settings.intensity,
                    ao_settings.bias,
                    ao_settings.power,
                    1.0 / (frame.fov_y_radians * 0.5).tan().max(0.001),
                ],
                dimensions: [self.width, self.height, frame.sample_count.max(1), 0],
                quality: [directions, steps, 0, 0],
            };
            queue.write_buffer(&self.ao_params_buffer, 0, bytemuck::bytes_of(&uniform));
        }
        let ao_bind_group = ao_enabled.then_some(self.ao_bind_group.as_ref()).flatten();

        let mut cpu_phases = Vec::new();
        let timing_slot = self
            .gpu_timing
            .as_mut()
            .and_then(|profiler| profiler.reserve_slot(submitted_frame));
        let query_set = timing_slot.and_then(|_| {
            self.gpu_timing
                .as_ref()
                .map(GpuTimestampProfiler::query_set)
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("HDR post-processing encoder"),
        });
        let mut phase_started = profile_cpu.then(Instant::now);
        if let Some(bind_group) = ao_bind_group.as_ref() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("depth-based GTAO ambient occlusion"),
                timestamp_writes: None,
            });
            if ao_frame.is_some_and(|frame| frame.sample_count > 1) {
                pass.set_pipeline(&self.ao_msaa_pipeline);
            } else {
                pass.set_pipeline(&self.ao_single_pipeline);
            }
            pass.set_bind_group(0, *bind_group, &[]);
            pass.dispatch_workgroups(self.width.div_ceil(8), self.height.div_ceil(8), 1);
        }
        record_cpu_phase(
            &mut cpu_phases,
            "GTAO ambient-occlusion command encoding",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        if settings.auto_exposure.enabled {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("GPU average log-luminance exposure adaptation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.exposure_pipeline);
            pass.set_bind_group(0, &self.exposure_bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        record_cpu_phase(
            &mut cpu_phases,
            "Auto exposure command encoding",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("quarter-resolution bloom extraction and blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: query_set.map(|query_set| wgpu::RenderPassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.bloom_pipeline);
            pass.set_bind_group(0, &self.bloom_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        record_cpu_phase(
            &mut cpu_phases,
            "Bloom command encoding",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tone mapping and display post-processing"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: query_set.map(|query_set| wgpu::RenderPassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: Some(2),
                    end_of_pass_write_index: Some(3),
                }),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        record_cpu_phase(
            &mut cpu_phases,
            "tone-mapping and display-effects command encoding",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        if let (Some(slot), Some(profiler)) = (timing_slot, self.gpu_timing.as_ref()) {
            profiler.encode_resolve(&mut encoder, slot);
        }
        queue.submit(Some(encoder.finish()));
        record_cpu_phase(
            &mut cpu_phases,
            "post-process queue submission",
            phase_started.take(),
        );
        if let (Some(slot), Some(profiler)) = (timing_slot, self.gpu_timing.as_mut()) {
            profiler.submitted(slot);
        }
        cpu_phases
    }
}

fn create_gtao_pipeline(
    device: &wgpu::Device,
    multisampled: bool,
    shader_source: &'static str,
) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(if multisampled {
            "GTAO multisampled depth layout"
        } else {
            "GTAO depth layout"
        }),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(if multisampled {
            "GTAO multisampled depth shader"
        } else {
            "GTAO depth shader"
        }),
        source: wgpu::ShaderSource::Wgsl(shader_source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("GTAO compute pipeline layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(if multisampled {
            "GTAO multisampled depth pipeline"
        } else {
            "GTAO depth pipeline"
        }),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("cs_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    (layout, pipeline)
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

#[cfg(test)]
mod anti_aliasing_tests {
    use super::{
        negotiate_msaa_sample_count, AmbientOcclusionQuality, AmbientOcclusionSettings,
        AntiAliasingMode, AutoExposureSettings, ChromaticAberrationSettings, ColorGradingSettings,
        FilmGrainSettings, PostProcessingSettings, VignetteSettings,
    };

    #[test]
    fn msaa_negotiation_uses_supported_count_and_falls_back_downward() {
        assert_eq!(negotiate_msaa_sample_count(8, &[1, 2, 4, 8]), 8);
        assert_eq!(negotiate_msaa_sample_count(8, &[1, 2, 4]), 4);
        assert_eq!(negotiate_msaa_sample_count(4, &[1, 2]), 2);
        assert_eq!(negotiate_msaa_sample_count(2, &[1]), 1);
        assert_eq!(negotiate_msaa_sample_count(1, &[1, 2, 4, 8]), 1);
    }

    #[test]
    fn anti_aliasing_modes_choose_expected_post_and_attachment_paths() {
        assert_eq!(AntiAliasingMode::Msaa4x.requested_samples(), 4);
        assert!(!AntiAliasingMode::Msaa4x.uses_fxaa());
        assert_eq!(AntiAliasingMode::Fxaa.requested_samples(), 1);
        assert!(AntiAliasingMode::Fxaa.uses_fxaa());
        assert_eq!(AntiAliasingMode::None.requested_samples(), 1);
    }

    #[test]
    fn ambient_occlusion_defaults_are_disabled_and_quality_samples_are_bounded() {
        let defaults = AmbientOcclusionSettings::default();
        assert!(!defaults.enabled);
        assert_eq!(AmbientOcclusionQuality::Low.sample_counts(), (4, 2));
        assert_eq!(AmbientOcclusionQuality::Medium.sample_counts(), (6, 3));
        assert_eq!(AmbientOcclusionQuality::High.sample_counts(), (8, 4));
        let cinematic = AmbientOcclusionSettings::cinematic().sanitized();
        assert!(cinematic.enabled);
        assert!((0.05..=8.0).contains(&cinematic.radius));
        assert!((0.0..=1.0).contains(&cinematic.intensity));
    }

    #[test]
    fn ambient_occlusion_sanitization_contains_nan_and_extreme_values() {
        let settings = AmbientOcclusionSettings {
            enabled: true,
            quality: AmbientOcclusionQuality::High,
            radius: f32::INFINITY,
            intensity: f32::NAN,
            bias: -100.0,
            power: 100.0,
        }
        .sanitized();
        assert_eq!(settings.radius, 1.25);
        assert_eq!(settings.intensity, 0.85);
        assert_eq!(settings.bias, 0.0001);
        assert_eq!(settings.power, 4.0);
    }

    #[test]
    fn post_processing_defaults_are_neutral_and_cinematic_is_bounded() {
        let neutral = PostProcessingSettings::default();
        assert!(!neutral.auto_exposure.enabled);
        assert_eq!(neutral.color_grading, ColorGradingSettings::default());
        assert_eq!(neutral.vignette, VignetteSettings::default());
        assert_eq!(neutral.film_grain, FilmGrainSettings::default());
        assert_eq!(
            neutral.chromatic_aberration,
            ChromaticAberrationSettings::default()
        );
        let cinematic = PostProcessingSettings::cinematic().sanitized();
        assert!(cinematic.auto_exposure.enabled);
        assert!((0.0..=1.0).contains(&cinematic.vignette.intensity));
        assert!((0.0..=8.0).contains(&cinematic.chromatic_aberration.intensity_pixels));
    }

    #[test]
    fn post_processing_sanitization_contains_nan_and_extreme_values() {
        let settings = PostProcessingSettings {
            auto_exposure: AutoExposureSettings {
                enabled: true,
                key_value: f32::NAN,
                min_exposure: -50.0,
                max_exposure: f32::INFINITY,
                speed_up: f32::NAN,
                speed_down: 100.0,
            },
            color_grading: ColorGradingSettings {
                exposure_compensation_ev: f32::INFINITY,
                contrast: -4.0,
                saturation: f32::NAN,
                temperature: 5.0,
                tint: -8.0,
            },
            vignette: VignetteSettings {
                intensity: 3.0,
                radius: f32::NAN,
                softness: -2.0,
            },
            film_grain: FilmGrainSettings { intensity: 2.0 },
            chromatic_aberration: ChromaticAberrationSettings {
                intensity_pixels: 100.0,
            },
        }
        .sanitized();
        assert_eq!(settings.auto_exposure.key_value, 0.18);
        assert_eq!(settings.auto_exposure.min_exposure, 0.01);
        assert_eq!(settings.auto_exposure.max_exposure, 8.0);
        assert_eq!(settings.auto_exposure.speed_down, 20.0);
        assert_eq!(settings.color_grading.exposure_compensation_ev, 0.0);
        assert_eq!(settings.color_grading.contrast, 0.0);
        assert_eq!(settings.color_grading.temperature, 1.0);
        assert_eq!(settings.color_grading.tint, -1.0);
        assert_eq!(settings.vignette.intensity, 1.0);
        assert_eq!(settings.vignette.radius, 0.78);
        assert_eq!(settings.vignette.softness, 0.01);
        assert_eq!(settings.film_grain.intensity, 0.15);
        assert_eq!(settings.chromatic_aberration.intensity_pixels, 8.0);
    }

    #[test]
    fn post_processing_wgsl_shaders_validate() {
        for (name, source) in [
            (
                "tone mapping/post-processing",
                include_str!("../shaders/tonemap_post.wgsl"),
            ),
            (
                "GPU auto exposure",
                include_str!("../shaders/auto_exposure.wgsl"),
            ),
        ] {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|error| panic!("{name} WGSL must parse: {error}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{name} WGSL must validate: {error}"));
        }
    }
}
