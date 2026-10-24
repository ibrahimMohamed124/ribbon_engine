# Active asset and texture scalability results

All statistics come from the raw, validated CSVs. Each active-asset count has five runs per presentation mode at a fixed 576 placements. Percentiles use linear interpolation at `(n - 1) * p`; brackets show min/max run-level P95 values. GPU values are asynchronous sequenced samples and counts are retained per run in the results CSV.

| Active unique assets | Mode | Metric | Sample counts | Median mean | Median P50 | Median P95 [min,max] | Median P99 |
|---:|---|---|---|---:|---:|---:|---:|
| 1 | fifo | Frame wall | 541 | 6.519 | 5.356 | 14.146 [14.129, 14.613] | 14.803 |
| 1 | fifo | Surface acquisition wall | 541 | 5.407 | 4.308 | 12.795 [12.692, 12.836] | 13.182 |
| 1 | fifo | Render graph CPU | 541 | 1.058 | 0.964 | 2.000 [1.645, 2.507] | 2.450 |
| 1 | fifo | Surface presentation wall | 541 | 0.079 | 0.068 | 0.172 [0.135, 0.187] | 0.241 |
| 1 | fifo | GPU shadow + mesh | 363,364,362,364,362 | 0.098 | 0.076 | 0.223 [0.219, 0.229] | 0.225 |
| 1 | fifo | GPU shadow passes | 363,364,362,364,362 | 0.014 | 0.008 | 0.023 [0.022, 0.025] | 0.024 |
| 1 | fifo | GPU mesh pass | 363,364,362,364,362 | 0.036 | 0.019 | 0.065 [0.062, 0.068] | 0.066 |
| 1 | fifo | GPU Bloom pass | 379,386,366,367,369 | 0.028 | 0.016 | 0.046 [0.045, 0.050] | 0.046 |
| 1 | fifo | GPU tone-mapping pass | 379,386,366,367,369 | 0.059 | 0.025 | 0.115 [0.109, 0.116] | 0.141 |
| 1 | fifo | Tracked GPU allocation estimate | 541 | 25539024.000 | 25539024.000 | 25539024.000 [25539024.000, 25539024.000] | 25539024.000 |
| 1 | fifo | Material texture allocation estimate | 541 | 87392.000 | 87392.000 | 87392.000 [87392.000, 87392.000] | 87392.000 |
| 1 | fifo | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| 1 | fifo | CPU resident asset estimate | 541 | 67920.000 | 67920.000 | 67920.000 [67920.000, 67920.000] | 67920.000 |
| 1 | fifo | Visible mesh instances | 541 | 576.000 | 576.000 | 576.000 [576.000, 576.000] | 576.000 |
| 1 | fifo | Visible draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| 1 | fifo | GPU model cache entries | 541,541,541,541,541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| 1 | fifo | Cumulative model uploads | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| 1 | mailbox | Frame wall | 541 | 0.687 | 0.621 | 1.363 [1.245, 2.164] | 1.789 |
| 1 | mailbox | Surface acquisition wall | 541 | 0.224 | 0.162 | 0.834 [0.485, 1.788] | 1.376 |
| 1 | mailbox | Render graph CPU | 541 | 0.402 | 0.344 | 0.746 [0.449, 0.832] | 1.142 |
| 1 | mailbox | Surface presentation wall | 541 | 0.048 | 0.040 | 0.088 [0.069, 0.123] | 0.144 |
| 1 | mailbox | GPU shadow + mesh | 416,399,478,382,360 | 0.041 | 0.035 | 0.065 [0.063, 0.066] | 0.067 |
| 1 | mailbox | GPU shadow passes | 416,399,478,382,360 | 0.008 | 0.007 | 0.008 [0.008, 0.008] | 0.008 |
| 1 | mailbox | GPU mesh pass | 416,399,478,382,360 | 0.017 | 0.017 | 0.019 [0.018, 0.019] | 0.019 |
| 1 | mailbox | GPU Bloom pass | 456,473,480,463,461 | 0.015 | 0.015 | 0.017 [0.016, 0.017] | 0.017 |
| 1 | mailbox | GPU tone-mapping pass | 456,473,480,463,461 | 0.023 | 0.023 | 0.026 [0.025, 0.026] | 0.026 |
| 1 | mailbox | Tracked GPU allocation estimate | 541 | 25539024.000 | 25539024.000 | 25539024.000 [25539024.000, 25539024.000] | 25539024.000 |
| 1 | mailbox | Material texture allocation estimate | 541 | 87392.000 | 87392.000 | 87392.000 [87392.000, 87392.000] | 87392.000 |
| 1 | mailbox | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| 1 | mailbox | CPU resident asset estimate | 541 | 67920.000 | 67920.000 | 67920.000 [67920.000, 67920.000] | 67920.000 |
| 1 | mailbox | Visible mesh instances | 541 | 576.000 | 576.000 | 576.000 [576.000, 576.000] | 576.000 |
| 1 | mailbox | Visible draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| 1 | mailbox | GPU model cache entries | 541,541,541,541,541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| 1 | mailbox | Cumulative model uploads | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| 8 | fifo | Frame wall | 541 | 6.553 | 5.672 | 14.167 [13.960, 14.385] | 15.142 |
| 8 | fifo | Surface acquisition wall | 541 | 5.456 | 4.424 | 12.745 [12.508, 12.912] | 13.403 |
| 8 | fifo | Render graph CPU | 541 | 1.069 | 0.916 | 2.089 [1.511, 2.531] | 2.772 |
| 8 | fifo | Surface presentation wall | 541 | 0.077 | 0.067 | 0.157 [0.132, 0.190] | 0.256 |
| 8 | fifo | GPU shadow + mesh | 363,374,362,370,363 | 0.116 | 0.097 | 0.263 [0.255, 0.270] | 0.266 |
| 8 | fifo | GPU shadow passes | 363,374,362,370,363 | 0.015 | 0.010 | 0.025 [0.022, 0.028] | 0.027 |
| 8 | fifo | GPU mesh pass | 363,374,362,370,363 | 0.054 | 0.028 | 0.101 [0.098, 0.104] | 0.104 |
| 8 | fifo | GPU Bloom pass | 378,402,400,385,366 | 0.033 | 0.041 | 0.051 [0.043, 0.057] | 0.052 |
| 8 | fifo | GPU tone-mapping pass | 378,402,400,385,366 | 0.073 | 0.061 | 0.134 [0.130, 0.135] | 0.148 |
| 8 | fifo | Tracked GPU allocation estimate | 541 | 26157404.000 | 26157404.000 | 26157404.000 [26157404.000, 26157404.000] | 26157404.000 |
| 8 | fifo | Material texture allocation estimate | 541 | 699052.000 | 699052.000 | 699052.000 [699052.000, 699052.000] | 699052.000 |
| 8 | fifo | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| 8 | fifo | CPU resident asset estimate | 541 | 543360.000 | 543360.000 | 543360.000 [543360.000, 543360.000] | 543360.000 |
| 8 | fifo | Visible mesh instances | 541 | 576.000 | 576.000 | 576.000 [576.000, 576.000] | 576.000 |
| 8 | fifo | Visible draw calls | 541 | 8.000 | 8.000 | 8.000 [8.000, 8.000] | 8.000 |
| 8 | fifo | GPU model cache entries | 541,541,541,541,541 | 8.000 | 8.000 | 8.000 [8.000, 8.000] | 8.000 |
| 8 | fifo | Cumulative model uploads | 541 | 8.000 | 8.000 | 8.000 [8.000, 8.000] | 8.000 |
| 8 | mailbox | Frame wall | 541 | 0.705 | 0.623 | 1.383 [1.211, 2.122] | 1.887 |
| 8 | mailbox | Surface acquisition wall | 541 | 0.235 | 0.205 | 0.796 [0.660, 1.743] | 1.301 |
| 8 | mailbox | Render graph CPU | 541 | 0.397 | 0.357 | 0.683 [0.471, 1.042] | 0.957 |
| 8 | mailbox | Surface presentation wall | 541 | 0.044 | 0.038 | 0.078 [0.056, 0.113] | 0.129 |
| 8 | mailbox | GPU shadow + mesh | 424,400,419,358,377 | 0.046 | 0.040 | 0.070 [0.069, 0.072] | 0.073 |
| 8 | mailbox | GPU shadow passes | 424,400,419,358,377 | 0.007 | 0.007 | 0.008 [0.008, 0.008] | 0.009 |
| 8 | mailbox | GPU mesh pass | 424,400,419,358,377 | 0.023 | 0.023 | 0.026 [0.025, 0.026] | 0.026 |
| 8 | mailbox | GPU Bloom pass | 479,469,470,463,466 | 0.016 | 0.015 | 0.017 [0.017, 0.017] | 0.018 |
| 8 | mailbox | GPU tone-mapping pass | 479,469,470,463,466 | 0.024 | 0.024 | 0.026 [0.026, 0.027] | 0.027 |
| 8 | mailbox | Tracked GPU allocation estimate | 541 | 26157404.000 | 26157404.000 | 26157404.000 [26157404.000, 26157404.000] | 26157404.000 |
| 8 | mailbox | Material texture allocation estimate | 541 | 699052.000 | 699052.000 | 699052.000 [699052.000, 699052.000] | 699052.000 |
| 8 | mailbox | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| 8 | mailbox | CPU resident asset estimate | 541 | 543360.000 | 543360.000 | 543360.000 [543360.000, 543360.000] | 543360.000 |
| 8 | mailbox | Visible mesh instances | 541 | 576.000 | 576.000 | 576.000 [576.000, 576.000] | 576.000 |
| 8 | mailbox | Visible draw calls | 541 | 8.000 | 8.000 | 8.000 [8.000, 8.000] | 8.000 |
| 8 | mailbox | GPU model cache entries | 541,541,541,541,541 | 8.000 | 8.000 | 8.000 [8.000, 8.000] | 8.000 |
| 8 | mailbox | Cumulative model uploads | 541 | 8.000 | 8.000 | 8.000 [8.000, 8.000] | 8.000 |
| 32 | fifo | Frame wall | 541 | 6.518 | 5.213 | 14.242 [13.855, 14.577] | 15.166 |
| 32 | fifo | Surface acquisition wall | 541 | 5.156 | 3.979 | 12.716 [12.407, 12.836] | 13.317 |
| 32 | fifo | Render graph CPU | 541 | 1.272 | 1.177 | 2.208 [1.165, 2.526] | 2.890 |
| 32 | fifo | Surface presentation wall | 541 | 0.078 | 0.065 | 0.159 [0.098, 0.193] | 0.243 |
| 32 | fifo | GPU shadow + mesh | 362,367,367,364,363 | 0.117 | 0.081 | 0.268 [0.263, 0.275] | 0.275 |
| 32 | fifo | GPU shadow passes | 362,367,367,364,363 | 0.015 | 0.009 | 0.025 [0.024, 0.029] | 0.027 |
| 32 | fifo | GPU mesh pass | 362,367,367,364,363 | 0.055 | 0.028 | 0.108 [0.104, 0.111] | 0.114 |
| 32 | fifo | GPU Bloom pass | 373,379,432,376,365 | 0.028 | 0.017 | 0.046 [0.043, 0.053] | 0.047 |
| 32 | fifo | GPU tone-mapping pass | 373,379,432,376,365 | 0.068 | 0.026 | 0.132 [0.130, 0.152] | 0.154 |
| 32 | fifo | Tracked GPU allocation estimate | 541 | 28277564.000 | 28277564.000 | 28277564.000 [28277564.000, 28277564.000] | 28277564.000 |
| 32 | fifo | Material texture allocation estimate | 541 | 2796172.000 | 2796172.000 | 2796172.000 [2796172.000, 2796172.000] | 2796172.000 |
| 32 | fifo | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| 32 | fifo | CPU resident asset estimate | 541 | 2173440.000 | 2173440.000 | 2173440.000 [2173440.000, 2173440.000] | 2173440.000 |
| 32 | fifo | Visible mesh instances | 541 | 576.000 | 576.000 | 576.000 [576.000, 576.000] | 576.000 |
| 32 | fifo | Visible draw calls | 541 | 32.000 | 32.000 | 32.000 [32.000, 32.000] | 32.000 |
| 32 | fifo | GPU model cache entries | 541,541,541,541,541 | 32.000 | 32.000 | 32.000 [32.000, 32.000] | 32.000 |
| 32 | fifo | Cumulative model uploads | 541 | 32.000 | 32.000 | 32.000 [32.000, 32.000] | 32.000 |
| 32 | mailbox | Frame wall | 541 | 0.730 | 0.651 | 1.526 [1.358, 2.180] | 1.938 |
| 32 | mailbox | Surface acquisition wall | 541 | 0.183 | 0.114 | 0.717 [0.149, 1.050] | 1.232 |
| 32 | mailbox | Render graph CPU | 541 | 0.495 | 0.412 | 0.955 [0.599, 1.871] | 1.379 |
| 32 | mailbox | Surface presentation wall | 541 | 0.049 | 0.040 | 0.095 [0.069, 0.180] | 0.145 |
| 32 | mailbox | GPU shadow + mesh | 443,436,501,533,440 | 0.048 | 0.040 | 0.071 [0.070, 0.073] | 0.075 |
| 32 | mailbox | GPU shadow passes | 443,436,501,533,440 | 0.008 | 0.008 | 0.009 [0.009, 0.009] | 0.010 |
| 32 | mailbox | GPU mesh pass | 443,436,501,533,440 | 0.024 | 0.024 | 0.027 [0.025, 0.027] | 0.028 |
| 32 | mailbox | GPU Bloom pass | 473,471,492,526,467 | 0.016 | 0.015 | 0.017 [0.016, 0.017] | 0.017 |
| 32 | mailbox | GPU tone-mapping pass | 473,471,492,526,467 | 0.024 | 0.024 | 0.026 [0.025, 0.026] | 0.027 |
| 32 | mailbox | Tracked GPU allocation estimate | 541 | 28277564.000 | 28277564.000 | 28277564.000 [28277564.000, 28277564.000] | 28277564.000 |
| 32 | mailbox | Material texture allocation estimate | 541 | 2796172.000 | 2796172.000 | 2796172.000 [2796172.000, 2796172.000] | 2796172.000 |
| 32 | mailbox | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| 32 | mailbox | CPU resident asset estimate | 541 | 2173440.000 | 2173440.000 | 2173440.000 [2173440.000, 2173440.000] | 2173440.000 |
| 32 | mailbox | Visible mesh instances | 541 | 576.000 | 576.000 | 576.000 [576.000, 576.000] | 576.000 |
| 32 | mailbox | Visible draw calls | 541 | 32.000 | 32.000 | 32.000 [32.000, 32.000] | 32.000 |
| 32 | mailbox | GPU model cache entries | 541,541,541,541,541 | 32.000 | 32.000 | 32.000 [32.000, 32.000] | 32.000 |
| 32 | mailbox | Cumulative model uploads | 541 | 32.000 | 32.000 | 32.000 [32.000, 32.000] | 32.000 |

## Interpretation

Placements, camera, lights, output size, presentation settings, and per-asset texture resolution are fixed; increasing active assets changes unique geometry/material/texture count and upload/cache footprint together. This measures asset-count/resource scaling, not the isolated cost of texture bytes versus mesh/material count. The sun remains the one available showcase light; no CLI control exists for a lighting-count sweep.

- Validated captures: 30; every capture has 541 contiguous frame samples.
- Raw captures: `target\profiles\render-scalability-assets\20261024-175552\captures`
- Run-level results: `target\profiles\render-scalability-assets\20261024-175552\asset-run-level-results.csv`
