//! Generic, renderer-owned warmup requests for standalone GPU meshes and PBR materials.
//!
//! CPU decoding and asset lifetime remain owned by callers/AssetServer. The queue only accepts
//! owned, already-decoded upload payloads; all `wgpu` calls are made by the render thread. Requests
//! are deduplicated by a caller-provided immutable `(namespace, id, generation)` key and retained
//! by explicit leases. Shader modules and render pipelines use the separate generation-aware
//! `shader_pipeline` registry; skeletal animation/skinning remains a separate renderer lifecycle
//! and is not claimed to be covered by this mesh/material queue.

use crate::mesh::Vertex;
use crate::renderer::{GpuSamplerDesc, MaterialId, MeshId, SamplerId, TextureId};
use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

/// Stable identity of one immutable GPU resource generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GpuResourceKey {
    /// Caller-defined asset/type namespace. Use a stable value for the resource class.
    pub namespace: u64,
    /// Caller-defined stable asset/resource identity.
    pub id: u64,
    /// Increment when CPU content changes (for example, after hot reload).
    pub generation: u32,
}

/// Owned decoded image data suitable for a renderer texture upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedTextureData {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

impl OwnedTextureData {
    fn estimated_bytes(&self) -> u64 {
        let base = u64::from(self.width)
            .saturating_mul(u64::from(self.height))
            .saturating_mul(4);
        // Renderer texture creation generates mip levels, whose total is bounded by roughly 4/3
        // of the base level. This is a pacing estimate, not driver-reported memory.
        base.saturating_add(base / 3)
    }

    fn validate(&self) -> Result<(), String> {
        if self.width == 0 || self.height == 0 {
            return Err("texture dimensions must be non-zero".to_owned());
        }
        let expected = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "texture dimensions overflow the RGBA8 byte count".to_owned())?;
        if expected != self.rgba8.len() as u64 {
            return Err(format!(
                "RGBA8 payload length mismatch: expected {expected} bytes, got {}",
                self.rgba8.len()
            ));
        }
        Ok(())
    }
}

/// Owned metallic-roughness material data, including all optional maps used by the current PBR
/// renderer. Images are copied into this payload by the producer, not by the render thread.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedPbrMaterialData {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub base_color_texture: Option<OwnedTextureData>,
    pub metallic_roughness_texture: Option<OwnedTextureData>,
    pub normal_texture: Option<OwnedTextureData>,
    pub occlusion_texture: Option<OwnedTextureData>,
    pub occlusion_strength: f32,
    pub emissive_texture: Option<OwnedTextureData>,
    pub emissive_factor: [f32; 3],
}

/// CPU payload accepted by the generic renderer warmup queue.
#[derive(Clone, Debug)]
pub enum GpuResourceData {
    Mesh {
        vertices: Vec<Vertex>,
        indices: Vec<u32>,
    },
    /// Standalone texture with an explicit color-space interpretation.
    Texture {
        image: OwnedTextureData,
        srgb: bool,
    },
    Sampler(GpuSamplerDesc),
    PbrMaterial(OwnedPbrMaterialData),
}

impl GpuResourceData {
    /// Estimated upload bytes used for per-frame admission. Texture estimates include a mip
    /// allowance; mesh data uses its exact vertex/index payload sizes.
    pub fn estimated_upload_bytes(&self) -> u64 {
        match self {
            Self::Mesh { vertices, indices } => (vertices.len() as u64)
                .saturating_mul(std::mem::size_of::<Vertex>() as u64)
                .saturating_add(
                    (indices.len() as u64).saturating_mul(std::mem::size_of::<u32>() as u64),
                ),
            Self::Texture { image, .. } => image.estimated_bytes(),
            Self::Sampler(_) => 64,
            Self::PbrMaterial(material) => {
                let textures = [
                    material.base_color_texture.as_ref(),
                    material.metallic_roughness_texture.as_ref(),
                    material.normal_texture.as_ref(),
                    material.occlusion_texture.as_ref(),
                    material.emissive_texture.as_ref(),
                ];
                textures
                    .into_iter()
                    .flatten()
                    .fold(256_u64, |bytes, texture| {
                        bytes.saturating_add(texture.estimated_bytes())
                    })
            }
        }
    }

