#!/usr/bin/env python3
"""Summarize Ribbon Engine render-profile CSV captures into a Markdown report.

The CSV is emitted by engine_render::profiling::RenderProfilePlugin. This utility uses only
Python's standard library so it does not add any engine/runtime dependency.
"""

from __future__ import annotations

import argparse
import csv
import math
import statistics
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable


def percentile(values: Iterable[float], percent: float) -> float | None:
    ordered = sorted(values)
    if not ordered:
        return None
    index = max(0, math.ceil(percent / 100.0 * len(ordered)) - 1)
    return ordered[min(index, len(ordered) - 1)]


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
                    elif key == "gpu_timestamp_queries_supported":
                        parsed[key] = 1 if (row.get(key) or "").strip().lower() == "true" else 0
                    else:
                        parsed[key] = number(row, key)
                frames.append(parsed)
            elif record_type == "pass":
                name = row.get("pass_name", "").strip()
                value = number(row, "pass_cpu_ns")
                if name and value is not None:
                    pass_values[name].append(value)
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


def ms(value: float | int | None) -> str:
    return "—" if value is None else f"{float(value) / 1_000_000.0:.3f}"


def fmt_bytes(value: int | float | None) -> str:
    return "—" if value is None else f"{float(value) / (1024.0 * 1024.0):.2f} MiB"


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
        "| Render graph pass | Samples | Mean CPU (ms) | P95 CPU (ms) | Peak CPU (ms) |",
        "|---|---:|---:|---:|---:|",
    ]
    ranked = sorted(
        pass_values.items(),
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

    peak = max(integer(frame, "total_bytes_estimate") for frame in frames)
    peak_cpu = max(integer(frame, "frame_cpu_ns") for frame in frames)
    lines = [
        "| Estimated allocation category | Latest | Peak in retained capture |",
        "|---|---:|---:|",
        f"| Total tracked bytes | {fmt_bytes(integer(latest, 'total_bytes_estimate'))} | {fmt_bytes(peak)} |",
    ]
    byte_categories = [
        ("Mesh buffers", "mesh_bytes_estimate"),
        ("Material data/textures", "material_bytes_estimate"),
        ("Renderer buffers", "buffer_bytes_estimate"),
        ("Depth/shadow targets", "depth_target_bytes_estimate"),
        ("Active transient textures", "active_transient_bytes_estimate"),
        ("Cached transient textures", "cached_transient_bytes_estimate"),
        ("Post-processing", "post_process_bytes_estimate"),
        ("Context target", "context_target_bytes_estimate"),
        ("Context readback staging", "context_readback_bytes_estimate"),
    ]
    for label, key in byte_categories:
        current = integer(latest, key)
        peak_category = max(integer(frame, key) for frame in frames)
        lines.append(f"| {label} | {fmt_bytes(current)} | {fmt_bytes(peak_category)} |")
    lines.extend([
        "",
        "| Counter | Latest |",
        "|---|---:|",
        f"| Live meshes | {integer(latest, 'live_meshes')} |",
        f"| Live materials | {integer(latest, 'live_materials')} |",
        f"| Active transient textures | {integer(latest, 'active_transient_textures')} |",
        f"| Cached transient textures | {integer(latest, 'cached_transient_textures')} |",
        f"| Transient texture allocations (cumulative) | {integer(latest, 'transient_texture_allocations')} |",
        f"| Transient texture reuses (cumulative) | {integer(latest, 'transient_texture_reuses')} |",
        f"| GPU timestamp samples completed (cumulative) | {integer(latest, 'gpu_timestamp_samples')} |",
        f"| GPU timestamp samples skipped (cumulative) | {integer(latest, 'gpu_timestamp_samples_skipped')} |",
        f"| Peak frame wall in retained samples | {ms(peak_cpu)} ms |",
    ])
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


def metrics_for(frames: list[dict[str, int | str | None]]) -> dict[str, list[float]]:
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
    metrics["GPU shadow + mesh"] = unique_gpu_values(frames, "gpu_shadow_and_mesh_ns")
    metrics["GPU mesh pass"] = unique_gpu_values(frames, "gpu_mesh_pass_ns")
    return metrics


def build_report(path: Path, frames: list[dict[str, int | str | None]], passes: dict[str, list[int]], compare_path: Path | None) -> str:
    metrics = metrics_for(frames)
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
        "- Values are from a bounded, post-warmup capture; GPU timings are asynchronously completed submissions.",
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
        "## Capture metadata (fill before comparing or sharing)",
        "",
        "- Source revision/archive: _fill in_",
        "- Operating system: _fill in_",
        "- GPU adapter and driver: _fill in_",
        "- Display/render resolution: _fill in_",
        "- Build profile and example command: _fill in_",
        "- Warmup and captured frame range: recorded above; note any scene/input variation here",
        "",
        "## Interpretation notes",
        "",
        "- `Frame wall` includes surface acquisition, render-graph execution, and `SurfaceTexture::present`; it is elapsed wall time, not CPU utilization.",
        "- Surface acquisition and presentation durations help identify waits outside render-graph work. They are wall-clock timings and may include driver or compositor behavior.",
        "- The frame-remainder metric is computed per sample as total frame wall minus acquisition, graph, and presentation; it represents setup/other work plus timing overhead, not a single named function.",
        "- Render graph pass durations are CPU-side encoding/invocation time and exclude asynchronous GPU execution.",
        "- GPU samples are deduplicated by submission sequence because an older result can remain visible across multiple captured frames.",
        "- Resource values are engine-side estimates, not driver-reported total VRAM usage.",
        "- Only change one major rendering variable at a time and rerun on the same machine, GPU adapter/driver, display resolution, release/debug profile, scene, warmup, and capture length.",
        "- Do not compare numbers from different GPU adapters or build profiles as if they were controlled performance tests.",
    ]
    if compare_path is not None:
        baseline_frames, _ = load_capture(compare_path)
        baseline_metrics = metrics_for(baseline_frames)
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
