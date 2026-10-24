# CPU rendering pipeline A/B comparison

## Experimental setup and conclusion

- **A (baseline):** profiler-enabled source with one `server.get(placement.model)` per model placement.
- **B (optimized):** deduplicates `(asset id, generation)` before `server.get`, resolving a shared model once per distinct handle per extracted scene. Live-handle and revision tracking behavior remains the same.
- This was the only intended A/B source difference. Both are release builds from `ce26091b710c6dab4ac38fc027bcda0efc65872c`, dirty because profiling work was uncommitted. A SHA-256: `8B8650638D514317E70DDABA8CE9A1B9E5BBCFEEB199A0D520DAA891EFDF920E`; B: `35A4C3227887B9DC7CBC08D872C193D71FAE7A220E29ECE75C9F871C5CDB273E`.
- Same environment: Windows 10.0.26200, Rust 1.99.1, Vulkan, NVIDIA GeForce RTX 2050 driver 581.86. Each run used 1280×800, desired frame latency 1, a static one-model/one-material workload, 64/576/1,600 placements, FIFO or Mailbox, 60 warmup frames, 600 total and 541 retained frames; five captures per size/mode.
- Each variant has 30 accepted captures. Every capture has 1,076–1,080 GPU timing records; GPU dropped/unavailable and surface/render-graph error counters are zero. Async GPU timing coverage is not one-to-one with frames; pending queries at capture boundaries are separately reported.
- A ran as one group followed by B; workload/mode order rotated within each group but variants were not interleaved. Temperature/background drift therefore limits whole-frame attribution. At medium and large sizes, all five asset-reconciliation P95 ranges are separated between variants.
- **Conclusion:** asset-cache reconciliation has a measurable phase improvement: median run-level P95 is about 68–73% lower for 576 and 1,600 placements in both modes. End-to-end frame time does not show a meaningful improvement: FIFO P95 shifts about +1–2%, while Mailbox frame and acquisition P95 are noisy and higher in B. No pixel-equivalence check was performed.

### Exact commands

Commands are from the repository root. Build A from the pre-optimization profiler source, copy its executable, then build B from the optimized source and copy it separately. Each capture script invocation uses the named prebuilt executable:

```powershell
cargo fmt --all -- --check
cargo check -p plugin_render3d -p engine_render
cargo build --release -p graphics_showcase
Copy-Item target\release\graphics_showcase.exe target\profiles\render-cpu-ab\graphics_showcase-baseline.exe
# Select the B source variant, then:
cargo build --release -p graphics_showcase
Copy-Item target\release\graphics_showcase.exe target\profiles\render-cpu-ab\graphics_showcase-optimized.exe
& .\tools\run_render_scalability_profile.ps1 -Executable 'target\profiles\render-cpu-ab\graphics_showcase-baseline.exe' -Variant 'cpu-phase-baseline' -OutputRoot 'target\profiles\render-cpu-ab\baseline' -BuildCommand 'cargo build --release -p graphics_showcase'
& .\tools\run_render_scalability_profile.ps1 -Executable 'target\profiles\render-cpu-ab\graphics_showcase-optimized.exe' -Variant 'cpu-phase-cache-dedup' -OutputRoot 'target\profiles\render-cpu-ab\optimized' -BuildCommand 'cargo build --release -p graphics_showcase'
python tools\analyze_render_scalability.py target\profiles\render-cpu-ab\baseline\20261024-183953\captures target\profiles\render-cpu-ab\baseline\20261024-183953
python tools\analyze_render_scalability.py target\profiles\render-cpu-ab\optimized\20261024-184601\captures target\profiles\render-cpu-ab\optimized\20261024-184601
python tools\compare_render_scalability_ab.py target\profiles\render-cpu-ab\baseline\20261024-183953\run-level-results.csv target\profiles\render-cpu-ab\optimized\20261024-184601\run-level-results.csv docs\performance-results\render-cpu-phase-ab-20261024.md --details-csv docs\performance-results\render-cpu-phase-ab-runs-20261024.csv
```

