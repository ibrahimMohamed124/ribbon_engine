#!/usr/bin/env python3
"""Validate and summarize repeated graphics_showcase scalability captures."""

from __future__ import annotations

import argparse
import csv
import re
import statistics
import sys
from collections import defaultdict
from pathlib import Path

import render_profile_report as profile

EXPECTED_ROWS = 541
EXPECTED = {
    ("small", "fifo", 64),
    ("small", "mailbox", 64),
    ("medium", "fifo", 576),
    ("medium", "mailbox", 576),
    ("large", "fifo", 1600),
    ("large", "mailbox", 1600),
}
REQUIRED_COLUMNS = {
    "frame_cpu_ns", "surface_acquire_ns", "graph_cpu_ns", "present_ns",
    "gpu_shadow_and_mesh_ns", "gpu_shadow_pass_ns", "gpu_mesh_pass_ns",
    "gpu_bloom_pass_ns", "gpu_tone_mapping_pass_ns", "gpu_sample_sequence",
    "gpu_post_sample_sequence", "gpu_timestamp_queries_supported",
    "gpu_timestamp_samples_skipped", "visible_mesh_instances", "visible_draw_calls",
    "shadow_draw_calls", "scene_model_placements", "total_bytes_estimate",
    "material_texture_bytes_estimate", "active_transient_bytes_estimate",
    "cached_transient_bytes_estimate", "cpu_asset_resident_bytes_estimate",
    "model_gpu_uploads_cumulative", "gpu_model_cache_entries",
    "renderer_instance_capacity",
}


def parse_capture_name(path: Path) -> tuple[str, str, int]:
    match = re.fullmatch(r"(small|medium|large)-(fifo|mailbox)-run(\d{2})", path.stem)
    if not match:
        raise ValueError(f"unexpected capture name: {path.name}")
    return match.group(1), match.group(2), int(match.group(3))


def metric_values(frames: list[dict[str, int | str | None]], passes: dict[str, list[int]]) -> dict[str, list[float]]:
    values = profile.metrics_for(frames, passes)
    for name, samples in passes.items():
        if name.startswith("CPU phase:"):
            values[name] = [float(sample) for sample in samples]
    values["Visible mesh instances"] = [float(row["visible_mesh_instances"]) for row in frames if row.get("visible_mesh_instances") is not None]
    values["Visible draw calls"] = [float(row["visible_draw_calls"]) for row in frames if row.get("visible_draw_calls") is not None]
    values["Shadow draw calls"] = [float(row["shadow_draw_calls"]) for row in frames if row.get("shadow_draw_calls") is not None]
    for label, field in {
        "Surface acquisition timeouts": "surface_timeouts",
        "Surface outdated reconfigurations": "surface_outdated",
        "Surface lost reconfigurations": "surface_lost",
        "Other surface errors": "surface_other_errors",
        "Render-graph errors": "render_graph_errors",
        "GPU samples pending at frame": "gpu_samples_pending",
        "GPU samples unavailable": "gpu_samples_unavailable",
        "GPU samples dropped": "gpu_samples_dropped",
    }.items():
        if all(row.get(field) is not None for row in frames):
            values[label] = [float(row[field]) for row in frames if row.get(field) is not None]
    values["Tracked GPU allocation estimate"] = [float(row["total_bytes_estimate"]) for row in frames if row.get("total_bytes_estimate") is not None]
    resource_metrics = {
        "Material texture allocation estimate": "material_texture_bytes_estimate",
        "Active transient texture estimate": "active_transient_bytes_estimate",
        "Cached transient texture estimate": "cached_transient_bytes_estimate",
        "CPU resident asset estimate": "cpu_asset_resident_bytes_estimate",
        "Cumulative model uploads": "model_gpu_uploads_cumulative",
        "GPU model cache entries": "gpu_model_cache_entries",
        "Renderer instance-buffer capacity": "renderer_instance_capacity",
    }
    for label, key in resource_metrics.items():
        values[label] = [float(row[key]) for row in frames if row.get(key) is not None]
    return values


