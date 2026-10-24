#!/usr/bin/env python3
"""Validate and summarize repeated active-asset/texture scalability captures."""

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
EXPECTED = {(count, mode, repeat) for count in (1, 8, 32) for mode in ("fifo", "mailbox") for repeat in range(1, 6)}


def parse_name(path: Path) -> tuple[int, str, int]:
    match = re.fullmatch(r"assets(01|08|32)-(fifo|mailbox)-run(\d{2})", path.stem)
    if not match:
        raise ValueError(f"unexpected capture name: {path.name}")
    return int(match.group(1)), match.group(2), int(match.group(3))


def validate(path: Path, count: int, mode: str):
    frames, passes = profile.load_capture(path)
    if len(frames) != EXPECTED_ROWS:
        raise ValueError(f"{path}: expected {EXPECTED_ROWS} frame rows, got {len(frames)}")
    frame_numbers = [int(row["frame"]) for row in frames]
    if any(right != left + 1 for left, right in zip(frame_numbers, frame_numbers[1:])):
        raise ValueError(f"{path}: frame sequence is not contiguous")
    for row in frames:
        checks = {
            "surface_present_mode": mode,
            "surface_frame_latency": 1,
            "gpu_timestamp_queries_supported": 1,
            "gpu_timestamp_samples_skipped": 0,
            "scene_model_placements": 576,
            "gpu_model_cache_entries": count,
        }
        for field, expected in checks.items():
            if row.get(field) != expected:
                raise ValueError(f"{path}: {field}={row.get(field)!r}, expected {expected!r}")
    log_path = path.with_suffix(".log")
    log = log_path.read_text(encoding="utf-8", errors="replace")
    for token in ("source_revision=", "source_dirty=", "executable_sha256=", "corpus_manifest_sha256=", "command=", "capture_process_exit_code=0", "[GPU ADAPTER]", "[GPU SURFACE]"):
        if token not in log:
            raise ValueError(f"{log_path}: missing provenance {token}")
    if "validation error" in log.lower() or "panicked at" in log.lower():
        raise ValueError(f"{log_path}: validation error/panic detected")
    report_path = path.with_suffix(".md")
    text = report_path.read_text(encoding="utf-8")
    for line in profile.summary_table(profile.metrics_for(frames))[2:]:
        if line not in text:
            raise ValueError(f"{report_path}: summary mismatch with raw CSV: {line}")
    return frames, passes


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("capture_dir", type=Path)
    parser.add_argument("output_dir", type=Path)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    captures = sorted(args.capture_dir.glob("assets*-run*.csv"))
    runs = []
    seen = set()
    for path in captures:
        count, mode, repeat = parse_name(path)
        key = (count, mode, repeat)
        if key in seen:
            raise ValueError(f"duplicate capture {key}")
        seen.add(key)
        frames, _ = validate(path, count, mode)
        metrics = profile.metrics_for(frames)
        for label, field in {
            "Tracked GPU allocation estimate": "total_bytes_estimate",
            "Material texture allocation estimate": "material_texture_bytes_estimate",
            "Active transient texture estimate": "active_transient_bytes_estimate",
            "CPU resident asset estimate": "cpu_asset_resident_bytes_estimate",
            "Visible mesh instances": "visible_mesh_instances",
            "Visible draw calls": "visible_draw_calls",
            "GPU model cache entries": "gpu_model_cache_entries",
            "Cumulative model uploads": "model_gpu_uploads_cumulative",
        }.items():
            metrics[label] = [float(row[field]) for row in frames if row.get(field) is not None]
        for metric, values in metrics.items():
            stats = profile.duration_summary(values)
            unit = "bytes" if "estimate" in metric else "count" if metric in {"Visible mesh instances", "Visible draw calls", "GPU model cache entries", "Cumulative model uploads"} else "ms"
            divisor = 1_000_000 if unit == "ms" else 1
            runs.append({
                "active_assets": count, "placements": 576, "mode": mode, "repeat": repeat,
                "metric": metric, "unit": unit, "samples": stats["samples"],
                "mean": None if stats["mean"] is None else stats["mean"] / divisor,
                "p50": None if stats["p50"] is None else stats["p50"] / divisor,
                "p95": None if stats["p95"] is None else stats["p95"] / divisor,
                "p99": None if stats["p99"] is None else stats["p99"] / divisor,
            })
    if seen != EXPECTED:
        raise ValueError(f"capture set mismatch; missing={sorted(EXPECTED-seen)}, unexpected={sorted(seen-EXPECTED)}")
    csv_path = args.output_dir / "asset-run-level-results.csv"
    with csv_path.open("w", newline="", encoding="utf-8") as destination:
        writer = csv.DictWriter(destination, fieldnames=list(runs[0]))
        writer.writeheader()
        writer.writerows(runs)
    grouped = defaultdict(list)
    for row in runs:
        grouped[(row["active_assets"], row["mode"], row["metric"])].append(row)
    metrics_order = [
        "Frame wall", "Surface acquisition wall", "Render graph CPU", "Surface presentation wall",
        "GPU shadow + mesh", "GPU shadow passes", "GPU mesh pass", "GPU Bloom pass",
        "GPU tone-mapping pass", "Tracked GPU allocation estimate",
        "Material texture allocation estimate", "Active transient texture estimate",
        "CPU resident asset estimate", "Visible mesh instances", "Visible draw calls",
        "GPU model cache entries", "Cumulative model uploads",
    ]
    lines = [
        "# Active asset and texture scalability results", "",
        "All statistics come from the raw, validated CSVs. Each active-asset count has five runs per presentation mode at a fixed 576 placements. Percentiles use linear interpolation at `(n - 1) * p`; brackets show min/max run-level P95 values. GPU values are asynchronous sequenced samples and counts are retained per run in the results CSV.", "",
        "| Active unique assets | Mode | Metric | Sample counts | Median mean | Median P50 | Median P95 [min,max] | Median P99 |", "|---:|---|---|---|---:|---:|---:|---:|"
    ]
    for count in (1, 8, 32):
        for mode in ("fifo", "mailbox"):
            for metric in metrics_order:
                group = grouped[(count, mode, metric)]
                samples = ",".join(str(row["samples"]) for row in group) if metric.startswith("GPU ") else str(group[0]["samples"])
                vals = {key: [float(row[key]) for row in group] for key in ("mean", "p50", "p95", "p99")}
                cells = [f"{statistics.median(vals[key]):.3f}" for key in ("mean", "p50")]
                p95 = statistics.median(vals["p95"])
                cells.append(f"{p95:.3f} [{min(vals['p95']):.3f}, {max(vals['p95']):.3f}]")
                cells.append(f"{statistics.median(vals['p99']):.3f}")
                lines.append(f"| {count} | {mode} | {metric} | {samples} | {cells[0]} | {cells[1]} | {cells[2]} | {cells[3]} |")
    lines.extend([
        "", "## Interpretation", "",
        "Placements, camera, lights, output size, presentation settings, and per-asset texture resolution are fixed; increasing active assets changes unique geometry/material/texture count and upload/cache footprint together. This measures asset-count/resource scaling, not the isolated cost of texture bytes versus mesh/material count. The sun remains the one available showcase light; no CLI control exists for a lighting-count sweep.", "",
        f"- Validated captures: {len(captures)}; every capture has {EXPECTED_ROWS} contiguous frame samples.",
        f"- Raw captures: `{args.capture_dir}`",
        f"- Run-level results: `{csv_path}`",
    ])
    report = args.output_dir / "asset-scalability-results.md"
    report.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Validated 30 asset captures; report: {report}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
