//! Generic, renderer-owned warmup requests for standalone GPU meshes and PBR materials.
//!
//! CPU decoding and asset lifetime remain owned by callers/AssetServer. The queue only accepts
//! owned, already-decoded upload payloads; all `wgpu` calls are made by the render thread. Requests
//! are deduplicated by a caller-provided immutable `(namespace, id, generation)` key and retained
//! by explicit leases. The queue intentionally does not pretend to upload shader/pipeline or
//! animation resources that the current renderer does not expose as independently managed assets.

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
            Self::PbrMaterial(_) => GpuResourceKind::PbrMaterial,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpuResourceKind {
    Mesh,
    Texture,
    Sampler,
    PbrMaterial,
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
}

pub(crate) struct GpuResourceJob {
    pub key: GpuResourceKey,
    pub payload: Arc<GpuResourceData>,
    pub estimated_bytes: u64,
}

impl GpuResourceWarmup {
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
            ..GpuResourceWarmupCounts::default()
        };
        for entry in self.entries.values() {
            match &entry.status {
                GpuResourceWarmupStatus::Queued => counts.queued += 1,
                GpuResourceWarmupStatus::Preparing => counts.preparing += 1,
                GpuResourceWarmupStatus::Ready(_) => counts.ready += 1,
                GpuResourceWarmupStatus::Failed(_) => counts.failed += 1,
            }
        }
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
            entry.payload = None;
            entry.handle = Some(handle);
            entry.status = GpuResourceWarmupStatus::Ready(handle);
            self.completed_uploads = self.completed_uploads.saturating_add(1);
        }
    }

    pub(crate) fn fail(&mut self, key: GpuResourceKey, message: String) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.status = GpuResourceWarmupStatus::Failed(message);
            self.failed_uploads = self.failed_uploads.saturating_add(1);
        }
    }

    /// Remove entries whose final lease has gone away. Returned handles must be released by the
    /// renderer thread; pending uploads/validation are retained until their completion is observed.
    pub(crate) fn collect_orphans(&mut self) -> Vec<GpuResourceHandle> {
        let orphaned: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                (entry.lease.strong_count() == 0
                    && !matches!(&entry.status, GpuResourceWarmupStatus::Preparing))
                .then_some(*key)
            })
            .collect();
        let mut handles = Vec::new();
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
}

#[cfg(test)]
mod tests {
    use super::{
        GpuResourceData, GpuResourceKey, GpuResourceWarmup, GpuResourceWarmupStatus,
        OwnedTextureData,
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
}
