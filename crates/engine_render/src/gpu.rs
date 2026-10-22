//! Wgpu context with an offscreen color target and synchronous CPU readback.

use crate::frame::Framebuffer;
use crate::graph::RenderError;
use engine_platform::Window;
use std::sync::{Arc, Mutex};

/// Stable label used by profile captures for the selected native-surface present mode.
pub(crate) fn present_mode_label(mode: wgpu::PresentMode) -> &'static str {
    match mode {
        wgpu::PresentMode::Fifo => "fifo",
        wgpu::PresentMode::FifoRelaxed => "fifo-relaxed",
        wgpu::PresentMode::Immediate => "immediate",
        wgpu::PresentMode::Mailbox => "mailbox",
        // Selection below resolves auto modes to concrete capabilities before configure.
        wgpu::PresentMode::AutoVsync => "auto-vsync",
        wgpu::PresentMode::AutoNoVsync => "auto-no-vsync",
    }
}

fn choose_present_mode(
    requested: &str,
    supported: &[wgpu::PresentMode],
) -> (wgpu::PresentMode, bool) {
    let normalized = requested.trim().to_ascii_lowercase().replace('_', "-");
    let has = |mode| supported.contains(&mode);
    match normalized.as_str() {
        "fifo" | "vsync" | "default" | "" => (wgpu::PresentMode::Fifo, false),
        // Resolve Auto* policies into an explicit, reported mode. This makes captures
        // reproducible and avoids relying on driver-specific implicit fallback choices.
        "auto-vsync" => {
            if has(wgpu::PresentMode::FifoRelaxed) {
                (wgpu::PresentMode::FifoRelaxed, false)
            } else {
                (wgpu::PresentMode::Fifo, false)
            }
        }
        "auto-no-vsync" | "no-vsync" => {
            if has(wgpu::PresentMode::Immediate) {
                (wgpu::PresentMode::Immediate, false)
            } else if has(wgpu::PresentMode::Mailbox) {
                (wgpu::PresentMode::Mailbox, false)
            } else {
                (wgpu::PresentMode::Fifo, false)
            }
        }
        "fifo-relaxed" if has(wgpu::PresentMode::FifoRelaxed) => {
            (wgpu::PresentMode::FifoRelaxed, false)
        }
        "immediate" if has(wgpu::PresentMode::Immediate) => (wgpu::PresentMode::Immediate, false),
        "mailbox" if has(wgpu::PresentMode::Mailbox) => (wgpu::PresentMode::Mailbox, false),
        "fifo-relaxed" | "immediate" | "mailbox" => (wgpu::PresentMode::Fifo, true),
        _ => (wgpu::PresentMode::Fifo, true),
    }
}

fn desired_frame_latency_from_env() -> (u32, bool) {
    match std::env::var("RIBBON_GPU_FRAME_LATENCY") {
        Err(_) => (1, false),
        Ok(value) => match value.trim().parse::<u32>() {
            Ok(latency @ 1..=3) => (latency, false),
            _ => (1, true),
        },
    }
}

/// An offscreen RGBA8 render target owned by a [`GpuContext`].
pub struct OffscreenTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
}

struct ReadbackBuffer {
    buffer: wgpu::Buffer,
    size_bytes: u64,
}

impl OffscreenTarget {
    /// Borrow the target's texture view for render passes.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

/// Descriptor for a reusable 2D transient render texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransientTextureDescriptor {
    /// Width, clamped to at least one pixel during allocation.
    pub width: u32,
    /// Height, clamped to at least one pixel during allocation.
    pub height: u32,
    /// Texture pixel format.
    pub format: wgpu::TextureFormat,
    /// GPU usages granted to the texture.
    pub usage: wgpu::TextureUsages,
}

impl TransientTextureDescriptor {
    /// Create a single-sample texture that can be rendered into and sampled later.
    pub fn color_target(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        }
    }
}

/// Generation-checked lease for one texture in a [`TransientTexturePool`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TransientTextureHandle {
    slot: usize,
    generation: u32,
}

struct PooledTexture {
    descriptor: TransientTextureDescriptor,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    leased: bool,
    bytes_estimate: Option<u64>,
}

struct TexturePoolSlot {
    generation: u32,
    texture: Option<PooledTexture>,
}

