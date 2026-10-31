use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub(crate) struct FrameSample {
    pub(crate) frame: u64,
    pub(crate) update_wall_ns: u64,
    pub(crate) render_frame_cpu_ns: Option<u64>,
    pub(crate) streaming_update_ns: u64,
    pub(crate) cpu_resident_bytes: u64,
    pub(crate) cpu_pinned_bytes: u64,
    pub(crate) cpu_unpinned_bytes: u64,
    pub(crate) gpu_estimate_bytes: Option<u64>,
    pub(crate) active_regions: usize,
    pub(crate) loading_regions: usize,
    pub(crate) dormant_regions: usize,
    pub(crate) failed_regions: usize,
    pub(crate) queued_asset_jobs: usize,
    pub(crate) active_asset_workers: usize,
    pub(crate) pending_asset_completions: usize,
    pub(crate) asset_completions_processed_last_poll: usize,
    pub(crate) asset_source_read_ns_last_poll: u64,
    pub(crate) asset_decode_ns_last_poll: u64,
    pub(crate) gpu_uploads_this_frame: Option<usize>,
    pub(crate) gpu_upload_bytes_this_frame: Option<u64>,
    pub(crate) gpu_frame_count: Option<u64>,
    pub(crate) gpu_model_uploads_this_frame: Option<usize>,
    pub(crate) gpu_generic_uploads_this_frame: Option<usize>,
    /// Last asynchronously completed GPU mesh-pass timestamp, if supported by the adapter.
    pub(crate) gpu_mesh_pass_ns: Option<u64>,
    pub(crate) gpu_timestamp_queries_supported: Option<bool>,
    pub(crate) gpu_live_streamed_textures: Option<usize>,
    pub(crate) gpu_live_streamed_samplers: Option<usize>,
    pub(crate) gpu_streamed_texture_bytes_estimate: Option<u64>,
    pub(crate) gpu_generic_resources_released: Option<u64>,
    pub(crate) concurrent_regions_active: Option<usize>,
}

pub(crate) fn write_frame_csv(path: &Path, samples: &[FrameSample]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(File::create(path)?);
    writeln!(
        writer,
        "frame,update_wall_ns,render_frame_cpu_ns,streaming_update_ns,cpu_resident_bytes_estimate,cpu_pinned_bytes_estimate,cpu_unpinned_bytes_estimate,gpu_bytes_estimate,active_regions,loading_regions,dormant_regions,failed_regions,queued_asset_jobs,active_asset_workers,pending_asset_completions,asset_completions_processed_last_poll,asset_source_read_ns_last_poll,asset_decode_ns_last_poll,gpu_uploads_this_frame,gpu_upload_bytes_this_frame,gpu_frame_count,gpu_model_uploads_this_frame,gpu_generic_uploads_this_frame,gpu_mesh_pass_timestamp_ns,gpu_timestamp_queries_supported,gpu_live_streamed_textures,gpu_live_streamed_samplers,gpu_streamed_texture_bytes_estimate,gpu_generic_resources_released,concurrent_regions_active"
    )?;
    for sample in samples {
        writeln!(
            writer,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            sample.frame,
            sample.update_wall_ns,
            optional(sample.render_frame_cpu_ns),
            sample.streaming_update_ns,
            sample.cpu_resident_bytes,
            sample.cpu_pinned_bytes,
            sample.cpu_unpinned_bytes,
            optional(sample.gpu_estimate_bytes),
            sample.active_regions,
            sample.loading_regions,
            sample.dormant_regions,
            sample.failed_regions,
            sample.queued_asset_jobs,
            sample.active_asset_workers,
            sample.pending_asset_completions,
            sample.asset_completions_processed_last_poll,
            sample.asset_source_read_ns_last_poll,
            sample.asset_decode_ns_last_poll,
            optional(sample.gpu_uploads_this_frame),
            optional(sample.gpu_upload_bytes_this_frame),
            optional(sample.gpu_frame_count),
            optional(sample.gpu_model_uploads_this_frame),
            optional(sample.gpu_generic_uploads_this_frame),
            optional(sample.gpu_mesh_pass_ns),
            optional(sample.gpu_timestamp_queries_supported),
            optional(sample.gpu_live_streamed_textures),
            optional(sample.gpu_live_streamed_samplers),
            optional(sample.gpu_streamed_texture_bytes_estimate),
            optional(sample.gpu_generic_resources_released),
            optional(sample.concurrent_regions_active),
        )?;
    }
    writer.flush()?;
    Ok(())
}

fn optional<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

