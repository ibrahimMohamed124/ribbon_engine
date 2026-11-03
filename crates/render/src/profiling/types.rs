//! Renderer-neutral timing and resource diagnostic data structures.
use ribbon_profiling::DurationSummary as RenderDurationSummary;

/// CPU duration for one render-graph pass, in nanoseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderPassTiming {
    /// Stable graph label for the pass.
    pub name: String,
    /// CPU time spent invoking this pass; does not include asynchronous GPU execution.
    pub cpu_time_ns: u64,
}

/// One asynchronously completed GPU timestamp sample, associated with the frame that observed
/// its readback. `sequence` identifies the earlier submission being measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderGpuTimingSample {
    pub source: &'static str,
    pub sequence: u64,
    pub submitted_frame: u64,
    pub observed_frame: u64,
    pub total_ns: u64,
    pub shadow_ns: Option<u64>,
    pub mesh_ns: Option<u64>,
    pub bloom_ns: Option<u64>,
    pub tone_mapping_ns: Option<u64>,
}

/// Latest GPU-timing summary published by a renderer backend.
///
/// The mesh/shadow and post-processing profilers run independently, so their latest completed
/// samples can have different sequence numbers. `mesh_sequence` and `post_sequence` preserve that
/// distinction rather than implying that all stage durations came from one GPU submission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderGpuTimingSummary {
    pub supported: bool,
    pub latest_total_ns: Option<u64>,
    pub latest_shadow_ns: Option<u64>,
    pub latest_mesh_ns: Option<u64>,
    pub mesh_sequence: Option<u64>,
    pub latest_bloom_ns: Option<u64>,
    pub latest_tone_mapping_ns: Option<u64>,
    pub post_sequence: Option<u64>,
    pub successful_samples: u64,
    pub skipped_samples: u64,
}

/// Measurements collected while executing one render graph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderGraphTiming {
    /// Wall-clock time spent running graph nodes on the CPU, in nanoseconds.
    pub cpu_time_ns: u64,
    /// Per-node CPU timing in execution order.
    pub passes: Vec<RenderPassTiming>,
}