    fn kind(&self) -> GpuResourceKind {
        match self {
            Self::Mesh { .. } => GpuResourceKind::Mesh,
            Self::Texture { .. } => GpuResourceKind::Texture,
            Self::Sampler(_) => GpuResourceKind::Sampler,
            Self::PbrMaterial(_) => GpuResourceKind::Material,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Mesh { vertices, indices } => {
                if vertices.is_empty() || indices.is_empty() {
                    return Err("mesh warmup requires at least one vertex and one index".to_owned());
                }
                if indices.len() % 3 != 0 {
                    return Err("mesh index count must describe a triangle list".to_owned());
                }
                if vertices.iter().any(|vertex| {
                    vertex
                        .position
                        .iter()
                        .chain(vertex.normal.iter())
                        .chain(vertex.uv.iter())
                        .any(|value| !value.is_finite())
                }) {
                    return Err("mesh contains non-finite vertex data".to_owned());
                }
                if let Some(index) = indices
                    .iter()
                    .copied()
                    .find(|index| *index as usize >= vertices.len())
                {
                    return Err(format!("mesh index {index} is outside the vertex array"));
                }
                Ok(())
            }
            Self::Texture { image, .. } => image.validate(),
            Self::Sampler(sampler) => sampler.validate(),
            Self::PbrMaterial(material) => {
                if material
                    .base_color
                    .iter()
                    .chain(material.emissive_factor.iter())
                    .any(|v| !v.is_finite())
                    || !material.metallic.is_finite()
                    || !material.roughness.is_finite()
                    || !material.occlusion_strength.is_finite()
                {
                    return Err("PBR material contains non-finite factors".to_owned());
                }
                for texture in [
                    material.base_color_texture.as_ref(),
                    material.metallic_roughness_texture.as_ref(),
                    material.normal_texture.as_ref(),
                    material.occlusion_texture.as_ref(),
                    material.emissive_texture.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    texture.validate()?;
                }
                Ok(())
            }
        }
    }
}

/// Resource class used by GPU residency accounting and class-specific budgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GpuResourceClass {
    Mesh,
    Texture,
    Sampler,
    Material,
}

type GpuResourceKind = GpuResourceClass;

/// Admission limits for the generic GPU residency manager. All byte values are estimates from
/// owned upload payloads, not driver-reported VRAM. `None` disables a particular limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuResidencyBudget {
    pub total_bytes: Option<u64>,
    pub mesh_bytes: Option<u64>,
    pub texture_bytes: Option<u64>,
    pub material_bytes: Option<u64>,
    pub sampler_count: Option<usize>,
}

impl Default for GpuResidencyBudget {
    fn default() -> Self {
        Self {
            total_bytes: Some(512 * 1024 * 1024),
            mesh_bytes: None,
            texture_bytes: Some(384 * 1024 * 1024),
            material_bytes: Some(384 * 1024 * 1024),
            sampler_count: Some(4096),
        }
    }
}

/// Generation-checked renderer handle returned after GPU validation succeeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuResourceHandle {
    Mesh(MeshId),
    Texture(TextureId),
    Sampler(SamplerId),
    Material(MaterialId),
}

/// Observable status of one generic GPU warmup request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuResourceWarmupStatus {
    Queued,
    Preparing,
    Ready(GpuResourceHandle),
    Failed(String),
}

/// Error returned when a generic resource request cannot be accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuResourceRequestError {
    InvalidPayload(String),
    KeyKindConflict,
    BudgetExceeded(String),
}

impl std::fmt::Display for GpuResourceRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPayload(message) => {
                write!(formatter, "invalid GPU resource payload: {message}")
            }
            Self::KeyKindConflict => write!(
                formatter,
                "GPU resource key is already used by another resource kind"
            ),
            Self::BudgetExceeded(message) => write!(
                formatter,
                "GPU resource residency budget exceeded: {message}"
            ),
        }
    }
}

impl std::error::Error for GpuResourceRequestError {}

/// Strong residency claim for a queued or prepared standalone GPU resource. Cloning a lease adds
/// another consumer; the renderer releases its cache entry only after the final lease is dropped.
#[derive(Clone, Debug)]
pub struct GpuResourceLease {
    key: GpuResourceKey,
    _token: Arc<()>,
}

impl GpuResourceLease {
    pub fn key(&self) -> GpuResourceKey {
        self.key
    }
}

