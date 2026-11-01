//! Backend-neutral CPU timing and estimated GPU resource diagnostics.
//!
//! GPU timestamps and resource estimates are optional data supplied by the backend that owns
//! the underlying allocations. Byte counts are estimates, not driver-reported VRAM totals.

use ribbon_app::{App, Stage};
use ribbon_plugins::Plugin;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

pub use ribbon_profiling::{
    format_capture_written_line, write_key_value_line, CaptureConfig, CaptureSession, Counters,
    DurationSummary as RenderDurationSummary, TimingSamples,
};

mod csv;
mod types;
pub use types::*;

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

fn completed_gpu_durations(
    samples: &[&RenderProfileSample],
    source: &'static str,
    select: impl Fn(&RenderGpuTimingSample) -> Option<u64>,
) -> Vec<u64> {
    samples
        .iter()
        .flat_map(|sample| sample.completed_gpu_samples.iter())
        .filter(|sample| sample.source == source)
        .filter_map(select)
        .collect()
}

impl RenderProfileRecorder {
    /// Create a recorder with a bounded history and a fixed CSV output path.
    pub fn from_capture_config(config: CaptureConfig) -> Self {
        Self::new(
            config.output_path,
            config.sample_capacity,
            config.warmup_frames,
        )
    }

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
        let mut sample = RenderProfileSample::from_diagnostics(diagnostics);
        sample
            .completed_gpu_samples
            .retain(|gpu| gpu.submitted_frame > self.warmup_frames);
        self.samples.push_back(sample);
    }

    /// Attach the elapsed CPU cost of cloning/retaining one sample to that sample.
    pub fn record_profile_overhead(&mut self, cpu_time_ns: u64) {
        if let Some(sample) = self.samples.back_mut() {
            sample.cpu_phases.push(RenderPassTiming {
                name: "profile recorder bookkeeping".to_owned(),
                cpu_time_ns,
            });
        }
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
            gpu_shadow_and_mesh: RenderDurationSummary::from_values(completed_gpu_durations(
                &samples,
                "shadow_mesh",
                |sample| Some(sample.total_ns),
            )),
            gpu_mesh_pass: RenderDurationSummary::from_values(completed_gpu_durations(
                &samples,
                "shadow_mesh",
                |sample| sample.mesh_ns,
            )),
            gpu_shadow_pass: RenderDurationSummary::from_values(completed_gpu_durations(
                &samples,
                "shadow_mesh",
                |sample| sample.shadow_ns,
            )),
            gpu_bloom_pass: RenderDurationSummary::from_values(completed_gpu_durations(
                &samples,
                "bloom_tone_mapping",
                |sample| sample.bloom_ns,
            )),
            gpu_tone_mapping_pass: RenderDurationSummary::from_values(completed_gpu_durations(
                &samples,
                "bloom_tone_mapping",
                |sample| sample.tone_mapping_ns,
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
                let started = std::time::Instant::now();
                let diagnostics = world.resource::<RenderDiagnostics>().clone();
                let recorder = world.resource_mut::<RenderProfileRecorder>();
                recorder.record_diagnostics(&diagnostics);
                let elapsed = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                recorder.record_profile_overhead(elapsed);
            }
        });
        app.add_system(Stage::Shutdown, |world| {
            if let Some(diagnostics) = world.try_resource::<RenderDiagnostics>() {
                println!(
                    "[RENDER PROFILE ERRORS] surface_timeout={} surface_outdated={} surface_lost={} surface_other={} render_graph={}",
                    diagnostics.surface_timeouts,
                    diagnostics.surface_outdated,
                    diagnostics.surface_lost,
                    diagnostics.surface_other_errors,
                    diagnostics.render_graph_errors,
                );
            }
            let Some(recorder) = world.try_resource::<RenderProfileRecorder>() else {
                return;
            };
            let output_path = recorder.output_path().to_path_buf();
            let summary = recorder.summary();
            let result = recorder.write_csv_file();
            match result {
                Ok(()) => {
                    println!(
                        "{}",
                        format_capture_written_line(summary.frame_count, &output_path.display().to_string())
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
