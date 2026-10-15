# GPU shader stages

- `present.wgsl` contains the CPU image upload's nearest-neighbor letterbox pass and the GPU full-screen passes for fog, caught/won noise, dim, brightness, and solid white. Its integer `hash4` matches `engine_core::math::hash4`.
- `ui.wgsl` contains the instanced rectangle and 5x7 glyph-pixel quad shaders. Glyph data is generated from the shared `engine_core::gfx::font::glyph` table.
- `world.wgsl` will contain the world DDA, materials, lighting, entities, and palette path in Phase 4; the CPU remains the world renderer until that port is complete.