pub(crate) fn percentile(values: &[u64], percentile: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = ((percentile.clamp(0.0, 1.0) * sorted.len() as f64).ceil() as usize)
        .max(1)
        .min(sorted.len());
    sorted[rank - 1]
}

pub(crate) fn write_markdown_report(
    path: &Path,
    title: &str,
    metadata: &[(String, String)],
    checks: &[(String, String)],
    samples: &[FrameSample],
    activation_latencies_ns: &[u64],
) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(File::create(path)?);
    writeln!(writer, "# {title}\n")?;
    writeln!(writer, "## Run configuration\n")?;
    writeln!(writer, "| Key | Value |\n|---|---|")?;
    for (key, value) in metadata {
        writeln!(writer, "| {} | {} |", escape_cell(key), escape_cell(value))?;
    }
    writeln!(writer, "\n## Checks\n")?;
    writeln!(writer, "| Check | Result |\n|---|---|")?;
    for (key, value) in checks {
        writeln!(writer, "| {} | {} |", escape_cell(key), escape_cell(value))?;
    }

    let frame_wall: Vec<_> = samples.iter().map(|sample| sample.update_wall_ns).collect();
    let render_wall: Vec<_> = samples
        .iter()
        .filter_map(|sample| sample.render_frame_cpu_ns)
        .collect();
    let stream_cpu: Vec<_> = samples
        .iter()
        .map(|sample| sample.streaming_update_ns)
        .filter(|value| *value > 0)
        .collect();
    let source_read: Vec<_> = samples
        .iter()
        .map(|sample| sample.asset_source_read_ns_last_poll)
        .filter(|value| *value > 0)
        .collect();
    let decode: Vec<_> = samples
        .iter()
        .map(|sample| sample.asset_decode_ns_last_poll)
        .filter(|value| *value > 0)
        .collect();
    let cpu_peak = samples
        .iter()
        .map(|sample| sample.cpu_resident_bytes)
        .max()
        .unwrap_or(0);
    let gpu_peak = samples
        .iter()
        .filter_map(|sample| sample.gpu_estimate_bytes)
        .max()
        .unwrap_or(0);
    writeln!(writer, "\n## Timing and residency\n")?;
    writeln!(
        writer,
        "| Metric | p50 | p95 | p99 | Peak |\n|---|---:|---:|---:|---:|"
    )?;
    writeln!(
        writer,
        "| App update wall time (ms) | {:.3} | {:.3} | {:.3} | {:.3} |",
        percentile(&frame_wall, 0.50) as f64 / 1_000_000.0,
        percentile(&frame_wall, 0.95) as f64 / 1_000_000.0,
        percentile(&frame_wall, 0.99) as f64 / 1_000_000.0,
        frame_wall.iter().copied().max().unwrap_or(0) as f64 / 1_000_000.0,
    )?;
    writeln!(
        writer,
        "| Render frame CPU time (ms) | {:.3} | {:.3} | {:.3} | {:.3} |",
        percentile(&render_wall, 0.50) as f64 / 1_000_000.0,
        percentile(&render_wall, 0.95) as f64 / 1_000_000.0,
        percentile(&render_wall, 0.99) as f64 / 1_000_000.0,
        render_wall.iter().copied().max().unwrap_or(0) as f64 / 1_000_000.0,
    )?;
    writeln!(
        writer,
        "| Streaming coordinator CPU (ms) | {:.3} | {:.3} | {:.3} | {:.3} |",
        percentile(&stream_cpu, 0.50) as f64 / 1_000_000.0,
        percentile(&stream_cpu, 0.95) as f64 / 1_000_000.0,
        percentile(&stream_cpu, 0.99) as f64 / 1_000_000.0,
        stream_cpu.iter().copied().max().unwrap_or(0) as f64 / 1_000_000.0,
    )?;
    if !source_read.is_empty() {
        writeln!(
            writer,
            "| Asset source-read time per poll (ms) | {:.3} | {:.3} | {:.3} | {:.3} |",
            percentile(&source_read, 0.50) as f64 / 1_000_000.0,
            percentile(&source_read, 0.95) as f64 / 1_000_000.0,
            percentile(&source_read, 0.99) as f64 / 1_000_000.0,
            source_read.iter().copied().max().unwrap_or(0) as f64 / 1_000_000.0,
        )?;
    }
    if !decode.is_empty() {
        writeln!(
            writer,
            "| Asset decode time per poll (ms) | {:.3} | {:.3} | {:.3} | {:.3} |",
            percentile(&decode, 0.50) as f64 / 1_000_000.0,
            percentile(&decode, 0.95) as f64 / 1_000_000.0,
            percentile(&decode, 0.99) as f64 / 1_000_000.0,
            decode.iter().copied().max().unwrap_or(0) as f64 / 1_000_000.0,
        )?;
    }
    if !activation_latencies_ns.is_empty() {
        writeln!(
            writer,
            "| Region activation latency (ms) | {:.3} | {:.3} | {:.3} | {:.3} |",
            percentile(activation_latencies_ns, 0.50) as f64 / 1_000_000.0,
            percentile(activation_latencies_ns, 0.95) as f64 / 1_000_000.0,
            percentile(activation_latencies_ns, 0.99) as f64 / 1_000_000.0,
            activation_latencies_ns.iter().copied().max().unwrap_or(0) as f64 / 1_000_000.0,
        )?;
    }
    writeln!(
        writer,
        "\n- Peak decoded CPU residency estimate: {:.2} MiB",
        cpu_peak as f64 / 1_048_576.0
    )?;
    if samples
        .iter()
        .any(|sample| sample.gpu_estimate_bytes.is_some())
    {
        writeln!(
            writer,
            "- Peak renderer GPU allocation estimate: {:.2} MiB",
            gpu_peak as f64 / 1_048_576.0
        )?;
        writeln!(
            writer,
            "- GPU memory values are renderer estimates, not driver-reported physical VRAM."
        )?;
    } else {
        writeln!(writer, "- GPU measurements: not available in this headless phase; no simulated GPU success is reported.")?;
    }
    writeln!(writer, "\n## Interpretation\n")?;
    writeln!(writer, "- CPU asset memory is measured using AssetServer decoded-asset estimates and strong-lease accounting.")?;
    writeln!(writer, "- Worker CSV columns report queued jobs, active workers, completion backlog, and source-read/decode time processed by the latest AssetServer poll. The timing table summarizes non-zero per-poll source-read/decode samples; these are worker-poll aggregates, not per-file latency distributions.")?;
    writeln!(writer, "- GPU residency and upload traffic are renderer estimates and must not be interpreted as exact VRAM usage.")?;
    writeln!(writer, "- Render frame CPU measurements are present only when an actual GPU window frame was rendered.")?;
    writeln!(writer, "- GPU mesh-pass timestamps are asynchronous completed samples; repeated values can refer to an earlier submitted frame and are not frame-aligned hardware counters.")?;
    writeln!(writer, "- Headless/update wall timing is CPU-side `App::update` wall time; it is not a GPU frame-time measurement.")?;
    writeln!(writer, "- Upload byte budgets permit one oversized first upload in a frame to avoid starvation; later uploads must fit the remaining byte budget.")?;
    writer.flush()?;
    Ok(())
}

