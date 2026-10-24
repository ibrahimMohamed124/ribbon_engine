# GPU shader stages (voxel raymarcher)

Loaded by `src/gpu/voxel.rs`. Frozen per `docs/RENDERER_PLAN.md`; the mesh renderer in `ribbon_plugin_render3d` replaces this path.

- `present.wgsl` contains nearest-neighbor letterbox presentation and GPU full-screen passes for fog, caught/won noise, dim, brightness, and solid white. Its integer `hash4` matches `ribbon_math::hash4`.
- `ui.wgsl` contains the instanced rectangle and 5x7 glyph-pixel quad shaders. Glyph data is generated from the shared `ribbon_render::frame::glyph` table.
- `world.wgsl` contains voxel DDA, material and entity shading, face AO, lighting, tone/gamma lookup, palette selection, and screen-space post effects.