def validate_capture(path: Path, workload: str, mode: str, repeat: int) -> tuple[list[dict[str, int | str | None]], dict[str, list[int]]]:
    frames, passes = profile.load_capture(path)
    with path.open("r", encoding="utf-8-sig", newline="") as source:
        header = set(csv.DictReader(source).fieldnames or [])
    missing = REQUIRED_COLUMNS - header
    if missing:
        raise ValueError(f"{path}: missing columns: {', '.join(sorted(missing))}")
    explicit_gpu_samples = "gpu_submission_frame" in header
    if explicit_gpu_samples:
        new_gpu_columns = {"gpu_samples_pending", "gpu_samples_unavailable", "gpu_samples_dropped"}
        missing_gpu = new_gpu_columns - header
        if missing_gpu:
            raise ValueError(f"{path}: incomplete explicit GPU diagnostics: {', '.join(sorted(missing_gpu))}")
    if len(frames) != EXPECTED_ROWS:
        raise ValueError(f"{path}: expected {EXPECTED_ROWS} frame rows, found {len(frames)}")
    frame_numbers = [int(row["frame"]) for row in frames]
    if any(right != left + 1 for left, right in zip(frame_numbers, frame_numbers[1:])):
        raise ValueError(f"{path}: frame sequence is not contiguous")
    for frame in frames:
        if frame.get("surface_present_mode") != mode or frame.get("surface_frame_latency") != 1:
            raise ValueError(f"{path}: mode/latency differs from requested {mode}/1")
        if frame.get("gpu_timestamp_queries_supported") != 1:
            raise ValueError(f"{path}: timestamp support is missing")
        if frame.get("gpu_timestamp_samples_skipped") != 0:
            raise ValueError(f"{path}: GPU timestamp samples were skipped")
        if explicit_gpu_samples and frame.get("gpu_samples_dropped") != 0:
            raise ValueError(f"{path}: GPU timestamp samples were dropped")
        if frame.get("scene_model_placements") != {"small": 64, "medium": 576, "large": 1600}[workload]:
            raise ValueError(f"{path}: scene placement count does not match {workload}")
        for key in ("frame_cpu_ns", "surface_acquire_ns", "graph_cpu_ns", "present_ns", "total_bytes_estimate"):
            value = frame.get(key)
            if not isinstance(value, int) or value < 0:
                raise ValueError(f"{path}: invalid {key} sample {value!r}")
    stage_counts = {
        metric: len(values)
        for metric, values in metric_values(frames, passes).items()
        if metric.startswith("GPU ") or metric.startswith("GPU sample:")
    }
    gpu_rows = []
    seen_gpu_sequences: set[tuple[str, int]] = set()
    with path.open("r", encoding="utf-8-sig", newline="") as source:
        for row in csv.DictReader(source):
            if row.get("record_type") != "gpu":
                continue
            sequence = int(row["gpu_sample_sequence"])
            source_name = row["pass_name"]
            submitted = int(row["gpu_submission_frame"])
            observed = int(row["frame"])
            if (source_name, sequence) in seen_gpu_sequences:
                raise ValueError(f"{path}: duplicate GPU sample sequence {source_name}/{sequence}")
            if submitted <= 0 or submitted > observed:
                raise ValueError(f"{path}: invalid GPU sample frame association {submitted}->{observed}")
            stage_columns = (
                ("gpu_shadow_and_mesh_ns", "gpu_shadow_pass_ns", "gpu_mesh_pass_ns")
                if source_name == "shadow_mesh"
                else ("gpu_bloom_pass_ns", "gpu_tone_mapping_pass_ns")
                if source_name == "bloom_tone_mapping"
                else ()
            )
            if not stage_columns or any(not row.get(column, "").strip() for column in stage_columns):
                raise ValueError(f"{path}: unknown GPU source or missing stage timing in {source_name}/{sequence}")
            seen_gpu_sequences.add((source_name, sequence))
            gpu_rows.append(row)
    if explicit_gpu_samples and not gpu_rows:
        raise ValueError(f"{path}: no explicit completed GPU sample records")
    # Async GPU coverage is reported from individual completion records; it is not expected to
    # equal the frame count because readbacks are nonblocking and outstanding results can remain.
    for metric, count in stage_counts.items():
        if metric.startswith("GPU") and count == 0:
            raise ValueError(f"{path}: no asynchronous samples for {metric}")

    log_path = path.with_suffix(".log")
    log = log_path.read_text(encoding="utf-8", errors="replace")
    for required in ("source_revision=", "source_dirty=", "variant=", "executable_sha256=", "command=", "capture_process_exit_code=0", "[GPU ADAPTER]", "[GPU SURFACE]", "[RENDER PROFILE ERRORS]"):
        if required not in log:
            raise ValueError(f"{log_path}: missing run provenance {required!r}")
    if "validation error" in log.lower() or "panicked at" in log.lower():
        raise ValueError(f"{log_path}: runtime validation error/panic detected")
    if f"model_placements={ {'small': 64, 'medium': 576, 'large': 1600}[workload] }" not in log:
        raise ValueError(f"{log_path}: workload parameters are missing")

    report_path = path.with_suffix(".md")
    report_text = report_path.read_text(encoding="utf-8")
    expected_rows = profile.summary_table(profile.metrics_for(frames, passes))
    for row in expected_rows[2:]:
        if row not in report_text:
            raise ValueError(f"{report_path}: report summary differs from raw CSV: {row}")
    return frames, passes


