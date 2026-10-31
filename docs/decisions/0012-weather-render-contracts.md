# 0012 — Renderer-neutral contracts for reusable weather

## Context

The weather simulation needs camera, transform, and mesh-instance data, but it does not need GPU
queues, render passes, pipeline internals, or extraction scheduling. Depending directly on the
render3d implementation would unnecessarily couple an engine plugin to a renderer plugin.

## Decision

Move renderer-neutral components and the reusable camera/mesh extraction records into
`ribbon_render_contracts`. `ribbon_plugin_render3d` continues to re-export the existing public types
and owns extraction and GPU submission. `ExtractedScene3D` and `ExtractedModel3D` remain in
`render3d`: the scene snapshot contains glTF-backed model handles, so moving that aggregate would
introduce an upward dependency from the engine contracts crate into a renderer/plugin-specific type. `ribbon_plugin_weather` depends on the contracts crate, not on `ribbon_plugin_render3d`.
ATHAR only configures `RainSettings`; the engine plugin owns rain mesh lifetime, movement, and reuse.

## Rejected alternatives

- Keep weather dependent on render3d: simpler in the short term, but creates the exact implementation
  dependency the architecture task asks to avoid.
- Keep rain simulation in ATHAR: preserves a game-local implementation but prevents reuse and leaves
  the expected engine weather plugin missing from the workspace.
- Move GPU/render graph code or the glTF-typed aggregate snapshot into contracts: that would make a data-only engine crate depend upward on renderer implementation or a typed glTF asset.

## Consequences

The package graph gains `ribbon_render_contracts` and `ribbon_plugin_weather`; no new third-party
dependency or version is introduced. The contracts crate depends on `ribbon_scene`, `ribbon_reflect`, and `glam`; its data types do not depend on `ribbon_assets`, `ribbon_plugin_gltf`, wgpu, or winit. The `ribbon_reflect` edge exists so `Reflect` implementations live beside the contract types, satisfying Rust orphan rules. Existing `ribbon_plugin_render3d` type paths remain available.
The weather defaults preserve ATHAR's 220 drops, speed range, radius, transform scale, and tint.
GPU output and system ordering still require verification in a Windows run.
