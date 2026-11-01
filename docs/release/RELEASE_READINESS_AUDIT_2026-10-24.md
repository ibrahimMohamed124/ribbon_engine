# Release-readiness audit — 2026-10-24

## Scope and source snapshot

This review was performed against the uploaded `engine.zip` source snapshot. It is the reference for this audit; the package names and workspace layout differ from earlier conversation context. The archive does not contain `.git`, so this review cannot verify the current local branch, commit, dirty state, or remote synchronization. It also does not include the raw Windows GPU capture directories described by the performance docs.

The purpose was to review asset cache invalidation and generation semantics, native/offscreen render-path handling, profiler sample lifetime, and release-readiness gaps without making speculative rendering changes.

## Changes made

### 1. Fixed offscreen GPU profiler completion-queue handling

`OffscreenVoxelRenderer::render` polls the mesh/shadow GPU timestamp profiler, but the offscreen API exposes the latest snapshot rather than a stream of completed samples. The completed-sample queue was never drained or cleared. The profiler has a four-entry bounded completion queue and increments `dropped_samples` if a new result arrives while the queue is full; in this caller, that could report artificial drops even though individual completion records are not part of the public offscreen API.

The renderer now clears consumed completion records immediately after polling. The same fix is applied to the offscreen tone mapper's Bloom/tone-mapping profiler. This does not wait for the GPU, clear the latest completed snapshot, change timestamp sequence values, or affect rendered output. It only prevents unconsumed completion records from accumulating in a caller that has no completion-stream API.

The native-window path is intentionally unchanged: its render diagnostics drains the completion queues when profiling capture is enabled and clears them when capture is disabled.

### 2. Added focused asset cache lifecycle assertions

In `crates/assets/src/lib.rs`:

- The existing memory-source hot-reload test now asserts that a successful reload increases the resident handle's content revision, which is the invalidation signal used by derived renderer caches.
- A focused test verifies that an unused asset can be evicted, that reloading its path retains its stable asset ID while advancing the generation, that the old handle remains stale, and that the new handle resolves the new content.

These checks specifically cover the asset revision and generation contracts relevant to renderer cache invalidation. They do not replace an end-to-end GPU test of glTF upload/reupload behavior.

## Source inspection findings

- The model-asset reconciliation path deduplicates repeated `(asset id, generation)` handles before checking residency and reading the revision. It only treats currently resident handles as live renderer-cache keys. Hot reload revision changes invalidate cached model GPU resources, while unload/reload uses a new handle generation. This path looked internally consistent by static inspection; a full workspace build and runtime hot-reload exercise remain necessary.
- The native window path records surface errors and render-graph errors, skips presenting a partially rendered frame when the graph fails, and rebuilds surface configuration on relevant resize/recovery paths. The minimized/zero-size path and no-camera/empty-scene behavior are explicitly handled. These paths were inspected but not run on a native display in this environment.
- The offscreen HDR target is returned to the transient pool only after the tone mapper/bind group is dropped on resize/recreation. The normal window frame loop does not perform a blocking device wait; synchronous readback and device teardown are separate operations.
- The post-processing submission merge is already present in this snapshot and was left unchanged. The prior A/B experiment did not establish a material, repeatable end-to-end frame-time improvement.

## Validation attempted in this environment

| Check | Result | Notes |
|---|---|---|
| Python syntax compilation | Passed | `python -m py_compile tools/*.py games/studs/tools/*.py` |
| Rust formatting | Not run | `rustfmt` is not installed in this environment. |
| Cargo checks/build/tests/Clippy | Not run | `cargo` and `rustc` are not installed in this environment. |
| Dependency policy checker | Blocked | `python tools/check_deps.py` shells out to Cargo, which is unavailable. |
| Processed asset manifest check | Blocked for this snapshot | `python games/studs/tools/fetch_assets.py --check` requires tracked-file metadata, but the supplied archive has no `.git` directory. Run it from the original Git checkout. |
| Native GPU / visual comparison | Not run | This environment cannot build or launch the Rust/WGPU project, and no baseline/current image pair was included. No pixel-equivalence claim is made. |

## Required verification in the original Windows checkout

Run from `D:\exp\ribbon_engine\engine` after applying the overlay:

```powershell
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace --examples
cargo test --locked --workspace
cargo build --locked --workspace --all-targets --no-default-features
cargo test --locked --workspace --no-default-features
python tools/check_deps.py
python games/studs/tools/fetch_assets.py --check
```

Then validate runtime behavior on the target machine: exercise the offscreen renderer long enough to complete more than four timestamp readbacks and confirm the dropped counter does not rise merely because no completion-stream API is being consumed; test model hot reload and unload/reload with repeated scene placements; and capture comparable baseline/current images for a scene with HDR/Bloom/tone mapping and a representative offscreen render. The asset-manifest command should be run only from a real Git checkout where its tracked-file checks work.

## Remaining release blockers / limitations

1. The Rust changes in this overlay have **not** been compiled or tested here because the Rust toolchain is absent. The listed Windows validation commands are required before merging.
2. No actual baseline-vs-current image comparison was performed; rendering correctness is not established by timing captures or by this static review.
3. GPU timestamps remain asynchronous and frame-associated; the offscreen public API exposes the latest snapshot only, not the full sequence of completed samples.
4. Performance docs note that some surface acquisition failure/timeout cases are not represented as separate failed-frame CSV rows, and resource byte counts are engine estimates rather than measured VRAM/RSS.
5. Therefore this audit does **not** declare Ribbon Engine v1.0.0 release-ready. It fixes one confirmed profiler-accounting bug and adds targeted asset lifecycle assertions, while preserving the remaining verification work explicitly.