def build_outputs(capture_dir: Path, output_dir: Path) -> None:
    captures = sorted(capture_dir.glob("*-run*.csv"))
    runs = []
    seen: set[tuple[str, str, int]] = set()
    adapters: set[str] = set()
    executable_hashes: set[str] = set()
    resolutions: set[str] = set()
    latencies: set[str] = set()
    source_revisions: set[str] = set()
    dirty_states: set[str] = set()
    variants: set[str] = set()
    expected_keys = {(scene, mode, repeat) for scene, mode, _ in EXPECTED for repeat in range(1, 6)}
    for path in captures:
        scene, mode, repeat = parse_capture_name(path)
        key = (scene, mode, repeat)
        if key in seen:
            raise ValueError(f"duplicate capture: {key}")
        seen.add(key)
        frames, passes = validate_capture(path, scene, mode, repeat)
        capture_log = path.with_suffix(".log").read_text(encoding="utf-8", errors="replace")
        adapter_line = next((line for line in capture_log.splitlines() if line.startswith("[GPU ADAPTER]")), None)
        surface_line = next((line for line in capture_log.splitlines() if line.startswith("[GPU SURFACE]")), None)
        if adapter_line is None or surface_line is None:
            raise ValueError(f"{path}: missing adapter/surface provenance")
        adapters.add(adapter_line)
        exe_match = re.search(r"^executable_sha256=([0-9A-Fa-f]+)$", capture_log, flags=re.MULTILINE)
        resolution_match = re.search(r"(?:^|; )resolution=([^;]+)", surface_line)
        latency_match = re.search(r"desired_maximum_frame_latency=(\d+)", surface_line)
        if not exe_match or not resolution_match or not latency_match:
            raise ValueError(f"{path}: incomplete executable/resolution/latency provenance")
        executable_hashes.add(exe_match.group(1).lower())
        resolutions.add(resolution_match.group(1))
        latencies.add(latency_match.group(1))
        revision_match = re.search(r"^source_revision=(\S+)$", capture_log, flags=re.MULTILINE)
        dirty_match = re.search(r"^source_dirty=(\S+)$", capture_log, flags=re.MULTILINE)
        if not revision_match or not dirty_match:
            raise ValueError(f"{path}: incomplete source provenance")
        source_revisions.add(revision_match.group(1))
        dirty_states.add(dirty_match.group(1))
        variant_match = re.search(r"^variant=(\S+)$", capture_log, flags=re.MULTILINE)
        if not variant_match:
            raise ValueError(f"{path}: missing variant identity")
        variants.add(variant_match.group(1))
        for metric, values in metric_values(frames, passes).items():
            stats = profile.duration_summary(values)
            count_metrics = {"Visible mesh instances", "Visible draw calls", "Shadow draw calls", "Cumulative model uploads", "GPU model cache entries", "Renderer instance-buffer capacity", "GPU samples pending at frame", "GPU samples unavailable", "GPU samples dropped", "Surface acquisition timeouts", "Surface outdated reconfigurations", "Surface lost reconfigurations", "Other surface errors", "Render-graph errors"}
            resource_metrics = {"Tracked GPU allocation estimate", "Material texture allocation estimate", "Active transient texture estimate", "Cached transient texture estimate", "CPU resident asset estimate"}
            unit = "count" if metric in count_metrics else "bytes" if metric in resource_metrics else "ms"
            divisor = 1_000_000 if unit == "ms" else 1
            runs.append({
                "scene": scene, "placements": {"small": 64, "medium": 576, "large": 1600}[scene],
                "mode": mode, "repeat": repeat, "metric": metric, "unit": unit,
                "samples": stats["samples"],
                "mean": None if stats["mean"] is None else stats["mean"] / divisor,
                "p50": None if stats["p50"] is None else stats["p50"] / divisor,
                "p95": None if stats["p95"] is None else stats["p95"] / divisor,
                "p99": None if stats["p99"] is None else stats["p99"] / divisor,
            })
    if seen != expected_keys:
        missing = sorted(expected_keys - seen)
        extra = sorted(seen - expected_keys)
        raise ValueError(f"capture set incomplete; missing={missing}; unexpected={extra}")
    if any(len(values) != 1 for values in (adapters, executable_hashes, resolutions, latencies, source_revisions, dirty_states, variants)):
        raise ValueError(
            "capture run is not controlled: adapter, executable hash, resolution, or frame latency varies"
        )

    csv_path = output_dir / "run-level-results.csv"
    with csv_path.open("w", newline="", encoding="utf-8") as destination:
        writer = csv.DictWriter(destination, fieldnames=list(runs[0]))
        writer.writeheader()
        writer.writerows(runs)

    by_group: dict[tuple[str, str, str], list[dict[str, object]]] = defaultdict(list)
    for run in runs:
        by_group[(str(run["scene"]), str(run["mode"]), str(run["metric"]))].append(run)
    lines = [
        "# Rendering scalability capture results", "",
        "All values below are calculated from the validated raw frame CSVs. Five valid runs are included for every workload/mode. Percentiles use linear interpolation at `(n - 1) * p`. GPU timestamp samples are asynchronous, individually sequenced, and summarized once per unique sequence; per-stage sample counts are shown in the run-level CSV.", "",
        "## Run medians and variability", "",
        "Each cell shows the median of five run-level statistic values, with `[min, max]` across runs. Duration values are milliseconds; draw/instance metrics are counts; allocation estimates are bytes.", "",
        "| Workload | Mode | Metric | Run samples | Median mean | Median P50 | Median P95 [min,max] | Median P99 |", "|---|---|---|---:|---:|---:|---:|---:|"
    ]
    metric_order = [
        "Frame wall", "Surface acquisition wall", "Render graph CPU", "Surface presentation wall",
        "Frame remainder (setup/outside measured phases)", "GPU shadow + mesh", "GPU shadow passes",
        "GPU mesh pass", "GPU Bloom pass", "GPU tone-mapping pass", "Visible mesh instances",
        "Visible draw calls", "Shadow draw calls", "Tracked GPU allocation estimate",
        "Material texture allocation estimate", "Active transient texture estimate",
        "Cached transient texture estimate", "CPU resident asset estimate",
        "Cumulative model uploads", "GPU model cache entries", "Renderer instance-buffer capacity",
        "GPU samples pending at frame", "GPU samples unavailable", "GPU samples dropped",
        "Surface acquisition timeouts", "Surface outdated reconfigurations", "Surface lost reconfigurations",
        "Other surface errors", "Render-graph errors",
    ]
    metric_order.extend(sorted({
        str(run["metric"]) for run in runs if str(run["metric"]).startswith("CPU phase:")
    }))
    for scene in ("small", "medium", "large"):
        for mode in ("fifo", "mailbox"):
            for metric in metric_order:
                group = by_group[(scene, mode, metric)]
                if metric in {"GPU shadow + mesh", "GPU shadow passes", "GPU mesh pass", "GPU Bloom pass", "GPU tone-mapping pass"}:
                    run_counts = ", ".join(str(item["samples"]) for item in group)
                else:
                    run_counts = str(group[0]["samples"])
                fields = []
                for column in ("mean", "p50", "p95", "p99"):
                    numbers = [float(item[column]) for item in group if item[column] is not None]
                    middle = statistics.median(numbers)
                    if column == "p95":
                        fields.append(f"{middle:.3f} [{min(numbers):.3f}, {max(numbers):.3f}]")
                    else:
                        fields.append(f"{middle:.3f}")
                lines.append(f"| {scene} ({ {'small':64,'medium':576,'large':1600}[scene] } placements) | {mode} | {metric} | {run_counts} | {fields[0]} | {fields[1]} | {fields[2]} | {fields[3]} |")

    lines.extend(["", "## P95 change with scene size", "", "Positive means the metric increased in larger workload compared with small; negative means it decreased.", "", "| Mode | Metric | Small | Medium vs small | Large vs small |", "|---|---|---:|---:|---:|"])
    for mode in ("fifo", "mailbox"):
        for metric in metric_order[:10]:
            medians = {}
            for scene in ("small", "medium", "large"):
                medians[scene] = statistics.median(float(item["p95"]) for item in by_group[(scene, mode, metric)])
            medium_delta = (medians["medium"] / medians["small"] - 1) * 100 if medians["small"] else 0
            large_delta = (medians["large"] / medians["small"] - 1) * 100 if medians["small"] else 0
            lines.append(f"| {mode} | {metric} | {medians['small']:.3f} ms | {medium_delta:+.1f}% | {large_delta:+.1f}% |")
    lines.extend([
        "", "## Workload interpretation", "",
        "The benchmark holds the model asset, material, sun light, camera transform, output size, and runtime settings fixed while increasing placement count. It therefore isolates placement/instance-count scaling more cleanly than the earlier lifecycle stress workload. These results do not isolate multi-asset upload, texture-memory, or changing-light-count scaling. `Visible mesh instances` and grouped draw calls are renderer-observed counts; the showcase repeats one model/material, so draw-call count may remain nearly constant while per-instance work rises.", "",
        "GPU timestamp query availability and skipped counts are checked per capture. Query samples are read asynchronously and may be fewer than frame rows due to frame association/capture boundaries; sample counts by run appear in the CSV. Surface acquisition is pacing/compositor wall time and must not be interpreted as GPU render cost. Tracked allocation bytes are estimates rather than driver-reported VRAM.", "",
        f"- Validated captures: {len(captures)}; each has {EXPECTED_ROWS} contiguous frame samples.",
        f"- Capture CSVs and logs: `{capture_dir}`",
        f"- Run-level statistics CSV: `{csv_path}`",
    ])
    (output_dir / "scalability-results.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("capture_dir", type=Path)
    parser.add_argument("output_dir", type=Path)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    build_outputs(args.capture_dir, args.output_dir)
    print(f"Validated 30 captures; report: {args.output_dir / 'scalability-results.md'}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
