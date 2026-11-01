# 0013 — Reflection implementation ownership for renderer contracts

## Context

Camera, mesh, light, and fog component data now live in `ribbon_render_contracts` so render plugins
and reusable systems can share them without depending on renderer internals. Their `Reflect`
implementations initially remained in `ribbon_plugin_render3d`, the former defining crate.

Rust's orphan rules prohibit `render3d` from implementing `ribbon_reflect::Reflect` for types that
are now defined in `ribbon_render_contracts`, because neither the trait nor those types are local.

## Decision

Move the `Reflect` implementations for contract-owned types into a private `reflection` module in
`ribbon_render_contracts`. Add a direct `ribbon_reflect` workspace dependency to that crate and
update `tools/allowed_deps.toml` and `Cargo.lock` in the same change. Keep the `ModelInstance`
implementation in `render3d` because `ModelInstance` is still defined there and contains a glTF handle.

## Rejected alternatives

- Keep the impls in `render3d`: rejected because it violates Rust's coherence/orphan rules.
- Duplicate local wrapper components in `render3d`: rejected because public ECS component identity and
  interoperability with weather/extraction would diverge.
- Move `ModelInstance` into the contracts crate: rejected because it would pull a glTF asset handle
  into the renderer-neutral layer and invert the dependency direction.

## Consequences

The contract crate now depends on `ribbon_scene`, `ribbon_reflect`, and `glam`, but still has no asset,
renderer, wgpu, or winit dependency. Reflection type names and field encodings are preserved; callers
continue to register the same public component types through `render3d`'s prefab registration API.
Compilation and prefab tests must be confirmed on the Windows workspace before accepting the change.
