# Ribbon Engine — Remaining Streaming Gaps: Continuation Report

## Baseline

This overlay targets the checkout that passed the user's Windows `tools/validate_streaming.ps1` run with 15/15 checks after the Render3D duplicate-import and validation hotfixes. The 15/15 result predates this overlay. No Rust build/test result is claimed for this new delta.

## 1. Shader and render-pipeline Asset Streaming Registry

Added `plugins/render3d/src/shader_pipeline.rs` and integrated it into `Render3dPlugin` / the render-owning pass.

- Stable `(namespace, asset ID, generation)` keys for shader modules and render pipelines.
- WGSL `.wgsl` CPU loader registered through `AssetServer`.
- `request_shader_asset` pins the originating `AssetRef<ShaderModuleAsset>` and derives the GPU key from the asset identity and generation.
- Strong shader and pipeline leases; a pipeline retains its shader dependency lease.
- Duplicate generation requests share a lease family. Conflicting source content or a changed pipeline recipe fingerprint under the same key is rejected; callers increment generation for changed content/recipe.
- Explicit Queued / Preparing / Ready / Failed states, bounded render-thread creation, async validation scopes (Validation and OutOfMemory), and ordered orphan release.
- WGSL bytes and pipeline builder recipes are retained. `reset_after_device_loss` drops device-owned shader/pipeline objects and requeues only still-leased generations for replay on the replacement device.
- Registry counters are exposed through RenderDiagnostics and the 170-column Render Profile CSV.

This is a real registry with generation/lease/readiness semantics, not an automatic introspector of arbitrary renderer pipeline state. Applications submit a stable `recipe_fingerprint` and a builder callback that recreates a pipeline from the new device and shader module.

## 2. Skeletal Animation / GPU Skinning

**Not implemented in this overlay.** The existing `.ranim` system remains Sprite Animation only. No skeletal glTF import, skin/joint hierarchy, inverse-bind matrices, per-vertex joint indices/weights, animation sampler/channel evaluation, bone-palette upload, or Main/Shadow GPU skinning path has been added. A placeholder component would not satisfy this requirement and was deliberately not added.

Completing this requires coordinated glTF importer, asset schema, vertex layout, animation evaluation, per-instance palette buffers, main and shadow shader changes, upload budgeting, generation/lease ownership, and device recovery work.

## 3. Global tracked-GPU admission budget

The global budget is now a hard **admission gate for tracked estimated GPU allocations** submitted by glTF model warmups, generic GPU resource warmups, and dynamic mesh uploads. It rejects new admissions when current tracked estimates plus the next estimated payload exceed the configured limit; it does not invalidate live resources. Renderer baseline allocations are initialized and included before model/generic admission decisions.

This is deliberately **not** described as a hard physical-VRAM cap. Portable `wgpu` does not expose exact driver/OS VRAM accounting. Opaque shader/pipeline allocations, swapchain/driver overhead, and other unknown allocations cannot be included as exact bytes. Existing model-cache-specific pressure remains a separate soft diagnostic budget. The global gate is exact only with respect to its tracked admission estimates and known upload paths.

## 4. Device recreation validation

An ignored test `real_adapter_recreation_rebuilds_retained_shader_and_pipeline` and
`tools/test_shader_pipeline_device_recovery.ps1` are added. The test requires a non-CPU wgpu adapter, creates and validates a shader/pipeline, resets the registry, destroys the first device, creates a second context on the same adapter identity, and verifies that leased recipes return to Ready.

This test has **not** been run here. It is also not a substitute for an interactive native-window `SURFACE_LOST` / `DeviceLost` stress test on the target GPU. Run it on the Windows GPU host using the script.

## 5. Multi-region / application-owned scene transition transaction

Added `SceneTransitionParticipant` and `register_scene_transition_participant`.

- Every connected region in the visibility group must satisfy visual and physics readiness first.
- Application participants prepare hidden/staged World state; `Ok(false)` keeps all regions staged.
- All participant commits execute synchronously in the coordinator Update before PostUpdate render extraction.
- A failed prepare/commit invokes reverse-order compensating rollback callbacks.
- Only after participants succeed are render staging markers removed for the connected region group and regions moved to Active.

This is a cooperative ECS/World transaction boundary. Participants must make prepare idempotent and implement rollback for their own mutations. It is **not** an automatic transaction over arbitrary ECS state or a wgpu RenderGraph transaction, and cannot roll back external side effects such as files, network requests, or driver work.

## Static checks performed for this package

- Parsed 44 TOML files successfully.
- Checked the Render Profile CSV header/row layout: 170 headers, 131 explicitly assigned resource indexes, no out-of-range or duplicate indexes in the explicit assignments.
- Verified source wiring for the registry, AssetServer loader, multi-region participant hooks, rollback test, and ignored real-adapter test.

These static checks do not replace `cargo fmt`, `cargo check`, Rust unit tests, Clippy, or hardware testing. This environment did not provide `cargo`, `rustc`, `rustfmt`, or PowerShell.