/// Statistics for transient texture allocations owned by a [`TransientTexturePool`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransientTexturePoolStats {
    /// Estimated bytes for all currently allocated textures, both leased and cached.
    pub allocated_bytes_estimate: u64,
    /// Estimated bytes in currently leased textures.
    pub active_bytes_estimate: u64,
    /// Estimated bytes in reusable cached textures.
    pub cached_bytes_estimate: u64,
    /// Number of currently leased textures.
    pub active_textures: usize,
    /// Number of cached reusable textures.
    pub cached_textures: usize,
    /// Number of textures whose format size is not accounted by the estimator.
    pub unknown_size_textures: usize,
    /// Peak estimated bytes observed since pool construction.
    pub peak_allocated_bytes_estimate: u64,
    /// Number of GPU texture allocations made by this pool.
    pub allocations: u64,
    /// Number of cache hits that reused an existing texture.
    pub reuses: u64,
}

/// Reuses compatible transient GPU render targets and tracks their explicit lifetime.
///
/// A handle is valid only while leased. Release it after the last command that references the
/// texture has been encoded and submitted to the same queue. Released textures remain cached
/// until the budget trim method evicts them; wgpu's submission/resource tracking keeps already
/// submitted work valid. This pool owns only its own textures and does not report driver-wide VRAM.
#[derive(Default)]
pub struct TransientTexturePool {
    slots: Vec<TexturePoolSlot>,
    allocations: u64,
    reuses: u64,
    peak_allocated_bytes_estimate: u64,
}

