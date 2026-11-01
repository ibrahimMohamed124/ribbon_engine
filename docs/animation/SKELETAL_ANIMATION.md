# glTF skeletal animation and GPU skinning

Ribbon imports glTF 2.0 skinning data into `ModelAsset` and evaluates node TRS animation on the
CPU. The render plugin consumes immutable joint palettes in the vertex stage. GPU resource
creation, buffer writes, and destruction remain owned by `ribbon_plugin_render3d`.

## Imported data and supported limits

- `JOINTS_0` / `WEIGHTS_0` and `JOINTS_1` / `WEIGHTS_1` are supported: up to eight influences
  per vertex. Weights are validated and normalized during import.
- `skins[].joints` and `inverseBindMatrices` are retained in source joint order. Missing inverse
  bind matrices follow glTF's identity default.
- Node translation, rotation, and scale tracks are supported with `STEP`, `LINEAR`, and
  `CUBICSPLINE` interpolation. Quaternion `LINEAR` interpolation uses slerp.
- Morph-target weight animation is not implemented. More than eight vertex influences are
  rejected rather than truncated. Skinned placements bypass static bind-pose frustum bounds because
  animated joints can move geometry outside those bounds; clip-space clipping remains authoritative.
- Joint palettes are evaluated in mesh-local space and consumed by both the main color and
  directional/spot shadow vertex shaders. The current frame has a fixed 65,536-matrix palette
  capacity. A skinned draw that exceeds remaining capacity is skipped safely; the renderer
  exposes post-budget/rejected draw counts and the number of matrices staged for upload through
  `RenderDiagnostics.resources` (and the backend `MeshRendererResourceStats` snapshot).

## Runtime use

Load the model through `AssetServer`, create a normal `ModelInstance`, and attach
`SkeletalAnimationPlayer::new(clip_index)` to the same ECS entity. The clip index refers to the
embedded `ModelAsset::skeletal_animations` array. Set `looping`, `playback_speed`, and `paused` on
the component to control playback. `AnimationPlugin` must be registered to advance players and
publish per-frame poses. No explicit GPU upload is needed from gameplay code.

A model with a skin but no player still renders with a bind-pose palette retained by the renderer.
A model with animation channels but no skin can animate its node-attached mesh placements when an
animation player is present.

The scene/streaming owner must keep the `ModelAsset` generation alive while its ModelInstance is
active. The renderer rejects stale poses whose model handle generation does not match the visible
instance.

## Validation

The `ribbon_plugin_gltf` tests cover skin import, joint attributes, inverse-bind data, animation
track import, and decoded-memory accounting. The `ribbon_plugin_animation` tests cover all three
interpolation modes and pose/palette evaluation. The normal `ribbon_plugin_render3d` suite validates
both color and shadow WGSL modules and the bounded-palette admission behavior. The adapter-backed
vertex test is intentionally ignored by the normal suite and must be run explicitly with
`tools/test_skeletal_skinning_gpu.ps1`; it asserts actual vertex-stage palette movement in the color
pass. The same skin offsets and matrices are used by directional and spot shadow passes, but a
separate pixel-level shadow-map assertion is not included. Run the workspace validation suite on
the target Windows/GPU host before shipping.
