# Skeletal Animation and GPU Skinning — Implementation Report

## Scope delivered in this overlay

- glTF 2.0 `skins`, `joints`, inverse-bind matrices, node hierarchy, and node TRS animation channels
  are imported into CPU-side `ModelAsset` data.
- `JOINTS_0` / `WEIGHTS_0` and `JOINTS_1` / `WEIGHTS_1` are decoded to eight influences per vertex.
  Influences are normalized, invalid joint references and malformed attributes are rejected, and
  more than eight influences are rejected rather than silently truncated.
- Translation, rotation, and scale channels support `STEP`, `LINEAR`, and `CUBICSPLINE`; quaternion
  linear interpolation uses slerp.
- `SkeletalAnimationPlayer` advances clips without blocking the game thread. `AnimationPlugin`
  evaluates local and global node transforms and publishes immutable, generation-tagged `SkeletalPose`
  values.
- Bind-pose palettes are prepared as a fallback when a model has skins but no runtime animation
  player, so weighted meshes do not fall back to an unskinned draw.
- The renderer uploads palettes to a fixed-capacity, renderer-owned storage buffer and performs
  vertex-stage linear blend skinning for color, directional-shadow, and spot-shadow draw paths.
  Eight influences are supported. The frame palette capacity is 65,536 matrices.
- Palette admission is checked before appending. An over-capacity draw is skipped without producing
  an out-of-range GPU offset. Skinned placements bypass bind-pose frustum bounds because animated
  joints can expand outside static bounds. The accepted/rejected draw counts and staged matrix count are exposed
  through `RenderDiagnostics.resources`, the backend resource snapshot, the Editor streaming/render
  diagnostics panel, and CSV columns 170–172 (zero-based; the CSV now has 173 columns).
- GPU buffer ownership and writes remain in `ribbon_plugin_render3d`; the animation plugin only
  evaluates CPU transforms and publishes immutable values.
- Voxel renderer clients initialize the new joint/weight vertex fields to zero, preserving static
  voxel rendering.

## Public runtime use

Load a glTF model through `AssetServer`, create a `ModelInstance`, and attach
`SkeletalAnimationPlayer::new(clip_index)` to the same entity. Register `AnimationPlugin` in the
application. The `clip_index` selects an animation embedded in the loaded `ModelAsset`. Use the
player's `looping`, `playback_speed`, and `paused` fields to control playback. Gameplay code does not
create or write GPU buffers.

A scene/streaming owner must keep the selected `ModelAsset` generation alive while the instance is
active. The render extractor validates pose asset ID and generation before consuming it.

## Tests added or extended

- glTF import fixture validates skin, parent hierarchy, inverse-bind data, eight joint influences,
  animation channels, and CPU memory accounting.
- Animation tests cover STEP, LINEAR, CUBICSPLINE (including a single-key constant clip), quaternion
  interpolation, node hierarchy, and mesh-local joint palette evaluation.
- Upload validation rejects positive weights with out-of-range joint indices, weighted vertices
  without a skin, and non-normalized small non-zero weight sums that would otherwise activate the WGSL
  skinning path with an invalid palette.
- Render3D tests validate both WGSL modules, bind-pose/model upload validation, and deterministic
  palette-capacity admission including overflow rejection without corrupting existing offsets.
- An adapter-backed render test is intentionally ignored by the normal test suite; the dedicated
  PowerShell script runs it and fails if a wgpu adapter is unavailable. It asserts actual vertex-stage
  palette movement in the color pass.
- Render CSV tests cover the new skeletal counters and field alignment.

## Known limits / truthful boundaries

- Morph-target (blend-shape) weight animation is not imported or evaluated.
- A vertex may use at most eight joint influences; files requiring more are rejected.
- The frame joint-palette storage capacity is fixed at 65,536 matrices. Draws beyond remaining capacity
  are dropped safely and counted; this is not an unlimited palette allocator.
- The hardware test asserts palette-driven movement in the color output. The directional and spot
  shadow shaders consume the same palette and are WGSL-validated, but no separate pixel-level
  shadow-map result test is currently supplied.
- The animation data is CPU-side and embedded in the ModelAsset generation. GPU skinning itself is
  renderer-owned; no per-clip GPU allocation or GPU-side animation evaluation is claimed.

## Validation status

This artifact-preparation environment has no `cargo`, `rustc`, or `rustfmt`, so the skeletal delta has
not been compiled or tested here. The previous checkout (before this skeletal delta) was reported by
the user to pass the workspace streaming validation 15/15, the separate RTX 2050/Vulkan shader and
pipeline device-recreation test, and the Render3D/Streaming targeted suites. Those results do not
prove this new skeletal delta builds.

Run on Windows from the workspace root:

```powershell
cargo fmt
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test -p ribbon_plugin_gltf --all-targets
cargo test -p ribbon_plugin_animation --all-targets
cargo test -p ribbon_plugin_render3d --all-targets
cargo test -p ribbon_plugin_streaming --all-targets
cargo test -p ribbon_plugin_editor_tools --all-targets
cargo clippy --workspace --all-targets -- -D warnings
py -3 tools/check_deps.py
cargo test --workspace --no-fail-fast
.\tools\test_skeletal_skinning_gpu.ps1
.\tools\test_shader_pipeline_device_recovery.ps1
```

The ordinary Render3D test suite should report the two hardware-only tests as ignored. Run both
PowerShell scripts separately on a GPU-equipped machine to obtain their real-adapter results.
