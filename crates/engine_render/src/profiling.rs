//! Backend-neutral CPU timing and estimated GPU resource diagnostics.
//!
//! GPU timestamps and resource estimates are optional data supplied by the backend that owns
//! the underlying allocations. Byte counts are estimates, not driver-reported VRAM totals.

use engine_app::{App, Stage};
use engine_plugins::Plugin;
use std::collections::{HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// CPU duration for one render-graph pass, in nanoseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderPassTiming {
    /// Stable graph label for the pass.
    pub name: String,
    /// CPU time spent invoking this pass; does not include asynchronous GPU execution.
    pub cpu_time_ns: u64,
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
    /// Estimated bytes used by live mesh vertex/index buffers.
    pub mesh_bytes_estimate: u64,
    /// Estimated bytes used by material factors and material textures.
    pub material_bytes_estimate: u64,
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
    /// Estimated bytes held by post-processing resources not allocated by the pool.
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
}

impl RenderResourceStats {
    /// Total currently tracked estimated bytes. Driver, allocator, and pipeline overhead is excluded.
    pub fn total_bytes_estimate(&self) -> u64 {
        self.mesh_bytes_estimate
            .saturating_add(self.material_bytes_estimate)
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
    /// Most recent completed GPU duration spanning directional shadow, optional spot shadow,
    /// and the main mesh pass, if supported.
    pub last_gpu_shadow_and_mesh_ns: Option<u64>,
    /// Most recent completed GPU duration for the main color/depth mesh pass, if supported.
    pub last_gpu_mesh_pass_ns: Option<u64>,
    /// Profiler sequence assigned when the most recent GPU sample was submitted.
    pub gpu_sample_sequence: Option<u64>,
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
    pub fn record_gpu_timing(
        &mut self,
        supported: bool,
        latest_total_ns: Option<u64>,
        latest_mesh_ns: Option<u64>,
        successful_samples: u64,
        skipped_samples: u64,
        sample_sequence: Option<u64>,
    ) {
        self.gpu_timestamp_queries_supported = supported;
        if let Some(duration) = latest_total_ns {
            self.last_gpu_shadow_and_mesh_ns = Some(duration);
            self.last_gpu_mesh_pass_ns = latest_mesh_ns;
            self.gpu_sample_sequence = sample_sequence;
        }
        self.gpu_timestamp_samples = successful_samples;
        self.gpu_timestamp_samples_skipped = skipped_samples;
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
    /// Submission sequence for the latest completed GPU timestamp sample.
    pub gpu_sample_sequence: Option<u64>,
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
}

impl RenderProfileSample {
    fn from_diagnostics(diagnostics: &RenderDiagnostics) -> Self {
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
            gpu_mesh_pass_ns: diagnostics.last_gpu_mesh_pass_ns,
            gpu_sample_sequence: diagnostics.gpu_sample_sequence,
            gpu_timestamp_queries_supported: diagnostics.gpu_timestamp_queries_supported,
            gpu_timestamp_samples: diagnostics.gpu_timestamp_samples,
            gpu_timestamp_samples_skipped: diagnostics.gpu_timestamp_samples_skipped,
            resources: diagnostics.resources.clone(),
            peak_tracked_gpu_bytes_estimate: diagnostics.peak_tracked_gpu_bytes_estimate,
            passes: diagnostics.passes.clone(),
        }
    }
}

/// Distribution summary for a set of CPU or GPU duration samples, in nanoseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderDurationSummary {
    /// Number of samples included in the distribution.
    pub sample_count: usize,
    /// Arithmetic mean.
    pub mean_ns: u64,
    /// Median (50th percentile).
    pub p50_ns: u64,
    /// 95th percentile using nearest-rank selection.
    pub p95_ns: u64,
    /// 99th percentile using nearest-rank selection.
    pub p99_ns: u64,
    /// Maximum value.
    pub peak_ns: u64,
}

