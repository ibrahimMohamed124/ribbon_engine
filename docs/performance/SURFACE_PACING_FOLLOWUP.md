# Surface pacing follow-up

This follow-up documents two target-side captures from the Ribbon Engine development workflow:

- FIFO vs Mailbox repeat comparison, dated 2026-10-24: see [`PERFORMANCE_BASELINES.md`](PERFORMANCE_BASELINES.md) for medians and interpretation.
- Adapter logging: `[GPU ADAPTER]` is emitted by `ribbon_render::GpuContext` and saved in each run log produced by `tools/perf/run_present_mode_comparison.ps1`.

The Mailbox default is a provisional choice for the measured machine, not a universal recommendation. The mode remains overridable using `RIBBON_GPU_PRESENT_MODE`; unsupported Mailbox requests fall back to FIFO. Do not infer which GPU was active from Windows' device list: use the adapter record emitted by `wgpu`. Re-run the comparison after GPU/driver/display changes or when changing the renderer's queue submission policy.

No claim is made that the selected mode alone explains the measured graph CPU or GPU timing changes. Render graph durations measure elapsed CPU-side execution/encoding and can include time spent in calls that experience backend/queue backpressure.