fn escape_cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::{write_frame_csv, FrameSample};
    use std::fs;

    #[test]
    fn markdown_report_exposes_asset_worker_io_and_decode_poll_timings() {
        let path = std::env::temp_dir().join(format!(
            "ribbon-streaming-stress-report-{}-timings.md",
            std::process::id()
        ));
        let sample = FrameSample {
            asset_source_read_ns_last_poll: 2_000_000,
            asset_decode_ns_last_poll: 3_000_000,
            ..FrameSample::default()
        };
        super::write_markdown_report(&path, "worker timings", &[], &[], &[sample], &[])
            .expect("write markdown report");
        let content = fs::read_to_string(&path).expect("read markdown report");
        assert!(content.contains("Asset source-read time per poll (ms)"));
        assert!(content.contains("Asset decode time per poll (ms)"));
        assert!(content.contains("not per-file latency distributions"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn frame_csv_has_stable_columns_and_blank_gpu_values_for_headless_samples() {
        let path = std::env::temp_dir().join(format!(
            "ribbon-streaming-stress-report-{}.csv",
            std::process::id()
        ));
        let sample = FrameSample {
            frame: 7,
            update_wall_ns: 1234,
            cpu_resident_bytes: 4096,
            ..FrameSample::default()
        };
        write_frame_csv(&path, &[sample]).expect("write frame CSV");
        let content = fs::read_to_string(&path).expect("read frame CSV");
        let mut lines = content.lines();
        let header = lines.next().expect("CSV header");
        let row = lines.next().expect("CSV row");
        assert_eq!(header.split(',').count(), 30);
        let fields: Vec<_> = row.split(',').collect();
        assert_eq!(fields.len(), 30);
        assert!(fields[25..].iter().all(|field| field.is_empty()));
        let _ = fs::remove_file(path);
    }
}