/// Estimated live GPU allocation and pool statistics published by a renderer backend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderResourceStats {
    /// Live uploaded meshes.
    pub live_meshes: usize,
    /// Live uploaded materials.
    pub live_materials: usize,
    /// Unique standalone GPU textures not already attributed to a material.
    pub live_streamed_textures: usize,
    /// Unique renderer-owned standalone samplers; samplers do not expose reliable byte sizes.
    pub live_streamed_samplers: usize,
    /// Estimated bytes for unique standalone GPU textures, included in total GPU residency.
    pub streamed_texture_bytes_estimate: u64,
    /// Camera-visible mesh instances prepared for the main color pass.
    pub visible_mesh_instances: usize,
    /// Grouped indexed draw calls issued by the main color pass.
    pub visible_draw_calls: usize,
    /// Particle instances drawn in the main 3D pass.
    pub particle_instances_drawn: usize,
    /// Particle batches drawn in the main 3D pass.
    pub particle_batches_drawn: usize,
    /// Grouped indexed draw calls issued by the directional shadow pass.
    pub shadow_draw_calls: usize,
    /// Skinned draws retained after the bounded per-frame joint-palette admission step.
    pub skinned_draws_after_palette_budget: usize,
    /// Skinned draws rejected because the remaining per-frame joint-palette capacity was insufficient.
    pub skinned_draws_rejected_palette_capacity: usize,
    /// Joint matrices staged for the current frame's bounded GPU storage-buffer upload.
    pub skin_palette_matrices_staged: usize,
    /// Estimated bytes used by live mesh vertex/index buffers.
    pub mesh_bytes_estimate: u64,
    /// Estimated bytes used by material factors and material textures (aggregate).
    pub material_bytes_estimate: u64,
    /// Number of unique live GPU textures referenced by material slots.
    pub live_material_textures: usize,
    /// Estimated bytes across unique live material textures including mip levels; included in
    /// `material_bytes_estimate`, not an additional total allocation category.
    pub material_texture_bytes_estimate: u64,
    /// Estimated material uniform bytes; included in `material_bytes_estimate`.
    pub material_uniform_bytes_estimate: u64,
    /// Cumulative texture requests served from the content-addressed GPU texture cache.
    pub material_texture_cache_hits: u64,
    /// Cumulative unique texture uploads caused by cache misses.
    pub material_texture_cache_misses: u64,
    /// Estimated bytes for renderer-owned uniform and instance buffers.
    pub buffer_bytes_estimate: u64,
    /// Estimated bytes for persistent shadow/depth targets.
    pub depth_target_bytes_estimate: u64,
    /// Number of currently leased transient textures.
    pub active_transient_textures: usize,
    /// Number of transient textures retained in cache and available for reuse.
    pub cached_transient_textures: usize,
    /// Bytes currently leased from transient texture pools.
    pub active_transient_bytes_estimate: u64,
    /// Bytes retained in transient pools but not currently leased.
    pub cached_transient_bytes_estimate: u64,
    /// Estimated bytes held by post-processing and auxiliary anti-aliasing targets not allocated by the pool.
    pub post_process_bytes_estimate: u64,
    /// Estimated bytes for the context-owned RGBA8 target.
    pub context_target_bytes_estimate: u64,
    /// Bytes retained by the reusable context-owned readback staging buffer.
    pub context_readback_bytes_estimate: u64,
    /// Peak estimate reported by the mesh-renderer backend; excludes pool/context/post-process totals.
    pub backend_peak_bytes_estimate: u64,
    /// Number of texture pool allocations since the pool was created.
    pub transient_texture_allocations: u64,
    /// Number of cache reuses since the pool was created.
    pub transient_texture_reuses: u64,
    /// Peak estimated bytes allocated in the transient texture pool since its creation.
    pub transient_pool_peak_bytes_estimate: u64,
    /// Cached/active pool textures whose format is not covered by the byte estimator.
    pub transient_unknown_size_textures: usize,
    /// Built-in ECS mesh placements extracted for the current frame.
    pub scene_mesh_instances: usize,
    /// glTF model placements extracted for the current frame.
    pub scene_model_placements: usize,
    /// Number of distinct glTF asset-generation keys currently uploaded by the render pass.
    pub gpu_model_cache_entries: usize,
    /// Estimated GPU upload bytes retained by the glTF model cache; this is a subset of mesh/material totals.
    pub model_gpu_cache_bytes_estimate: u64,
    /// Soft glTF model-cache residency budget in estimated bytes, or zero when disabled.
    pub model_gpu_cache_budget_bytes: u64,
    /// Estimated glTF model-cache bytes above the configured soft budget.
    pub model_gpu_cache_over_budget_bytes: u64,
    /// Cumulative transitions into a higher glTF model-cache memory-pressure level.
    pub model_gpu_cache_pressure_events: u64,
    /// Current glTF model-cache pressure level: 0 normal, 1 warning, 2 high, 3 critical.
    pub model_gpu_cache_pressure_level: u8,
    /// Number of generation-aware shader module records retained by the renderer registry.
    pub shader_modules: usize,
    /// Shader modules whose wgpu validation completed successfully.
    pub shader_modules_ready: usize,
    /// Shader modules that failed wgpu validation.
    pub shader_modules_failed: usize,
    /// Number of generation-aware render pipeline records retained by the renderer registry.
    pub render_pipelines: usize,
    /// Render pipelines whose wgpu validation completed successfully.
    pub render_pipelines_ready: usize,
    /// Render pipelines whose builder or wgpu validation failed.
    pub render_pipelines_failed: usize,
    /// Shader/pipeline error-scope validations currently awaiting asynchronous completion.
    pub shader_pipeline_pending_validations: usize,
    /// CPU bytes retained as WGSL source for device recreation; not GPU residency.
    pub shader_pipeline_retained_wgsl_bytes_estimate: u64,
    /// Number of WGSL AssetServer generations pinned by the shader registry.
    pub shader_pipeline_source_asset_leases: usize,
    /// Cumulative shader/pipeline object creation attempts submitted on the render thread.
    pub shader_pipeline_creations_started: u64,
    /// Cumulative shader/pipeline validation failures.
    pub shader_pipeline_validation_failures: u64,
    /// Cumulative shader/pipeline generations released after their final lease ended.
    pub shader_pipeline_released_generations: u64,
    /// Estimated bytes across currently tracked renderer-owned GPU allocations. This excludes
    /// opaque driver/OS allocations and resources without a portable size estimate.
    pub gpu_global_tracked_bytes_estimate: u64,
    /// Configured global tracked-GPU admission budget, or zero when disabled.
    pub gpu_global_budget_bytes: u64,
    /// Tracked GPU estimate above the configured admission budget.
    pub gpu_global_over_budget_bytes: u64,
    /// Global tracked-GPU pressure level: 0 normal, 1 warning, 2 high, 3 critical.
    pub gpu_global_pressure_level: u8,
    /// Cumulative transitions into a higher global tracked-GPU pressure level.
    pub gpu_global_pressure_events: u64,
    /// Cumulative upload admissions rejected by the global tracked-GPU budget.
    pub gpu_global_budget_rejections: u64,
    /// Cumulative distinct glTF model uploads for the current render-pass lifetime.
    pub model_gpu_uploads_cumulative: u64,
    /// Cumulative glTF GPU model cache entries released from the current render-pass lifetime.
    pub model_gpu_releases_cumulative: u64,
    /// Requested explicit model warmups, including CPU-pending and GPU-pending entries.
    pub model_warmup_requested: usize,
    /// Warmup requests still waiting for asynchronous CPU asset loading/commit.
    pub model_warmup_waiting_for_cpu: usize,
    /// Warmup requests with CPU data resident but not yet GPU-ready.
    pub model_warmup_waiting_for_gpu: usize,
    /// Warmup requests whose GPU resources completed validation.
    pub model_warmup_ready: usize,
    /// Warmup requests that failed CPU loading or GPU validation.
    pub model_warmup_failed: usize,
    /// Number of distinct model GPU cache uploads started in the most recent frame.
    pub model_gpu_uploads_this_frame: usize,
    /// Estimated bytes submitted by model GPU uploads during the most recent render frame.
    /// This is upload traffic telemetry, not live VRAM residency.
    pub model_gpu_upload_bytes_this_frame: u64,
    /// Current power-of-two capacity of both renderer instance buffers.
    pub renderer_instance_capacity: usize,
    /// Cumulative instance-buffer capacity increases over the current renderer lifetime.
    pub renderer_instance_buffer_grows_cumulative: u64,
    /// Cumulative instance-buffer capacity decreases over the current renderer lifetime.
    pub renderer_instance_buffer_shrinks_cumulative: u64,
    /// Estimated decoded CPU-side bytes currently held by the asset server.
    /// Kept separate from the GPU total to avoid mixing CPU and GPU allocation categories.
    pub cpu_asset_resident_bytes_estimate: u64,
    /// Estimated CPU bytes attributed to decoded geometry allocations.
    pub cpu_asset_geometry_bytes_estimate: u64,
    /// Estimated CPU bytes attributed to decoded texture pixels and mip payloads.
    pub cpu_asset_texture_bytes_estimate: u64,
    /// Estimated CPU bytes attributed to decoded material records.
    pub cpu_asset_material_bytes_estimate: u64,
    /// Estimated CPU bytes attributed to decoded animation data where loaders expose it.
    pub cpu_asset_animation_bytes_estimate: u64,
    /// Estimated CPU bytes attributed to scene descriptions and hierarchy metadata.
    pub cpu_asset_scene_metadata_bytes_estimate: u64,
    /// Estimated CPU bytes from loaders without a more specific category.
    pub cpu_asset_other_bytes_estimate: u64,
    /// Estimated CPU bytes protected by strong asset leases.
    pub cpu_asset_pinned_bytes_estimate: u64,
    /// Estimated CPU bytes eligible for budget eviction.
    pub cpu_asset_unpinned_bytes_estimate: u64,
    /// Worker source-read elapsed time represented by completions processed in the last AssetServer poll.
    pub asset_worker_source_read_ns_last_poll: u64,
    /// Worker decode elapsed time represented by completions processed in the last AssetServer poll.
    pub asset_worker_decode_ns_last_poll: u64,
    /// Number of worker completion records committed or discarded in the last AssetServer poll.
    pub asset_worker_completions_processed_last_poll: usize,
    /// Decoded records currently staged for a later owner-thread commit (channel backlog excluded).
    pub asset_worker_pending_completions: usize,
    /// Number of decoded CPU assets currently resident.
    pub cpu_asset_loaded_count: usize,
    /// Number of resident CPU assets protected by at least one strong lease.
    pub cpu_asset_pinned_count: usize,
    /// Whether CPU asset-cache budget eviction is enabled for this server.
    pub cpu_asset_budget_enabled: bool,
    /// Configured unpinned CPU cache budget, in bytes (ignored when budget is disabled).
    pub cpu_asset_budget_bytes: u64,
    /// Unpinned resident CPU bytes over the configured budget.
    pub cpu_asset_over_budget_bytes: u64,
    /// Cumulative number of resident CPU assets evicted specifically for the cache budget.
    pub cpu_asset_budget_evictions_cumulative: u64,
    /// Registered scene/partition regions known to the streaming coordinator.
    pub streaming_region_count: usize,
    /// Regions currently loading CPU/GPU assets or preparing physics/render entities.
    pub streaming_loading_regions: usize,
    /// Prepared regions waiting for explicit activation or observer entry.
    pub streaming_ready_regions: usize,
    /// Regions whose streamed entities are active in the ECS world.
    pub streaming_active_regions: usize,
    /// Inactive regions retaining CPU leases or GPU warmup residency.
    pub streaming_dormant_regions: usize,
    /// Regions whose preparation or activation failed and require retry/inspection.
    pub streaming_failed_regions: usize,
    /// Source/decode jobs waiting in the bounded AssetServer worker queue.
    pub streaming_asset_jobs_queued: usize,
    /// Fixed capacity of the bounded AssetServer worker queue.
    pub streaming_asset_queue_capacity: usize,
    /// Region asset paths still waiting for the coordinator's per-update submission budget.
    pub streaming_asset_requests_pending: usize,
    /// AssetServer worker threads currently executing source/decode jobs.
    pub streaming_asset_workers_active: usize,
    /// CPU time spent in the streaming coordinator update, measured only when profiling is enabled.
    pub streaming_cpu_update_ns: u64,
    /// Snapshot of decoded CPU cache residency estimate; informational and not additive to GPU bytes.
    pub streaming_cpu_memory_bytes_estimate: u64,
    /// Current effective AssetServer budget for decoded unpinned CPU cache entries.
    pub streaming_cpu_cache_budget_bytes: u64,
    /// Decoded unpinned CPU cache estimate above its current effective budget.
    pub streaming_cpu_cache_over_budget_bytes: u64,
    /// Configured target for total decoded CPU asset residency; zero is ambiguous with a `Some(0)` target.
    pub streaming_cpu_resident_budget_bytes: u64,
    /// Total decoded CPU asset residency above the configured streaming target.
    pub streaming_cpu_resident_over_budget_bytes: u64,
    /// Snapshot of renderer-tracked GPU allocation estimate; same allocations as renderer totals.
    pub streaming_gpu_memory_bytes_estimate: u64,
    /// Cumulative count of soft GPU-budget pressure events handled by streaming.
    pub streaming_memory_pressure_events: u64,
    /// Cumulative decoded CPU assets evicted by the AssetServer unpinned-cache budget.
    pub streaming_cpu_budget_evictions_cumulative: u64,
    /// Invalid region-state transitions rejected by the coordinator.
    pub streaming_invalid_state_transitions: u64,
    /// Worst observed per-region request-to-active duration, in nanoseconds.
    pub streaming_max_activation_latency_ns: u64,
    /// Stable id of the region responsible for the worst observed activation latency.
    pub streaming_slowest_region_id: String,
    /// Latest recorded request-stage duration for the slowest region.
    pub streaming_slowest_region_requested_ns: u64,
    /// Latest recorded CPU asset/decode stage duration for the slowest region.
    pub streaming_slowest_region_cpu_loading_ns: u64,
    /// Latest recorded GPU warmup stage duration for the slowest region.
    pub streaming_slowest_region_gpu_warmup_ns: u64,
    /// Latest recorded ready-wait stage duration for the slowest region.
    pub streaming_slowest_region_ready_wait_ns: u64,
    /// Latest recorded physics preparation stage duration for the slowest region.
    pub streaming_slowest_region_physics_ns: u64,
    /// Latest recorded activation/commit stage duration for the slowest region.
    pub streaming_slowest_region_activation_ns: u64,
    /// Elapsed duration of the slowest region's current in-flight state.
    pub streaming_slowest_region_current_state_ns: u64,
    /// Request-to-active duration for the slowest region's most recent completed attempt.
    pub streaming_slowest_region_last_activation_ns: u64,
    /// Standalone mesh/material uploads queued for the renderer-owned generic warmup path.
    pub generic_gpu_warmup_queued: usize,
    /// Generic resource uploads submitted but awaiting asynchronous wgpu validation.
    pub generic_gpu_warmup_preparing: usize,
    /// Generic resource uploads whose GPU validation completed successfully.
    pub generic_gpu_warmup_ready: usize,
    /// Generic resource warmup requests that failed payload or GPU validation.
    pub generic_gpu_warmup_failed: usize,
    /// Cumulative standalone mesh/material GPU uploads that passed validation.
    pub generic_gpu_uploads_cumulative: u64,
    /// Cumulative standalone mesh/material GPU upload validation failures.
    pub generic_gpu_upload_failures_cumulative: u64,
    /// Cumulative generic GPU resources released after their final lease was dropped.
    pub generic_gpu_releases_cumulative: u64,
    /// Generic standalone GPU resource uploads started in the latest frame.
    pub generic_gpu_uploads_this_frame: usize,
    /// Estimated generic GPU upload traffic submitted in the latest frame.
    pub generic_gpu_upload_bytes_this_frame: u64,
    /// Generic GPU bytes held by ready standalone leases.
    pub generic_gpu_resident_bytes_estimate: u64,
    /// CPU-side source payload retained by generic GPU leases for device-loss reconstruction.
    /// This is not GPU residency and is tracked separately to avoid double-counting.
    pub generic_gpu_recovery_payload_bytes_estimate: u64,
    /// Cumulative device-loss events and replacement-device recovery results.
    pub gpu_device_loss_events: u64,
    pub gpu_device_recovery_attempts: u64,
    pub gpu_device_recovery_successes: u64,
    pub gpu_device_recovery_failures: u64,
    /// Most recently observed device-loss reason, if one was provided by wgpu.
    pub gpu_last_device_loss_message: String,
    /// Generic GPU bytes reserved by queued or in-flight uploads.
    pub generic_gpu_queued_bytes_estimate: u64,
    /// Largest generic GPU total/class budget overage, in estimated bytes.
    pub generic_gpu_budget_overage_bytes: u64,
    /// Number of orphaned generic GPU resources evicted by residency admission.
    pub generic_gpu_evictions_cumulative: u64,
    /// Cumulative GPU residency pressure events handled by the generic manager.
    pub generic_gpu_memory_pressure_events: u64,
    /// Cumulative generic GPU requests rejected because live/in-flight leases exhaust budgets.
    pub generic_gpu_budget_rejections_cumulative: u64,
    /// Current number of samplers above the configured sampler-count limit.
    pub generic_gpu_sampler_count_over_budget: usize,
}

