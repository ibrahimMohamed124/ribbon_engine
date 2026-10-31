//! Explicit, renderer-owned residency requests for glTF model GPU resources.
//!
//! CPU decoding remains owned by `AssetServer`. Requests are serviced from the mesh pass, where
//! the owning `wgpu::Device` and `Queue` are available. A warmup request retains its uploaded
//! model-cache entry until released, even when the active scene has no placement for that model.

use ribbon_assets::Handle;
use ribbon_plugin_gltf::ModelAsset;

/// Renderer-owned GPU upload pacing policy. One oversized model is admitted when the frame has
/// not uploaded any model yet, avoiding starvation for assets larger than the configured byte
/// budget. The byte count is estimated from the CPU model and is not driver VRAM accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuUploadBudget {
    pub max_models_per_frame: usize,
    pub max_bytes_per_frame: Option<u64>,
}

impl Default for GpuUploadBudget {
    fn default() -> Self {
        Self {
            max_models_per_frame: 2,
            max_bytes_per_frame: Some(64 * 1024 * 1024),
        }
    }
}

impl GpuUploadBudget {
    pub fn normalized(self) -> Self {
        Self {
            max_models_per_frame: self.max_models_per_frame.max(1),
            max_bytes_per_frame: self.max_bytes_per_frame,
        }
    }

    /// Whether the next model may be submitted in this frame. A single oversized first model is
    /// admitted to avoid permanent starvation; later models must fit both configured limits.
    pub fn allows_upload(
        self,
        uploads_this_frame: usize,
        bytes_this_frame: u64,
        next_upload_bytes: u64,
    ) -> bool {
        let budget = self.normalized();
        if uploads_this_frame >= budget.max_models_per_frame {
            return false;
        }
        if uploads_this_frame == 0 {
            return true;
        }
        match budget.max_bytes_per_frame {
            Some(max_bytes) => bytes_this_frame.saturating_add(next_upload_bytes) <= max_bytes,
            None => true,
        }
    }
}

/// GPU preparation state for one requested model generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelWarmupStatus {
    /// The asynchronous AssetServer request has not committed decoded CPU model data yet.
    WaitingForCpu,
    /// CPU data is resident and the renderer is waiting for a budgeted upload or GPU validation.
    WaitingForGpu,
    /// Model geometry, material resources, and textures were uploaded and the validation scope
    /// completed without a wgpu validation error.
    Ready,
    /// CPU decoding, model validation, or GPU upload/resource creation failed. This is terminal
    /// until explicitly requested again; it is never silently treated as ready.
    Failed(String),
}

#[derive(Clone, Debug)]
struct Entry {
    handle: Handle<ModelAsset>,
    path: String,
    status: ModelWarmupStatus,
    priority: i32,
    sequence: u64,
}

/// Requests GPU model preparation without creating a visible scene placement.
///
/// Keep requests while a transition depends on the warmed resources. Call [`release`](Self::release)
/// after real scene placements have been spawned; the render cache then remains live through those
/// placements without a one-frame eviction gap.
#[derive(Default)]
pub struct ModelWarmup {
    entries: Vec<Entry>,
    next_sequence: u64,
}

impl ModelWarmup {
    /// Request warmup for `handle`, retaining the diagnostic `path` for actionable failures.
    /// Duplicate requests for the same ID/generation share one entry and never cause a second GPU
    /// upload. Calling `request` again retries GPU/validation failures on the same handle. A CPU
    /// loader failure must first be retried through `AssetServer` to obtain a fresh generation.
    pub fn request(&mut self, handle: Handle<ModelAsset>, path: impl Into<String>) {
        self.request_with_priority(handle, path, 0);
    }

