#!/usr/bin/env python3
"""Summarize Ribbon Engine render-profile CSV captures into a Markdown report.

The CSV is emitted by engine_render::profiling::RenderProfilePlugin. This utility uses only
Python's standard library so it does not add any engine/runtime dependency.
"""

from __future__ import annotations

import argparse
import csv
import math
import platform
import statistics
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable


def percentile(values: Iterable[float], percent: float) -> float | None:
    ordered = sorted(values)
    if not ordered:
        return None
    # Match compare_profile_runs.py: linear interpolation at (n - 1) * p.
    # Keeping one convention makes per-capture and across-run percentiles agree.
    position = (len(ordered) - 1) * (percent / 100.0)
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    fraction = position - lower
    return ordered[lower] * (1.0 - fraction) + ordered[upper] * fraction


def duration_summary(values: list[float]) -> dict[str, float | int | None]:
    return {
        "samples": len(values),
        "mean": statistics.fmean(values) if values else None,
        "p50": percentile(values, 50),
        "p95": percentile(values, 95),
        "p99": percentile(values, 99),
        "peak": max(values) if values else None,
    }


def number(record: dict[str, str], key: str) -> int | None:
    value = (record.get(key) or "").strip()
    if not value:
        return None
    try:
        return int(value)
    except ValueError as exc:
        raise ValueError(f"invalid integer in column {key!r}: {value!r}") from exc


def load_capture(path: Path) -> tuple[list[dict[str, int | str | None]], dict[str, list[int]]]:
    frames: list[dict[str, int | str | None]] = []
    pass_values: dict[str, list[int]] = defaultdict(list)
    with path.open("r", encoding="utf-8-sig", newline="") as source:
        reader = csv.DictReader(source)
        required = {
            "record_type",
            "frame",
            "frame_cpu_ns",
            "graph_cpu_ns",
            "gpu_sample_sequence",
            "gpu_shadow_and_mesh_ns",
            "gpu_mesh_pass_ns",
            "total_bytes_estimate",
            "peak_frame_cpu_ns",
            "transient_texture_allocations",
            "transient_texture_reuses",
        }
        missing = required.difference(reader.fieldnames or [])
        if missing:
            raise ValueError(f"{path}: missing required columns: {', '.join(sorted(missing))}")
        for row in reader:
            record_type = row.get("record_type", "")
            if record_type == "frame":
                parsed: dict[str, int | str | None] = {"record_type": "frame"}
                for key in (reader.fieldnames or []):
                    if key in {"record_type", "pass_name", "surface_present_mode"}:
                        parsed[key] = row.get(key, "")
                    elif key in {"gpu_timestamp_queries_supported", "cpu_asset_budget_enabled"}:
                        parsed[key] = 1 if (row.get(key) or "").strip().lower() == "true" else 0
                    else:
                        parsed[key] = number(row, key)
                frames.append(parsed)
            elif record_type == "pass":
                name = row.get("pass_name", "").strip()
                value = number(row, "pass_cpu_ns")
                if name and value is not None:
                    pass_values[f"Render graph: {name}"].append(value)
            elif record_type == "phase":
                name = row.get("pass_name", "").strip()
                value = number(row, "pass_cpu_ns")
                if name and value is not None:
                    pass_values[f"CPU phase: {name}"].append(value)
            elif record_type == "gpu":
                source_name = row.get("pass_name", "").strip()
                for key, label in (
                    ("gpu_shadow_and_mesh_ns", "GPU shadow + mesh"),
                    ("gpu_shadow_pass_ns", "GPU shadow passes"),
                    ("gpu_mesh_pass_ns", "GPU mesh pass"),
                    ("gpu_bloom_pass_ns", "GPU Bloom pass"),
                    ("gpu_tone_mapping_pass_ns", "GPU tone-mapping pass"),
                ):
                    if label == "GPU shadow + mesh" and source_name != "shadow_mesh":
                        continue
                    value = number(row, key)
                    if value is not None:
                        pass_values[f"GPU sample: {label}"].append(value)
    if not frames:
        raise ValueError(f"{path}: no frame samples found; capture must exceed the warmup count")
    return frames, pass_values


def values_for(frames: list[dict[str, int | str | None]], key: str) -> list[float]:
    return [float(value) for frame in frames if (value := frame.get(key)) is not None]