The companion CSV gives the median and minimum/maximum across five captures for mean, P50, P95 and P99, plus percent changes, for all compared workload metrics. The original run-level CSVs retain the individual runs. The tables below emphasize P95 and show the median of five per-run P95s with the five-run minimum and maximum in brackets.

Each cell shows the median run-level P95 in milliseconds; brackets give the minimum and maximum of five runs. Percent change is `(optimized / baseline - 1) × 100`; negative means lower latency. These are P95s over 541 recorded frames, then summarized across five captures.

## Target phase: asset-cache reconciliation

| Placements | Mode | Baseline P95 [min,max] | Optimized P95 [min,max] | Change |
|---:|---|---:|---:|---:|
| 64 | fifo | 0.024 [0.022, 0.029] | 0.015 [0.014, 0.018] | -35.3% |
| 64 | mailbox | 0.009 [0.007, 0.023] | 0.007 [0.006, 0.007] | -23.9% |
| 576 | fifo | 0.157 [0.119, 0.173] | 0.050 [0.044, 0.052] | -67.9% |
| 576 | mailbox | 0.057 [0.041, 0.067] | 0.016 [0.013, 0.021] | -72.8% |
| 1600 | fifo | 0.384 [0.344, 0.391] | 0.104 [0.100, 0.109] | -72.9% |
| 1600 | mailbox | 0.132 [0.108, 0.212] | 0.041 [0.032, 0.055] | -69.2% |

## End-to-end and graph timings

| Placements | Mode | Metric | Baseline P95 [min,max] | Optimized P95 [min,max] | Change |
|---:|---|---|---:|---:|---:|
| 64 | fifo | Frame wall | 14.227 [14.046, 14.385] | 14.464 [14.271, 14.618] | +1.7% |
| 64 | fifo | Surface acquisition wall | 13.111 [12.832, 13.227] | 13.163 [13.145, 13.749] | +0.4% |
| 64 | fifo | Render graph CPU | 1.351 [1.251, 1.508] | 1.492 [1.342, 1.646] | +10.5% |
| 64 | mailbox | Frame wall | 1.511 [1.288, 3.494] | 2.242 [1.718, 2.610] | +48.4% |
| 64 | mailbox | Surface acquisition wall | 1.183 [0.797, 2.275] | 1.696 [1.282, 2.229] | +43.4% |
| 64 | mailbox | Render graph CPU | 0.502 [0.367, 1.267] | 0.569 [0.439, 0.616] | +13.5% |
| 576 | fifo | Frame wall | 14.160 [13.898, 14.359] | 14.395 [13.963, 14.559] | +1.7% |
| 576 | fifo | Surface acquisition wall | 12.803 [12.635, 12.909] | 13.095 [12.886, 13.332] | +2.3% |
| 576 | fifo | Render graph CPU | 1.825 [1.604, 1.906] | 1.809 [1.590, 1.970] | -0.9% |
| 576 | mailbox | Frame wall | 1.336 [1.096, 1.833] | 1.746 [1.317, 2.506] | +30.7% |
| 576 | mailbox | Surface acquisition wall | 0.484 [0.321, 1.317] | 1.209 [0.935, 2.069] | +149.9% |
| 576 | mailbox | Render graph CPU | 0.841 [0.436, 1.071] | 0.624 [0.496, 0.725] | -25.7% |
| 1600 | fifo | Frame wall | 14.193 [14.016, 14.456] | 14.403 [14.242, 14.427] | +1.5% |
| 1600 | fifo | Surface acquisition wall | 12.634 [12.355, 12.762] | 12.802 [12.706, 13.020] | +1.3% |
| 1600 | fifo | Render graph CPU | 2.435 [2.042, 2.445] | 2.038 [1.853, 2.149] | -16.3% |
| 1600 | mailbox | Frame wall | 1.201 [0.884, 2.568] | 2.010 [1.525, 2.344] | +67.3% |
| 1600 | mailbox | Surface acquisition wall | 0.178 [0.129, 0.563] | 1.008 [0.710, 1.687] | +467.5% |
| 1600 | mailbox | Render graph CPU | 0.986 [0.646, 2.031] | 0.901 [0.762, 1.087] | -8.7% |

## Other CPU phase P95s

