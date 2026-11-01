//! Explicit, renderer-owned residency requests for glTF model GPU resources.
//!
//! CPU decoding remains owned by `AssetServer`. Requests are serviced from the mesh pass, where
//! the owning `wgpu::Device` and `Queue` are available. A warmup request retains its uploaded
//! model-cache entry until released, even when the active scene has no placement for that model.

use ribbon_assets::Handle;
use ribbon_plugin_gltf::ModelAsset;

/// Renderer-owned GPU upload pacing policy. `max_models_per_frame` retains its compatibility
/// name, but is an operation cap shared by glTF model uploads and generic mesh/material uploads.
/// One oversized first operation is admitted to avoid starvation for assets larger than the byte
/// budget. Byte counts are CPU-payload estimates, not driver VRAM accounting.
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

    /// Whether the next GPU upload operation may be submitted in this frame. A single oversized
    /// first operation is admitted to avoid permanent starvation; later uploads must fit both
    /// configured limits.
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
/// Soft residency budget for the renderer's glTF model cache. The limit never evicts live scene
/// placements or required warmups; it produces pressure levels and over-budget diagnostics while
/// the existing liveness pass evicts only models no longer referenced by either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelGpuResidencyBudget {
    /// Estimated model-cache bytes before a pressure level is reported. `None` disables reporting.
    pub max_bytes: Option<u64>,
    /// Percentage of the limit at which warning pressure begins.
    pub warning_percent: u8,
    /// Percentage of the limit at which high pressure begins.
    pub high_percent: u8,
    /// Percentage of the limit at which critical pressure begins.
    pub critical_percent: u8,
}

impl Default for ModelGpuResidencyBudget {
    fn default() -> Self {
        Self {
            max_bytes: Some(1024 * 1024 * 1024),
            warning_percent: 75,
            high_percent: 90,
            critical_percent: 100,
        }
    }
}

impl ModelGpuResidencyBudget {
    pub fn normalized(self) -> Self {
        let warning_percent = self.warning_percent.clamp(1, 100);
        let high_percent = self.high_percent.clamp(warning_percent, 100);
        let critical_percent = self.critical_percent.clamp(high_percent, 100);
        Self {
            max_bytes: self.max_bytes,
            warning_percent,
            high_percent,
            critical_percent,
        }
    }

    /// Return pressure level: 0 normal, 1 warning, 2 high, 3 critical.
    pub fn pressure_level(self, resident_bytes: u64) -> u8 {
        let Some(limit) = self.max_bytes else {
            return 0;
        };
        if limit == 0 {
            return u8::from(resident_bytes > 0) * 3;
        }
        let budget = self.normalized();
        let usage_percent = (u128::from(resident_bytes) * 100 / u128::from(limit)).min(100) as u8;
        if usage_percent >= budget.critical_percent {
            3
        } else if usage_percent >= budget.high_percent {
            2
        } else if usage_percent >= budget.warning_percent {
            1
        } else {
            0
        }
    }
}

/// Global admission budget for renderer-owned GPU allocations whose sizes can be estimated.
///
/// This is a hard admission limit for tracked estimates, not a claim about physical VRAM: driver,
/// swapchain, and opaque backend allocations may not be visible to the renderer. Existing live
/// resources are never destroyed to satisfy this limit; new uploads wait or are rejected until
/// enough tracked capacity is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalGpuResidencyBudget {
    /// Maximum total tracked GPU bytes. `None` disables the global admission limit.
    pub max_tracked_bytes: Option<u64>,
    /// Percentage of the limit at which warning pressure begins.
    pub warning_percent: u8,
    /// Percentage of the limit at which high pressure begins.
    pub high_percent: u8,
    /// Percentage of the limit at which critical pressure begins.
    pub critical_percent: u8,
}

impl Default for GlobalGpuResidencyBudget {
    fn default() -> Self {
        Self {
            max_tracked_bytes: Some(2 * 1024 * 1024 * 1024),
            warning_percent: 75,
            high_percent: 90,
            critical_percent: 100,
        }
    }
}

impl GlobalGpuResidencyBudget {
    /// Normalize pressure thresholds while preserving the configured byte limit.
    pub fn normalized(self) -> Self {
        let warning_percent = self.warning_percent.clamp(1, 100);
        let high_percent = self.high_percent.clamp(warning_percent, 100);
        let critical_percent = self.critical_percent.clamp(high_percent, 100);
        Self {
            max_tracked_bytes: self.max_tracked_bytes,
            warning_percent,
            high_percent,
            critical_percent,
        }
    }