struct Entry {
    kind: GpuResourceKind,
    payload: Option<Arc<GpuResourceData>>,
    lease: Weak<()>,
    status: GpuResourceWarmupStatus,
    priority: i32,
    sequence: u64,
    waited_frames: u32,
    estimated_bytes: u64,
    handle: Option<GpuResourceHandle>,
}

/// Thread-safe-by-schedule request registry. Mutation occurs during the app/render schedule; it
/// contains no `wgpu` objects and can be inspected by diagnostics without taking GPU ownership.
#[derive(Default)]
pub struct GpuResourceWarmup {
    entries: BTreeMap<GpuResourceKey, Entry>,
    next_sequence: u64,
    completed_uploads: u64,
    failed_uploads: u64,
    released_resources: u64,
    budget: GpuResidencyBudget,
    pending_releases: Vec<GpuResourceHandle>,
    memory_pressure_events: u64,
    evicted_resources: u64,
    budget_rejections: u64,
    last_evicted_class: Option<GpuResourceClass>,
}

pub(crate) struct GpuResourceJob {
    pub key: GpuResourceKey,
    pub payload: Arc<GpuResourceData>,
    pub estimated_bytes: u64,
}

impl GpuResourceWarmup {
    /// Replace admission limits. Existing leases are never invalidated; newly queued work must
    /// satisfy the new limits, and eligible orphaned entries are evicted on demand.
    pub fn set_budget(&mut self, budget: GpuResidencyBudget) {
        self.budget = budget;
    }

    pub fn budget(&self) -> GpuResidencyBudget {
        self.budget
    }

    /// Queue or share a request. A key identifies immutable content: increment `generation` when
    /// the payload changes. Repeated callers share the same upload and residency lease family.
    pub fn request(
        &mut self,
        key: GpuResourceKey,
        payload: GpuResourceData,
        priority: i32,
    ) -> Result<GpuResourceLease, GpuResourceRequestError> {
        payload
            .validate()
            .map_err(GpuResourceRequestError::InvalidPayload)?;
        let kind = payload.kind();
        let estimated_bytes = payload.estimated_upload_bytes();
        if let Some(entry) = self.entries.get_mut(&key) {
            if entry.kind != kind {
                return Err(GpuResourceRequestError::KeyKindConflict);
            }
            entry.priority = entry.priority.max(priority);
            if let Some(existing) = entry.lease.upgrade() {
                if matches!(&entry.status, GpuResourceWarmupStatus::Failed(_)) {
                    entry.status = GpuResourceWarmupStatus::Queued;
                    if entry.payload.is_none() {
                        entry.payload = Some(Arc::new(payload));
                    }
                }
                return Ok(GpuResourceLease {
                    key,
                    _token: existing,
                });
            }
            // A new consumer arrived before the render thread collected the expired lease. Keep
            // an already-resident handle and re-pin it instead of uploading duplicate resources.
            let token = Arc::new(());
            entry.lease = Arc::downgrade(&token);
            if matches!(&entry.status, GpuResourceWarmupStatus::Failed(_)) {
                entry.status = GpuResourceWarmupStatus::Queued;
                if entry.payload.is_none() {
                    entry.payload = Some(Arc::new(payload));
                }
            }
            return Ok(GpuResourceLease { key, _token: token });
        }

        if let Err(error) = self.make_room(kind, estimated_bytes) {
            self.budget_rejections = self.budget_rejections.saturating_add(1);
            return Err(error);
        }

        let token = Arc::new(());
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.entries.insert(
            key,
            Entry {
                kind,
                payload: Some(Arc::new(payload)),
                lease: Arc::downgrade(&token),
                status: GpuResourceWarmupStatus::Queued,
                priority,
                sequence,
                waited_frames: 0,
                estimated_bytes,
                handle: None,
            },
        );
        Ok(GpuResourceLease { key, _token: token })
    }

    /// Query the status associated with an owned lease.
    pub fn status(&self, lease: &GpuResourceLease) -> Option<GpuResourceWarmupStatus> {
        self.entries
            .get(&lease.key)
            .map(|entry| entry.status.clone())
    }

    /// Raise a queued request's priority without creating another upload.
    pub fn promote(&mut self, lease: &GpuResourceLease, priority: i32) -> bool {
        let Some(entry) = self.entries.get_mut(&lease.key) else {
            return false;
        };
        entry.priority = entry.priority.max(priority);
        true
    }