| Placements | Mode | Phase | Baseline [min,max] | Optimized [min,max] | Change |
|---:|---|---|---:|---:|---:|
| 64 | fifo | Bloom command encoding | 0.063 [0.053, 0.079] | 0.074 [0.065, 0.086] | +17.9% |
| 64 | fifo | GPU timestamp polling | 0.020 [0.019, 0.033] | 0.024 [0.020, 0.031] | +19.0% |
| 64 | fifo | built-in scene conversion | 0.001 [0.000, 0.001] | 0.001 [0.001, 0.001] | +0.0% |
| 64 | fifo | draw validation, sorting, and visibility | 0.017 [0.015, 0.019] | 0.018 [0.017, 0.019] | +12.1% |
| 64 | fifo | instance buffer updates | 0.043 [0.041, 0.049] | 0.054 [0.043, 0.062] | +23.8% |
| 64 | fifo | instance data preparation | 0.010 [0.005, 0.018] | 0.010 [0.004, 0.012] | +3.1% |
| 64 | fifo | mesh pass command encoding | 0.071 [0.062, 0.075] | 0.091 [0.075, 0.094] | +28.4% |
| 64 | fifo | mesh queue submission | 0.117 [0.115, 0.134] | 0.134 [0.129, 0.149] | +14.3% |
| 64 | fifo | model/voxel resolution and draw assembly | 0.021 [0.018, 0.028] | 0.023 [0.020, 0.024] | +10.0% |
| 64 | fifo | post-process queue submission | 0.118 [0.103, 0.124] | 0.130 [0.112, 0.145] | +9.7% |
| 64 | fifo | profile recorder bookkeeping | 0.014 [0.013, 0.020] | 0.013 [0.011, 0.019] | -6.4% |
| 64 | fifo | shadow pass command encoding | 0.108 [0.092, 0.132] | 0.132 [0.105, 0.138] | +22.1% |
| 64 | fifo | tone-mapping command encoding | 0.042 [0.037, 0.045] | 0.046 [0.043, 0.050] | +11.5% |
| 64 | fifo | uniform preparation and updates | 0.012 [0.012, 0.014] | 0.015 [0.013, 0.016] | +22.0% |
| 64 | mailbox | Bloom command encoding | 0.024 [0.015, 0.055] | 0.018 [0.016, 0.027] | -27.9% |
| 64 | mailbox | GPU timestamp polling | 0.011 [0.006, 0.027] | 0.013 [0.009, 0.017] | +19.5% |
| 64 | mailbox | built-in scene conversion | 0.000 [0.000, 0.001] | 0.000 [0.000, 0.000] | +0.0% |
| 64 | mailbox | draw validation, sorting, and visibility | 0.009 [0.006, 0.019] | 0.010 [0.008, 0.011] | +12.4% |
| 64 | mailbox | instance buffer updates | 0.022 [0.014, 0.054] | 0.024 [0.017, 0.037] | +8.7% |
| 64 | mailbox | instance data preparation | 0.004 [0.004, 0.012] | 0.006 [0.003, 0.008] | +38.6% |
| 64 | mailbox | mesh pass command encoding | 0.028 [0.018, 0.062] | 0.027 [0.021, 0.037] | -6.0% |
| 64 | mailbox | mesh queue submission | 0.067 [0.059, 0.120] | 0.060 [0.055, 0.082] | -10.5% |
| 64 | mailbox | model/voxel resolution and draw assembly | 0.010 [0.006, 0.023] | 0.012 [0.009, 0.013] | +11.5% |
| 64 | mailbox | post-process queue submission | 0.068 [0.061, 0.132] | 0.058 [0.050, 0.086] | -15.3% |
| 64 | mailbox | profile recorder bookkeeping | 0.008 [0.006, 0.013] | 0.010 [0.008, 0.011] | +23.1% |
| 64 | mailbox | shadow pass command encoding | 0.052 [0.032, 0.111] | 0.052 [0.041, 0.064] | -0.4% |
| 64 | mailbox | tone-mapping command encoding | 0.017 [0.012, 0.035] | 0.014 [0.012, 0.017] | -17.6% |
| 64 | mailbox | uniform preparation and updates | 0.005 [0.004, 0.012] | 0.006 [0.005, 0.007] | +23.1% |
| 576 | fifo | Bloom command encoding | 0.069 [0.060, 0.091] | 0.082 [0.057, 0.088] | +20.1% |
| 576 | fifo | GPU timestamp polling | 0.026 [0.023, 0.026] | 0.023 [0.023, 0.029] | -9.7% |
| 576 | fifo | built-in scene conversion | 0.001 [0.000, 0.001] | 0.001 [0.001, 0.001] | +0.0% |
| 576 | fifo | draw validation, sorting, and visibility | 0.097 [0.086, 0.119] | 0.100 [0.098, 0.114] | +3.4% |
| 576 | fifo | instance buffer updates | 0.076 [0.063, 0.082] | 0.076 [0.069, 0.091] | -0.3% |
| 576 | fifo | instance data preparation | 0.044 [0.025, 0.046] | 0.030 [0.026, 0.038] | -31.6% |
| 576 | fifo | mesh pass command encoding | 0.077 [0.073, 0.100] | 0.090 [0.070, 0.110] | +16.8% |
| 576 | fifo | mesh queue submission | 0.131 [0.116, 0.180] | 0.151 [0.121, 0.164] | +15.4% |
| 576 | fifo | model/voxel resolution and draw assembly | 0.120 [0.089, 0.132] | 0.118 [0.116, 0.140] | -1.5% |
| 576 | fifo | post-process queue submission | 0.130 [0.097, 0.189] | 0.144 [0.107, 0.165] | +11.0% |
| 576 | fifo | profile recorder bookkeeping | 0.013 [0.011, 0.018] | 0.013 [0.011, 0.016] | +0.0% |
| 576 | fifo | shadow pass command encoding | 0.129 [0.116, 0.137] | 0.131 [0.115, 0.140] | +1.7% |
| 576 | fifo | tone-mapping command encoding | 0.042 [0.038, 0.048] | 0.050 [0.039, 0.058] | +19.4% |
| 576 | fifo | uniform preparation and updates | 0.014 [0.013, 0.015] | 0.015 [0.013, 0.017] | +7.6% |
| 576 | mailbox | Bloom command encoding | 0.028 [0.013, 0.038] | 0.020 [0.015, 0.033] | -27.7% |
| 576 | mailbox | GPU timestamp polling | 0.019 [0.005, 0.020] | 0.012 [0.009, 0.017] | -36.1% |
| 576 | mailbox | built-in scene conversion | 0.000 [0.000, 0.000] | 0.000 [0.000, 0.000] | -50.0% |
| 576 | mailbox | draw validation, sorting, and visibility | 0.062 [0.041, 0.078] | 0.052 [0.042, 0.072] | -15.9% |
| 576 | mailbox | instance buffer updates | 0.044 [0.019, 0.050] | 0.033 [0.024, 0.042] | -24.5% |
| 576 | mailbox | instance data preparation | 0.026 [0.013, 0.034] | 0.020 [0.018, 0.025] | -23.0% |
| 576 | mailbox | mesh pass command encoding | 0.041 [0.025, 0.051] | 0.030 [0.024, 0.038] | -27.5% |
| 576 | mailbox | mesh queue submission | 0.080 [0.058, 0.095] | 0.066 [0.053, 0.087] | -17.6% |
| 576 | mailbox | model/voxel resolution and draw assembly | 0.061 [0.044, 0.068] | 0.046 [0.038, 0.057] | -25.1% |
| 576 | mailbox | post-process queue submission | 0.057 [0.055, 0.098] | 0.055 [0.041, 0.069] | -4.0% |
| 576 | mailbox | profile recorder bookkeeping | 0.008 [0.007, 0.010] | 0.007 [0.005, 0.008] | -12.7% |
| 576 | mailbox | shadow pass command encoding | 0.071 [0.040, 0.079] | 0.051 [0.038, 0.070] | -27.7% |
| 576 | mailbox | tone-mapping command encoding | 0.019 [0.009, 0.021] | 0.015 [0.012, 0.018] | -21.5% |
| 576 | mailbox | uniform preparation and updates | 0.008 [0.004, 0.011] | 0.006 [0.005, 0.008] | -24.4% |
| 1600 | fifo | Bloom command encoding | 0.074 [0.064, 0.091] | 0.066 [0.057, 0.092] | -11.3% |
| 1600 | fifo | GPU timestamp polling | 0.024 [0.021, 0.037] | 0.023 [0.021, 0.037] | -4.1% |
| 1600 | fifo | built-in scene conversion | 0.000 [0.000, 0.001] | 0.001 [0.000, 0.001] | +25.0% |
| 1600 | fifo | draw validation, sorting, and visibility | 0.207 [0.180, 0.235] | 0.214 [0.185, 0.227] | +3.1% |
| 1600 | fifo | instance buffer updates | 0.137 [0.104, 0.156] | 0.107 [0.100, 0.141] | -22.3% |
| 1600 | fifo | instance data preparation | 0.075 [0.062, 0.082] | 0.073 [0.058, 0.083] | -2.7% |
| 1600 | fifo | mesh pass command encoding | 0.096 [0.084, 0.108] | 0.093 [0.077, 0.121] | -3.4% |
| 1600 | fifo | mesh queue submission | 0.162 [0.142, 0.175] | 0.141 [0.121, 0.175] | -13.0% |
| 1600 | fifo | model/voxel resolution and draw assembly | 0.243 [0.223, 0.286] | 0.262 [0.241, 0.265] | +7.9% |
| 1600 | fifo | post-process queue submission | 0.147 [0.111, 0.172] | 0.113 [0.101, 0.155] | -23.6% |
| 1600 | fifo | profile recorder bookkeeping | 0.011 [0.011, 0.013] | 0.014 [0.011, 0.015] | +19.1% |
| 1600 | fifo | shadow pass command encoding | 0.132 [0.118, 0.142] | 0.135 [0.113, 0.164] | +2.6% |
| 1600 | fifo | tone-mapping command encoding | 0.043 [0.041, 0.047] | 0.044 [0.035, 0.058] | +2.3% |
| 1600 | fifo | uniform preparation and updates | 0.015 [0.013, 0.016] | 0.016 [0.012, 0.016] | +4.7% |
| 1600 | mailbox | Bloom command encoding | 0.025 [0.015, 0.073] | 0.023 [0.020, 0.032] | -8.4% |
| 1600 | mailbox | GPU timestamp polling | 0.013 [0.007, 0.022] | 0.013 [0.010, 0.021] | +3.1% |
| 1600 | mailbox | built-in scene conversion | 0.000 [0.000, 0.000] | 0.000 [0.000, 0.000] | +0.0% |
| 1600 | mailbox | draw validation, sorting, and visibility | 0.108 [0.079, 0.162] | 0.108 [0.103, 0.125] | -0.2% |
| 1600 | mailbox | instance buffer updates | 0.048 [0.034, 0.081] | 0.048 [0.042, 0.054] | +0.8% |
| 1600 | mailbox | instance data preparation | 0.048 [0.034, 0.080] | 0.042 [0.034, 0.050] | -12.6% |
| 1600 | mailbox | mesh pass command encoding | 0.040 [0.023, 0.093] | 0.035 [0.030, 0.047] | -12.1% |
| 1600 | mailbox | mesh queue submission | 0.075 [0.052, 0.174] | 0.068 [0.062, 0.090] | -9.6% |
| 1600 | mailbox | model/voxel resolution and draw assembly | 0.107 [0.079, 0.141] | 0.101 [0.080, 0.115] | -6.1% |
| 1600 | mailbox | post-process queue submission | 0.051 [0.032, 0.169] | 0.047 [0.037, 0.052] | -7.7% |
| 1600 | mailbox | profile recorder bookkeeping | 0.008 [0.006, 0.012] | 0.008 [0.007, 0.009] | -1.3% |
| 1600 | mailbox | shadow pass command encoding | 0.060 [0.035, 0.138] | 0.052 [0.048, 0.080] | -13.0% |
| 1600 | mailbox | tone-mapping command encoding | 0.017 [0.011, 0.041] | 0.015 [0.013, 0.021] | -12.3% |
| 1600 | mailbox | uniform preparation and updates | 0.007 [0.004, 0.012] | 0.007 [0.006, 0.009] | +10.6% |

