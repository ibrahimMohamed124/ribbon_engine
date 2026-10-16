# GPU shader stages

- `present.wgsl` contains nearest-neighbor letterbox presentation and GPU full-screen passes for fog, caught/won noise, dim, brightness, and solid white. Its integer `hash4` matches `engine_core::math::hash4`.
- `ui.wgsl` contains the instanced rectangle and 5x7 glyph-pixel quad shaders. Glyph data is generated from the shared `engine_core::gfx::font::glyph` table.
- `world.wgsl` contains voxel DDA, material and entity shading, face AO, lighting, tone/gamma lookup, palette selection, and screen-space post effects.
