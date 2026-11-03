# ADR 0016: Engine particle system

The weather-only mesh effect is replaced by a reusable particle plugin. `ribbon_plugin_particles` owns deterministic CPU simulation and writes batches through `ribbon_render_contracts`; `ribbon_plugin_render3d` owns GPU upload and drawing. Neither package depends on the other. The weather plugin is removed and ATHAR owns the timing and placement of its rain emitter.

This boundary keeps simulation independent of GPU implementation while allowing the renderer to draw particles in its existing mesh pass. The current system does not provide textured sprites, soft depth fade, compute simulation, collision with physics, lighting, or editor reflection.