    /// Current queue/residency counters for renderer diagnostics.
    pub fn counts(&self) -> GpuResourceWarmupCounts {
        let mut counts = GpuResourceWarmupCounts {
            completed_uploads: self.completed_uploads,
            failed_uploads: self.failed_uploads,
            released_resources: self.released_resources,
            memory_pressure_events: self.memory_pressure_events,
            evicted_resources: self.evicted_resources,
            budget_rejections: self.budget_rejections,
            last_evicted_class: self.last_evicted_class,
            ..GpuResourceWarmupCounts::default()
        };
        for entry in self.entries.values() {
            counts.recovery_payload_bytes_estimate =
                counts.recovery_payload_bytes_estimate.saturating_add(
                    entry
                        .payload
                        .as_ref()
                        .map_or(0, |payload| payload.estimated_upload_bytes()),
                );
            match &entry.status {
                GpuResourceWarmupStatus::Queued => {
                    counts.queued += 1;
                    counts.queued_bytes_estimate = counts
                        .queued_bytes_estimate
                        .saturating_add(entry.estimated_bytes);
                }
                GpuResourceWarmupStatus::Preparing => {
                    counts.preparing += 1;
                    counts.queued_bytes_estimate = counts
                        .queued_bytes_estimate
                        .saturating_add(entry.estimated_bytes);
                }
                GpuResourceWarmupStatus::Ready(_) => {
                    counts.ready += 1;
                    counts.resident_bytes_estimate = counts
                        .resident_bytes_estimate
                        .saturating_add(entry.estimated_bytes);
                }
                GpuResourceWarmupStatus::Failed(_) => counts.failed += 1,
            }
        }
        let (total, meshes, textures, materials, samplers) = self.usage();
        counts.budget_overage_bytes = total
            .saturating_sub(self.budget.total_bytes.unwrap_or(u64::MAX))
            .max(meshes.saturating_sub(self.budget.mesh_bytes.unwrap_or(u64::MAX)))
            .max(textures.saturating_sub(self.budget.texture_bytes.unwrap_or(u64::MAX)))
            .max(materials.saturating_sub(self.budget.material_bytes.unwrap_or(u64::MAX)));
        counts.sampler_count_over_budget =
            samplers.saturating_sub(self.budget.sampler_count.unwrap_or(usize::MAX));
        counts
    }

    pub(crate) fn next_upload_bytes(&self) -> Option<u64> {
        self.entries
            .values()
            .filter(|entry| {
                matches!(&entry.status, GpuResourceWarmupStatus::Queued)
                    && entry.lease.strong_count() > 0
            })
            .max_by_key(|entry| {
                (
                    entry
                        .priority
                        .saturating_add((entry.waited_frames / 30) as i32),
                    std::cmp::Reverse(entry.sequence),
                )
            })
            .and_then(|entry| {
                entry
                    .payload
                    .as_ref()
                    .map(|payload| payload.estimated_upload_bytes())
            })
    }