impl RenderResourceStats {
    /// Total currently tracked estimated bytes. Driver, allocator, and pipeline overhead is excluded.
    pub fn total_bytes_estimate(&self) -> u64 {
        self.mesh_bytes_estimate
            .saturating_add(self.material_bytes_estimate)
            .saturating_add(self.streamed_texture_bytes_estimate)
            .saturating_add(self.buffer_bytes_estimate)
            .saturating_add(self.depth_target_bytes_estimate)
            .saturating_add(self.active_transient_bytes_estimate)
            .saturating_add(self.cached_transient_bytes_estimate)
            .saturating_add(self.post_process_bytes_estimate)
            .saturating_add(self.context_target_bytes_estimate)
            .saturating_add(self.context_readback_bytes_estimate)
    }
}

/// Latest CPU/GPU measurements available for the native renderer.
///
/// This resource can be inspected from the app's `World`. GPU timestamps are produced
/// asynchronously and may describe an earlier frame; `gpu_sample_sequence` identifies the
/// submitted sample. No GPU timestamp is reported when the adapter
/// does not expose timestamp queries or when every readback slot is busy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderDiagnostics {
    /// Number of window frames whose CPU render path reached presentation.
    pub frame_count: u64,
    /// Most recent elapsed wall duration from surface acquisition through presentation, in nanoseconds.
    pub last_frame_cpu_ns: u64,
    /// Running arithmetic mean of the full frame wall duration in nanoseconds.
    pub average_frame_cpu_ns: u64,
    /// Highest full frame wall duration since this resource was created.
    pub peak_frame_cpu_ns: u64,
    /// Wall time spent acquiring a surface texture for the most recently presented frame.
    pub last_surface_acquire_ns: u64,
    /// Wall time spent in `SurfaceTexture::present` for the most recently presented frame.
    pub last_present_ns: u64,
    /// Actual configured native-surface present mode, using a stable lowercase label.
    pub surface_present_mode: &'static str,
    /// Configured desired maximum frame latency (frames in flight) for the native surface.
    pub surface_frame_latency: u32,
    /// CPU time spent executing render-graph nodes, excluding surface acquisition/presentation.
    pub last_graph_cpu_ns: u64,
    /// Most recent render-graph pass timings.
    pub passes: Vec<RenderPassTiming>,
    /// Fine-grained CPU phase timings for the most recently rendered graph frame.
    pub cpu_phases: Vec<RenderPassTiming>,
    /// GPU results newly completed by asynchronous readback on the most recent render frame.
    pub completed_gpu_samples: Vec<RenderGpuTimingSample>,
    /// GPU readbacks currently pending across render timestamp profilers.
    pub gpu_samples_pending: u64,
    /// GPU timestamp attempts that could not be issued because profiling is unavailable.
    pub gpu_samples_unavailable: u64,
    /// GPU timestamp samples lost to mapping/data failures or bounded completion-queue overflow.
    pub gpu_samples_dropped: u64,
    /// Native-surface acquisition timeouts that did not produce a frame.
    pub surface_timeouts: u64,
    /// Surface reconfiguration events reported as Outdated.
    pub surface_outdated: u64,
    /// Surface reconfiguration events reported as Lost.
    pub surface_lost: u64,
    /// Other terminal surface acquisition errors.
    pub surface_other_errors: u64,
    /// Render-graph failures after successful surface acquisition.
    pub render_graph_errors: u64,
    /// Cumulative `wgpu` device-loss notifications handled by the native window renderer.
    pub gpu_device_loss_events: u64,
    /// Cumulative attempts to create a replacement adapter/device after loss.
    pub gpu_device_recovery_attempts: u64,
    /// Successful replacement-device creations after device loss.
    pub gpu_device_recovery_successes: u64,
    /// Failed replacement-device creations; the renderer retries at a bounded cadence.
    pub gpu_device_recovery_failures: u64,
    /// Most recent diagnostic reported by the device-lost callback.
    pub gpu_last_device_loss_message: String,
    /// Most recent completed GPU duration spanning directional shadow, optional spot shadow,
    /// and the main mesh pass, if supported.
    pub last_gpu_shadow_and_mesh_ns: Option<u64>,
    /// Most recent completed GPU duration for the shadow passes.
    pub last_gpu_shadow_pass_ns: Option<u64>,
    /// Most recent completed GPU duration for the main color/depth mesh pass, if supported.
    pub last_gpu_mesh_pass_ns: Option<u64>,
    /// Most recent completed GPU duration for the bloom render pass.
    pub last_gpu_bloom_pass_ns: Option<u64>,
    /// Most recent completed GPU duration for tone mapping.
    pub last_gpu_tone_mapping_pass_ns: Option<u64>,
    /// Profiler sequence assigned when the most recent GPU sample was submitted.
    pub gpu_sample_sequence: Option<u64>,
    /// Profiler sequence for the latest completed Bloom/tone-mapping sample.
    pub gpu_post_sample_sequence: Option<u64>,
    /// True when the backend device supports GPU timestamp queries.
    pub gpu_timestamp_queries_supported: bool,
    /// Number of successful asynchronous GPU timestamp samples.
    pub gpu_timestamp_samples: u64,
    /// Number of frames that could not be GPU-timed because all readback slots were occupied.
    pub gpu_timestamp_samples_skipped: u64,
    /// Last published allocation estimates.
    pub resources: RenderResourceStats,
    /// Peak total tracked GPU allocation estimate published since creation.
    pub peak_tracked_gpu_bytes_estimate: u64,
}