def unique_gpu_values(frames: list[dict[str, int | str | None]], key: str) -> list[float]:
    # Timestamp results remain visible until a newer readback completes. Deduplicate by submission
    # sequence so async polling does not give old GPU samples artificial extra weight.
    seen: set[int] = set()
    result: list[float] = []
    for frame in frames:
        sequence = frame.get("gpu_sample_sequence")
        value = frame.get(key)
        if not isinstance(sequence, int) or value is None or sequence in seen:
            continue
        seen.add(sequence)
        result.append(float(value))
    return result


def unique_gpu_post_values(frames: list[dict[str, int | str | None]], key: str) -> list[float]:
    """Deduplicate asynchronous post-process measurements by their own submission sequence."""
    seen: set[int] = set()
    result: list[float] = []
    for frame in frames:
        sequence = frame.get("gpu_post_sample_sequence")
        value = frame.get(key)
        if not isinstance(sequence, int) or value is None or sequence in seen:
            continue
        seen.add(sequence)
        result.append(float(value))
    return result


def ms(value: float | int | None) -> str:
    return "—" if value is None else f"{float(value) / 1_000_000.0:.3f}"


def fmt_bytes(value: int | float | None) -> str:
    if value is None:
        return "—"
    exact = max(0, int(value))
    kib = 1024
    mib = kib * kib
    if exact >= mib:
        return f"{exact / mib:.2f} MiB ({exact:,} B)"
    if exact >= kib:
        return f"{exact / kib:.2f} KiB ({exact:,} B)"
    return f"{exact:,} B"