## Available GPU timings

| Placements | Mode | Metric | Baseline median P95 | Optimized median P95 | Change |
|---:|---|---|---:|---:|---:|
| 64 | fifo | GPU Bloom pass | 0.047 | 0.036 | -23.9% |
| 64 | fifo | GPU mesh pass | 0.071 | 0.055 | -21.5% |
| 64 | fifo | GPU shadow + mesh | 0.219 | 0.200 | -8.9% |
| 64 | fifo | GPU shadow passes | 0.013 | 0.009 | -30.8% |
| 64 | fifo | GPU tone-mapping pass | 0.161 | 0.129 | -19.6% |
| 64 | mailbox | GPU Bloom pass | 0.017 | 0.016 | -5.3% |
| 64 | mailbox | GPU mesh pass | 0.023 | 0.023 | +0.0% |
| 64 | mailbox | GPU shadow + mesh | 0.066 | 0.063 | -3.1% |
| 64 | mailbox | GPU shadow passes | 0.005 | 0.005 | +0.0% |
| 64 | mailbox | GPU tone-mapping pass | 0.026 | 0.026 | +0.0% |
| 576 | fifo | GPU Bloom pass | 0.046 | 0.036 | -22.2% |
| 576 | fifo | GPU mesh pass | 0.102 | 0.070 | -31.5% |
| 576 | fifo | GPU shadow + mesh | 0.261 | 0.238 | -8.9% |
| 576 | fifo | GPU shadow passes | 0.023 | 0.018 | -18.2% |
| 576 | fifo | GPU tone-mapping pass | 0.136 | 0.123 | -9.8% |
| 576 | mailbox | GPU Bloom pass | 0.017 | 0.017 | +0.0% |
| 576 | mailbox | GPU mesh pass | 0.022 | 0.022 | +0.0% |
| 576 | mailbox | GPU shadow + mesh | 0.068 | 0.066 | -3.0% |
| 576 | mailbox | GPU shadow passes | 0.008 | 0.008 | +0.0% |
| 576 | mailbox | GPU tone-mapping pass | 0.026 | 0.026 | +0.0% |
| 1600 | fifo | GPU Bloom pass | 0.070 | 0.046 | -33.6% |
| 1600 | fifo | GPU mesh pass | 0.151 | 0.135 | -10.2% |
| 1600 | fifo | GPU shadow + mesh | 0.331 | 0.305 | -7.7% |
| 1600 | fifo | GPU shadow passes | 0.047 | 0.039 | -17.4% |
| 1600 | fifo | GPU tone-mapping pass | 0.134 | 0.127 | -5.2% |
| 1600 | mailbox | GPU Bloom pass | 0.017 | 0.017 | +0.0% |
| 1600 | mailbox | GPU mesh pass | 0.034 | 0.033 | -3.0% |
| 1600 | mailbox | GPU shadow + mesh | 0.084 | 0.084 | +0.0% |
| 1600 | mailbox | GPU shadow passes | 0.018 | 0.018 | +0.0% |
| 1600 | mailbox | GPU tone-mapping pass | 0.026 | 0.026 | +0.0% |

## Capture validation

- Baseline run-level CSV: `target\profiles\render-cpu-ab\baseline\20261024-183953\run-level-results.csv`
- Optimized run-level CSV: `target\profiles\render-cpu-ab\optimized\20261024-184601\run-level-results.csv`
- Baseline manifest, logs and raw frame/GPU/phase CSVs: `target\profiles\render-cpu-ab\baseline\20261024-183953\captures`.
- Optimized manifest, logs and raw frame/GPU/phase CSVs: `target\profiles\render-cpu-ab\optimized\20261024-184601\captures`.
- Both datasets must contain exactly five runs per scene, mode, and metric; capture validation separately verifies 541 contiguous frame rows, GPU sample association/coverage, configuration, and report-to-CSV agreement.
