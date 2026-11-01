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


def load_capture(path: Path, budget_ms: float, expected_frames: int | None = None) -> dict:
    frames: list[dict[str, str]] = []
    pass_cpu_ns: dict[str, list[float]] = defaultdict(list)
    with path.open("r", newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        if not reader.fieldnames or "record_type" not in reader.fieldnames:
            raise ValueError(f"{path}: missing record_type CSV column")
        for row in reader:
            if row.get("record_type") == "frame":
                frames.append(row)
            elif row.get("record_type") == "pass":
                pass_name = (row.get("pass_name") or "").strip()
                pass_time = number(row.get("pass_cpu_ns"))
                if pass_name and pass_time is not None:
                    pass_cpu_ns[pass_name].append(pass_time)
    if not frames:
        raise ValueError(f"{path}: no frame records")
    if expected_frames is not None and len(frames) != expected_frames:
        raise ValueError(
            f"{path}: incomplete capture has {len(frames)} frame records; expected {expected_frames}"
        )

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
        if value is None:
            raise ValueError(f"{path}: frame record is missing a valid frame_cpu_ns value")
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
        "passes": {name: stats_ms(values) for name, values in pass_cpu_ns.items()},
    }


def median_or_none(values: Iterable[float | None]) -> float | None:
    usable = [value for value in values if value is not None]
    return median(usable) if usable else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True, help="directory containing *-runNN.csv files")
    parser.add_argument("--output", type=Path, required=True, help="Markdown report path")
    parser.add_argument("--budget-ms", type=float, default=16.667, help="frame-wall threshold used for exceedance rate")
    parser.add_argument(
        "--expected-frames",
        type=int,
        help="required frame-record count for every capture; rejects incomplete profiles",
    )
    args = parser.parse_args()
    if args.budget_ms <= 0:
        parser.error("--budget-ms must be positive")
    if args.expected_frames is not None and args.expected_frames <= 0:
        parser.error("--expected-frames must be positive")

    paths = sorted(path for path in args.input_dir.glob("*-run*.csv") if path.is_file())
    if not paths:
        parser.error(f"no *-run*.csv captures found in {args.input_dir}")

    captures = [load_capture(path, args.budget_ms, args.expected_frames) for path in paths]
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
        "| Configuration | Run | Resolved mode / latency | Frames | Frame P50 (ms) | Frame P95 (ms) | Frame P99 (ms) | Above budget | Acquire P95 (ms) | Graph P95 (ms) | Present P95 (ms) | GPU shadow+mesh P95 (ms) |",
        "|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for capture in captures:
        metrics = capture["metrics"]
        lines.append(
            f"| {capture['config']} | {capture['run'] or '—'} | {capture['mode']} / {capture['latency']} | "
            f"{capture['frames']} | {fmt_ms(metrics['Frame wall']['p50'])} | {fmt_ms(metrics['Frame wall']['p95'])} | {fmt_ms(metrics['Frame wall']['p99'])} | "
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
        "| Configuration | Captures | Frame P50 median [min, max] ms | Frame P95 median [min, max] ms | Frame P99 median [min, max] ms | Acquire P95 median [min, max] ms | Graph P95 median [min, max] ms | Samples above budget median |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
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
            f"| {config} | {len(runs)} | {range_text(run_metric('Frame wall', 'p50'))} | {range_text(run_metric('Frame wall', 'p95'))} | "
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

    # When the canonical FIFO and Mailbox groups are present, add a direct comparison.
    # Compare the median of per-run statistics; never pool frame rows across runs.
    fifo_runs = groups.get("fifo", [])
    mailbox_runs = groups.get("mailbox", [])
    if fifo_runs and mailbox_runs:
        def range_text(values: Iterable[float | None]) -> str:
            usable = [value for value in values if value is not None]
            if not usable:
                return "—"
            mid = median(usable)
            return f"{mid:.3f} [{min(usable):.3f}, {max(usable):.3f}]"

        def group_metric(runs: list[dict], metric: str, stat: str) -> list[float | None]:
            return [run["metrics"][metric][stat] for run in runs]

        def delta_text(fifo_values: list[float | None], mailbox_values: list[float | None]) -> str:
            fifo_mid = median_or_none(fifo_values)
            mailbox_mid = median_or_none(mailbox_values)
            if fifo_mid is None or mailbox_mid is None or fifo_mid == 0:
                return "n/a"
            delta = (mailbox_mid - fifo_mid) / fifo_mid * 100.0
            direction = "slower" if delta > 0 else "faster" if delta < 0 else "unchanged"
            return f"{delta:+.2f}% ({direction})"

        fifo_modes = sorted({run["mode"] for run in fifo_runs})
        mailbox_modes = sorted({run["mode"] for run in mailbox_runs})
        fifo_latencies = sorted({run["latency"] for run in fifo_runs})
        mailbox_latencies = sorted({run["latency"] for run in mailbox_runs})
        fifo_frame_counts = sorted({run["frames"] for run in fifo_runs})
        mailbox_frame_counts = sorted({run["frames"] for run in mailbox_runs})
        setup_valid = (
            all(run["mode"] == "fifo" for run in fifo_runs)
            and all(run["mode"] == "mailbox" for run in mailbox_runs)
            and len(fifo_latencies) == 1
            and len(mailbox_latencies) == 1
            and fifo_latencies[0] != "not recorded"
            and mailbox_latencies[0] != "not recorded"
            and fifo_latencies == mailbox_latencies
            and len(fifo_frame_counts) == 1
            and len(mailbox_frame_counts) == 1
            and fifo_frame_counts == mailbox_frame_counts
        )
        lines.extend([
            "",
            "## Direct FIFO vs Mailbox comparison",
            "",
            f"- FIFO captures: {len(fifo_runs)}; resolved modes: `{', '.join(fifo_modes)}`; frame-latency values: `{', '.join(fifo_latencies)}`; retained frame counts: `{', '.join(str(value) for value in fifo_frame_counts)}`.",
            f"- Mailbox captures: {len(mailbox_runs)}; resolved modes: `{', '.join(mailbox_modes)}`; frame-latency values: `{', '.join(mailbox_latencies)}`; retained frame counts: `{', '.join(str(value) for value in mailbox_frame_counts)}`.",
        ])
        if setup_valid:
            lines.append("Resolved present modes, frame-latency settings and retained frame counts match the requested A/B setup. The values below summarize run-level statistics; frame rows are not pooled.")
        else:
            lines.append("> **Invalid direct A/B setup:** resolved modes, frame-latency settings, or retained frame counts differ. Do not attribute timing deltas to present mode until captures are repeated with matching settings.")

        comparison_rows = [
            ("Frame wall mean (ms)", "Frame wall", "mean"),
            ("Frame wall P50 (ms)", "Frame wall", "p50"),
            ("Frame wall P95 (ms)", "Frame wall", "p95"),
            ("Frame wall P99 (ms)", "Frame wall", "p99"),
            ("Frame wall peak (ms)", "Frame wall", "peak"),
            ("Surface acquisition mean (ms)", "Surface acquisition", "mean"),
            ("Surface acquisition P50 (ms)", "Surface acquisition", "p50"),
            ("Surface acquisition P95 (ms)", "Surface acquisition", "p95"),
            ("Surface acquisition P99 (ms)", "Surface acquisition", "p99"),
            ("Render graph CPU mean (ms)", "Render graph CPU", "mean"),
            ("Render graph CPU P50 (ms)", "Render graph CPU", "p50"),
            ("Render graph CPU P95 (ms)", "Render graph CPU", "p95"),
            ("Render graph CPU P99 (ms)", "Render graph CPU", "p99"),
            ("Surface presentation mean (ms)", "Surface presentation", "mean"),
            ("Surface presentation P50 (ms)", "Surface presentation", "p50"),
            ("Surface presentation P95 (ms)", "Surface presentation", "p95"),
            ("Surface presentation P99 (ms)", "Surface presentation", "p99"),
            ("GPU shadow + mesh mean (ms)", "GPU shadow + mesh", "mean"),
            ("GPU shadow + mesh P50 (ms)", "GPU shadow + mesh", "p50"),
            ("GPU shadow + mesh P95 (ms)", "GPU shadow + mesh", "p95"),
            ("GPU shadow + mesh P99 (ms)", "GPU shadow + mesh", "p99"),
            ("GPU mesh pass mean (ms)", "GPU mesh pass", "mean"),
            ("GPU mesh pass P50 (ms)", "GPU mesh pass", "p50"),
            ("GPU mesh pass P95 (ms)", "GPU mesh pass", "p95"),
            ("GPU mesh pass P99 (ms)", "GPU mesh pass", "p99"),
        ]
        lines.extend([
            "",
            "| Metric | FIFO median [min, max] | Mailbox median [min, max] | Mailbox vs FIFO median delta |",
            "|---|---:|---:|---:|",
        ])
        for label, metric, stat in comparison_rows:
            fifo_values = group_metric(fifo_runs, metric, stat)
            mailbox_values = group_metric(mailbox_runs, metric, stat)
            lines.append(
                f"| {label} | {range_text(fifo_values)} | {range_text(mailbox_values)} | "
                f"{delta_text(fifo_values, mailbox_values)} |"
            )
        def range_pct_text(values: Iterable[float | None]) -> str:
            usable = [value for value in values if value is not None]
            if not usable:
                return "—"
            mid = median(usable)
            return f"{mid:.2f} [{min(usable):.2f}, {max(usable):.2f}]"

        fifo_over = [run["over_budget_pct"] for run in fifo_runs]
        mailbox_over = [run["over_budget_pct"] for run in mailbox_runs]
        lines.append(
            f"| Samples above frame budget (%) | {range_pct_text(fifo_over)} | {range_pct_text(mailbox_over)} | n/a (FIFO median may be zero) |"
        )

        common_passes = sorted(set.intersection(
            *(set(run["passes"]) for run in fifo_runs + mailbox_runs)
        )) if fifo_runs and mailbox_runs else []
        if common_passes:
            def pass_p95_values(runs: list[dict], name: str) -> list[float | None]:
                return [run["passes"].get(name, {}).get("p95") for run in runs]

            pass_rows = []
            for name in common_passes:
                fifo_pass = pass_p95_values(fifo_runs, name)
                mailbox_pass = pass_p95_values(mailbox_runs, name)
                fifo_mid = median_or_none(fifo_pass)
                mailbox_mid = median_or_none(mailbox_pass)
                gap = abs((mailbox_mid or 0.0) - (fifo_mid or 0.0))
                pass_rows.append((gap, name, fifo_pass, mailbox_pass))
            pass_rows.sort(reverse=True, key=lambda row: row[0])
            lines.extend([
                "",
                "### Render-graph pass CPU P95 comparison",
                "",
                "Pass durations are elapsed CPU-side node times and may include time spent inside backend/queue calls; they are not GPU execution timings.",
                "",
                "| Pass | FIFO median [min, max] ms | Mailbox median [min, max] ms | Mailbox vs FIFO median delta |",
                "|---|---:|---:|---:|",
            ])
            for _, name, fifo_pass, mailbox_pass in pass_rows:
                safe_name = name.replace("|", "\\|").replace("\n", " ")
                lines.append(
                    f"| {safe_name} | {range_text(fifo_pass)} | {range_text(mailbox_pass)} | "
                    f"{delta_text(fifo_pass, mailbox_pass)} |"
                )

        lines.append("")
        lines.append("Positive timing deltas mean Mailbox was slower on that statistic; negative deltas mean Mailbox was faster. Frame rows contain only successfully recorded frames; acquisition errors and dropped frames are not currently counted as separate CSV records. Results are adapter-, display-, driver- and workload-specific.")

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