    /// Return pressure level: 0 normal, 1 warning, 2 high, 3 critical.
    pub fn pressure_level(self, tracked_bytes: u64) -> u8 {
        let Some(limit) = self.max_tracked_bytes else {
            return 0;
        };
        if limit == 0 {
            return if tracked_bytes == 0 { 0 } else { 3 };
        }
        let budget = self.normalized();
        let usage_percent = (u128::from(tracked_bytes) * 100 / u128::from(limit)).min(100) as u8;
        if usage_percent >= budget.critical_percent {
            3
        } else if usage_percent >= budget.high_percent {
            2
        } else if usage_percent >= budget.warning_percent {
            1
        } else {
            0
        }
    }

    /// Whether a new estimated allocation fits without evicting any existing live resource.
    pub fn allows(self, current_tracked_bytes: u64, next_allocation_bytes: u64) -> bool {
        self.max_tracked_bytes.is_none_or(|limit| {
            current_tracked_bytes.saturating_add(next_allocation_bytes) <= limit
        })
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

    /// A replacement device invalidates every uploaded model handle. Keep requests alive so the
    /// owning renderer can warm them again from AssetServer's retained CPU assets.
    pub(crate) fn reset_after_device_loss(&mut self) {
        for entry in &mut self.entries {
            if matches!(&entry.status, ModelWarmupStatus::Ready) {
                entry.status = ModelWarmupStatus::WaitingForGpu;
            }
        }
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
    use super::{GlobalGpuResidencyBudget, GpuUploadBudget, ModelWarmup, ModelWarmupStatus};
    use ribbon_assets::{AssetServer, FsSource};
    use ribbon_plugin_gltf::{GltfLoader, ModelAsset};
    use std::path::Path;

    #[test]
    fn global_gpu_budget_enforces_tracked_admission_without_evicting_live_resources() {
        let budget = GlobalGpuResidencyBudget {
            max_tracked_bytes: Some(100),
            warning_percent: 75,
            high_percent: 90,
            critical_percent: 100,
        };
        assert!(budget.allows(60, 40));
        assert!(!budget.allows(60, 41));
        assert!(budget.allows(0, 100));
        assert!(!budget.allows(1, 100));
        assert_eq!(budget.pressure_level(74), 0);
        assert_eq!(budget.pressure_level(75), 1);
        assert_eq!(budget.pressure_level(90), 2);
        assert_eq!(budget.pressure_level(100), 3);
    }

    #[test]
    fn global_gpu_budget_normalizes_thresholds_and_supports_disabled_limit() {
        let normalized = GlobalGpuResidencyBudget {
            max_tracked_bytes: Some(100),
            warning_percent: 0,
            high_percent: 5,
            critical_percent: 4,
        }
        .normalized();
        assert_eq!(normalized.warning_percent, 1);
        assert_eq!(normalized.high_percent, 5);
        assert_eq!(normalized.critical_percent, 5);
        assert!(!normalized.allows(u64::MAX, 1));

        let disabled = GlobalGpuResidencyBudget {
            max_tracked_bytes: None,
            ..GlobalGpuResidencyBudget::default()
        };
        assert!(disabled.allows(u64::MAX, u64::MAX));
        assert_eq!(disabled.pressure_level(u64::MAX), 0);
    }

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
    fn model_gpu_residency_budget_reports_warning_high_and_critical_pressure() {
        use super::ModelGpuResidencyBudget;
        let budget = ModelGpuResidencyBudget::default();
        assert_eq!(budget.pressure_level(700 * 1024 * 1024), 0);
        assert_eq!(budget.pressure_level(800 * 1024 * 1024), 1);
        assert_eq!(budget.pressure_level(950 * 1024 * 1024), 2);
        assert_eq!(budget.pressure_level(1024 * 1024 * 1024), 3);
        assert_eq!(
            ModelGpuResidencyBudget {
                max_bytes: Some(0),
                ..budget
            }
            .pressure_level(1),
            3
        );
        assert_eq!(
            ModelGpuResidencyBudget {
                max_bytes: None,
                ..budget
            }
            .pressure_level(u64::MAX),
            0
        );
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