def summary_table(metrics: dict[str, list[float]]) -> list[str]:
    lines = [
        "| Metric | Samples | Mean (ms) | P50 (ms) | P95 (ms) | P99 (ms) | Peak (ms) |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for label, values in metrics.items():
        stats = duration_summary(values)
        lines.append(
            f"| {label} | {stats['samples']} | {ms(stats['mean'])} | {ms(stats['p50'])} | "
            f"{ms(stats['p95'])} | {ms(stats['p99'])} | {ms(stats['peak'])} |"
        )
    return lines


def pass_table(pass_values: dict[str, list[int]]) -> list[str]:
    lines = [
        "| CPU graph pass / render phase | Samples | Mean elapsed (ms) | P95 elapsed (ms) | Peak elapsed (ms) |",
        "|---|---:|---:|---:|---:|",
    ]
    ranked = sorted(
        ((name, values) for name, values in pass_values.items() if not name.startswith("GPU sample:")),
        key=lambda item: percentile(item[1], 95) or 0,
        reverse=True,
    )[:12]
    for name, values in ranked:
        stats = duration_summary([float(value) for value in values])
        safe_name = name.replace("|", "\\|").replace("\n", " ")
        lines.append(
            f"| {safe_name} | {stats['samples']} | {ms(stats['mean'])} | "
            f"{ms(stats['p95'])} | {ms(stats['peak'])} |"
        )
    if not ranked:
        lines.append("| No per-pass CPU timings were captured | 0 | — | — | — |")
    return lines


def current_resource_summary(frames: list[dict[str, int | str | None]]) -> list[str]:
    latest = frames[-1]

    def integer(row: dict[str, int | str | None], key: str) -> int:
        value = row.get(key)
        return value if isinstance(value, int) else 0

    def peak(key: str) -> int:
        return max(integer(frame, key) for frame in frames)

    peak_cpu = peak("frame_cpu_ns")
    lines = [
        "| Estimated allocation category | Latest | Peak in retained capture |",
        "|---|---:|---:|",
        f"| Total tracked bytes | {fmt_bytes(integer(latest, 'total_bytes_estimate'))} | {fmt_bytes(peak('total_bytes_estimate'))} |",
    ]
    byte_categories = [("Mesh buffers", "mesh_bytes_estimate")]
    if "material_texture_bytes_estimate" in latest:
        # The two rows below are a breakdown of material_bytes_estimate, not extra allocations.
        # Their sum equals the aggregate material category and therefore avoids double counting.
        byte_categories.extend([
            ("Material textures (deduplicated)", "material_texture_bytes_estimate"),
            ("Material uniform buffers", "material_uniform_bytes_estimate"),
        ])
    else:
        # Keep reports from captures produced before the texture-memory breakdown readable.
        byte_categories.append(("Material data/textures", "material_bytes_estimate"))
    byte_categories.extend([
        ("Renderer buffers", "buffer_bytes_estimate"),
        ("Depth/shadow targets", "depth_target_bytes_estimate"),
        ("Active transient textures", "active_transient_bytes_estimate"),
        ("Cached transient textures", "cached_transient_bytes_estimate"),
        ("Post-processing", "post_process_bytes_estimate"),
        ("Context target", "context_target_bytes_estimate"),
        ("Context readback staging", "context_readback_bytes_estimate"),
    ])
    for label, key in byte_categories:
        lines.append(
            f"| {label} | {fmt_bytes(integer(latest, key))} | {fmt_bytes(peak(key))} |"
        )

    if "cpu_asset_resident_bytes_estimate" in latest:
        lines.extend([
            "",
            "### CPU-side asset cache (separate from GPU allocation totals)",
            "",
            "| Cache metric | Latest | Peak in retained capture |",
            "|---|---:|---:|",
        ])
        for label, key in [
            ("Resident decoded asset bytes", "cpu_asset_resident_bytes_estimate"),
            ("Pinned asset bytes", "cpu_asset_pinned_bytes_estimate"),
            ("Unpinned cache bytes", "cpu_asset_unpinned_bytes_estimate"),
            ("Configured unpinned-cache budget", "cpu_asset_budget_bytes"),
            ("Unpinned bytes over budget", "cpu_asset_over_budget_bytes"),
        ]:
            if key == "cpu_asset_budget_bytes" and not integer(latest, "cpu_asset_budget_enabled"):
                latest_value = "disabled"
                peak_value = "disabled"
            else:
                latest_value = fmt_bytes(integer(latest, key))
                peak_value = fmt_bytes(peak(key))
            lines.append(f"| {label} | {latest_value} | {peak_value} |")
        for label, key in [
            ("Loaded CPU assets", "cpu_asset_loaded_count"),
            ("Pinned CPU assets", "cpu_asset_pinned_count"),
        ]:
            lines.append(f"| {label} | {integer(latest, key)} | {peak(key)} |")
        lines.extend([
            "",
            f"- Budget-based evictions at capture end: {integer(latest, 'cpu_asset_budget_evictions_cumulative')}",
            f"- Budget enabled: {'yes' if integer(latest, 'cpu_asset_budget_enabled') else 'no'}",
            "- CPU bytes use loader estimates based on decoded container/vector capacities, not process RSS or allocator telemetry.",
            "- The configured budget applies to unpinned cache entries. Active strong leases are protected and may raise total resident CPU bytes above the cache budget.",
        ])

    live_metrics = [
        ("Live meshes", "live_meshes"),
        ("Live materials", "live_materials"),
        ("Active transient textures", "active_transient_textures"),
        ("Cached transient textures", "cached_transient_textures"),
    ]
    if "live_material_textures" in latest:
        live_metrics.append(("Unique live material GPU textures", "live_material_textures"))
    if "gpu_model_cache_entries" in latest:
        live_metrics.extend([
            ("GPU model cache entries", "gpu_model_cache_entries"),
            ("GPU voxel-mesh cache entries", "gpu_voxel_mesh_cache_entries"),
        ])
    lines.extend([
        "",
        "| Live resource/cache metric | Latest | Peak in retained capture |",
        "|---|---:|---:|",
    ])
    for label, key in live_metrics:
        lines.append(f"| {label} | {integer(latest, key)} | {peak(key)} |")

    cumulative_metrics = [
        ("Transient texture allocations", "transient_texture_allocations"),
        ("Transient texture reuses", "transient_texture_reuses"),
        ("Renderer instance-buffer grows", "renderer_instance_buffer_grows_cumulative"),
        ("Renderer instance-buffer shrinks", "renderer_instance_buffer_shrinks_cumulative"),
        ("GPU timestamp samples completed", "gpu_timestamp_samples"),
        ("GPU timestamp samples skipped", "gpu_timestamp_samples_skipped"),
        ("GPU samples pending at frame", "gpu_samples_pending"),
        ("GPU samples unavailable (cumulative)", "gpu_samples_unavailable"),
        ("GPU samples dropped (cumulative)", "gpu_samples_dropped"),
        ("Surface acquisition timeouts", "surface_timeouts"),
        ("Surface outdated reconfigurations", "surface_outdated"),
        ("Surface lost reconfigurations", "surface_lost"),
        ("Other surface errors", "surface_other_errors"),
        ("Render-graph errors", "render_graph_errors"),
    ]
    if "material_texture_cache_hits" in latest:
        cumulative_metrics.extend([
            ("Material texture-cache hits", "material_texture_cache_hits"),
            ("Material texture-cache misses/uploads", "material_texture_cache_misses"),
        ])
    if "gpu_model_cache_entries" in latest:
        cumulative_metrics.extend([
            ("Model GPU uploads (distinct cache entries)", "model_gpu_uploads_cumulative"),
            ("Model GPU releases (distinct cache entries)", "model_gpu_releases_cumulative"),
            ("Voxel GPU uploads (distinct cache entries)", "voxel_mesh_gpu_uploads_cumulative"),
            ("Voxel GPU releases (distinct cache entries)", "voxel_mesh_gpu_releases_cumulative"),
        ])
    lines.extend([
        "",
        "| Cumulative counter | Latest value |",
        "|---|---:|",
    ])
    for label, key in cumulative_metrics:
        lines.append(f"| {label} | {integer(latest, key)} |")
    lines.append(f"| Peak frame wall in retained samples | {ms(peak_cpu)} ms |")

    if "gpu_model_cache_entries" in latest:
        workload_metrics = [
            ("Scene mesh instances", "scene_mesh_instances"),
            ("Scene glTF model placements", "scene_model_placements"),
            ("Scene voxel placements", "scene_voxel_placements"),
            ("Renderer instance-buffer capacity", "renderer_instance_capacity"),
        ]
        lines.extend([
            "",
            "| Large-scene workload metric | Latest | Peak in retained capture |",
            "|---|---:|---:|",
        ])
        for label, key in workload_metrics:
            lines.append(f"| {label} | {integer(latest, key)} | {peak(key)} |")
    return lines


def comparison_table(current: dict[str, list[float]], baseline: dict[str, list[float]]) -> list[str]:
    lines = [
        "| Metric | Current P95 (ms) | Baseline P95 (ms) | Delta |",
        "|---|---:|---:|---:|",
    ]
    for label in (
        "Frame wall",
        "Surface acquisition wall",
        "Render graph CPU",
        "Surface presentation wall",
        "Frame remainder (setup/outside measured phases)",
        "GPU shadow + mesh",
        "GPU mesh pass",
    ):
        cur = percentile(current.get(label, []), 95)
        base = percentile(baseline.get(label, []), 95)
        if cur is None or base is None:
            delta = "not available"
        elif base == 0:
            delta = "n/a (baseline is zero)"
        else:
            delta_pct = (cur - base) / base * 100.0
            delta = f"{delta_pct:+.1f}% {'(slower)' if delta_pct > 0 else '(faster)' if delta_pct < 0 else '(same)'}"
        lines.append(f"| {label} | {ms(cur)} | {ms(base)} | {delta} |")
    return lines


def metrics_for(
    frames: list[dict[str, int | str | None]],
    pass_values: dict[str, list[int]] | None = None,
) -> dict[str, list[float]]:
    metrics = {"Frame wall": values_for(frames, "frame_cpu_ns")}
    # These phase columns were added after the first baseline CSV schema. Keep old captures usable.
    if any("surface_acquire_ns" in frame for frame in frames):
        metrics["Surface acquisition wall"] = values_for(frames, "surface_acquire_ns")
    metrics["Render graph CPU"] = values_for(frames, "graph_cpu_ns")
    if any("present_ns" in frame for frame in frames):
        metrics["Surface presentation wall"] = values_for(frames, "present_ns")
    if any("surface_acquire_ns" in frame and "present_ns" in frame for frame in frames):
        metrics["Frame remainder (setup/outside measured phases)"] = [
            float(max(
                0,
                int(frame.get("frame_cpu_ns") or 0)
                - int(frame.get("surface_acquire_ns") or 0)
                - int(frame.get("graph_cpu_ns") or 0)
                - int(frame.get("present_ns") or 0),
            ))
            for frame in frames
        ]
    gpu_metrics = pass_values or {}
    metrics["GPU shadow + mesh"] = [float(value) for value in gpu_metrics.get("GPU sample: GPU shadow + mesh", [])] or unique_gpu_values(frames, "gpu_shadow_and_mesh_ns")
    if any("gpu_shadow_pass_ns" in frame for frame in frames):
        metrics["GPU shadow passes"] = [float(value) for value in gpu_metrics.get("GPU sample: GPU shadow passes", [])] or unique_gpu_values(frames, "gpu_shadow_pass_ns")
    metrics["GPU mesh pass"] = [float(value) for value in gpu_metrics.get("GPU sample: GPU mesh pass", [])] or unique_gpu_values(frames, "gpu_mesh_pass_ns")
    if any("gpu_bloom_pass_ns" in frame for frame in frames):
        metrics["GPU Bloom pass"] = [float(value) for value in gpu_metrics.get("GPU sample: GPU Bloom pass", [])] or unique_gpu_post_values(frames, "gpu_bloom_pass_ns")
        metrics["GPU tone-mapping pass"] = [float(value) for value in gpu_metrics.get("GPU sample: GPU tone-mapping pass", [])] or unique_gpu_post_values(frames, "gpu_tone_mapping_pass_ns")
    return metrics


def capture_metadata_lines(capture: Path) -> list[str]:
    """Collect metadata available in the matching capture log."""
    log_path = capture.with_suffix(".log")
    try:
        log_text = log_path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        log_text = ""

    log_lines = [line.strip() for line in log_text.splitlines()]
    adapter_line = next(
        (line for line in log_lines if "[GPU ADAPTER]" in line), ""
    )
    surface_line = next(
        (line for line in log_lines if "[GPU SURFACE]" in line), ""
    )

    def surface_value(key: str) -> str:
        prefix = f"{key}="
        clean_line = surface_line.replace("[GPU SURFACE]", "")
        for item in clean_line.split(";"):
            item = item.strip()
            if item.startswith(prefix):
                return item[len(prefix):].strip()
        return ""

    repo_root = Path(__file__).resolve().parent.parent
    revision = "not recorded (Git metadata unavailable)"
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=repo_root, capture_output=True, text=True, check=False
        )
        if result.returncode == 0 and result.stdout.strip():
            revision = f"`{result.stdout.strip()}`"
            status = subprocess.run(
                ["git", "status", "--porcelain"],
                cwd=repo_root, capture_output=True, text=True, check=False
            )
            if status.returncode == 0 and status.stdout.strip():
                revision += " (working tree modified)"
    except OSError:
        pass

    profile = "not recorded"
    if any("Finished `release` profile" in line for line in log_lines):
        profile = "release"
    elif any("Finished `dev` profile" in line for line in log_lines):
        profile = "dev/debug"

    running = next(
        (line for line in log_lines if "Running `" in line), ""
    )
    command = ""
    if running:
        command = running.split("Running `", 1)[1].rsplit("`", 1)[0]

    if profile == "not recorded" and "target\\release\\" in command:
        profile = "release"
    elif profile == "not recorded" and "target\\debug\\" in command:
        profile = "debug"

    build_info = (
        f"{profile}; `{command}`" if command
        else f"{profile}; command not recorded in matching log"
    )
    adapter_info = (
        adapter_line.replace("[GPU ADAPTER]", "", 1).strip()
        if adapter_line else "not recorded (matching .log missing)"
    )
    surface_info = (
        surface_line.replace("[GPU SURFACE]", "", 1).strip()
        if surface_line else "not recorded (matching .log missing)"
    )
    error_info = next(
        (line for line in log_lines if line.startswith("[RENDER PROFILE ERRORS]")),
        "not recorded in matching log",
    )
    resolution = surface_value("resolution")
    if resolution:
        resolution += " (native window surface)"
    else:
        resolution = "not recorded in this capture log"

    return [
        f"- Repository revision observed while generating this report (not capture/build provenance): {revision}",
        f"- Operating system: {platform.platform()}",
        f"- GPU adapter and driver: {adapter_info}",
        f"- Display/render resolution: {resolution}",
        f"- Build profile and example command: {build_info}",
        f"- Surface configuration: {surface_info}",
        f"- Surface/render-graph error counters at shutdown: {error_info}",
    ]



