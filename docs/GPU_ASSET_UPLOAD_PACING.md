# GPU asset upload pacing

Date: 2026-10-23

## Why this exists

The user-provided multi-asset lifecycle CSV (`multi-asset-lifecycle-32x32-active32-prefetch32.csv`, capture generated at `2026-10-23T17:33:18Z`) identifies a single-frame CPU stall during reactivation of the second batch of model assets:

- Frame 602 frame wall: 43.8668 ms.
- Frame 602 render-graph CPU: 43.7298 ms.
- Frame 602 `mesh pass [0]` CPU: 43.5411 ms.
- Surface acquire and present for the same frame: 0.0637 ms and 0.0650 ms.
- Model GPU cache entries rose from 0 to 32 in this frame.
- The cumulative model GPU upload counter increased from 32 to 64 in this frame; live meshes and materials rose by 32 each.

This strongly correlates the hitch with synchronous first-use model resource creation in the render pass. GPU timestamps are asynchronous, so their per-row values are not treated as a perfectly aligned measurement of work initiated by that exact row; the overall capture's GPU pass timings remain much smaller than the CPU mesh-pass peak.

## Policy

`plugin_render3d` now limits synchronous creation of missing model GPU-cache entries to two distinct model asset keys per render frame (`MAX_MODEL_GPU_UPLOADS_PER_FRAME`). A key is the model handle ID plus generation. Existing cache hits continue rendering normally; placements that require an entry beyond the per-frame allowance are skipped for that frame and retried by the normal render loop on a later frame.

This spreads model first-use work across frames while keeping all GPU resource creation on the render-thread-owned renderer. The cumulative upload counter already present in render-profile CSV is sufficient to verify the policy: its per-frame delta should not exceed two during model-cache population, except if this metric's semantics change in a later revision.

## Scope and limitations

- This is a count-based burst limit, not a hard millisecond or byte budget. A single exceptionally large model can still take a long time to upload.
- The scene can be partially rendered while remaining model keys are uploaded over later frames. This is intentional progressive activation, not proof that all assets were uploaded on the first visible frame.
- Voxel chunk uploads still use their existing path; this change is targeted at the measured model-upload burst. Profile a multi-chunk workload before choosing a voxel upload policy.
- CPU decoded-asset cache budgeting remains separate from GPU upload pacing. Leases protect active CPU assets; the CPU budget only applies to unpinned cache entries.

## Required target-side verification

After rebuilding on the Windows checkout, rerun the existing 32-active/32-prefetch multi-asset lifecycle capture and inspect its CSV:

1. Confirm no frame adds more than two model GPU cache entries while the cache is warming.
2. Confirm the 43 ms mesh-pass spike no longer recurs during the batch transition.
3. Confirm all 32 assets eventually appear in the GPU model cache and all placements are visible after warm-up.
4. Confirm lifecycle teardown returns the GPU cache entries to zero and the upload/release cumulative counters remain consistent.

No tests were added or modified, and no Cargo command or runtime was executed while preparing this change.

## Latest user-side confirmation

The user-side focused summary generated after the two-upload limit was added reports a maximum cumulative model-GPU-upload delta of two between adjacent captured samples. The lifecycle capture's frame-wall peak was 15.991 ms, its peak render-graph CPU time was 7.617 ms, and it retained a maximum of 32 GPU model-cache entries. The configured unpinned CPU asset cache remained within its 1 MiB budget. This supports the count-based pacing policy for the tested capture window; it does not prove behavior before the first captured row or for very large single assets.

Surface acquisition still accounted for most of the slowest frame samples in that capture. The user then supplied three repeated captures each for FIFO and Mailbox: Mailbox reduced run-level median surface-acquisition P95 from 13.796 ms to 8.340 ms, frame-wall P95 from 14.666 ms to 8.866 ms, and frame-wall P99 from 16.489 ms to 13.059 ms. The native `engine_render::GpuContext` therefore now prefers Mailbox when supported and falls back to FIFO; FIFO remains explicitly selectable through `RIBBON_GPU_PRESENT_MODE=fifo`. See `docs/SURFACE_PRESENT_MODE_COMPARISON.md` for the full values and the limitation that Mailbox peaks still slightly exceed the 16.667 ms reference budget.