impl RenderDiagnostics {
    /// Record the timing of a successfully presented frame without phase breakdown.
    ///
    /// This compatibility method records zero for surface acquisition and presentation details.
    pub fn record_frame(&mut self, frame_wall_ns: u64, graph: RenderGraphTiming) {
        self.record_frame_phases(frame_wall_ns, 0, 0, graph);
    }

    /// Record a presented frame with elapsed wall time split by surface acquire and present phases.
    pub fn record_frame_phases(
        &mut self,
        frame_wall_ns: u64,
        surface_acquire_ns: u64,
        present_ns: u64,
        graph: RenderGraphTiming,
    ) {
        self.frame_count = self.frame_count.saturating_add(1);
        self.last_frame_cpu_ns = frame_wall_ns;
        self.last_surface_acquire_ns = surface_acquire_ns;
        self.last_present_ns = present_ns;
        self.peak_frame_cpu_ns = self.peak_frame_cpu_ns.max(frame_wall_ns);
        if self.frame_count == 1 {
            self.average_frame_cpu_ns = frame_wall_ns;
        } else {
            // Incremental mean avoids retaining an unbounded history of frame samples.
            let previous = u128::from(self.average_frame_cpu_ns);
            let sample = u128::from(frame_wall_ns);
            let previous_count = u128::from(self.frame_count - 1);
            let count = u128::from(self.frame_count);
            self.average_frame_cpu_ns =
                ((previous * previous_count + sample) / count).min(u128::from(u64::MAX)) as u64;
        }
        self.last_graph_cpu_ns = graph.cpu_time_ns;
        self.passes = graph.passes;
    }

