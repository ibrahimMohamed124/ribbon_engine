# Post-processing command submission merge

`ToneMapper::render` formerly created and submitted one command buffer for the bloom pass, then created and submitted a second command buffer for tone mapping. It now records both ordered render passes in the same encoder and submits once.

This change reduces the number of post-processing queue submissions from two to one per call. It does not guarantee a frame-time improvement; verify with the existing profile capture workflow. Other clear submissions and renderer call sites are intentionally unchanged because they serve additional window/offscreen paths and have not been proven redundant across all paths.

## Controlled A/B experiment (2026-10-24)

### Variants and controls

Both variants were based on source commit `78e0a0b76f8fb83e665fb7f8dd56a97400629904` in separate detached worktrees. A restored `plugins/render3d/src/post.rs` from `7a56ec3^`, which records Bloom and tone mapping in two encoders and submits twice. B used the current implementation, which records the same two ordered render passes in one encoder and submits once. The only source difference was `ToneMapper::render` in that file. HDR/bloom texture ownership, pass order, clears, tone-map output, and window/offscreen call sites were unchanged.

Each variant passed `cargo fmt --all --check` and `cargo build --release -p graphics_showcase` (both exit code 0). Build outputs were isolated in `target-A` and `target-B`. The executable SHA-256 values were:

- A: `6A3EFE64C98DC2D3C4596C0C58428F0A2D309A3C675AA7FEE8B1D8D664BD3F83`
- B: `B239098D51B11272186A0948F57C2E9ACB7A1CB3A2375C7EEE595F76EDCEEC7B`

There were ten captures for each combination of variant and presentation mode (40 captures total). The order alternated by repetition, reversing whether A or B ran first for each mode. All used the same generated 64-model corpus (identical corpus hash), release profile, 1,050 total frames, 60 warmup frames, 991 retained frame rows, grid 32, 32 active placements, 32 prefetched assets, 1 MiB unpinned CPU-cache budget, 300-frame lifecycle, 128-pixel textures, and desired frame latency 1. All selected the requested FIFO or Mailbox mode at 1280 × 800 on NVIDIA GeForce RTX 2050, driver 581.86, Vulkan.

### P95 results

Each value is the median of ten run-level P95 values; brackets show the observed minimum and maximum across those ten runs. The paired column counts same-repetition comparisons where B's P95 was lower than A's.

| Mode | Metric | A median [min, max] ms | B median [min, max] ms | B vs A | Paired runs lower |
|---|---|---:|---:|---:|---:|
| FIFO | Frame wall | 14.186 [13.881, 14.390] | 14.240 [13.784, 15.303] | +0.38% | 3/10 |
| FIFO | Surface acquisition | 13.215 [13.010, 13.542] | 13.279 [13.016, 14.366] | +0.49% | 4/10 |
| FIFO | Render graph CPU | 2.094 [1.719, 2.467] | 2.063 [1.765, 2.346] | -1.46% | 5/10 |
| FIFO | Surface presentation | 0.154 [0.126, 0.204] | 0.142 [0.130, 0.202] | -7.68% | 4/10 |
| FIFO | GPU shadow + mesh | 0.290 [0.093, 0.315] | 0.287 [0.092, 0.317] | -1.14% | 3/10 |
| FIFO | GPU mesh pass | 0.121 [0.042, 0.175] | 0.120 [0.047, 0.198] | -0.68% | 2/10 |
| Mailbox | Frame wall | 2.156 [1.564, 6.893] | 2.043 [1.770, 2.603] | -5.22% | 6/10 |
| Mailbox | Surface acquisition | 1.120 [0.662, 5.925] | 1.025 [0.840, 1.781] | -8.51% | 6/10 |
| Mailbox | Render graph CPU | 0.734 [0.576, 1.325] | 0.703 [0.511, 0.872] | -4.28% | 9/10 |
| Mailbox | Surface presentation | 0.075 [0.051, 0.114] | 0.060 [0.048, 0.080] | -20.44% | 9/10 |
| Mailbox | GPU shadow + mesh | 0.073 [0.071, 0.075] | 0.075 [0.072, 0.076] | +2.10% | 3/10 |
| Mailbox | GPU mesh pass | 0.026 [0.026, 0.027] | 0.026 [0.026, 0.026] | +0.00% | 1/10 |

Means, P50 and P99 for every metric, plus all individual run results and paired deltas, are in `target/profiles/post-submit-ab/20261024-validation/captures/experiment-results.md` and `run-level-results.csv`. For example, Mailbox frame-wall mean was 0.725 [0.573, 1.259] ms in A and 0.670 [0.594, 0.734] ms in B (-7.53%); P50 was 0.459 [0.436, 0.599] vs 0.442 [0.421, 0.469] ms (-3.70%); P99 was 5.687 [2.654, 7.962] vs 4.681 [2.626, 5.884] ms (-17.70%). The large ranges and only six of ten paired frame-P95 improvements show that tail-frame results are variable.

### Interpretation and limits

The most repeatable direct signal is a small CPU-side reduction in the Mailbox render-graph P95: B was lower in nine of ten paired repetitions, with a 4.28% median-of-runs reduction (about 31 microseconds). Mailbox surface-presentation P95 was also lower in nine of ten pairs, but the absolute median difference was about 15 microseconds. FIFO graph P95 changed by only -1.46%, with five of ten pairs lower; FIFO frame-wall P95 was 0.38% higher in B. The overall Mailbox frame-wall P95 median was 5.22% lower, but B was lower in only six of ten pairs and the run ranges overlap.

Surface-acquisition P95 was 8.51% lower by the across-run medians for Mailbox, but only six of ten same-repetition comparisons were lower, and its run-to-run spread was large. Acquisition occurs before the render graph and these results do not establish that fewer post-processing submissions caused that change. The available GPU timestamp metrics cover shadow/mesh work, not Bloom or tone mapping, and showed no improvement. No separate counter records acquisition errors or dropped frames; all captures did produce 991 contiguous successful frame records and skipped zero GPU timestamp samples.

**Conclusion: inconclusive for a material end-to-end frame-time improvement.** This experiment supports a small CPU-side Mailbox graph/present benefit, but it does not establish a reliable frame-wall improvement or a surface-acquisition improvement attributable to the merge. It provides no evidence of higher GPU throughput. No further rendering change is justified by these measurements alone. Compilation and all 40 runs succeeded; no pixel-by-pixel image comparison was performed.

### Commands and artifacts

The release build command was run from each variant worktree with that variant's own target directory:

```powershell
$env:CARGO_TARGET_DIR = '<experiment-root>\target-A' # target-B for variant B
cargo fmt --all --check
cargo build --release -p graphics_showcase
```

The ten-round capture sequence was run with these exact orchestration commands from the project root:

```powershell
& .\target\profiles\post-submit-ab\20261024-validation\run-ab-experiment.ps1
& .\target\profiles\post-submit-ab\20261024-validation\run-extra-five.ps1
python .\target\profiles\post-submit-ab\20261024-validation\analyze_ab.py .\target\profiles\post-submit-ab\20261024-validation
```

Each per-capture `.log` records the executable path and hash, source/variant identity, exact argument list, environment-selected presentation mode and latency, start/end time, exit code, adapter, driver, resolution, and report status. Build logs record both formatting/build exit codes and executable hashes. The captures, logs, reports, manifests, executable hashes and machine-readable results are retained under `target/profiles/post-submit-ab/20261024-validation/`; the base branch and its committed source were left unchanged during the experiment.
