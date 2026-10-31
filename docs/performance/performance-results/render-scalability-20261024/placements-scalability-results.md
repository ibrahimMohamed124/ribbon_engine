# Rendering scalability capture results

All values below are calculated from the validated raw frame CSVs. Five valid runs are included for every workload/mode. Percentiles use linear interpolation at `(n - 1) * p`. GPU timestamp samples are asynchronous, individually sequenced, and summarized once per unique sequence; per-stage sample counts are shown in the run-level CSV.

## Run medians and variability

Each cell shows the median of five run-level statistic values, with `[min, max]` across runs. Duration values are milliseconds; draw/instance metrics are counts; allocation estimates are bytes.

| Workload | Mode | Metric | Run samples | Median mean | Median P50 | Median P95 [min,max] | Median P99 |
|---|---|---|---:|---:|---:|---:|---:|
| small (64 placements) | fifo | Frame wall | 541 | 6.655 | 6.054 | 14.038 [13.921, 14.109] | 14.512 |
| small (64 placements) | fifo | Surface acquisition wall | 541 | 5.870 | 5.256 | 13.000 [12.842, 13.184] | 13.363 |
| small (64 placements) | fifo | Render graph CPU | 541 | 0.698 | 0.670 | 1.201 [1.144, 1.467] | 1.527 |
| small (64 placements) | fifo | Surface presentation wall | 541 | 0.076 | 0.070 | 0.129 [0.118, 0.153] | 0.195 |
| small (64 placements) | fifo | Frame remainder (setup/outside measured phases) | 541 | 0.010 | 0.009 | 0.020 [0.017, 0.029] | 0.045 |
| small (64 placements) | fifo | GPU shadow + mesh | 362, 363, 363, 362, 361 | 0.106 | 0.083 | 0.222 [0.217, 0.229] | 0.225 |
| small (64 placements) | fifo | GPU shadow passes | 362, 363, 363, 362, 361 | 0.008 | 0.007 | 0.013 [0.013, 0.014] | 0.014 |
| small (64 placements) | fifo | GPU mesh pass | 362, 363, 363, 362, 361 | 0.044 | 0.047 | 0.073 [0.069, 0.078] | 0.075 |
| small (64 placements) | fifo | GPU Bloom pass | 390, 391, 389, 391, 365 | 0.032 | 0.044 | 0.049 [0.047, 0.053] | 0.050 |
| small (64 placements) | fifo | GPU tone-mapping pass | 390, 391, 389, 391, 365 | 0.076 | 0.113 | 0.150 [0.125, 0.155] | 0.166 |
| small (64 placements) | fifo | Visible mesh instances | 541 | 64.000 | 64.000 | 64.000 [64.000, 64.000] | 64.000 |
| small (64 placements) | fifo | Visible draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| small (64 placements) | fifo | Shadow draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| small (64 placements) | fifo | Tracked GPU allocation estimate | 541 | 25298384.000 | 25298384.000 | 25298384.000 [25298384.000, 25298384.000] | 25298384.000 |
| small (64 placements) | fifo | Material texture allocation estimate | 541 | 352.000 | 352.000 | 352.000 [352.000, 352.000] | 352.000 |
| small (64 placements) | fifo | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| small (64 placements) | fifo | Cached transient texture estimate | 541 | 0.000 | 0.000 | 0.000 [0.000, 0.000] | 0.000 |
| small (64 placements) | fifo | CPU resident asset estimate | 541 | 2640.000 | 2640.000 | 2640.000 [2640.000, 2640.000] | 2640.000 |
| small (64 placements) | fifo | Cumulative model uploads | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| small (64 placements) | fifo | GPU model cache entries | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| small (64 placements) | fifo | Renderer instance-buffer capacity | 541 | 64.000 | 64.000 | 64.000 [64.000, 64.000] | 64.000 |
| small (64 placements) | mailbox | Frame wall | 541 | 0.703 | 0.709 | 1.138 [1.060, 2.952] | 1.521 |
| small (64 placements) | mailbox | Surface acquisition wall | 541 | 0.441 | 0.479 | 0.845 [0.698, 2.192] | 1.292 |
| small (64 placements) | mailbox | Render graph CPU | 541 | 0.214 | 0.194 | 0.371 [0.352, 1.080] | 0.514 |
| small (64 placements) | mailbox | Surface presentation wall | 541 | 0.038 | 0.037 | 0.065 [0.060, 0.124] | 0.090 |
| small (64 placements) | mailbox | Frame remainder (setup/outside measured phases) | 541 | 0.002 | 0.002 | 0.004 [0.003, 0.013] | 0.006 |
| small (64 placements) | mailbox | GPU shadow + mesh | 349, 421, 427, 361, 424 | 0.050 | 0.045 | 0.066 [0.065, 0.103] | 0.070 |
| small (64 placements) | mailbox | GPU shadow passes | 349, 421, 427, 361, 424 | 0.004 | 0.004 | 0.005 [0.005, 0.006] | 0.006 |
| small (64 placements) | mailbox | GPU mesh pass | 349, 421, 427, 361, 424 | 0.020 | 0.019 | 0.023 [0.022, 0.023] | 0.023 |
| small (64 placements) | mailbox | GPU Bloom pass | 465, 477, 474, 470, 450 | 0.015 | 0.015 | 0.016 [0.016, 0.017] | 0.017 |
| small (64 placements) | mailbox | GPU tone-mapping pass | 465, 477, 474, 470, 450 | 0.023 | 0.023 | 0.026 [0.026, 0.026] | 0.026 |
| small (64 placements) | mailbox | Visible mesh instances | 541 | 64.000 | 64.000 | 64.000 [64.000, 64.000] | 64.000 |
| small (64 placements) | mailbox | Visible draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| small (64 placements) | mailbox | Shadow draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| small (64 placements) | mailbox | Tracked GPU allocation estimate | 541 | 25298384.000 | 25298384.000 | 25298384.000 [25298384.000, 25298384.000] | 25298384.000 |
| small (64 placements) | mailbox | Material texture allocation estimate | 541 | 352.000 | 352.000 | 352.000 [352.000, 352.000] | 352.000 |
| small (64 placements) | mailbox | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| small (64 placements) | mailbox | Cached transient texture estimate | 541 | 0.000 | 0.000 | 0.000 [0.000, 0.000] | 0.000 |
| small (64 placements) | mailbox | CPU resident asset estimate | 541 | 2640.000 | 2640.000 | 2640.000 [2640.000, 2640.000] | 2640.000 |
| small (64 placements) | mailbox | Cumulative model uploads | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| small (64 placements) | mailbox | GPU model cache entries | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| small (64 placements) | mailbox | Renderer instance-buffer capacity | 541 | 64.000 | 64.000 | 64.000 [64.000, 64.000] | 64.000 |
| medium (576 placements) | fifo | Frame wall | 541 | 6.527 | 5.522 | 14.120 [13.890, 14.162] | 14.613 |
| medium (576 placements) | fifo | Surface acquisition wall | 541 | 5.508 | 4.596 | 12.839 [12.648, 12.982] | 13.221 |
| medium (576 placements) | fifo | Render graph CPU | 541 | 0.909 | 0.852 | 1.659 [1.520, 1.815] | 2.150 |
| medium (576 placements) | fifo | Surface presentation wall | 541 | 0.074 | 0.070 | 0.137 [0.128, 0.151] | 0.200 |
| medium (576 placements) | fifo | Frame remainder (setup/outside measured phases) | 541 | 0.010 | 0.008 | 0.018 [0.016, 0.021] | 0.039 |
| medium (576 placements) | fifo | GPU shadow + mesh | 367, 365, 363, 377, 368 | 0.112 | 0.080 | 0.259 [0.244, 0.264] | 0.267 |
| medium (576 placements) | fifo | GPU shadow passes | 367, 365, 363, 377, 368 | 0.014 | 0.014 | 0.024 [0.019, 0.026] | 0.025 |
| medium (576 placements) | fifo | GPU mesh pass | 367, 365, 363, 377, 368 | 0.047 | 0.037 | 0.099 [0.092, 0.102] | 0.104 |
| medium (576 placements) | fifo | GPU Bloom pass | 394, 399, 387, 397, 378 | 0.030 | 0.030 | 0.048 [0.038, 0.052] | 0.051 |
| medium (576 placements) | fifo | GPU tone-mapping pass | 394, 399, 387, 397, 378 | 0.072 | 0.044 | 0.143 [0.133, 0.157] | 0.161 |
| medium (576 placements) | fifo | Visible mesh instances | 541 | 576.000 | 576.000 | 576.000 [576.000, 576.000] | 576.000 |
| medium (576 placements) | fifo | Visible draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| medium (576 placements) | fifo | Shadow draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| medium (576 placements) | fifo | Tracked GPU allocation estimate | 541 | 25451984.000 | 25451984.000 | 25451984.000 [25451984.000, 25451984.000] | 25451984.000 |
| medium (576 placements) | fifo | Material texture allocation estimate | 541 | 352.000 | 352.000 | 352.000 [352.000, 352.000] | 352.000 |
| medium (576 placements) | fifo | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| medium (576 placements) | fifo | Cached transient texture estimate | 541 | 0.000 | 0.000 | 0.000 [0.000, 0.000] | 0.000 |
| medium (576 placements) | fifo | CPU resident asset estimate | 541 | 2640.000 | 2640.000 | 2640.000 [2640.000, 2640.000] | 2640.000 |
| medium (576 placements) | fifo | Cumulative model uploads | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| medium (576 placements) | fifo | GPU model cache entries | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| medium (576 placements) | fifo | Renderer instance-buffer capacity | 541 | 1024.000 | 1024.000 | 1024.000 [1024.000, 1024.000] | 1024.000 |
| medium (576 placements) | mailbox | Frame wall | 541 | 0.694 | 0.634 | 1.339 [0.966, 2.163] | 1.861 |
| medium (576 placements) | mailbox | Surface acquisition wall | 541 | 0.315 | 0.253 | 0.927 [0.568, 1.714] | 1.476 |
| medium (576 placements) | mailbox | Render graph CPU | 541 | 0.315 | 0.276 | 0.560 [0.439, 0.785] | 0.775 |
| medium (576 placements) | mailbox | Surface presentation wall | 541 | 0.043 | 0.039 | 0.076 [0.061, 0.105] | 0.108 |
| medium (576 placements) | mailbox | Frame remainder (setup/outside measured phases) | 541 | 0.002 | 0.002 | 0.005 [0.003, 0.010] | 0.008 |
| medium (576 placements) | mailbox | GPU shadow + mesh | 344, 320, 455, 382, 339 | 0.042 | 0.038 | 0.066 [0.065, 0.069] | 0.070 |
| medium (576 placements) | mailbox | GPU shadow passes | 344, 320, 455, 382, 339 | 0.008 | 0.007 | 0.008 [0.008, 0.009] | 0.009 |
| medium (576 placements) | mailbox | GPU mesh pass | 344, 320, 455, 382, 339 | 0.019 | 0.018 | 0.022 [0.020, 0.022] | 0.022 |
| medium (576 placements) | mailbox | GPU Bloom pass | 455, 479, 481, 467, 471 | 0.016 | 0.015 | 0.017 [0.017, 0.017] | 0.018 |
| medium (576 placements) | mailbox | GPU tone-mapping pass | 455, 479, 481, 467, 471 | 0.024 | 0.024 | 0.026 [0.026, 0.026] | 0.027 |
| medium (576 placements) | mailbox | Visible mesh instances | 541 | 576.000 | 576.000 | 576.000 [576.000, 576.000] | 576.000 |
| medium (576 placements) | mailbox | Visible draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| medium (576 placements) | mailbox | Shadow draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| medium (576 placements) | mailbox | Tracked GPU allocation estimate | 541 | 25451984.000 | 25451984.000 | 25451984.000 [25451984.000, 25451984.000] | 25451984.000 |
| medium (576 placements) | mailbox | Material texture allocation estimate | 541 | 352.000 | 352.000 | 352.000 [352.000, 352.000] | 352.000 |
| medium (576 placements) | mailbox | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| medium (576 placements) | mailbox | Cached transient texture estimate | 541 | 0.000 | 0.000 | 0.000 [0.000, 0.000] | 0.000 |
| medium (576 placements) | mailbox | CPU resident asset estimate | 541 | 2640.000 | 2640.000 | 2640.000 [2640.000, 2640.000] | 2640.000 |
| medium (576 placements) | mailbox | Cumulative model uploads | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| medium (576 placements) | mailbox | GPU model cache entries | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| medium (576 placements) | mailbox | Renderer instance-buffer capacity | 541 | 1024.000 | 1024.000 | 1024.000 [1024.000, 1024.000] | 1024.000 |
| large (1600 placements) | fifo | Frame wall | 541 | 6.212 | 4.486 | 14.081 [13.816, 14.146] | 14.589 |
| large (1600 placements) | fifo | Surface acquisition wall | 541 | 4.857 | 3.146 | 12.447 [12.342, 12.612] | 12.796 |
| large (1600 placements) | fifo | Render graph CPU | 541 | 1.271 | 1.193 | 2.319 [1.766, 2.521] | 2.666 |
| large (1600 placements) | fifo | Surface presentation wall | 541 | 0.075 | 0.069 | 0.141 [0.110, 0.146] | 0.213 |
| large (1600 placements) | fifo | Frame remainder (setup/outside measured phases) | 541 | 0.011 | 0.008 | 0.019 [0.013, 0.028] | 0.037 |
| large (1600 placements) | fifo | GPU shadow + mesh | 370, 370, 370, 396, 374 | 0.158 | 0.119 | 0.338 [0.334, 0.345] | 0.341 |
| large (1600 placements) | fifo | GPU shadow passes | 370, 370, 370, 396, 374 | 0.032 | 0.018 | 0.051 [0.050, 0.055] | 0.052 |
| large (1600 placements) | fifo | GPU mesh pass | 370, 370, 370, 396, 374 | 0.074 | 0.033 | 0.152 [0.148, 0.155] | 0.155 |
| large (1600 placements) | fifo | GPU Bloom pass | 403, 400, 409, 419, 403 | 0.034 | 0.018 | 0.062 [0.056, 0.079] | 0.080 |
| large (1600 placements) | fifo | GPU tone-mapping pass | 403, 400, 409, 419, 403 | 0.074 | 0.027 | 0.135 [0.134, 0.136] | 0.138 |
| large (1600 placements) | fifo | Visible mesh instances | 541 | 1600.000 | 1600.000 | 1600.000 [1600.000, 1600.000] | 1600.000 |
| large (1600 placements) | fifo | Visible draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| large (1600 placements) | fifo | Shadow draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| large (1600 placements) | fifo | Tracked GPU allocation estimate | 541 | 25615824.000 | 25615824.000 | 25615824.000 [25615824.000, 25615824.000] | 25615824.000 |
| large (1600 placements) | fifo | Material texture allocation estimate | 541 | 352.000 | 352.000 | 352.000 [352.000, 352.000] | 352.000 |
| large (1600 placements) | fifo | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| large (1600 placements) | fifo | Cached transient texture estimate | 541 | 0.000 | 0.000 | 0.000 [0.000, 0.000] | 0.000 |
| large (1600 placements) | fifo | CPU resident asset estimate | 541 | 2640.000 | 2640.000 | 2640.000 [2640.000, 2640.000] | 2640.000 |
| large (1600 placements) | fifo | Cumulative model uploads | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| large (1600 placements) | fifo | GPU model cache entries | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| large (1600 placements) | fifo | Renderer instance-buffer capacity | 541 | 2048.000 | 2048.000 | 2048.000 [2048.000, 2048.000] | 2048.000 |
| large (1600 placements) | mailbox | Frame wall | 541 | 0.620 | 0.561 | 1.024 [0.818, 1.376] | 1.430 |
| large (1600 placements) | mailbox | Surface acquisition wall | 541 | 0.104 | 0.049 | 0.270 [0.241, 0.914] | 0.672 |
| large (1600 placements) | mailbox | Render graph CPU | 541 | 0.448 | 0.400 | 0.682 [0.540, 0.862] | 0.923 |
| large (1600 placements) | mailbox | Surface presentation wall | 541 | 0.037 | 0.033 | 0.061 [0.056, 0.083] | 0.092 |
| large (1600 placements) | mailbox | Frame remainder (setup/outside measured phases) | 541 | 0.003 | 0.002 | 0.006 [0.004, 0.009] | 0.011 |
| large (1600 placements) | mailbox | GPU shadow + mesh | 448, 485, 491, 490, 491 | 0.060 | 0.054 | 0.084 [0.083, 0.088] | 0.090 |
| large (1600 placements) | mailbox | GPU shadow passes | 448, 485, 491, 490, 491 | 0.017 | 0.016 | 0.018 [0.016, 0.018] | 0.018 |
| large (1600 placements) | mailbox | GPU mesh pass | 448, 485, 491, 490, 491 | 0.030 | 0.029 | 0.033 [0.030, 0.033] | 0.033 |
| large (1600 placements) | mailbox | GPU Bloom pass | 465, 480, 489, 490, 498 | 0.016 | 0.015 | 0.017 [0.016, 0.018] | 0.018 |
| large (1600 placements) | mailbox | GPU tone-mapping pass | 465, 480, 489, 490, 498 | 0.024 | 0.024 | 0.026 [0.025, 0.027] | 0.027 |
| large (1600 placements) | mailbox | Visible mesh instances | 541 | 1600.000 | 1600.000 | 1600.000 [1600.000, 1600.000] | 1600.000 |
| large (1600 placements) | mailbox | Visible draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| large (1600 placements) | mailbox | Shadow draw calls | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| large (1600 placements) | mailbox | Tracked GPU allocation estimate | 541 | 25615824.000 | 25615824.000 | 25615824.000 [25615824.000, 25615824.000] | 25615824.000 |
| large (1600 placements) | mailbox | Material texture allocation estimate | 541 | 352.000 | 352.000 | 352.000 [352.000, 352.000] | 352.000 |
| large (1600 placements) | mailbox | Active transient texture estimate | 541 | 8192000.000 | 8192000.000 | 8192000.000 [8192000.000, 8192000.000] | 8192000.000 |
| large (1600 placements) | mailbox | Cached transient texture estimate | 541 | 0.000 | 0.000 | 0.000 [0.000, 0.000] | 0.000 |
| large (1600 placements) | mailbox | CPU resident asset estimate | 541 | 2640.000 | 2640.000 | 2640.000 [2640.000, 2640.000] | 2640.000 |
| large (1600 placements) | mailbox | Cumulative model uploads | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| large (1600 placements) | mailbox | GPU model cache entries | 541 | 1.000 | 1.000 | 1.000 [1.000, 1.000] | 1.000 |
| large (1600 placements) | mailbox | Renderer instance-buffer capacity | 541 | 2048.000 | 2048.000 | 2048.000 [2048.000, 2048.000] | 2048.000 |

