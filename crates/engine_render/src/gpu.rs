//! Wgpu context with an offscreen color target and synchronous CPU readback.

use crate::frame::Framebuffer;
use crate::graph::RenderError;
use engine_platform::Window;
use std::sync::Arc;

/// An offscreen RGBA8 render target owned by a [`GpuContext`].
pub struct OffscreenTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
}

impl OffscreenTarget {
    /// Borrow the target's texture view for render passes.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

/// Shared instance, adapter, device, queue, optional surface, and offscreen target.
pub struct GpuContext {
    _instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    target: OffscreenTarget,
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
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Voxelworks engine render device"),
                required_features: wgpu::Features::empty(),
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
            let config =
                wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format,
                    width: window.inner_size().width.max(1),
                    height: window.inner_size().height.max(1),
                    present_mode: wgpu::PresentMode::Fifo,
                    desired_maximum_frame_latency: 2,
                    alpha_mode: capabilities.alpha_modes.first().copied().ok_or_else(|| {
                        RenderError("wgpu surface exposes no alpha modes".to_owned())
                    })?,
                    view_formats: vec![],
                };
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

    /// Resize the offscreen target and surface, if present.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.target = create_target(&self.device, width, height);
        if let (Some(surface), Some(config)) = (&self.surface, &mut self.surface_config) {
            config.width = width;
            config.height = height;
            surface.configure(&self.device, config);
        }
    }

    /// Read the offscreen RGBA target into a packed 24-bit RGB framebuffer.
    pub fn readback(&self) -> Result<Framebuffer, RenderError> {
        let bytes_per_row = self.target.width.saturating_mul(4);
        let padded_bytes_per_row = bytes_per_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen framebuffer readback"),
            size: u64::from(padded_bytes_per_row) * u64::from(self.target.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
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
                buffer: &readback,
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
                        (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2]);
                }
            }
        }
        readback.unmap();
        Ok(framebuffer)
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