    pub(crate) fn take_next_job(&mut self) -> Option<GpuResourceJob> {
        let key = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                matches!(&entry.status, GpuResourceWarmupStatus::Queued)
                    && entry.lease.strong_count() > 0
            })
            .max_by_key(|(_, entry)| {
                (
                    entry
                        .priority
                        .saturating_add((entry.waited_frames / 30) as i32),
                    std::cmp::Reverse(entry.sequence),
                )
            })
            .map(|(key, _)| *key)?;
        for (other_key, entry) in &mut self.entries {
            if *other_key != key && matches!(&entry.status, GpuResourceWarmupStatus::Queued) {
                entry.waited_frames = entry.waited_frames.saturating_add(1);
            }
        }
        let entry = self.entries.get_mut(&key)?;
        let payload = entry.payload.clone()?;
        entry.status = GpuResourceWarmupStatus::Preparing;
        Some(GpuResourceJob {
            key,
            estimated_bytes: payload.estimated_upload_bytes(),
            payload,
        })
    }

    pub(crate) fn complete(&mut self, key: GpuResourceKey, handle: GpuResourceHandle) {
        if let Some(entry) = self.entries.get_mut(&key) {
            // Keep the CPU descriptor for the lifetime of the strong lease. It is required to
            // recreate the GPU object after a device-loss event; its bytes are reported separately
            // from GPU residency and remain bounded by the queue's same admission accounting.
            entry.handle = Some(handle);
            entry.status = GpuResourceWarmupStatus::Ready(handle);
            self.completed_uploads = self.completed_uploads.saturating_add(1);
        }
    }

    /// Drop stale device handles and queue every leased in-flight/ready payload for upload on
    /// the replacement device. This method owns no wgpu objects and must be called by the render
    /// thread after the renderer's old-device caches have been dropped.
    pub(crate) fn reset_after_device_loss(&mut self) {
        self.pending_releases.clear();
        self.entries
            .retain(|_, entry| entry.lease.strong_count() > 0);
        for entry in self.entries.values_mut() {
            entry.handle = None;
            entry.waited_frames = 0;
            if matches!(
                &entry.status,
                GpuResourceWarmupStatus::Ready(_) | GpuResourceWarmupStatus::Preparing
            ) {
                entry.status = GpuResourceWarmupStatus::Queued;
            }
        }
    }

    pub(crate) fn fail(&mut self, key: GpuResourceKey, message: String) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.status = GpuResourceWarmupStatus::Failed(message);
            self.failed_uploads = self.failed_uploads.saturating_add(1);
        }
    }

    fn usage(&self) -> (u64, u64, u64, u64, usize) {
        let mut total = 0_u64;
        let mut meshes = 0_u64;
        let mut textures = 0_u64;
        let mut materials = 0_u64;
        let mut samplers = 0_usize;
        for entry in self.entries.values() {
            if matches!(&entry.status, GpuResourceWarmupStatus::Failed(_)) {
                continue;
            }
            total = total.saturating_add(entry.estimated_bytes);
            match entry.kind {
                GpuResourceClass::Mesh => meshes = meshes.saturating_add(entry.estimated_bytes),
                GpuResourceClass::Texture => {
                    textures = textures.saturating_add(entry.estimated_bytes)
                }
                GpuResourceClass::Material => {
                    materials = materials.saturating_add(entry.estimated_bytes)
                }
                GpuResourceClass::Sampler => samplers = samplers.saturating_add(1),
            }
        }
        (total, meshes, textures, materials, samplers)
    }

    fn make_room(
        &mut self,
        requested_class: GpuResourceClass,
        requested_bytes: u64,
    ) -> Result<(), GpuResourceRequestError> {
        let (mut total, mut meshes, mut textures, mut materials, mut samplers) = self.usage();
        let fits = |total: u64,
                    meshes: u64,
                    textures: u64,
                    materials: u64,
                    samplers: usize,
                    budget: GpuResidencyBudget| {
            budget.total_bytes.is_none_or(|limit| total <= limit)
                && budget.mesh_bytes.is_none_or(|limit| meshes <= limit)
                && budget.texture_bytes.is_none_or(|limit| textures <= limit)
                && budget.material_bytes.is_none_or(|limit| materials <= limit)
                && budget.sampler_count.is_none_or(|limit| samplers <= limit)
        };
        let add_requested = |class: GpuResourceClass,
                             bytes: u64,
                             total: &mut u64,
                             meshes: &mut u64,
                             textures: &mut u64,
                             materials: &mut u64,
                             samplers: &mut usize| {
            *total = (*total).saturating_add(bytes);
            match class {
                GpuResourceClass::Mesh => *meshes = (*meshes).saturating_add(bytes),
                GpuResourceClass::Texture => *textures = (*textures).saturating_add(bytes),
                GpuResourceClass::Material => *materials = (*materials).saturating_add(bytes),
                GpuResourceClass::Sampler => *samplers = (*samplers).saturating_add(1),
            }
        };
        let mut projected = (total, meshes, textures, materials, samplers);
        add_requested(
            requested_class,
            requested_bytes,
            &mut projected.0,
            &mut projected.1,
            &mut projected.2,
            &mut projected.3,
            &mut projected.4,
        );
        if fits(
            projected.0,
            projected.1,
            projected.2,
            projected.3,
            projected.4,
            self.budget,
        ) {
            return Ok(());
        }
        self.memory_pressure_events = self.memory_pressure_events.saturating_add(1);

        // Plan eviction without mutating the registry first. Only expired leases are candidates;
        // Preparing entries are excluded because their async validation still owns a GPU handle.
        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                (entry.lease.strong_count() == 0
                    && !matches!(&entry.status, GpuResourceWarmupStatus::Preparing)
                    && !matches!(&entry.status, GpuResourceWarmupStatus::Failed(_)))
                .then_some((
                    *key,
                    entry.priority,
                    entry.sequence,
                    entry.kind,
                    entry.estimated_bytes,
                ))
            })
            .collect();
        candidates.sort_by_key(|(_, priority, sequence, _, _)| (*priority, *sequence));
        let mut selected = Vec::new();
        for (key, _, _, class, bytes) in candidates {
            selected.push(key);
            total = total.saturating_sub(bytes);
            match class {
                GpuResourceClass::Mesh => meshes = meshes.saturating_sub(bytes),
                GpuResourceClass::Texture => textures = textures.saturating_sub(bytes),
                GpuResourceClass::Material => materials = materials.saturating_sub(bytes),
                GpuResourceClass::Sampler => samplers = samplers.saturating_sub(1),
            }
            let mut next = (total, meshes, textures, materials, samplers);
            add_requested(
                requested_class,
                requested_bytes,
                &mut next.0,
                &mut next.1,
                &mut next.2,
                &mut next.3,
                &mut next.4,
            );
            if fits(next.0, next.1, next.2, next.3, next.4, self.budget) {
                break;
            }
        }
        let mut final_usage = (total, meshes, textures, materials, samplers);
        add_requested(
            requested_class,
            requested_bytes,
            &mut final_usage.0,
            &mut final_usage.1,
            &mut final_usage.2,
            &mut final_usage.3,
            &mut final_usage.4,
        );
        if !fits(
            final_usage.0,
            final_usage.1,
            final_usage.2,
            final_usage.3,
            final_usage.4,
            self.budget,
        ) {
            return Err(GpuResourceRequestError::BudgetExceeded(format!(
                "GPU residency budget cannot admit {requested_class:?} ({requested_bytes} estimated bytes); live leases and in-flight uploads are not evictable"
            )));
        }

        for key in selected {
            if let Some(entry) = self.entries.remove(&key) {
                if let Some(handle) = entry.handle {
                    self.pending_releases.push(handle);
                }
                self.last_evicted_class = Some(entry.kind);
                self.evicted_resources = self.evicted_resources.saturating_add(1);
            }
        }
        Ok(())
    }

    /// Remove entries whose final lease has gone away. Returned handles must be released by the
    /// renderer thread; pending uploads/validation are retained until their completion is observed.
    pub(crate) fn collect_orphans(&mut self) -> Vec<GpuResourceHandle> {
        let mut handles = std::mem::take(&mut self.pending_releases);
        self.released_resources = self.released_resources.saturating_add(handles.len() as u64);
        let orphaned: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                (entry.lease.strong_count() == 0
                    && !matches!(&entry.status, GpuResourceWarmupStatus::Preparing))
                .then_some(*key)
            })
            .collect();
        for key in orphaned {
            if let Some(entry) = self.entries.remove(&key) {
                if let Some(handle) = entry.handle {
                    handles.push(handle);
                    self.released_resources = self.released_resources.saturating_add(1);
                }
            }
        }
        handles
    }
}

