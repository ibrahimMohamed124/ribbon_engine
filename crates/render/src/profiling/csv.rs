use super::*;

impl RenderProfileRecorder {
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
            "live_material_textures",
            "material_texture_bytes_estimate",
            "material_uniform_bytes_estimate",
            "material_texture_cache_hits",
            "material_texture_cache_misses",
            "scene_mesh_instances",
            "scene_model_placements",
            "gpu_model_cache_entries",
            "model_gpu_uploads_cumulative",
            "model_gpu_releases_cumulative",
            "renderer_instance_capacity",
            "renderer_instance_buffer_grows_cumulative",
            "renderer_instance_buffer_shrinks_cumulative",
            "cpu_asset_resident_bytes_estimate",
            "cpu_asset_pinned_bytes_estimate",
            "cpu_asset_unpinned_bytes_estimate",
            "cpu_asset_loaded_count",
            "cpu_asset_pinned_count",
            "cpu_asset_budget_enabled",
            "cpu_asset_budget_bytes",
            "cpu_asset_over_budget_bytes",
            "cpu_asset_budget_evictions_cumulative",
            "gpu_shadow_pass_ns",
            "gpu_bloom_pass_ns",
            "gpu_tone_mapping_pass_ns",
            "gpu_post_sample_sequence",
            "visible_mesh_instances",
            "visible_draw_calls",
            "shadow_draw_calls",
            "gpu_samples_pending",
            "gpu_samples_unavailable",
            "gpu_samples_dropped",
            "gpu_submission_frame",
            "surface_timeouts",
            "surface_outdated",
            "surface_lost",
            "surface_other_errors",
            "render_graph_errors",
            "model_warmup_requested",
            "model_warmup_waiting_for_cpu",
            "model_warmup_waiting_for_gpu",
            "model_warmup_ready",
            "model_warmup_failed",
            "model_gpu_uploads_this_frame",
            "streaming_region_count",
            "streaming_loading_regions",
            "streaming_ready_regions",
            "streaming_active_regions",
            "streaming_dormant_regions",
            "streaming_failed_regions",
            "streaming_asset_jobs_queued",
            "streaming_asset_workers_active",
            "streaming_cpu_update_ns",
            "streaming_cpu_memory_bytes_estimate",
            "streaming_gpu_memory_bytes_estimate",
            "streaming_memory_pressure_events",
            "streaming_asset_requests_pending",
            "streaming_cpu_budget_evictions_cumulative",
            "streaming_invalid_state_transitions",
            "streaming_asset_queue_capacity",
            "streaming_cpu_resident_budget_bytes",
            "streaming_cpu_resident_over_budget_bytes",
            "streaming_cpu_cache_budget_bytes",
            "streaming_cpu_cache_over_budget_bytes",
            "model_gpu_upload_bytes_this_frame",
            "asset_worker_source_read_ns_last_poll",
            "asset_worker_decode_ns_last_poll",
            "asset_worker_completions_processed_last_poll",
            "asset_worker_pending_completions",
            "generic_gpu_warmup_queued",
            "generic_gpu_warmup_preparing",
            "generic_gpu_warmup_ready",
            "generic_gpu_warmup_failed",
            "generic_gpu_uploads_cumulative",
            "generic_gpu_upload_failures_cumulative",
            "generic_gpu_releases_cumulative",
            "generic_gpu_uploads_this_frame",
            "generic_gpu_upload_bytes_this_frame",
            "live_streamed_textures",
            "live_streamed_samplers",
            "streamed_texture_bytes_estimate",
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
            row[66] = sample.gpu_samples_pending.to_string();
            row[67] = sample.gpu_samples_unavailable.to_string();
            row[68] = sample.gpu_samples_dropped.to_string();
            row[70] = sample.surface_timeouts.to_string();
            row[71] = sample.surface_outdated.to_string();
            row[72] = sample.surface_lost.to_string();
            row[73] = sample.surface_other_errors.to_string();
            row[74] = sample.render_graph_errors.to_string();
            row[75] = resources.model_warmup_requested.to_string();
            row[76] = resources.model_warmup_waiting_for_cpu.to_string();
            row[77] = resources.model_warmup_waiting_for_gpu.to_string();
            row[78] = resources.model_warmup_ready.to_string();
            row[79] = resources.model_warmup_failed.to_string();
            row[80] = resources.model_gpu_uploads_this_frame.to_string();
            row[81] = resources.streaming_region_count.to_string();
            row[82] = resources.streaming_loading_regions.to_string();
            row[83] = resources.streaming_ready_regions.to_string();
            row[84] = resources.streaming_active_regions.to_string();
            row[85] = resources.streaming_dormant_regions.to_string();
            row[86] = resources.streaming_failed_regions.to_string();
            row[87] = resources.streaming_asset_jobs_queued.to_string();
            row[88] = resources.streaming_asset_workers_active.to_string();
            row[89] = resources.streaming_cpu_update_ns.to_string();
            row[90] = resources.streaming_cpu_memory_bytes_estimate.to_string();
            row[91] = resources.streaming_gpu_memory_bytes_estimate.to_string();
            row[92] = resources.streaming_memory_pressure_events.to_string();
            row[93] = resources.streaming_asset_requests_pending.to_string();
            row[94] = resources
                .streaming_cpu_budget_evictions_cumulative
                .to_string();
            row[95] = resources.streaming_invalid_state_transitions.to_string();
            row[96] = resources.streaming_asset_queue_capacity.to_string();
            row[97] = resources.streaming_cpu_resident_budget_bytes.to_string();
            row[98] = resources
                .streaming_cpu_resident_over_budget_bytes
                .to_string();
            row[99] = resources.streaming_cpu_cache_budget_bytes.to_string();
            row[100] = resources.streaming_cpu_cache_over_budget_bytes.to_string();
            row[101] = resources.model_gpu_upload_bytes_this_frame.to_string();
            row[102] = resources.asset_worker_source_read_ns_last_poll.to_string();
            row[103] = resources.asset_worker_decode_ns_last_poll.to_string();
            row[104] = resources
                .asset_worker_completions_processed_last_poll
                .to_string();
            row[105] = resources.asset_worker_pending_completions.to_string();
            row[106] = resources.generic_gpu_warmup_queued.to_string();
            row[107] = resources.generic_gpu_warmup_preparing.to_string();
            row[108] = resources.generic_gpu_warmup_ready.to_string();
            row[109] = resources.generic_gpu_warmup_failed.to_string();
            row[110] = resources.generic_gpu_uploads_cumulative.to_string();
            row[111] = resources.generic_gpu_upload_failures_cumulative.to_string();
            row[112] = resources.generic_gpu_releases_cumulative.to_string();
            row[113] = resources.generic_gpu_uploads_this_frame.to_string();
            row[114] = resources.generic_gpu_upload_bytes_this_frame.to_string();
            row[115] = resources.live_streamed_textures.to_string();
            row[116] = resources.live_streamed_samplers.to_string();
            row[117] = resources.streamed_texture_bytes_estimate.to_string();
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
            row[37] = resources.live_material_textures.to_string();
            row[38] = resources.material_texture_bytes_estimate.to_string();
            row[39] = resources.material_uniform_bytes_estimate.to_string();
            row[40] = resources.material_texture_cache_hits.to_string();
            row[41] = resources.material_texture_cache_misses.to_string();
            row[42] = resources.scene_mesh_instances.to_string();
            row[43] = resources.scene_model_placements.to_string();
            row[44] = resources.gpu_model_cache_entries.to_string();
            row[45] = resources.model_gpu_uploads_cumulative.to_string();
            row[46] = resources.model_gpu_releases_cumulative.to_string();
            row[47] = resources.renderer_instance_capacity.to_string();
            row[48] = resources
                .renderer_instance_buffer_grows_cumulative
                .to_string();
            row[49] = resources
                .renderer_instance_buffer_shrinks_cumulative
                .to_string();
            row[50] = resources.cpu_asset_resident_bytes_estimate.to_string();
            row[51] = resources.cpu_asset_pinned_bytes_estimate.to_string();
            row[52] = resources.cpu_asset_unpinned_bytes_estimate.to_string();
            row[53] = resources.cpu_asset_loaded_count.to_string();
            row[54] = resources.cpu_asset_pinned_count.to_string();
            row[55] = resources.cpu_asset_budget_enabled.to_string();
            if resources.cpu_asset_budget_enabled {
                row[56] = resources.cpu_asset_budget_bytes.to_string();
            }
            row[57] = resources.cpu_asset_over_budget_bytes.to_string();
            row[58] = resources.cpu_asset_budget_evictions_cumulative.to_string();
            row[59] = csv_option(sample.gpu_shadow_pass_ns);
            row[60] = csv_option(sample.gpu_bloom_pass_ns);
            row[61] = csv_option(sample.gpu_tone_mapping_pass_ns);
            row[62] = csv_option(sample.gpu_post_sample_sequence);
            row[63] = resources.visible_mesh_instances.to_string();
            row[64] = resources.visible_draw_calls.to_string();
            row[65] = resources.shadow_draw_calls.to_string();
            write_csv_record(&mut writer, row)?;
            for pass in &sample.passes {
                let mut pass_row = vec![String::new(); HEADER.len()];
                pass_row[0] = "pass".to_owned();
                pass_row[1] = sample.frame.to_string();
                pass_row[31] = pass.name.clone();
                pass_row[32] = pass.cpu_time_ns.to_string();
                write_csv_record(&mut writer, pass_row)?;
            }
            for phase in &sample.cpu_phases {
                let mut phase_row = vec![String::new(); HEADER.len()];
                phase_row[0] = "phase".to_owned();
                phase_row[1] = sample.frame.to_string();
                phase_row[31] = phase.name.clone();
                phase_row[32] = phase.cpu_time_ns.to_string();
                write_csv_record(&mut writer, phase_row)?;
            }
            for gpu in &sample.completed_gpu_samples {
                let mut gpu_row = vec![String::new(); HEADER.len()];
                gpu_row[0] = "gpu".to_owned();
                gpu_row[1] = gpu.observed_frame.to_string();
                gpu_row[6] = gpu.total_ns.to_string();
                gpu_row[7] = csv_option(gpu.mesh_ns);
                gpu_row[8] = gpu.sequence.to_string();
                gpu_row[69] = gpu.submitted_frame.to_string();
                gpu_row[31] = gpu.source.to_owned();
                gpu_row[59] = csv_option(gpu.shadow_ns);
                gpu_row[60] = csv_option(gpu.bloom_ns);
                gpu_row[61] = csv_option(gpu.tone_mapping_ns);
                if gpu.source == "bloom_tone_mapping" {
                    gpu_row[62] = gpu.sequence.to_string();
                }
                write_csv_record(&mut writer, gpu_row)?;
            }
        }
        writer.flush()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_keeps_existing_fields_and_appends_warmup_columns_without_reordering() {
        let mut recorder = RenderProfileRecorder::new("unused-test-profile.csv", 2, 0);
        let mut diagnostics = RenderDiagnostics {
            frame_count: 1,
            ..RenderDiagnostics::default()
        };
        diagnostics.resources.model_warmup_requested = 7;
        diagnostics.resources.model_gpu_uploads_this_frame = 2;
        diagnostics.resources.model_gpu_upload_bytes_this_frame = 8 * 1024 * 1024;
        diagnostics.resources.asset_worker_source_read_ns_last_poll = 11;
        diagnostics.resources.asset_worker_decode_ns_last_poll = 22;
        diagnostics
            .resources
            .asset_worker_completions_processed_last_poll = 3;
        diagnostics.resources.asset_worker_pending_completions = 4;
        diagnostics.resources.streaming_region_count = 3;
        diagnostics.resources.streaming_loading_regions = 1;
        diagnostics.resources.streaming_cpu_memory_bytes_estimate = 4096;
        diagnostics.resources.streaming_asset_requests_pending = 2;
        diagnostics
            .resources
            .streaming_cpu_budget_evictions_cumulative = 5;
        diagnostics.resources.streaming_invalid_state_transitions = 1;
        diagnostics.resources.streaming_asset_queue_capacity = 256;
        diagnostics.resources.streaming_cpu_resident_budget_bytes = 2 * 1024 * 1024 * 1024;
        diagnostics
            .resources
            .streaming_cpu_resident_over_budget_bytes = 128;
        diagnostics.resources.streaming_cpu_cache_budget_bytes = 1024;
        diagnostics.resources.streaming_cpu_cache_over_budget_bytes = 64;
        diagnostics.resources.generic_gpu_warmup_queued = 6;
        diagnostics.resources.generic_gpu_warmup_preparing = 5;
        diagnostics.resources.generic_gpu_warmup_ready = 4;
        diagnostics.resources.generic_gpu_warmup_failed = 3;
        diagnostics.resources.generic_gpu_uploads_cumulative = 12;
        diagnostics.resources.generic_gpu_upload_failures_cumulative = 2;
        diagnostics.resources.generic_gpu_releases_cumulative = 7;
        diagnostics.resources.generic_gpu_uploads_this_frame = 2;
        diagnostics.resources.generic_gpu_upload_bytes_this_frame = 16 * 1024 * 1024;
        diagnostics.resources.live_streamed_textures = 3;
        diagnostics.resources.live_streamed_samplers = 2;
        diagnostics.resources.streamed_texture_bytes_estimate = 4096;
        recorder.record_diagnostics(&diagnostics);

        let mut output = Vec::new();
        recorder
            .write_csv(&mut output)
            .expect("CSV serialization succeeds");
        let text = String::from_utf8(output).expect("CSV is UTF-8");
        let mut rows = text.lines();
        let header: Vec<_> = rows.next().expect("header row").split(',').collect();
        let frame: Vec<_> = rows.next().expect("frame row").split(',').collect();

        assert_eq!(header.len(), 118);
        assert_eq!(frame.len(), header.len());
        assert_eq!(header[75], "model_warmup_requested");
        assert_eq!(header[80], "model_gpu_uploads_this_frame");
        assert_eq!(header[81], "streaming_region_count");
        assert_eq!(header[92], "streaming_memory_pressure_events");
        assert_eq!(header[93], "streaming_asset_requests_pending");
        assert_eq!(header[94], "streaming_cpu_budget_evictions_cumulative");
        assert_eq!(header[95], "streaming_invalid_state_transitions");
        assert_eq!(header[96], "streaming_asset_queue_capacity");
        assert_eq!(header[97], "streaming_cpu_resident_budget_bytes");
        assert_eq!(header[98], "streaming_cpu_resident_over_budget_bytes");
        assert_eq!(header[99], "streaming_cpu_cache_budget_bytes");
        assert_eq!(header[100], "streaming_cpu_cache_over_budget_bytes");
        assert_eq!(header[101], "model_gpu_upload_bytes_this_frame");
        assert_eq!(header[102], "asset_worker_source_read_ns_last_poll");
        assert_eq!(header[103], "asset_worker_decode_ns_last_poll");
        assert_eq!(header[104], "asset_worker_completions_processed_last_poll");
        assert_eq!(header[105], "asset_worker_pending_completions");
        assert_eq!(header[106], "generic_gpu_warmup_queued");
        assert_eq!(header[107], "generic_gpu_warmup_preparing");
        assert_eq!(header[108], "generic_gpu_warmup_ready");
        assert_eq!(header[109], "generic_gpu_warmup_failed");
        assert_eq!(header[110], "generic_gpu_uploads_cumulative");
        assert_eq!(header[111], "generic_gpu_upload_failures_cumulative");
        assert_eq!(header[112], "generic_gpu_releases_cumulative");
        assert_eq!(header[113], "generic_gpu_uploads_this_frame");
        assert_eq!(header[114], "generic_gpu_upload_bytes_this_frame");
        assert_eq!(header[115], "live_streamed_textures");
        assert_eq!(header[116], "live_streamed_samplers");
        assert_eq!(header[117], "streamed_texture_bytes_estimate");
        assert_eq!(frame[115], "3");
        assert_eq!(frame[116], "2");
        assert_eq!(frame[117], "4096");
        assert_eq!(frame[75], "7");
        assert_eq!(frame[80], "2");
        assert_eq!(frame[81], "3");
        assert_eq!(frame[101], (8 * 1024 * 1024).to_string());
        assert_eq!(frame[102], "11");
        assert_eq!(frame[103], "22");
        assert_eq!(frame[104], "3");
        assert_eq!(frame[105], "4");
        assert_eq!(frame[106], "6");
        assert_eq!(frame[107], "5");
        assert_eq!(frame[108], "4");
        assert_eq!(frame[109], "3");
        assert_eq!(frame[110], "12");
        assert_eq!(frame[111], "2");
        assert_eq!(frame[112], "7");
        assert_eq!(frame[113], "2");
        assert_eq!(frame[114], (16 * 1024 * 1024).to_string());
        assert_eq!(frame[82], "1");
        assert_eq!(frame[90], "4096");
        assert_eq!(frame[93], "2");
        assert_eq!(frame[94], "5");
        assert_eq!(frame[97], "2147483648");
        assert_eq!(frame[98], "128");
        assert_eq!(frame[99], "1024");
        assert_eq!(frame[100], "64");
        assert_eq!(frame[95], "1");
        assert_eq!(frame[96], "256");
    }
}