impl TransientTexturePool {
    /// Acquire a matching cached target or allocate a new target on `device`.
    pub fn acquire(
        &mut self,
        device: &wgpu::Device,
        descriptor: TransientTextureDescriptor,
    ) -> TransientTextureHandle {
        let descriptor = TransientTextureDescriptor {
            width: descriptor.width.max(1),
            height: descriptor.height.max(1),
            ..descriptor
        };
        let cached_slot = self.slots.iter().position(|slot| {
            slot.texture
                .as_ref()
                .is_some_and(|texture| !texture.leased && texture.descriptor == descriptor)
        });
        let index = if let Some(index) = cached_slot {
            self.reuses = self.reuses.saturating_add(1);
            index
        } else {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pooled transient render target"),
                size: wgpu::Extent3d {
                    width: descriptor.width,
                    height: descriptor.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: descriptor.format,
                usage: descriptor.usage,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bytes_estimate = estimate_texture_bytes(descriptor);
            let reusable_index = self.slots.iter().position(|slot| slot.texture.is_none());
            let index = if let Some(index) = reusable_index {
                let slot = &mut self.slots[index];
                slot.generation = slot.generation.wrapping_add(1).max(1);
                slot.texture = Some(PooledTexture {
                    descriptor,
                    _texture: texture,
                    view,
                    leased: true,
                    bytes_estimate,
                });
                index
            } else {
                let index = self.slots.len();
                self.slots.push(TexturePoolSlot {
                    generation: 1,
                    texture: Some(PooledTexture {
                        descriptor,
                        _texture: texture,
                        view,
                        leased: true,
                        bytes_estimate,
                    }),
                });
                index
            };
            self.allocations = self.allocations.saturating_add(1);
            let allocated = self.stats().allocated_bytes_estimate;
            self.peak_allocated_bytes_estimate = self.peak_allocated_bytes_estimate.max(allocated);
            return TransientTextureHandle {
                slot: index,
                generation: self.slots[index].generation,
            };
        };
        let slot = &mut self.slots[index];
        slot.generation = slot.generation.wrapping_add(1).max(1);
        if let Some(texture) = slot.texture.as_mut() {
            texture.leased = true;
        }
        TransientTextureHandle {
            slot: index,
            generation: slot.generation,
        }
    }

    /// Get the view for a currently leased handle. Cached or stale handles return `None`.
    pub fn view(&self, handle: TransientTextureHandle) -> Option<&wgpu::TextureView> {
        let slot = self.slots.get(handle.slot)?;
        if slot.generation != handle.generation {
            return None;
        }
        let texture = slot.texture.as_ref()?;
        texture.leased.then_some(&texture.view)
    }

    /// Get the descriptor for a currently leased handle.
    pub fn descriptor(&self, handle: TransientTextureHandle) -> Option<TransientTextureDescriptor> {
        let slot = self.slots.get(handle.slot)?;
        if slot.generation != handle.generation {
            return None;
        }
        let texture = slot.texture.as_ref()?;
        texture.leased.then_some(texture.descriptor)
    }

    /// Release a lease. The underlying texture remains cached for compatible future uses.
    pub fn release(&mut self, handle: TransientTextureHandle) -> bool {
        let Some(slot) = self.slots.get_mut(handle.slot) else {
            return false;
        };
        if slot.generation != handle.generation {
            return false;
        }
        let Some(texture) = slot.texture.as_mut() else {
            return false;
        };
        if !texture.leased {
            return false;
        }
        texture.leased = false;
        true
    }

    /// Drop cached (not leased) textures until their estimated bytes fit `budget_bytes`.
    ///
    /// Active leases are excluded from this budget and are never evicted. Cached textures with
    /// unknown sizes are dropped first so they cannot accumulate outside the accounting limit.
    /// Slot indices remain stable so live handles cannot be invalidated by vector compaction.
    pub fn trim_unused_to_budget(&mut self, budget_bytes: u64) -> TransientTexturePoolStats {
        for slot in self.slots.iter_mut().rev() {
            let Some(texture) = slot.texture.as_ref() else {
                continue;
            };
            if !texture.leased && texture.bytes_estimate.is_none() {
                slot.texture = None;
            }
        }
        let mut cached_bytes = self.stats().cached_bytes_estimate;
        if cached_bytes <= budget_bytes {
            return self.stats();
        }
        for slot in self.slots.iter_mut().rev() {
            if cached_bytes <= budget_bytes {
                break;
            }
            let Some(texture) = slot.texture.as_ref() else {
                continue;
            };
            if texture.leased {
                continue;
            }
            let bytes = texture.bytes_estimate.unwrap_or(0);
            cached_bytes = cached_bytes.saturating_sub(bytes);
            slot.texture = None;
        }
        self.stats()
    }

    /// Snapshot current counts and byte estimates.
    pub fn stats(&self) -> TransientTexturePoolStats {
        let mut stats = TransientTexturePoolStats {
            peak_allocated_bytes_estimate: self.peak_allocated_bytes_estimate,
            allocations: self.allocations,
            reuses: self.reuses,
            ..TransientTexturePoolStats::default()
        };
        for texture in self.slots.iter().filter_map(|slot| slot.texture.as_ref()) {
            if let Some(bytes) = texture.bytes_estimate {
                stats.allocated_bytes_estimate =
                    stats.allocated_bytes_estimate.saturating_add(bytes);
                if texture.leased {
                    stats.active_bytes_estimate = stats.active_bytes_estimate.saturating_add(bytes);
                } else {
                    stats.cached_bytes_estimate = stats.cached_bytes_estimate.saturating_add(bytes);
                }
            } else {
                stats.unknown_size_textures = stats.unknown_size_textures.saturating_add(1);
            }
            if texture.leased {
                stats.active_textures = stats.active_textures.saturating_add(1);
            } else {
                stats.cached_textures = stats.cached_textures.saturating_add(1);
            }
        }
        stats
    }
}

fn estimate_texture_bytes(descriptor: TransientTextureDescriptor) -> Option<u64> {
    let bytes_per_pixel = match descriptor.format {
        wgpu::TextureFormat::R8Unorm
        | wgpu::TextureFormat::R8Snorm
        | wgpu::TextureFormat::R8Uint
        | wgpu::TextureFormat::R8Sint => 1_u64,
        wgpu::TextureFormat::R16Uint
        | wgpu::TextureFormat::R16Sint
        | wgpu::TextureFormat::R16Float
        | wgpu::TextureFormat::Rg8Unorm
        | wgpu::TextureFormat::Rg8Snorm
        | wgpu::TextureFormat::Rg8Uint
        | wgpu::TextureFormat::Rg8Sint => 2_u64,
        wgpu::TextureFormat::R32Uint
        | wgpu::TextureFormat::R32Sint
        | wgpu::TextureFormat::R32Float
        | wgpu::TextureFormat::Rg16Float
        | wgpu::TextureFormat::Rgba8Unorm
        | wgpu::TextureFormat::Rgba8UnormSrgb
        | wgpu::TextureFormat::Bgra8Unorm
        | wgpu::TextureFormat::Bgra8UnormSrgb
        | wgpu::TextureFormat::Depth32Float => 4_u64,
        wgpu::TextureFormat::Rg32Float | wgpu::TextureFormat::Rgba16Float => 8_u64,
        wgpu::TextureFormat::Rgba32Float => 16_u64,
        _ => return None,
    };
    Some(
        u64::from(descriptor.width.max(1))
            .saturating_mul(u64::from(descriptor.height.max(1)))
            .saturating_mul(bytes_per_pixel),
    )
}

/// Shared instance, adapter, device, queue, optional surface, and offscreen target.
pub struct GpuContext {
    // Fields drop in declaration order. The surface and every device-owned object must be
    // released before the instance that created them; destroying a Vulkan instance first
    // crashes on exit (STATUS_ACCESS_VIOLATION on Windows).
    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    target: OffscreenTarget,
    // Synchronous readback is still part of the public contract, but reuse its staging buffer
    // instead of allocating a new GPU buffer for every call.
    readback_buffer: Mutex<Option<ReadbackBuffer>>,
    queue: wgpu::Queue,
    device: wgpu::Device,
    adapter: wgpu::Adapter,
    _instance: wgpu::Instance,
}

impl Drop for GpuContext {
    fn drop(&mut self) {
        // Let in-flight work finish so the surface/swapchain is idle when it is destroyed.
        let _ = self.device.poll(wgpu::PollType::Wait);
    }
}

impl GpuContext {
    /// Create an offscreen-only context.
    pub async fn new_offscreen(width: u32, height: u32) -> Result<Self, RenderError> {
        Self::new(None, width, height).await
    }

