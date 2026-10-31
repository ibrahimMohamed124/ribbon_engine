# Ribbon Engine — Render3D test and Clippy follow-up (v4)

## Diagnostics addressed

The Windows validation run after v3 reported two remaining issues:

1. `prefab::tests::model_instance_round_trips_as_a_path_not_a_runtime_handle` failed with `MissingAsset("models/room.glb")`.
2. Workspace Clippy with `-D warnings` rejected the generic GPU warmup `loop` as `clippy::while_let_loop`.

## Changes

### Prefab round-trip test

The restore-side `AssetServer` previously registered a `MemorySource` but did not request or load the declared `ModelAsset` before calling `instantiate_with_context`. The model codec intentionally resolves only an already CPU-ready asset and must not perform synchronous I/O while restoring ECS components.

The test now explicitly prepares `models/room.glb` on the restore server using `AssetServer::load` before placing that server into `PrefabLoadContext`. This is test setup only; it preserves the runtime contract that scene streaming must resolve dependencies and establish CPU readiness before prefab instantiation. No synchronous load was added to the prefab codec or runtime streaming path.

### Clippy

`MeshPass::process_generic_gpu_warmups` now expresses its queue-draining loop with `while let Some(next_bytes) = ...`, retaining the same early exits for upload budget exhaustion or a missing job. Upload count and byte accounting remain in the loop body and are unchanged.

## Verification status

User-supplied Windows results before v4:

- `cargo fmt --all -- --check`: PASS.
- `cargo check -p ribbon_plugin_streaming --all-targets`: PASS.
- `cargo check --workspace --all-targets`: PASS.
- `ribbon_assets`: 20/20 PASS.
- `ribbon_scene`: 9/9 PASS.
- `ribbon_render`: 12/12 PASS.
- `ribbon_plugin_gltf`: 3/3 PASS.
- `ribbon_plugin_physics`: 24/24 PASS.
- `ribbon_plugin_streaming`: 42/42 PASS.
- `ribbon_plugin_editor_tools`: 1/1 PASS.
- `athar`: 21/21 PASS.
- `ribbon_plugin_render3d`: 30/31 PASS; one prefab round-trip fixture failed because its restore-side server had not prepared the referenced asset.
- Workspace tests: failed only because the same Render3D test failed.
- Workspace Clippy: failed on `clippy::while_let_loop` in `plugins/render3d/src/lib.rs`.
- Dependency policy: PASS (39 workspace packages).

The v4 edits are source-reviewed against those diagnostics but have not been compiled in this preparation environment because Rust/Cargo is unavailable here. Re-run the focused Render3D test and Clippy on Windows before treating v4 as validated.