    /// Request renderer warmup with an explicit scheduling priority. Higher values are serviced
    /// first by the renderer; equal priorities preserve request order. A later consumer can raise
    /// an existing shared request's priority without creating another upload entry.
    pub fn request_with_priority(
        &mut self,
        handle: Handle<ModelAsset>,
        path: impl Into<String>,
        priority: i32,
    ) {
        let key = model_key(handle);
        let path = path.into();
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| model_key(entry.handle) == key)
        {
            entry.path = path;
            entry.priority = entry.priority.max(priority);
            if matches!(&entry.status, ModelWarmupStatus::Failed(_)) {
                entry.status = ModelWarmupStatus::WaitingForCpu;
            }
            return;
        }
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.entries.push(Entry {
            handle,
            path,
            status: ModelWarmupStatus::WaitingForCpu,
            priority,
            sequence,
        });
    }

    /// Inspect the warmup state for an exact handle generation.
    pub fn status(&self, handle: Handle<ModelAsset>) -> Option<&ModelWarmupStatus> {
        let key = model_key(handle);
        self.entries
            .iter()
            .find(|entry| model_key(entry.handle) == key)
            .map(|entry| &entry.status)
    }

    /// Stop retaining a warmup-only cache entry. Any active scene placement still retains it.
    pub fn release(&mut self, handle: Handle<ModelAsset>) -> bool {
        let key = model_key(handle);
        let old_len = self.entries.len();
        self.entries.retain(|entry| model_key(entry.handle) != key);
        self.entries.len() != old_len
    }

    /// Number of current requests, including terminal failures awaiting gameplay acknowledgement.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn snapshots(&self) -> Vec<ModelWarmupSnapshot> {
        let mut snapshots: Vec<_> = self
            .entries
            .iter()
            .map(|entry| ModelWarmupSnapshot {
                handle: entry.handle,
                path: entry.path.clone(),
                status: entry.status.clone(),
                priority: entry.priority,
                sequence: entry.sequence,
            })
            .collect();
        snapshots.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.sequence.cmp(&right.sequence))
        });
        snapshots
    }

    pub(crate) fn set_status(
        &mut self,
        handle: Handle<ModelAsset>,
        status: ModelWarmupStatus,
    ) -> bool {
        let key = model_key(handle);
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| model_key(entry.handle) == key)
        else {
            return false;
        };
        if entry.status == status {
            return false;
        }
        entry.status = status;
        true
    }

    pub(crate) fn counts(&self) -> ModelWarmupCounts {
        let mut counts = ModelWarmupCounts {
            requested: self.entries.len(),
            ..ModelWarmupCounts::default()
        };
        for entry in &self.entries {
            match &entry.status {
                ModelWarmupStatus::WaitingForCpu => counts.waiting_for_cpu += 1,
                ModelWarmupStatus::WaitingForGpu => counts.waiting_for_gpu += 1,
                ModelWarmupStatus::Ready => counts.ready += 1,
                ModelWarmupStatus::Failed(_) => counts.failed += 1,
            }
        }
        counts
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ModelWarmupSnapshot {
    pub handle: Handle<ModelAsset>,
    pub path: String,
    pub status: ModelWarmupStatus,
    pub priority: i32,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ModelWarmupCounts {
    pub requested: usize,
    pub waiting_for_cpu: usize,
    pub waiting_for_gpu: usize,
    pub ready: usize,
    pub failed: usize,
}

pub(crate) fn model_key(handle: Handle<ModelAsset>) -> (u64, u32) {
    (handle.id(), handle.generation())
}

#[cfg(test)]
mod tests {
    use super::{GpuUploadBudget, ModelWarmup, ModelWarmupStatus};
    use ribbon_assets::{AssetServer, FsSource};
    use ribbon_plugin_gltf::{GltfLoader, ModelAsset};
    use std::path::Path;

    fn loaded_handle() -> (ModelAsset, ribbon_assets::Handle<ModelAsset>) {
        let mut server = AssetServer::default();
        server.register_loader(GltfLoader);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models");
        server.add_source(0, FsSource::new(path));
        let handle = server
            .load::<ModelAsset>("checker_cube.glb")
            .expect("packaged test model loads");
        (
            server.get(handle).expect("CPU model resident").clone(),
            handle,
        )
    }

    #[test]
    fn warmup_request_is_independent_of_visible_scene_placements_and_deduplicated() {
        let (_model, handle) = loaded_handle();
        let mut warmup = ModelWarmup::default();
        warmup.request(handle, "checker_cube.glb");
        warmup.request(handle, "checker_cube.glb");
        assert_eq!(warmup.len(), 1);
        assert_eq!(
            warmup.status(handle),
            Some(&ModelWarmupStatus::WaitingForCpu)
        );
        assert_eq!(warmup.counts().requested, 1);
        assert!(warmup.release(handle));
        assert!(warmup.is_empty());
        assert!(!warmup.release(handle));
    }

    #[test]
    fn gpu_warmups_are_priority_ordered_and_shared_requests_can_be_upgraded() {
        let mut server = AssetServer::default();
        server.register_loader(GltfLoader);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models");
        server.add_source(0, FsSource::new(path));
        let normal = server.load::<ModelAsset>("checker_cube.glb").unwrap();
        let urgent = server.load::<ModelAsset>("bare_triangle.glb").unwrap();

        let mut warmup = ModelWarmup::default();
        warmup.request_with_priority(normal, "checker_cube.glb", 10);
        warmup.request_with_priority(urgent, "bare_triangle.glb", 0);
        warmup.request_with_priority(urgent, "bare_triangle.glb", 50);

        assert_eq!(
            warmup.len(),
            2,
            "shared requests must not create duplicate GPU work"
        );
        let snapshots = warmup.snapshots();
        assert_eq!(snapshots[0].handle, urgent);
        assert_eq!(snapshots[0].priority, 50);
        assert_eq!(snapshots[1].handle, normal);
        assert_eq!(snapshots[1].priority, 10);

        let mut fifo = ModelWarmup::default();
        fifo.request_with_priority(normal, "checker_cube.glb", 5);
        fifo.request_with_priority(urgent, "bare_triangle.glb", 5);
        let fifo_snapshots = fifo.snapshots();
        assert_eq!(fifo_snapshots[0].handle, normal);
        assert_eq!(fifo_snapshots[1].handle, urgent);
        assert!(fifo_snapshots[0].sequence < fifo_snapshots[1].sequence);
    }

    #[test]
    fn failed_warmup_can_only_retry_after_an_explicit_request() {
        let (_model, handle) = loaded_handle();
        let mut warmup = ModelWarmup::default();
        warmup.request(handle, "checker_cube.glb");
        warmup.set_status(
            handle,
            ModelWarmupStatus::Failed("upload failed".to_owned()),
        );
        warmup.request(handle, "checker_cube.glb");
        assert_eq!(
            warmup.status(handle),
            Some(&ModelWarmupStatus::WaitingForCpu)
        );
    }
    #[test]
    fn upload_budget_normalizes_model_count_and_preserves_byte_policy() {
        let bounded = GpuUploadBudget {
            max_models_per_frame: 0,
            max_bytes_per_frame: Some(4096),
        }
        .normalized();
        assert_eq!(bounded.max_models_per_frame, 1);
        assert_eq!(bounded.max_bytes_per_frame, Some(4096));

        let unbounded = GpuUploadBudget {
            max_models_per_frame: 4,
            max_bytes_per_frame: None,
        }
        .normalized();
        assert_eq!(unbounded.max_models_per_frame, 4);
        assert_eq!(unbounded.max_bytes_per_frame, None);
    }

    #[test]
    fn upload_budget_enforces_count_and_bytes_without_starving_oversized_first_model() {
        let budget = GpuUploadBudget {
            max_models_per_frame: 2,
            max_bytes_per_frame: Some(10),
        };
        assert!(budget.allows_upload(0, 0, 100));
        assert!(budget.allows_upload(1, 4, 6));
        assert!(!budget.allows_upload(1, 5, 6));
        assert!(!budget.allows_upload(2, 0, 1));

        let unbounded = GpuUploadBudget {
            max_models_per_frame: 1,
            max_bytes_per_frame: None,
        };
        assert!(unbounded.allows_upload(0, 0, u64::MAX));
        assert!(!unbounded.allows_upload(1, 0, 0));
    }
}
