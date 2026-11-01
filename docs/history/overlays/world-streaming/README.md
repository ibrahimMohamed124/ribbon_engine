# Ribbon Engine — Production Streaming continuation overlay

This is a cumulative source overlay relative to the uploaded `Ribbon_Engine(3).zip` baseline. It bundles the prior streaming continuation/fix chain plus the latest source changes in one archive. It is intended to be applied to that baseline checkout, not extracted over an unrelated Ribbon revision.

## Apply safely

1. Extract the ZIP to a staging directory outside the Ribbon checkout.
2. Open PowerShell in that extracted directory.
3. Run:

```powershell
powershell -ExecutionPolicy Bypass -File .\apply_world_streaming_overlay.ps1 -ProjectRoot "D:\path\to\Ribbon_Engine"
```

The script copies only shipped files, skips byte-identical content, validates destination paths, and creates timestamped backups before replacing existing files.

## Included implementation

- AssetServer-backed asynchronous loading, priority upgrades/ageing, request deduplication, leases, cancellation, dependency readiness, hot reload, and pin-aware CPU cache eviction.
- Scene/Region lifecycle coordinator with asset/GPU/physics readiness, Region-owned entity/collider cleanup, revision-aware SceneDoc/prefab refresh, and error/cancellation handling.
- Authored `.worldpart` AABB partitions with bounds/content validation, explicit dependencies/neighbors, multiple observers, distance policies, hysteresis, and neighbor prefetch.
- Renderer-owned Mesh/Texture/Sampler/PBR Material warmup with generation-aware keys, validation, bounded scheduling, byte/class budgets, leases, safe orphan eviction, and render-thread release. glTF model-cache residency also has a configurable soft byte budget (`ModelGpuResidencyBudget`) with warning/high/critical pressure telemetry; it never invalidates active placements or required warmups.
- GPU device-loss callback signaling, render-thread teardown, bounded device-recreation retries, renderer pass resource reconstruction, and requeue of leased generic GPU payloads from retained CPU data.
- Region render staging and connected multi-Region Scene visibility commit before PostUpdate extraction; explicit preview policy remains supported.
- Decoded CPU memory categories for glTF geometry/textures/materials/scene metadata, `.ranim` sprite animation clips, `.scenedoc` and `.worldpart` metadata, and a clearly labeled `Other` fallback for aggregate-only loaders.
- `.ranim` RON sprite AnimationClip loading, validation, hot reload through AssetServer, and non-blocking clip playback state while retaining the existing `SpriteAnimator` API.
- Region phase timing, event timelines, retained slowest activation breakdown, CPU/GPU estimates, memory-pressure and recovery diagnostics, 152-column Render Profile CSV, and editor Streaming/Partition panels.
- ATHAR room-to-village consumption of the generic scene/region APIs plus expanded stress/pressure harness and engine-level tests.

## Important boundaries

The generic GPU residency registry manages standalone Mesh/Texture/Sampler/PBR Material payloads and is **not** a global physical VRAM limiter. The glTF model cache now reports a separate soft residency budget and over-budget pressure, but it does not reject active-scene uploads or forcibly evict live placements. Its estimated bytes are a subset of the renderer mesh/material estimates, not an additional physical allocation. Surfaces, driver allocations, and all transient GPU allocations are still not covered by one hard cap; byte values are estimates. Renderer-owned embedded shader modules and pipelines are dropped and reconstructed through their owning render passes after device recovery, but there is no separate generation-keyed Shader/Pipeline asset streaming registry. `.ranim` is CPU-side sprite animation data; this continuation does not add skeletal glTF animation or a GPU skinning pipeline. Partition authoring is RON-based; there is no graphical painting/procedural partition-authoring tool.

The Scene visibility barrier commits the connected Scene/Region group during one ECS Update before render extraction. It is not a rollback-capable render-graph transaction for arbitrary application-created entities outside the streaming ownership contract.

## Validation status

Static checks in this artifact-preparation environment include parsing all 44 TOML files, source-level invariant scans, CSV schema/index checks, Python syntax checks, ZIP integrity, and SHA-256 verification. This environment does not include `cargo`, `rustc`, `rustfmt`, PowerShell, or a native GPU, so the latest cumulative source has **not** been compiled, Rust-tested, Clippy-checked, or exercised on Windows/GPU here. A prior Windows/GPU run predates the latest source delta and is not evidence for these new changes.

Before merging, run the commands in `STREAMING_GAP_COMPLETION_REPORT.md`, then run both the headless soak and the native-GPU `-ExpectGpuPressure` stress configuration. Treat the report's validation tables and remaining boundaries as part of this delivery.
