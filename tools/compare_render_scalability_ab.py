#!/usr/bin/env python3
"""Compare two validated run-level rendering scalability reports."""

from __future__ import annotations

import argparse
import csv
import statistics
from collections import defaultdict
from pathlib import Path


def read_results(path: Path) -> dict[tuple[str, str, str], list[dict[str, str]]]:
    groups: dict[tuple[str, str, str], list[dict[str, str]]] = defaultdict(list)
    with path.open("r", encoding="utf-8-sig", newline="") as source:
        for row in csv.DictReader(source):
            key = (row["scene"], row["mode"], row["metric"])
            groups[key].append(row)
    expected = {(scene, mode) for scene in ("small", "medium", "large") for mode in ("fifo", "mailbox")}
    if not groups:
        raise ValueError(f"{path}: no run-level results")
    for (scene, mode, metric), runs in groups.items():
        if (scene, mode) not in expected:
            raise ValueError(f"{path}: unexpected workload/mode {(scene, mode)}")
        if len(runs) != 5:
            raise ValueError(f"{path}: expected five repeats for {(scene, mode, metric)}, found {len(runs)}")
        repeats = sorted(int(run["repeat"]) for run in runs)
        if repeats != [1, 2, 3, 4, 5]:
            raise ValueError(f"{path}: incomplete repeats for {(scene, mode, metric)}: {repeats}")
    return groups


def stats(groups: dict[tuple[str, str, str], list[dict[str, str]]], key: tuple[str, str, str]) -> tuple[float, float, float]:
    rows = groups[key]
    values = sorted(float(row["p95"]) for row in rows)
    return statistics.median(values), values[0], values[-1]


def percentile_stats(
    groups: dict[tuple[str, str, str], list[dict[str, str]]],
    key: tuple[str, str, str],
    percentile: str,
) -> tuple[float, float, float]:
    values = sorted(float(row[percentile]) for row in groups[key])
    return statistics.median(values), values[0], values[-1]


def cell(value: tuple[float, float, float]) -> str:
    median, low, high = value
    return f"{median:.3f} [{low:.3f}, {high:.3f}]"


