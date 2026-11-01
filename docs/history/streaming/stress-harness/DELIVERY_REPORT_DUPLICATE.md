# Ribbon Engine Streaming Stress Harness — Delivery Report

## Delivered example

`examples/streaming_stress` is an engine-level diagnostic example. It does not add game-specific gameplay or create a new game. The PowerShell runner orchestrates a release build, deterministic asset generation, headless stress tests, and a native-window GPU stress phase.

## Covered scenarios

1. **Region churn:** drives a synthetic observer across 2–256 collider regions for 1–10 forward/reverse cycles, applies a configurable active-region cap, and verifies final unload after a 48-frame minimum settle period.
2. **Memory monitoring:** records AssetServer decoded CPU residency and pinned/unpinned estimates; the runner separately samples process working set and private bytes every 250 ms. The GPU phase records renderer GPU allocation estimates. GPU estimates are not physical VRAM measurements.
3. **GPU upload budgets:** loads a generated corpus of distinct textured GLB models, queues standalone texture and sampler warmups, records upload count/bytes per actual rendered frame, and checks the configured count/byte limits while preserving the renderer's documented oversized-first-upload exception.
4. **Cancellation:** directly cancels an active AssetServer worker read and separately unloads a real SceneDoc-backed region while its worker is blocked in cancellable source IO. Both cases verify worker drain and ownership release.
5. **Shared resources:** exercises duplicate CPU AssetRef requests and lease pinning, shared logical region references held by two Scenes, common model references across GPU regions, and duplicate generic GPU warmup keys.
6. **Frame time and latency:** writes per-frame CSV and Markdown percentile summaries for App update wall time, render CPU frame time, StreamingCoordinator CPU time, region activation latency, per-poll AssetServer source-read/decode time, and asynchronous GPU mesh-pass timestamps when supported.

## Output files

The run is written to `target/streaming-stress-runs/<timestamp>/`. It contains the orchestrator summary, build/corpus logs, phase stdout/stderr, process-memory samples, headless/GPU frame CSV files, and the RenderProfile CSV.

## Validation performed while preparing this overlay

- Parsed the workspace, example manifest, and dependency-policy TOML using Python's TOML parser.
- Verified that all example local path dependencies resolve inside the supplied repository.
- Verified the example package is present in Cargo.lock and the dependency-policy allowlist.
- Checked the 25-column frame CSV header and row-format placeholder count.
- Ran the standard-library GLB corpus generator for eight models and parsed the GLB headers, JSON chunks, BIN chunks, buffer-view ranges, and embedded PNG image references.
- Verified the final ZIP structure and SHA-256 manifest.

**Not performed here:** `cargo fmt`, `cargo check`, `cargo test`, `cargo clippy`, PowerShell execution, native window creation, and hardware GPU validation. The current artifact-preparation environment does not provide Cargo/Rust or PowerShell. These commands are included for execution on the Windows development machine; no Rust compile or runtime pass is claimed.

## Important measurement limits

- Process memory is an OS process-level sample, not a per-asset allocation ledger.
- CPU residency is the AssetServer's decoded-asset estimate and lease accounting.
- GPU memory is the renderer's managed allocation estimate, not driver-reported physical VRAM.
- Timestamp-query metrics are asynchronous and may refer to a previously completed frame.
- Headless mode intentionally leaves GPU metrics blank; actual GPU checks run only in the native GPU phase.

## Apply

Extract the ZIP and run `apply_streaming_stress_examples.ps1` from its root, passing the Ribbon Engine workspace root. The script backs up replaced files and verifies each copied file with SHA-256.