impl RenderDurationSummary {
    fn from_values(values: impl IntoIterator<Item = u64>) -> Self {
        let mut values: Vec<u64> = values.into_iter().collect();
        if values.is_empty() {
            return Self::default();
        }
        values.sort_unstable();
        let sum = values
            .iter()
            .fold(0_u128, |acc, value| acc.saturating_add(u128::from(*value)));
        let percentile = |percentage: usize| {
            let rank = (percentage * values.len()).div_ceil(100).max(1);
            values[rank.saturating_sub(1).min(values.len() - 1)]
        };
        Self {
            sample_count: values.len(),
            mean_ns: (sum / values.len() as u128).min(u128::from(u64::MAX)) as u64,
            p50_ns: percentile(50),
            p95_ns: percentile(95),
            p99_ns: percentile(99),
            peak_ns: *values.last().unwrap_or(&0),
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

/// Bounded recorder for post-warmup render diagnostics.
///
/// No frame history is retained unless a [`RenderProfilePlugin`] (or a recorder used directly)
/// is installed. The default capacity is finite, so long-running capture cannot grow memory
/// without bound. GPU query results are sampled as last-completed values and must not be treated
/// as synchronous measurements of the same-numbered CPU frame.
#[derive(Clone, Debug)]
pub struct RenderProfileRecorder {
    output_path: PathBuf,
    capacity: usize,
    warmup_frames: u64,
    last_seen_frame: u64,
    samples: VecDeque<RenderProfileSample>,
}

fn unique_gpu_durations(
    samples: &[&RenderProfileSample],
    select: impl Fn(&RenderProfileSample) -> Option<u64>,
) -> Vec<u64> {
    let mut seen = HashSet::new();
    samples
        .iter()
        .filter_map(|sample| {
            let sequence = sample.gpu_sample_sequence?;
            let duration = select(sample)?;
            seen.insert(sequence).then_some(duration)
        })
        .collect()
}

impl RenderProfileRecorder {
    /// Create a recorder with a bounded history and a fixed CSV output path.
    pub fn new(path: impl Into<PathBuf>, capacity: usize, warmup_frames: u64) -> Self {
        Self {
            output_path: path.into(),
            capacity: capacity.max(1),
            warmup_frames,
            last_seen_frame: 0,
            samples: VecDeque::with_capacity(capacity.max(1)),
        }
    }

    /// Destination selected for the CSV report.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Maximum number of retained frame samples.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Configured number of successful frames excluded as warmup.
    pub fn warmup_frames(&self) -> u64 {
        self.warmup_frames
    }

    /// Retained post-warmup samples, in capture order.
    pub fn samples(&self) -> impl ExactSizeIterator<Item = &RenderProfileSample> {
        self.samples.iter()
    }

    fn should_capture_frame(&self, frame: u64) -> bool {
        frame > self.warmup_frames && frame > self.last_seen_frame
    }

    /// Capture a diagnostics snapshot once per successfully presented frame.
    pub fn record_diagnostics(&mut self, diagnostics: &RenderDiagnostics) {
        let frame = diagnostics.frame_count;
        if frame == 0 || frame <= self.last_seen_frame {
            return;
        }
        self.last_seen_frame = frame;
        if frame <= self.warmup_frames {
            return;
        }
        if self.samples.len() == self.capacity {
            let _ = self.samples.pop_front();
        }
        self.samples
            .push_back(RenderProfileSample::from_diagnostics(diagnostics));
    }

    /// Calculate mean, p50, p95, p99, and peak duration statistics plus resource counters.
    pub fn summary(&self) -> RenderProfileSummary {
        let samples: Vec<_> = self.samples.iter().collect();
        let latest = samples.last().copied();
        RenderProfileSummary {
            frame_count: samples.len(),
            first_frame: samples.first().map(|sample| sample.frame),
            last_frame: latest.map(|sample| sample.frame),
            frame_cpu: RenderDurationSummary::from_values(
                samples.iter().map(|sample| sample.frame_cpu_ns),
            ),
            surface_acquire: RenderDurationSummary::from_values(
                samples.iter().map(|sample| sample.surface_acquire_ns),
            ),
            present: RenderDurationSummary::from_values(
                samples.iter().map(|sample| sample.present_ns),
            ),
            graph_cpu: RenderDurationSummary::from_values(
                samples.iter().map(|sample| sample.graph_cpu_ns),
            ),
            gpu_shadow_and_mesh: RenderDurationSummary::from_values(unique_gpu_durations(
                &samples,
                |sample| sample.gpu_shadow_and_mesh_ns,
            )),
            gpu_mesh_pass: RenderDurationSummary::from_values(unique_gpu_durations(
                &samples,
                |sample| sample.gpu_mesh_pass_ns,
            )),
            latest_tracked_bytes_estimate: latest
                .map(|sample| sample.resources.total_bytes_estimate())
                .unwrap_or(0),
            peak_tracked_bytes_estimate: samples
                .iter()
                .map(|sample| sample.peak_tracked_gpu_bytes_estimate)
                .max()
                .unwrap_or(0),
            transient_texture_allocations: latest
                .map(|sample| sample.resources.transient_texture_allocations)
                .unwrap_or(0),
            transient_texture_reuses: latest
                .map(|sample| sample.resources.transient_texture_reuses)
                .unwrap_or(0),
            gpu_timestamp_samples: latest
                .map(|sample| sample.gpu_timestamp_samples)
                .unwrap_or(0),
            gpu_timestamp_samples_skipped: latest
                .map(|sample| sample.gpu_timestamp_samples_skipped)
                .unwrap_or(0),
        }
    }

    /// Write a stable, normalized CSV: one `frame` record and zero or more `pass` records per
    /// sample. Strings are escaped according to RFC 4180. No third-party CSV dependency is used.
    pub fn write_csv(&self, mut writer: impl Write) -> io::Result<()> {
        const HEADER: &[&str] = &[
            "record_type",
            "frame",
            "frame_cpu_ns",
            "average_frame_cpu_ns",
            "peak_frame_cpu_ns",
            "graph_cpu_ns",
            "gpu_shadow_and_mesh_ns",
            "gpu_mesh_pass_ns",
            "gpu_sample_sequence",
            "gpu_timestamp_queries_supported",
            "gpu_timestamp_samples",
            "gpu_timestamp_samples_skipped",
            "live_meshes",
            "live_materials",
            "mesh_bytes_estimate",
            "material_bytes_estimate",
            "buffer_bytes_estimate",
            "depth_target_bytes_estimate",
            "active_transient_textures",
            "cached_transient_textures",
            "active_transient_bytes_estimate",
            "cached_transient_bytes_estimate",
            "post_process_bytes_estimate",
            "context_target_bytes_estimate",
            "context_readback_bytes_estimate",
            "backend_peak_bytes_estimate",
            "transient_texture_allocations",
            "transient_texture_reuses",
            "transient_pool_peak_bytes_estimate",
            "transient_unknown_size_textures",
            "total_bytes_estimate",
            "pass_name",
            "pass_cpu_ns",
            "surface_acquire_ns",
            "present_ns",
            "surface_present_mode",
            "surface_frame_latency",
        ];
        write_csv_record(&mut writer, HEADER.iter().map(|field| (*field).to_owned()))?;
        for sample in &self.samples {
            let resources = &sample.resources;
            let mut row = vec![String::new(); HEADER.len()];
            row[0] = "frame".to_owned();
            row[1] = sample.frame.to_string();
            row[2] = sample.frame_cpu_ns.to_string();
            row[3] = sample.average_frame_cpu_ns.to_string();
            row[4] = sample.peak_frame_cpu_ns.to_string();
            row[5] = sample.graph_cpu_ns.to_string();
            row[6] = csv_option(sample.gpu_shadow_and_mesh_ns);
            row[7] = csv_option(sample.gpu_mesh_pass_ns);
            row[8] = csv_option(sample.gpu_sample_sequence);
            row[9] = sample.gpu_timestamp_queries_supported.to_string();
            row[10] = sample.gpu_timestamp_samples.to_string();
            row[11] = sample.gpu_timestamp_samples_skipped.to_string();
            row[12] = resources.live_meshes.to_string();
            row[13] = resources.live_materials.to_string();
            row[14] = resources.mesh_bytes_estimate.to_string();
            row[15] = resources.material_bytes_estimate.to_string();
            row[16] = resources.buffer_bytes_estimate.to_string();
            row[17] = resources.depth_target_bytes_estimate.to_string();
            row[18] = resources.active_transient_textures.to_string();
            row[19] = resources.cached_transient_textures.to_string();
            row[20] = resources.active_transient_bytes_estimate.to_string();
            row[21] = resources.cached_transient_bytes_estimate.to_string();
            row[22] = resources.post_process_bytes_estimate.to_string();
            row[23] = resources.context_target_bytes_estimate.to_string();
            row[24] = resources.context_readback_bytes_estimate.to_string();
            row[25] = resources.backend_peak_bytes_estimate.to_string();
            row[26] = resources.transient_texture_allocations.to_string();
            row[27] = resources.transient_texture_reuses.to_string();
            row[28] = resources.transient_pool_peak_bytes_estimate.to_string();
            row[29] = resources.transient_unknown_size_textures.to_string();
            row[30] = resources.total_bytes_estimate().to_string();
            row[33] = sample.surface_acquire_ns.to_string();
            row[34] = sample.present_ns.to_string();
            row[35] = sample.surface_present_mode.to_owned();
            row[36] = sample.surface_frame_latency.to_string();
            write_csv_record(&mut writer, row)?;
            for pass in &sample.passes {
                let mut pass_row = vec![String::new(); HEADER.len()];
                pass_row[0] = "pass".to_owned();
                pass_row[1] = sample.frame.to_string();
                pass_row[31] = pass.name.clone();
                pass_row[32] = pass.cpu_time_ns.to_string();
                write_csv_record(&mut writer, pass_row)?;
            }
        }
        writer.flush()
    }

    /// Create the parent directory and write the complete retained capture to the configured CSV.
    pub fn write_csv_file(&self) -> io::Result<()> {
        if let Some(parent) = self
            .output_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let file = File::create(&self.output_path)?;
        self.write_csv(BufWriter::new(file))
    }
}

/// Opt-in capture plugin. It samples [`RenderDiagnostics`] after `Stage::Render`, excludes a
/// configurable warmup window, and writes CSV at `Stage::Shutdown`. Install after `GpuWindowPlugin`
/// and `Render3dPlugin`; no frame history or file IO is added when this plugin is absent.
pub struct RenderProfilePlugin {
    output_path: PathBuf,
    capacity: usize,
    warmup_frames: u64,
}

impl RenderProfilePlugin {
    /// Create an opt-in profile capture that writes CSV when the application shuts down.
    /// Defaults to 3,600 retained samples and 120 warmup frames.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            output_path: path.into(),
            capacity: 3_600,
            warmup_frames: 120,
        }
    }

    /// Set the maximum retained samples (zero is clamped to one).
    pub fn max_samples(mut self, capacity: usize) -> Self {
        self.capacity = capacity.max(1);
        self
    }

    /// Set the number of initial successfully rendered frames to exclude.
    pub fn warmup_frames(mut self, frames: u64) -> Self {
        self.warmup_frames = frames;
        self
    }
}

impl Plugin for RenderProfilePlugin {
    fn name(&self) -> &'static str {
        "RenderProfilePlugin"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["GpuWindowPlugin", "Render3dPlugin"]
    }