    /// Record the resolved native surface configuration for profile metadata.
    pub fn record_surface_config(&mut self, present_mode: &'static str, frame_latency: u32) {
        self.surface_present_mode = present_mode;
        self.surface_frame_latency = frame_latency;
    }

    /// Publish GPU timestamp-query availability and the latest completed sample.
    pub fn record_gpu_timing(&mut self, timing: RenderGpuTimingSummary) {
        self.gpu_timestamp_queries_supported = timing.supported;
        if let Some(duration) = timing.latest_total_ns {
            self.last_gpu_shadow_and_mesh_ns = Some(duration);
            self.last_gpu_shadow_pass_ns = timing.latest_shadow_ns;
            self.last_gpu_mesh_pass_ns = timing.latest_mesh_ns;
            self.gpu_sample_sequence = timing.mesh_sequence;
        }
        if timing.latest_bloom_ns.is_some() || timing.latest_tone_mapping_ns.is_some() {
            self.last_gpu_bloom_pass_ns = timing.latest_bloom_ns;
            self.last_gpu_tone_mapping_pass_ns = timing.latest_tone_mapping_ns;
            self.gpu_post_sample_sequence = timing.post_sequence;
        }
        self.gpu_timestamp_samples = timing.successful_samples;
        self.gpu_timestamp_samples_skipped = timing.skipped_samples;
    }

