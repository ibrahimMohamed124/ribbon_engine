//! Reusable transient GPU texture pooling and allocation accounting.

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