    fn build(&self, app: &mut App) {
        app.insert_resource(RenderProfileRecorder::new(
            self.output_path.clone(),
            self.capacity,
            self.warmup_frames,
        ));
        app.add_system(Stage::Last, |world| {
            let Some(frame) = world
                .try_resource::<RenderDiagnostics>()
                .map(|diagnostics| diagnostics.frame_count)
            else {
                return;
            };
            let should_capture = world
                .try_resource::<RenderProfileRecorder>()
                .is_some_and(|recorder| recorder.should_capture_frame(frame));
            if should_capture {
                let diagnostics = world.resource::<RenderDiagnostics>().clone();
                world
                    .resource_mut::<RenderProfileRecorder>()
                    .record_diagnostics(&diagnostics);
            }
        });
        app.add_system(Stage::Shutdown, |world| {
            let Some(recorder) = world.try_resource::<RenderProfileRecorder>() else {
                return;
            };
            let output_path = recorder.output_path().to_path_buf();
            let summary = recorder.summary();
            let result = recorder.write_csv_file();
            match result {
                Ok(()) => {
                    println!(
                        "[RENDER PROFILE] wrote {} frame samples to {}",
                        summary.frame_count,
                        output_path.display()
                    );
                    if summary.frame_count > 0 {
                        println!(
                            "[RENDER PROFILE] frame wall mean/p95/peak = {:.3}/{:.3}/{:.3} ms; acquire p95 = {:.3} ms; graph CPU p95 = {:.3} ms; present p95 = {:.3} ms; tracked resource peak estimate = {:.2} MiB",
                            ns_to_ms(summary.frame_cpu.mean_ns),
                            ns_to_ms(summary.frame_cpu.p95_ns),
                            ns_to_ms(summary.frame_cpu.peak_ns),
                            ns_to_ms(summary.surface_acquire.p95_ns),
                            ns_to_ms(summary.graph_cpu.p95_ns),
                            ns_to_ms(summary.present.p95_ns),
                            summary.peak_tracked_bytes_estimate as f64 / (1024.0 * 1024.0)
                        );
                        println!(
                            "[RENDER PROFILE] GPU samples completed/skipped = {}/{}; transient texture allocations/reuses = {}/{}",
                            summary.gpu_timestamp_samples,
                            summary.gpu_timestamp_samples_skipped,
                            summary.transient_texture_allocations,
                            summary.transient_texture_reuses
                        );
                    } else {
                        eprintln!(
                            "[RENDER PROFILE] no post-warmup samples captured; increase --frames or reduce warmup"
                        );
                    }
                }
                Err(error) => eprintln!(
                    "[RENDER PROFILE] could not write {}: {error}",
                    output_path.display()
                ),
            }
        });
    }
}

fn ns_to_ms(nanoseconds: u64) -> f64 {
    nanoseconds as f64 / 1_000_000.0
}

fn csv_option<T: ToString>(value: Option<T>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn write_csv_record(
    writer: &mut impl Write,
    fields: impl IntoIterator<Item = String>,
) -> io::Result<()> {
    let mut first = true;
    for field in fields {
        if !first {
            writer.write_all(b",")?;
        }
        first = false;
        if field
            .bytes()
            .any(|byte| matches!(byte, b',' | b'"' | b'\r' | b'\n'))
        {
            writer.write_all(b"\"")?;
            writer.write_all(field.replace('"', "\"\"").as_bytes())?;
            writer.write_all(b"\"")?;
        } else {
            writer.write_all(field.as_bytes())?;
        }
    }
    writer.write_all(b"\r\n")
}