def change(before: float, after: float) -> str:
    if before == 0:
        return "n/a"
    return f"{(after / before - 1.0) * 100:+.1f}%"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("baseline", type=Path, help="baseline run-level-results.csv")
    parser.add_argument("optimized", type=Path, help="optimized run-level-results.csv")
    parser.add_argument("output", type=Path, help="Markdown output path")
    parser.add_argument("--details-csv", type=Path, help="optional full metric comparison CSV")
    args = parser.parse_args()
    baseline = read_results(args.baseline)
    optimized = read_results(args.optimized)
    if baseline.keys() != optimized.keys():
        raise ValueError("baseline and optimized results do not contain identical workload/mode/metric groups")

    phases = sorted({metric for _, _, metric in baseline if metric.startswith("CPU phase:")})
    modes = ("fifo", "mailbox")
    scenes = (("small", 64), ("medium", 576), ("large", 1600))
    lines = [
        "# CPU rendering pipeline A/B comparison", "",
        "## Experimental setup and conclusion", "",
        "- **A (baseline):** profiler-enabled source with one `server.get(placement.model)` per model placement.",
        "- **B (optimized):** deduplicates `(asset id, generation)` before `server.get`, resolving a shared model once per distinct handle per extracted scene. Live-handle and revision tracking behavior remains the same.",
        "- This was the only intended A/B source difference. Both are release builds from `ce26091b710c6dab4ac38fc027bcda0efc65872c`, dirty because profiling work was uncommitted. A SHA-256: `8B8650638D514317E70DDABA8CE9A1B9E5BBCFEEB199A0D520DAA891EFDF920E`; B: `35A4C3227887B9DC7CBC08D872C193D71FAE7A220E29ECE75C9F871C5CDB273E`.",
        "- Same environment: Windows 10.0.26200, Rust 1.99.1, Vulkan, NVIDIA GeForce RTX 2050 driver 581.86. Each run used 1280×800, desired frame latency 1, a static one-model/one-material workload, 64/576/1,600 placements, FIFO or Mailbox, 60 warmup frames, 600 total and 541 retained frames; five captures per size/mode.",
        "- Each variant has 30 accepted captures. Every capture has 1,076–1,080 GPU timing records; GPU dropped/unavailable and surface/render-graph error counters are zero. Async GPU timing coverage is not one-to-one with frames; pending queries at capture boundaries are separately reported.",
        "- A ran as one group followed by B; workload/mode order rotated within each group but variants were not interleaved. Temperature/background drift therefore limits whole-frame attribution. At medium and large sizes, all five asset-reconciliation P95 ranges are separated between variants.",
        "- **Conclusion:** asset-cache reconciliation has a measurable phase improvement: median run-level P95 is about 68–73% lower for 576 and 1,600 placements in both modes. End-to-end frame time does not show a meaningful improvement: FIFO P95 shifts about +1–2%, while Mailbox frame and acquisition P95 are noisy and higher in B. No pixel-equivalence check was performed.",
        "", "### Exact commands", "",
        "Commands are from the repository root. Build A from the pre-optimization profiler source, copy its executable, then build B from the optimized source and copy it separately. Each capture script invocation uses the named prebuilt executable:", "",
        "The A/B captures predate this package-namespace rename; the historical package IDs in the recorded commands below are preserved as-run.", "",
        "```powershell",
        "cargo fmt --all -- --check",
        "cargo check -p plugin_render3d -p engine_render",
        "cargo build --release -p graphics_showcase",
        "Copy-Item target\\release\\graphics_showcase.exe target\\profiles\\render-cpu-ab\\graphics_showcase-baseline.exe",
        "# Select the B source variant, then:",
        "cargo build --release -p graphics_showcase",
        "Copy-Item target\\release\\graphics_showcase.exe target\\profiles\\render-cpu-ab\\graphics_showcase-optimized.exe",
        "& .\\tools\\run_render_scalability_profile.ps1 -Executable 'target\\profiles\\render-cpu-ab\\graphics_showcase-baseline.exe' -Variant 'cpu-phase-baseline' -OutputRoot 'target\\profiles\\render-cpu-ab\\baseline' -BuildCommand 'cargo build --release -p graphics_showcase'",
        "& .\\tools\\run_render_scalability_profile.ps1 -Executable 'target\\profiles\\render-cpu-ab\\graphics_showcase-optimized.exe' -Variant 'cpu-phase-cache-dedup' -OutputRoot 'target\\profiles\\render-cpu-ab\\optimized' -BuildCommand 'cargo build --release -p graphics_showcase'",
        "python tools\\analyze_render_scalability.py target\\profiles\\render-cpu-ab\\baseline\\20261024-183953\\captures target\\profiles\\render-cpu-ab\\baseline\\20261024-183953",
        "python tools\\analyze_render_scalability.py target\\profiles\\render-cpu-ab\\optimized\\20261024-184601\\captures target\\profiles\\render-cpu-ab\\optimized\\20261024-184601",
        "python tools\\compare_render_scalability_ab.py target\\profiles\\render-cpu-ab\\baseline\\20261024-183953\\run-level-results.csv target\\profiles\\render-cpu-ab\\optimized\\20261024-184601\\run-level-results.csv docs\\performance-results\\render-cpu-phase-ab-20261024.md --details-csv docs\\performance-results\\render-cpu-phase-ab-runs-20261024.csv",
        "```", "",
        "The companion CSV gives the median and minimum/maximum across five captures for mean, P50, P95 and P99, plus percent changes, for all compared workload metrics. The original run-level CSVs retain the individual runs. The tables below emphasize P95 and show the median of five per-run P95s with the five-run minimum and maximum in brackets.", "",
        "Each cell shows the median run-level P95 in milliseconds; brackets give the minimum and maximum of five runs. Percent change is `(optimized / baseline - 1) × 100`; negative means lower latency. These are P95s over 541 recorded frames, then summarized across five captures.", "",
        "## Target phase: asset-cache reconciliation", "",
        "| Placements | Mode | Baseline P95 [min,max] | Optimized P95 [min,max] | Change |",
        "|---:|---|---:|---:|---:|",
    ]
    details: list[dict[str, str]] = []

    def add_detail(scene: str, placements: int, mode: str, metric: str) -> tuple[float, float, float]:
        before = stats(baseline, (scene, mode, metric))
        after = stats(optimized, (scene, mode, metric))
        detail = {
            "scene": scene, "placements": str(placements), "mode": mode, "metric": metric,
            "baseline_median_p95_ms": f"{before[0]:.6f}",
            "baseline_min_p95_ms": f"{before[1]:.6f}",
            "baseline_max_p95_ms": f"{before[2]:.6f}",
            "optimized_median_p95_ms": f"{after[0]:.6f}",
            "optimized_min_p95_ms": f"{after[1]:.6f}",
            "optimized_max_p95_ms": f"{after[2]:.6f}",
            "change_percent": f"{((after[0] / before[0]) - 1) * 100:.6f}" if before[0] else "",
            "baseline_samples_per_run": ",".join(row["samples"] for row in sorted(baseline[(scene, mode, metric)], key=lambda row: int(row["repeat"]))),
            "optimized_samples_per_run": ",".join(row["samples"] for row in sorted(optimized[(scene, mode, metric)], key=lambda row: int(row["repeat"]))),
        }
        for percentile in ("mean", "p50", "p95", "p99"):
            before_stats = percentile_stats(baseline, (scene, mode, metric), percentile)
            after_stats = percentile_stats(optimized, (scene, mode, metric), percentile)
            detail[f"baseline_median_{percentile}"] = f"{before_stats[0]:.6f}"
            detail[f"baseline_min_{percentile}"] = f"{before_stats[1]:.6f}"
            detail[f"baseline_max_{percentile}"] = f"{before_stats[2]:.6f}"
            detail[f"optimized_median_{percentile}"] = f"{after_stats[0]:.6f}"
            detail[f"optimized_min_{percentile}"] = f"{after_stats[1]:.6f}"
            detail[f"optimized_max_{percentile}"] = f"{after_stats[2]:.6f}"
            detail[f"{percentile}_change_percent"] = f"{((after_stats[0] / before_stats[0]) - 1) * 100:.6f}" if before_stats[0] else ""
        details.append(detail)
        return before, after

    for scene, placements in scenes:
        for mode in modes:
            before, after = add_detail(scene, placements, mode, "CPU phase: asset cache reconciliation")
            lines.append(f"| {placements} | {mode} | {cell(before)} | {cell(after)} | {change(before[0], after[0])} |")

    outer_metrics = ("Frame wall", "Surface acquisition wall", "Render graph CPU")
    lines.extend([
        "", "## End-to-end and graph timings", "",
        "| Placements | Mode | Metric | Baseline P95 [min,max] | Optimized P95 [min,max] | Change |",
        "|---:|---|---|---:|---:|---:|",
    ])
    for scene, placements in scenes:
        for mode in modes:
            for metric in outer_metrics:
                before, after = add_detail(scene, placements, mode, metric)
                lines.append(f"| {placements} | {mode} | {metric} | {cell(before)} | {cell(after)} | {change(before[0], after[0])} |")

    lines.extend([
        "", "## Other CPU phase P95s", "",
        "| Placements | Mode | Phase | Baseline [min,max] | Optimized [min,max] | Change |",
        "|---:|---|---|---:|---:|---:|",
    ])
    for scene, placements in scenes:
        for mode in modes:
            for phase in phases:
                if phase == "CPU phase: asset cache reconciliation":
                    continue
                before, after = add_detail(scene, placements, mode, phase)
                name = phase.removeprefix("CPU phase: ")
                lines.append(f"| {placements} | {mode} | {name} | {cell(before)} | {cell(after)} | {change(before[0], after[0])} |")

    gpu_timing_metrics = {
        "GPU shadow + mesh",
        "GPU shadow passes",
        "GPU mesh pass",
        "GPU Bloom pass",
        "GPU tone-mapping pass",
    }
    gpu_metrics = sorted({metric for _, _, metric in baseline if metric in gpu_timing_metrics})
    if gpu_metrics:
        lines.extend([
            "", "## Available GPU timings", "",
            "| Placements | Mode | Metric | Baseline median P95 | Optimized median P95 | Change |",
            "|---:|---|---|---:|---:|---:|",
        ])
        for scene, placements in scenes:
            for mode in modes:
                for metric in gpu_metrics:
                    before, after = add_detail(scene, placements, mode, metric)
                    lines.append(f"| {placements} | {mode} | {metric} | {before[0]:.3f} | {after[0]:.3f} | {change(before[0], after[0])} |")

    lines.extend([
        "", "## Capture validation", "",
        f"- Baseline run-level CSV: `{args.baseline}`",
        f"- Optimized run-level CSV: `{args.optimized}`",
        "- Baseline manifest, logs and raw frame/GPU/phase CSVs: `target\\profiles\\render-cpu-ab\\baseline\\20261024-183953\\captures`.",
        "- Optimized manifest, logs and raw frame/GPU/phase CSVs: `target\\profiles\\render-cpu-ab\\optimized\\20261024-184601\\captures`.",
        "- Both datasets must contain exactly five runs per scene, mode, and metric; capture validation separately verifies 541 contiguous frame rows, GPU sample association/coverage, configuration, and report-to-CSV agreement.",
    ])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(lines) + "\n", encoding="utf-8")
    if args.details_csv:
        args.details_csv.parent.mkdir(parents=True, exist_ok=True)
        with args.details_csv.open("w", encoding="utf-8", newline="") as destination:
            writer = csv.DictWriter(destination, fieldnames=list(details[0]))
            writer.writeheader()
            writer.writerows(details)
    print(f"wrote A/B comparison: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