    /// Publish a current backend resource snapshot.
    pub fn record_resources(&mut self, resources: RenderResourceStats) {
        self.peak_tracked_gpu_bytes_estimate = self
            .peak_tracked_gpu_bytes_estimate
            .max(resources.total_bytes_estimate());
        self.resources = resources;
    }

    /// Add GPU context-owned allocations that are outside plugin render passes.
    pub fn add_context_target_bytes(&mut self, bytes: u64) {
        self.resources.context_target_bytes_estimate = bytes;
        self.peak_tracked_gpu_bytes_estimate = self
            .peak_tracked_gpu_bytes_estimate
            .max(self.resources.total_bytes_estimate());
    }

    /// Add context-owned staging bytes retained for synchronous framebuffer readback.
    pub fn add_context_readback_bytes(&mut self, bytes: u64) {
        self.resources.context_readback_bytes_estimate = bytes;
        self.peak_tracked_gpu_bytes_estimate = self
            .peak_tracked_gpu_bytes_estimate
            .max(self.resources.total_bytes_estimate());
    }
}

/// One retained sample from the native-window renderer. Frame/acquire/present values are elapsed
/// wall durations, graph/pass values are CPU-side invocation times, and GPU values are asynchronous
/// timestamp-query results that may refer to an earlier submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderProfileSample {
    /// Successfully presented window-frame counter.
    pub frame: u64,
    /// Whole render-system CPU duration in nanoseconds.
    pub frame_cpu_ns: u64,
    /// Running mean full-frame wall duration at this sample.
    pub average_frame_cpu_ns: u64,
    /// Peak full-frame wall duration observed since diagnostics were created.
    pub peak_frame_cpu_ns: u64,
    /// Wall time spent acquiring the surface texture.
    pub surface_acquire_ns: u64,
    /// Wall time spent inside surface presentation.
    pub present_ns: u64,
    /// Resolved native surface present mode used by the capture.
    pub surface_present_mode: &'static str,
    /// Desired maximum frame latency configured for the surface.
    pub surface_frame_latency: u32,
    /// CPU duration of render-graph node execution.
    pub graph_cpu_ns: u64,
    /// Latest available asynchronous GPU duration for shadow + main mesh rendering.
    pub gpu_shadow_and_mesh_ns: Option<u64>,
    /// Latest available asynchronous GPU duration for the main mesh pass.
    pub gpu_mesh_pass_ns: Option<u64>,
    /// GPU timestamp duration for all shadow passes.
    pub gpu_shadow_pass_ns: Option<u64>,
    /// GPU timestamp duration for Bloom.
    pub gpu_bloom_pass_ns: Option<u64>,
    /// GPU timestamp duration for tone mapping.
    pub gpu_tone_mapping_pass_ns: Option<u64>,
    /// Submission sequence for the latest completed GPU timestamp sample.
    pub gpu_sample_sequence: Option<u64>,
    /// Sequence for the latest completed Bloom/tone-mapping sample.
    pub gpu_post_sample_sequence: Option<u64>,
    /// Whether the renderer's adapter supports timestamp queries.
    pub gpu_timestamp_queries_supported: bool,
    /// Cumulative successful timestamp samples since diagnostics were created.
    pub gpu_timestamp_samples: u64,
    /// Cumulative frames skipped because profiler readback slots were full.
    pub gpu_timestamp_samples_skipped: u64,
    /// Resource estimate and pool counters at the time the sample was captured.
    pub resources: RenderResourceStats,
    /// Peak aggregate resource estimate seen by the diagnostics object.
    pub peak_tracked_gpu_bytes_estimate: u64,
    /// Per-node CPU durations from this frame's render graph.
    pub passes: Vec<RenderPassTiming>,
    /// Fine-grained CPU render phases captured on this frame.
    pub cpu_phases: Vec<RenderPassTiming>,
    /// GPU timestamp samples whose asynchronous readback completed on this frame.
    pub completed_gpu_samples: Vec<RenderGpuTimingSample>,
    pub gpu_samples_pending: u64,
    pub gpu_samples_unavailable: u64,
    pub gpu_samples_dropped: u64,
    pub surface_timeouts: u64,
    pub surface_outdated: u64,
    pub surface_lost: u64,
    pub surface_other_errors: u64,
    pub render_graph_errors: u64,
}

