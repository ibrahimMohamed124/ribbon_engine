# Ribbon skeletal GPU skinning vertex-layout fix

This incremental overlay fixes the GPU vertex-input layout for the `Vertex` structure used by `ribbon_plugin_render3d`.

## Root cause

`Vertex` stores all eight `joints` first, then all eight `weights`. The old `vertex_attr_array!` declaration packed the shader attributes sequentially, causing the second four joint indices to be exposed as `WEIGHTS_0` and the first four weights to be exposed as `JOINTS_1`. On the real GPU this left the skinning weight sum at zero for the regression test, so the palette translations were not applied.

## Apply

Extract this ZIP into the repository root and overwrite the two files:

- `plugins/render3d/src/renderer.rs`
- `plugins/render3d/src/renderer/tests.rs`

The `renderer.rs` included here preserves the `RenderTarget`/Clippy refactor from `Ribbon_Render3D_Clippy_Args_Fix`.

## Validate on Windows

```powershell
cargo fmt
cargo fmt --all -- --check
cargo test -p ribbon_plugin_render3d vertex_attributes_match_the_interleaved_joint_and_weight_storage_layout
cargo test -p ribbon_plugin_render3d --all-targets
cargo clippy --workspace --all-targets -- -D warnings
./tools/test_skeletal_skinning_gpu.ps1
cargo test --workspace --no-fail-fast
```

The final GPU command is important: it runs the previously ignored real-adapter regression test. A passing Rust compile or headless suite alone does not establish that GPU skinning works on hardware.