## P95 change with scene size

Positive means the metric increased in larger workload compared with small; negative means it decreased.

| Mode | Metric | Small | Medium vs small | Large vs small |
|---|---|---:|---:|---:|
| fifo | Frame wall | 14.038 ms | +0.6% | +0.3% |
| fifo | Surface acquisition wall | 13.000 ms | -1.2% | -4.3% |
| fifo | Render graph CPU | 1.201 ms | +38.2% | +93.1% |
| fifo | Surface presentation wall | 0.129 ms | +6.2% | +8.8% |
| fifo | Frame remainder (setup/outside measured phases) | 0.020 ms | -12.9% | -6.9% |
| fifo | GPU shadow + mesh | 0.222 ms | +16.6% | +52.3% |
| fifo | GPU shadow passes | 0.013 ms | +76.9% | +284.6% |
| fifo | GPU mesh pass | 0.073 ms | +36.6% | +108.5% |
| fifo | GPU Bloom pass | 0.049 ms | -2.1% | +26.5% |
| fifo | GPU tone-mapping pass | 0.150 ms | -4.1% | -9.6% |
| mailbox | Frame wall | 1.138 ms | +17.7% | -9.9% |
| mailbox | Surface acquisition wall | 0.845 ms | +9.7% | -68.1% |
| mailbox | Render graph CPU | 0.371 ms | +51.2% | +84.1% |
| mailbox | Surface presentation wall | 0.065 ms | +16.8% | -6.5% |
| mailbox | Frame remainder (setup/outside measured phases) | 0.004 ms | +31.4% | +71.4% |
| mailbox | GPU shadow + mesh | 0.066 ms | +0.0% | +28.1% |
| mailbox | GPU shadow passes | 0.005 ms | +60.0% | +260.0% |
| mailbox | GPU mesh pass | 0.023 ms | -4.5% | +45.5% |
| mailbox | GPU Bloom pass | 0.016 ms | +6.2% | +6.2% |
| mailbox | GPU tone-mapping pass | 0.026 ms | +0.0% | +0.0% |

## Workload interpretation

The benchmark holds the model asset, material, sun light, camera transform, output size, and runtime settings fixed while increasing placement count. It therefore isolates placement/instance-count scaling more cleanly than the earlier lifecycle stress workload. These results do not isolate multi-asset upload, texture-memory, or changing-light-count scaling. `Visible mesh instances` and grouped draw calls are renderer-observed counts; the showcase repeats one model/material, so draw-call count may remain nearly constant while per-instance work rises.

GPU timestamp query availability and skipped counts are checked per capture. Query samples are read asynchronously and may be fewer than frame rows due to frame association/capture boundaries; sample counts by run appear in the CSV. Surface acquisition is pacing/compositor wall time and must not be interpreted as GPU render cost. Tracked allocation bytes are estimates rather than driver-reported VRAM.

- Validated captures: 30; each has 541 contiguous frame samples.
- Capture CSVs and logs: `profiles\render-scalability\20261024-133006\captures`
- Run-level statistics CSV: `profiles\render-scalability\20261024-133006\run-level-results.csv`