impl RenderProfileSample {
    pub(super) fn from_diagnostics(diagnostics: &RenderDiagnostics) -> Self {
        Self {
            frame: diagnostics.frame_count,
            frame_cpu_ns: diagnostics.last_frame_cpu_ns,
            average_frame_cpu_ns: diagnostics.average_frame_cpu_ns,
            peak_frame_cpu_ns: diagnostics.peak_frame_cpu_ns,
            surface_acquire_ns: diagnostics.last_surface_acquire_ns,
            present_ns: diagnostics.last_present_ns,
            surface_present_mode: diagnostics.surface_present_mode,
            surface_frame_latency: diagnostics.surface_frame_latency,
            graph_cpu_ns: diagnostics.last_graph_cpu_ns,
            gpu_shadow_and_mesh_ns: diagnostics.last_gpu_shadow_and_mesh_ns,
            gpu_shadow_pass_ns: diagnostics.last_gpu_shadow_pass_ns,
            gpu_mesh_pass_ns: diagnostics.last_gpu_mesh_pass_ns,
            gpu_bloom_pass_ns: diagnostics.last_gpu_bloom_pass_ns,
            gpu_tone_mapping_pass_ns: diagnostics.last_gpu_tone_mapping_pass_ns,
            gpu_sample_sequence: diagnostics.gpu_sample_sequence,
            gpu_post_sample_sequence: diagnostics.gpu_post_sample_sequence,
            gpu_timestamp_queries_supported: diagnostics.gpu_timestamp_queries_supported,
            gpu_timestamp_samples: diagnostics.gpu_timestamp_samples,
            gpu_timestamp_samples_skipped: diagnostics.gpu_timestamp_samples_skipped,
            resources: diagnostics.resources.clone(),
            peak_tracked_gpu_bytes_estimate: diagnostics.peak_tracked_gpu_bytes_estimate,
            passes: diagnostics.passes.clone(),
            cpu_phases: diagnostics.cpu_phases.clone(),
            completed_gpu_samples: diagnostics.completed_gpu_samples.clone(),
            gpu_samples_pending: diagnostics.gpu_samples_pending,
            gpu_samples_unavailable: diagnostics.gpu_samples_unavailable,
            gpu_samples_dropped: diagnostics.gpu_samples_dropped,
            surface_timeouts: diagnostics.surface_timeouts,
            surface_outdated: diagnostics.surface_outdated,
            surface_lost: diagnostics.surface_lost,
            surface_other_errors: diagnostics.surface_other_errors,
            render_graph_errors: diagnostics.render_graph_errors,
        }
    }
}