    /// Create a context compatible with a window surface and an offscreen target.
    pub async fn new_for_window(
        window: Arc<Window>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        Self::new(Some(window), width, height).await
    }

    async fn new(
        window: Option<Arc<Window>>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = window
            .as_ref()
            .map(|window| instance.create_surface(window.clone()))
            .transpose()
            .map_err(|error| RenderError(format!("could not create wgpu surface: {error}")))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface.as_ref(),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|error| RenderError(format!("no compatible wgpu adapter: {error}")))?;
        // RenderPassTimestampWrites requires TIMESTAMP_QUERY. Keep it optional so limited
        // adapters retain the regular rendering path without profiling.
        let adapter_features = adapter.features();
        let required_features = if adapter_features.contains(wgpu::Features::TIMESTAMP_QUERY) {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Ribbon Engine render device"),
                required_features,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| RenderError(format!("could not create wgpu device: {error}")))?;
        let surface_config = if let (Some(surface), Some(window)) = (&surface, &window) {
            let capabilities = surface.get_capabilities(&adapter);
            let format = capabilities
                .formats
                .iter()
                .copied()
                .find(wgpu::TextureFormat::is_srgb)
                .or_else(|| capabilities.formats.first().copied())
                .ok_or_else(|| RenderError("wgpu surface exposes no color formats".to_owned()))?;
            let requested_mode =
                std::env::var("RIBBON_GPU_PRESENT_MODE").unwrap_or_else(|_| "fifo".to_owned());
            let (present_mode, mode_fallback) =
                choose_present_mode(&requested_mode, &capabilities.present_modes);
            let (desired_maximum_frame_latency, latency_fallback) =
                desired_frame_latency_from_env();
            if mode_fallback {
                eprintln!(
                    "[GPU SURFACE] unsupported/unknown RIBBON_GPU_PRESENT_MODE={requested_mode:?}; falling back to fifo"
                );
            }
            if latency_fallback {
                eprintln!(
                    "[GPU SURFACE] invalid RIBBON_GPU_FRAME_LATENCY; expected 1, 2, or 3, using 1"
                );
            }
            let config =
                wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format,
                    width: window.inner_size().width.max(1),
                    height: window.inner_size().height.max(1),
                    present_mode,
                    desired_maximum_frame_latency,
                    alpha_mode: capabilities.alpha_modes.first().copied().ok_or_else(|| {
                        RenderError("wgpu surface exposes no alpha modes".to_owned())
                    })?,
                    view_formats: vec![],
                };
            eprintln!(
                "[GPU SURFACE] requested_mode={requested_mode:?}; selected_mode={}; desired_maximum_frame_latency={}; supported_modes={:?}",
                present_mode_label(config.present_mode),
                config.desired_maximum_frame_latency,
                capabilities.present_modes,
            );
            surface.configure(&device, &config);
            Some(config)
        } else {
            None
        };
        let target = create_target(&device, width, height);
        Ok(Self {
            _instance: instance,
            adapter,
            device,
            queue,
            surface,
            surface_config,
            target,
            readback_buffer: Mutex::new(None),
        })
    }

    /// Borrow the selected adapter.
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// Borrow the device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Borrow the queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Borrow the optional window surface.
    pub fn surface(&self) -> Option<&wgpu::Surface<'static>> {
        self.surface.as_ref()
    }

    /// Borrow the surface configuration, when a surface exists.
    pub fn surface_config(&self) -> Option<&wgpu::SurfaceConfiguration> {
        self.surface_config.as_ref()
    }

    /// Borrow the offscreen target.
    pub fn target(&self) -> &OffscreenTarget {
        &self.target
    }

    /// Estimated bytes held by the context's RGBA8 offscreen target.
    ///
    /// This estimates the texture image only and excludes backend/allocator overhead.
    pub fn target_bytes_estimate(&self) -> u64 {
        u64::from(self.target.width)
            .saturating_mul(u64::from(self.target.height))
            .saturating_mul(4)
    }

    /// Estimated bytes retained by the reusable synchronous framebuffer readback buffer.
    pub fn readback_buffer_bytes_estimate(&self) -> u64 {
        self.readback_buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|buffer| buffer.size_bytes)
            .unwrap_or(0)
    }

    /// Resize the offscreen target and surface, if present.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.target = create_target(&self.device, width, height);
        *self
            .readback_buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        if let (Some(surface), Some(config)) = (&self.surface, &mut self.surface_config) {
            config.width = width;
            config.height = height;
            surface.configure(&self.device, config);
        }
    }

    /// Read the offscreen RGBA target into a packed 24-bit RGB framebuffer.
    /// The staging buffer is cached between calls and resized only when necessary.
    pub fn readback(&self) -> Result<Framebuffer, RenderError> {
        let bytes_per_row = self.target.width.saturating_mul(4);
        let padded_bytes_per_row = bytes_per_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let required_bytes =
            u64::from(padded_bytes_per_row).saturating_mul(u64::from(self.target.height));
        let mut cached = self
            .readback_buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cached
            .as_ref()
            .is_none_or(|buffer| buffer.size_bytes < required_bytes)
        {
            *cached = Some(ReadbackBuffer {
                buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("offscreen framebuffer readback"),
                    size: required_bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                size_bytes: required_bytes,
            });
        }
        let readback = &cached
            .as_ref()
            .expect("readback buffer was allocated above")
            .buffer;
        let result = (|| -> Result<Framebuffer, RenderError> {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("offscreen target readback"),
                });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.target.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(self.target.height),
                    },
                },
                wgpu::Extent3d {
                    width: self.target.width,
                    height: self.target.height,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit(Some(encoder.finish()));
            let slice = readback.slice(..);
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            self.device
                .poll(wgpu::PollType::Wait)
                .map_err(|error| RenderError(format!("GPU readback wait failed: {error}")))?;
            receiver
                .recv()
                .map_err(|error| RenderError(format!("GPU readback callback failed: {error}")))?
                .map_err(|error| RenderError(format!("GPU framebuffer map failed: {error}")))?;
            let mut framebuffer =
                Framebuffer::new(self.target.width as usize, self.target.height as usize);
            {
                let mapped = slice.get_mapped_range();
                for (y, row) in mapped
                    .chunks_exact(padded_bytes_per_row as usize)
                    .take(self.target.height as usize)
                    .enumerate()
                {
                    for (x, rgba) in row
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .take(self.target.width as usize)
                        .enumerate()
                    {
                        framebuffer.pixels[y * self.target.width as usize + x] =
                            (u32::from(rgba[0]) << 16)
                                | (u32::from(rgba[1]) << 8)
                                | u32::from(rgba[2]);
                    }
                }
            }
            readback.unmap();
            Ok(framebuffer)
        })();
        if result.is_err() {
            // A failed map/poll can leave the buffer unusable. Drop it and recreate on demand.
            *cached = None;
        }
        result
    }
}

fn create_target(device: &wgpu::Device, width: u32, height: u32) -> OffscreenTarget {
    let width = width.max(1);
    let height = height.max(1);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen color target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    OffscreenTarget {
        texture,
        view,
        width,
        height,
    }
}