/// Snapshot counters for the generic GPU resource queue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuResourceWarmupCounts {
    pub queued: usize,
    pub preparing: usize,
    pub ready: usize,
    pub failed: usize,
    pub completed_uploads: u64,
    pub failed_uploads: u64,
    pub released_resources: u64,
    pub resident_bytes_estimate: u64,
    /// CPU payload bytes retained so an active lease can rebuild its resource after device loss.
    pub recovery_payload_bytes_estimate: u64,
    pub queued_bytes_estimate: u64,
    pub budget_overage_bytes: u64,
    pub sampler_count_over_budget: usize,
    pub memory_pressure_events: u64,
    pub evicted_resources: u64,
    pub budget_rejections: u64,
    pub last_evicted_class: Option<GpuResourceClass>,
}

#[cfg(test)]
mod tests {
    use super::{
        GpuResidencyBudget, GpuResourceData, GpuResourceKey, GpuResourceWarmup,
        GpuResourceWarmupStatus, OwnedPbrMaterialData, OwnedTextureData,
    };
    use crate::mesh::Vertex;
    use crate::renderer::GpuSamplerDesc;

    fn key(id: u64) -> GpuResourceKey {
        GpuResourceKey {
            namespace: 1,
            id,
            generation: 0,
        }
    }

    fn triangle() -> GpuResourceData {
        GpuResourceData::Mesh {
            vertices: vec![
                Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [1.0, 0.0],
                },
                Vertex {
                    position: [0.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn duplicate_requests_share_one_queued_upload_and_promote_priority() {
        let mut queue = GpuResourceWarmup::default();
        let first = queue.request(key(1), triangle(), 1).unwrap();
        let second = queue.request(key(1), triangle(), 20).unwrap();
        assert_eq!(queue.counts().queued, 1);
        assert!(queue.promote(&first, 30));
        assert_eq!(queue.status(&second), Some(GpuResourceWarmupStatus::Queued));
    }

    #[test]
    fn expired_lease_does_not_evict_preparing_upload_before_validation() {
        let mut queue = GpuResourceWarmup::default();
        let lease = queue.request(key(1), triangle(), 1).unwrap();
        assert!(queue.take_next_job().is_some());
        drop(lease);
        assert!(queue.collect_orphans().is_empty());
        queue.fail(key(1), "validation failed".into());
        assert!(queue.collect_orphans().is_empty());
        assert_eq!(queue.counts().released_resources, 0);
    }

    #[test]
    fn device_loss_requeues_leased_gpu_resources_from_retained_payloads() {
        let mut queue = GpuResourceWarmup::default();
        let lease = queue.request(key(9), triangle(), 4).unwrap();
        let job = queue
            .take_next_job()
            .expect("queued payload is schedulable");
        // A preparing upload is just as invalidated by device loss as an already-ready handle.
        assert_eq!(job.key, lease.key());
        assert_eq!(
            queue.status(&lease),
            Some(GpuResourceWarmupStatus::Preparing)
        );
        assert!(queue.counts().recovery_payload_bytes_estimate > 0);

        queue.reset_after_device_loss();
        assert_eq!(queue.status(&lease), Some(GpuResourceWarmupStatus::Queued));
        assert!(
            queue.take_next_job().is_some(),
            "retained payload must be uploaded again"
        );
    }

    #[test]
    fn invalid_mesh_payload_is_rejected_before_render_thread_upload() {
        let mut queue = GpuResourceWarmup::default();
        let result = queue.request(
            key(1),
            GpuResourceData::Mesh {
                vertices: vec![Vertex {
                    position: [0.0; 3],
                    normal: [0.0; 3],
                    uv: [0.0; 2],
                }],
                indices: vec![0, 1, 2],
            },
            0,
        );
        assert!(result.is_err());
        assert_eq!(queue.counts().queued, 0);
    }

    #[test]
    fn standalone_texture_payload_is_validated_and_deduplicated() {
        let mut queue = GpuResourceWarmup::default();
        let payload = || GpuResourceData::Texture {
            image: OwnedTextureData {
                width: 2,
                height: 1,
                rgba8: vec![255; 8],
            },
            srgb: true,
        };
        let first = queue.request(key(2), payload(), 1).unwrap();
        let second = queue.request(key(2), payload(), 9).unwrap();
        assert_eq!(queue.counts().queued, 1);
        assert_eq!(queue.status(&second), Some(GpuResourceWarmupStatus::Queued));
        assert!(queue.promote(&first, 10));

        let invalid = GpuResourceData::Texture {
            image: OwnedTextureData {
                width: 2,
                height: 2,
                rgba8: vec![0; 4],
            },
            srgb: false,
        };
        assert!(queue.request(key(3), invalid, 0).is_err());
        assert_eq!(queue.counts().queued, 1);
    }

    #[test]
    fn standalone_sampler_is_validated_and_deduplicated() {
        let mut queue = GpuResourceWarmup::default();
        let first = queue
            .request(
                key(4),
                GpuResourceData::Sampler(GpuSamplerDesc::default()),
                0,
            )
            .unwrap();
        let second = queue
            .request(
                key(4),
                GpuResourceData::Sampler(GpuSamplerDesc::default()),
                5,
            )
            .unwrap();
        assert_eq!(queue.counts().queued, 1);
        assert_eq!(queue.status(&first), queue.status(&second));

        let invalid = GpuSamplerDesc {
            lod_min_clamp: 4.0,
            lod_max_clamp: 1.0,
            ..GpuSamplerDesc::default()
        };
        assert!(queue
            .request(key(5), GpuResourceData::Sampler(invalid), 0)
            .is_err());
        assert_eq!(queue.counts().queued, 1);
    }

    fn tiny_texture() -> GpuResourceData {
        GpuResourceData::Texture {
            image: OwnedTextureData {
                width: 1,
                height: 1,
                rgba8: vec![255; 4],
            },
            srgb: true,
        }
    }

    #[test]
    fn gpu_budget_rejects_overcommit_without_invalidating_a_live_lease() {
        let mut queue = GpuResourceWarmup::default();
        queue.set_budget(GpuResidencyBudget {
            total_bytes: None,
            mesh_bytes: None,
            texture_bytes: Some(5),
            material_bytes: None,
            sampler_count: None,
        });
        let retained = queue.request(key(10), tiny_texture(), 100).unwrap();
        let rejected = queue.request(key(11), tiny_texture(), 0);
        assert!(matches!(
            rejected,
            Err(super::GpuResourceRequestError::BudgetExceeded(_))
        ));
        assert_eq!(
            queue.status(&retained),
            Some(GpuResourceWarmupStatus::Queued)
        );
        assert_eq!(queue.counts().budget_rejections, 1);
        assert_eq!(queue.counts().memory_pressure_events, 1);
        assert_eq!(queue.counts().queued, 1);
    }

    #[test]
    fn mesh_and_material_class_budgets_are_enforced_independently() {
        let mut mesh_queue = GpuResourceWarmup::default();
        mesh_queue.set_budget(GpuResidencyBudget {
            total_bytes: None,
            mesh_bytes: Some(1),
            texture_bytes: None,
            material_bytes: None,
            sampler_count: None,
        });
        assert!(matches!(
            mesh_queue.request(key(40), triangle(), 0),
            Err(super::GpuResourceRequestError::BudgetExceeded(_))
        ));

        let mut material_queue = GpuResourceWarmup::default();
        material_queue.set_budget(GpuResidencyBudget {
            total_bytes: None,
            mesh_bytes: None,
            texture_bytes: None,
            material_bytes: Some(128),
            sampler_count: None,
        });
        let material = OwnedPbrMaterialData {
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 1.0,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive_texture: None,
            emissive_factor: [0.0; 3],
        };
        assert!(matches!(
            material_queue.request(key(41), GpuResourceData::PbrMaterial(material), 0),
            Err(super::GpuResourceRequestError::BudgetExceeded(_))
        ));
    }

    #[test]
    fn gpu_budget_evicts_only_expired_queued_leases_by_priority() {
        let mut queue = GpuResourceWarmup::default();
        queue.set_budget(GpuResidencyBudget {
            total_bytes: Some(5),
            mesh_bytes: None,
            texture_bytes: Some(5),
            material_bytes: None,
            sampler_count: None,
        });
        let expired = queue.request(key(20), tiny_texture(), 1).unwrap();
        drop(expired);
        let retained = queue.request(key(21), tiny_texture(), 100).unwrap();
        assert_eq!(queue.counts().queued, 1);
        assert_eq!(
            queue.status(&retained),
            Some(GpuResourceWarmupStatus::Queued)
        );
        let counts = queue.counts();
        assert_eq!(counts.evicted_resources, 1);
        assert_eq!(counts.memory_pressure_events, 1);
        assert_eq!(counts.budget_rejections, 0);
    }

    #[test]
    fn sampler_count_budget_is_enforced_independently_of_byte_budget() {
        let mut queue = GpuResourceWarmup::default();
        queue.set_budget(GpuResidencyBudget {
            total_bytes: None,
            mesh_bytes: None,
            texture_bytes: None,
            material_bytes: None,
            sampler_count: Some(1),
        });
        let first = queue
            .request(
                key(30),
                GpuResourceData::Sampler(GpuSamplerDesc::default()),
                10,
            )
            .unwrap();
        assert!(matches!(
            queue.request(
                key(31),
                GpuResourceData::Sampler(GpuSamplerDesc::default()),
                0
            ),
            Err(super::GpuResourceRequestError::BudgetExceeded(_))
        ));
        assert_eq!(queue.status(&first), Some(GpuResourceWarmupStatus::Queued));
        assert_eq!(queue.counts().sampler_count_over_budget, 0);
    }
}
