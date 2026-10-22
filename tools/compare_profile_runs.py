#!/usr/bin/env python3
"""Summarize repeated Ribbon Engine render-profile captures by configuration.

Capture files must be named <configuration>-runNN.csv and be in one input directory.
No third-party Python packages are required.
"""
from __future__ import annotations

import argparse
import csv
import math
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from statistics import mean, median
from typing import Iterable

NS_PER_MS = 1_000_000.0
METRICS = (
    ("frame_cpu_ns", "Frame wall"),
    ("surface_acquire_ns", "Surface acquisition"),
    ("graph_cpu_ns", "Render graph CPU"),
    ("present_ns", "Surface presentation"),
    ("gpu_shadow_and_mesh_ns", "GPU shadow + mesh"),
    ("gpu_mesh_pass_ns", "GPU mesh pass"),
)


def number(value: str | None) -> float | None:
    if value is None or value.strip() == "":
        return None
    try:
        result = float(value)
    except ValueError:
        return None
    return result if math.isfinite(result) else None


def quantile(values: list[float], probability: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    position = (len(ordered) - 1) * probability
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    fraction = position - lower
    return ordered[lower] * (1.0 - fraction) + ordered[upper] * fraction


def fmt_ms(value: float | None) -> str:
    return "—" if value is None else f"{value:.3f}"


def fmt_pct(value: float | None) -> str:
    return "—" if value is None else f"{value:.2f}%"


def stats_ms(values_ns: list[float]) -> dict[str, float | None]:
    values = [value / NS_PER_MS for value in values_ns]
    if not values:
        return {key: None for key in ("mean", "p50", "p95", "p99", "peak")}
    return {
        "mean": mean(values),
        "p50": quantile(values, 0.50),
        "p95": quantile(values, 0.95),
        "p99": quantile(values, 0.99),
        "peak": max(values),
    }


def load_capture(path: Path, budget_ms: float) -> dict:
    frames: list[dict[str, str]] = []
    with path.open("r", newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        if not reader.fieldnames or "record_type" not in reader.fieldnames:
            raise ValueError(f"{path}: missing record_type CSV column")
        for row in reader:
            if row.get("record_type") == "frame":
                frames.append(row)
    if not frames:
        raise ValueError(f"{path}: no frame records")

    metric_ns: dict[str, list[float]] = defaultdict(list)
    gpu_unique: dict[str, dict[str, float]] = defaultdict(dict)
    frame_ms: list[float] = []
    for frame in frames:
        for column, label in METRICS:
            value = number(frame.get(column))
            if value is None:
                continue
            if label.startswith("GPU "):
                sequence = number(frame.get("gpu_sample_sequence"))
                if sequence is None:
                    continue
                # The same completed timestamp may be observed on multiple later frames.
                gpu_unique[label][str(int(sequence))] = value
            else:
                metric_ns[label].append(value)
        value = number(frame.get("frame_cpu_ns"))
        if value is not None:
            frame_ms.append(value / NS_PER_MS)

    for label, samples in gpu_unique.items():
        metric_ns[label] = list(samples.values())

    first = frames[0]
    config = path.stem.rsplit("-run", 1)[0]
    run_suffix = path.stem.rsplit("-run", 1)[-1]
    try:
        run_number = int(run_suffix)
    except ValueError:
        run_number = 0

    metrics = {label: stats_ms(metric_ns.get(label, [])) for _, label in METRICS}
    over_budget = (sum(value > budget_ms for value in frame_ms) / len(frame_ms) * 100.0) if frame_ms else None
    return {
        "path": path,
        "config": config,
        "run": run_number,
        "frames": len(frame_ms),
        "mode": first.get("surface_present_mode") or "not recorded",
        "latency": first.get("surface_frame_latency") or "not recorded",
        "over_budget_pct": over_budget,
        "metrics": metrics,
    }


def median_or_none(values: Iterable[float | None]) -> float | None:
    usable = [value for value in values if value is not None]
    return median(usable) if usable else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True, help="directory containing *-runNN.csv files")
    parser.add_argument("--output", type=Path, required=True, help="Markdown report path")
    parser.add_argument("--budget-ms", type=float, default=16.667, help="frame-wall threshold used for exceedance rate")
    args = parser.parse_args()
    if args.budget_ms <= 0:
        parser.error("--budget-ms must be positive")

    paths = sorted(path for path in args.input_dir.glob("*-run*.csv") if path.is_file())
    if not paths:
        parser.error(f"no *-run*.csv captures found in {args.input_dir}")

    captures = [load_capture(path, args.budget_ms) for path in paths]
    groups: dict[str, list[dict]] = defaultdict(list)
    for capture in captures:
        groups[capture["config"]].append(capture)

    lines = [
        "# Ribbon Engine Repeated Surface Profile Comparison",
        "",
        f"- Generated (UTC): `{datetime.now(timezone.utc).isoformat(timespec='seconds')}`",
        f"- Input directory: `{args.input_dir}`",
        f"- Frame-wall threshold: `{args.budget_ms:.3f} ms`",
        "- Per-configuration summary uses run-level percentiles; frames from different runs are not pooled.",
        "- GPU timestamp samples are deduplicated by submission sequence per capture.",
        "",
        "## Per-capture results",
        "",
        "| Configuration | Run | Resolved mode / latency | Frames | Frame P95 (ms) | Frame P99 (ms) | Above budget | Acquire P95 (ms) | Graph P95 (ms) | Present P95 (ms) | GPU shadow+mesh P95 (ms) |",
        "|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for capture in captures:
        metrics = capture["metrics"]
        lines.append(
            f"| {capture['config']} | {capture['run'] or '—'} | {capture['mode']} / {capture['latency']} | "
            f"{capture['frames']} | {fmt_ms(metrics['Frame wall']['p95'])} | {fmt_ms(metrics['Frame wall']['p99'])} | "
            f"{fmt_pct(capture['over_budget_pct'])} | {fmt_ms(metrics['Surface acquisition']['p95'])} | "
            f"{fmt_ms(metrics['Render graph CPU']['p95'])} | {fmt_ms(metrics['Surface presentation']['p95'])} | "
            f"{fmt_ms(metrics['GPU shadow + mesh']['p95'])} |"
        )

    lines.extend([
        "",
        "## Across-run repeatability",
        "",
        "Values below are the median of each configuration's run-level metrics; bracketed values are the observed minimum and maximum across captures.",
        "",
        "| Configuration | Captures | Frame P95 median [min, max] ms | Frame P99 median [min, max] ms | Acquire P95 median [min, max] ms | Graph P95 median [min, max] ms | Samples above budget median |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ])
    warnings: list[str] = []
    for config, runs in sorted(groups.items()):
        def run_metric(metric: str, stat: str) -> list[float | None]:
            return [run["metrics"][metric][stat] for run in runs]

        def range_text(values: list[float | None]) -> str:
            usable = [value for value in values if value is not None]
            if not usable:
                return "—"
            mid = median(usable)
            return f"{mid:.3f} [{min(usable):.3f}, {max(usable):.3f}]"

        lines.append(
            f"| {config} | {len(runs)} | {range_text(run_metric('Frame wall', 'p95'))} | "
            f"{range_text(run_metric('Frame wall', 'p99'))} | {range_text(run_metric('Surface acquisition', 'p95'))} | "
            f"{range_text(run_metric('Render graph CPU', 'p95'))} | {fmt_pct(median_or_none(run['over_budget_pct'] for run in runs))} |"
        )
        if len(runs) < 3:
            warnings.append(
                f"> **Repeatability warning:** `{config}` has only {len(runs)} capture(s); "
                "collect at least three before generalizing the result."
            )

    lines.extend([""])
    lines.extend(warnings)
    lines.extend([
        "",
        "## Interpretation",
        "",
        "- Prefer lower and repeatable frame-wall P95/P99 without unacceptable visual artifacts.",
        "- FIFO latency settings should be compared with the same resolved present mode. `immediate`/no-vsync is a separate trade-off because it may tear and use more power.",
        "- Similar averages with different P95/P99 values indicate a pacing/tail-latency change, not necessarily higher average throughput.",
        "- Results are specific to this machine, display mode, driver, build profile, scene and power state.",
        "",
    ])

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(lines), encoding="utf-8")
    print(f"wrote report: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