def build_report(path: Path, frames: list[dict[str, int | str | None]], passes: dict[str, list[int]], compare_path: Path | None) -> str:
    metrics = metrics_for(frames, passes)
    first_frame = frames[0].get("frame")
    last_frame = frames[-1].get("frame")
    generated = datetime.now(timezone.utc).isoformat(timespec="seconds")
    lines = [
        "# Ribbon Engine Render Profile Report",
        "",
        f"- Capture: `{path}`",
        f"- Generated (UTC): `{generated}`",
        f"- Frame samples: **{len(frames)}** (engine frames {first_frame}–{last_frame})",
        f"- Surface present mode: `{frames[-1].get('surface_present_mode') or 'not recorded'}`",
        f"- Desired maximum frame latency: `{frames[-1].get('surface_frame_latency') or 'not recorded'}`",
        "- Values are from a bounded, post-warmup capture. GPU timings are explicit asynchronous completion records with submission and observation frames; CPU phase rows are opt-in and excluded from normal rendering.",
        "",
        "## Timing summary",
        "",
        *summary_table(metrics),
        "",
        "## Render graph pass CPU timings",
        "",
        *pass_table(passes),
        "",
        "## Resource snapshot",
        "",
        *current_resource_summary(frames),
        "",
        "## Capture metadata (automatic where available)",
        "",
        *capture_metadata_lines(path),
        "- Warmup and captured frame range: recorded above; note any scene/input variation here",
        "",
        "## Interpretation notes",
        "",
        "- `Frame wall` includes surface acquisition, render-graph execution, and `SurfaceTexture::present`; it is elapsed wall time, not CPU utilization.",
        "- Surface acquisition and presentation durations help identify waits outside render-graph work. They are wall-clock timings and may include driver or compositor behavior.",
        "- The frame-remainder metric is computed per sample as total frame wall minus acquisition, graph, and presentation; it represents setup/other work plus timing overhead, not a single named function.",
        "- Render graph pass durations are CPU-side encoding/invocation time and exclude asynchronous GPU execution.",
        "- Each GPU record represents one successfully decoded asynchronous sample and includes its submission sequence/frame plus the frame that observed completion. Pending, unavailable, skipped, and dropped counts are reported separately; outstanding readbacks are never waited on.",
        "- CPU render-phase timings are opt-in when a bounded profile recorder is installed. They are elapsed CPU-side call timings, may include driver backpressure, and do not measure GPU execution or CPU utilization. Profile-recorder bookkeeping is included as its own phase; shutdown file writing is not a frame phase.",
        "- Surface timeout, outdated, lost, other acquisition errors and render-graph failures are counted and emitted in the capture log at shutdown; frame CSV rows include the counters observed by each successful frame.",
        "- Resource values are engine-side estimates, not driver-reported total VRAM usage.",
        "- Material texture bytes are counted once per unique live texture content and color-space format; material uniforms are shown separately as a breakdown of the aggregate material estimate.",
        "- Texture-cache hits include repeated maps across materials and roles where their pixel content and sRGB/linear interpretation are identical.",
        "- Only change one major rendering variable at a time and rerun on the same machine, GPU adapter/driver, display resolution, release/debug profile, scene, warmup, and capture length.",
        "- Do not compare numbers from different GPU adapters or build profiles as if they were controlled performance tests.",
    ]
    if compare_path is not None:
        baseline_frames, baseline_passes = load_capture(compare_path)
        baseline_metrics = metrics_for(baseline_frames, baseline_passes)
        current_mode = frames[-1].get("surface_present_mode")
        baseline_mode = baseline_frames[-1].get("surface_present_mode")
        current_latency = frames[-1].get("surface_frame_latency")
        baseline_latency = baseline_frames[-1].get("surface_frame_latency")
        config_warning = []
        if (current_mode, current_latency) != (baseline_mode, baseline_latency):
            config_warning = [
                "",
                "**Note:** current and baseline captures use different surface presentation settings; interpret timing deltas as a configuration experiment, not a like-for-like renderer regression test.",
            ]
        lines.extend([
            "",
            "## Comparison with baseline",
            "",
            f"Baseline capture: `{compare_path}` ({len(baseline_frames)} frame samples)",
            *config_warning,
            "",
            *comparison_table(metrics, baseline_metrics),
            "",
            "Negative time deltas indicate lower latency in the current capture; positive deltas indicate higher latency.",
        ])
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path, help="CSV written by RenderProfilePlugin")
    parser.add_argument("--compare", type=Path, help="Optional baseline CSV to compare with")
    parser.add_argument("--output", type=Path, help="Markdown output path (default: print to stdout)")
    args = parser.parse_args()

    try:
        frames, passes = load_capture(args.capture)
        report = build_report(args.capture, frames, passes, args.compare)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(report, encoding="utf-8")
            print(f"wrote report: {args.output}")
        else:
            sys.stdout.write(report)
    except (OSError, ValueError, csv.Error) as exc:
        print(f"render profile report failed: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
