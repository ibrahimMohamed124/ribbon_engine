//! Wgpu context with an offscreen color target and synchronous CPU readback.

mod transient;
use crate::frame::Framebuffer;
use crate::graph::RenderError;
use ribbon_platform::Window;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
pub use transient::{
    TransientTextureDescriptor, TransientTextureHandle, TransientTexturePool,
    TransientTexturePoolStats,
};

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
    device_lost: Arc<AtomicBool>,
    device_lost_message: Arc<Mutex<Option<String>>>,
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
        let adapter_info = adapter.get_info();
        let adapter_features = adapter.features();
        eprintln!(
            "[GPU ADAPTER] name={:?}; vendor=0x{:04X}; device=0x{:04X}; device_type={:?}; driver={:?}; driver_info={:?}; backend={:?}; power_preference=HighPerformance; surface_compatible={}; timestamp_queries_supported={}",
            adapter_info.name,
            adapter_info.vendor,
            adapter_info.device,
            adapter_info.device_type,
            adapter_info.driver,
            adapter_info.driver_info,
            adapter_info.backend,
            surface.is_some(),
            adapter_features.contains(wgpu::Features::TIMESTAMP_QUERY),
        );
        // RenderPassTimestampWrites requires TIMESTAMP_QUERY. Keep it optional so limited
        // adapters retain the regular rendering path without profiling.
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
        // Device loss is different from a lost/outdated surface: every pipeline, buffer, view,
        // and bind group created from this device must be rebuilt. Keep the callback limited to
        // a thread-safe signal; the render-owning thread performs the actual teardown/recovery.
        let device_lost = Arc::new(AtomicBool::new(false));
        let device_lost_message = Arc::new(Mutex::new(None));
        let lost_flag = Arc::clone(&device_lost);
        let lost_message = Arc::clone(&device_lost_message);
        device.set_device_lost_callback(move |reason, message| {
            *lost_message
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some(format!("{reason:?}: {message}"));
            lost_flag.store(true, Ordering::Release);
        });
        let surface_config = if let (Some(surface), Some(window)) = (&surface, &window) {
            let capabilities = surface.get_capabilities(&adapter);
            let format = capabilities
                .formats
                .iter()
                .copied()
                .find(wgpu::TextureFormat::is_srgb)
                .or_else(|| capabilities.formats.first().copied())
                .ok_or_else(|| RenderError("wgpu surface exposes no color formats".to_owned()))?;
            // Mailbox is the measured default for the current target setup. Keep the
            // override and a FIFO fallback so the choice remains portable and observable.
            let requested_mode =
                std::env::var("RIBBON_GPU_PRESENT_MODE").unwrap_or_else(|_| "mailbox".to_owned());
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
                "[GPU SURFACE] requested_mode={requested_mode:?}; selected_mode={}; desired_maximum_frame_latency={}; resolution={}x{}; supported_modes={:?}",
                present_mode_label(config.present_mode),
                config.desired_maximum_frame_latency,
                config.width,
                config.height,
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
            device_lost,
            device_lost_message,
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

    /// Whether wgpu has reported that this context's device was lost. The flag is set by the
    /// device-lost callback and consumed by the window presenter on the render-owning thread.
    pub fn is_device_lost(&self) -> bool {
        self.device_lost.load(Ordering::Acquire)
    }

    /// Human-readable device-loss reason captured by wgpu, when available.
    pub fn device_lost_message(&self) -> Option<String> {
        self.device_lost_message
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
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