/// Summary over the post-warmup window retained by [`RenderProfileRecorder`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderProfileSummary {
    /// Number of retained post-warmup frame samples.
    pub frame_count: usize,
    /// First and last engine frame numbers included in the summary.
    pub first_frame: Option<u64>,
    pub last_frame: Option<u64>,
    /// Distribution of whole-frame wall durations, including acquisition, graph execution and present.
    pub frame_cpu: RenderDurationSummary,
    /// Distribution of surface acquisition wall durations.
    pub surface_acquire: RenderDurationSummary,
    /// Distribution of presentation wall durations.
    pub present: RenderDurationSummary,
    /// Distribution of render-graph CPU durations.
    pub graph_cpu: RenderDurationSummary,
    /// Distribution of available asynchronous GPU shadow-plus-mesh durations.
    pub gpu_shadow_and_mesh: RenderDurationSummary,
    /// Distribution of available asynchronous GPU mesh-pass durations.
    pub gpu_mesh_pass: RenderDurationSummary,
    /// Distribution of available asynchronous GPU shadow-pass durations.
    pub gpu_shadow_pass: RenderDurationSummary,
    /// Distribution of available asynchronous GPU Bloom durations.
    pub gpu_bloom_pass: RenderDurationSummary,
    /// Distribution of available asynchronous GPU tone-mapping durations.
    pub gpu_tone_mapping_pass: RenderDurationSummary,
    /// Latest currently tracked estimated bytes retained by the sampled renderer.
    pub latest_tracked_bytes_estimate: u64,
    /// Cumulative high-water estimate carried by diagnostics at the latest retained sample.
    pub peak_tracked_bytes_estimate: u64,
    /// Latest cumulative allocation/reuse counts from the transient pool.
    pub transient_texture_allocations: u64,
    pub transient_texture_reuses: u64,
    /// Latest cumulative completed/skipped GPU timestamp counts.
    pub gpu_timestamp_samples: u64,
    pub gpu_timestamp_samples_skipped: u64,
}
