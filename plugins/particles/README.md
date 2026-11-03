# Particle plugin

`ParticlePlugin` simulates deterministic CPU particles in `Stage::Update` and writes batches into the renderer-neutral `ParticleRenderData` resource. It owns no GPU resources. `ParticleEmitter::rain()` is the engine replacement for ATHAR's old weather meshes.

Emitter dimensions are authored in local space and scale is ignored; `World` particles retain their spawn transform while `Local` particles follow the emitter's current global transform. The simulation clamps frame delta to 0.1 seconds, caps each emitter and the global total, and reuses the render data batch/instance buffers between frames where possible.

Textured sprites, flipbooks, soft particles, GPU compute, sub-emitters, trails, mesh particles, lighting/shadows/fog, physics collision, and editor reflection are not implemented.
