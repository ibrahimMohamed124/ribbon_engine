# Engine particles

`ribbon_plugin_particles` simulates emitters on the CPU in `Stage::Update` and writes batches of plain data to `ParticleRenderData`. `ribbon_plugin_render3d` extracts that data and draws it in the existing HDR mesh pass. The contracts crate contains only particle enums, instances, batches, and the frame resource.

```mermaid
flowchart LR
  E[ParticleEmitter components] --> S[ParticlePlugin CPU simulation]
  S --> D[ParticleRenderData contract]
  D --> X[Render3d PostUpdate extraction]
  X --> G[GPU particle pass inside meshes pass]
  G --> H[HDR resolve and post processing]
```

## API

- `ParticleEmitter` defines rate, bursts, shape, simulation space, lifetime, velocity, size/color curves, forces, ground response, render blend/shape, and deterministic seed.
- `ParticlePlugin` installs `ParticleBudget`, `ParticleDiagnostics`, and the `ParticleRenderData` resource, then updates each emitter.
- `Pcg32` provides repeatable sampling without a global random source.
- `ParticleRenderData.batches` contains one batch per emitting entity. World particles retain their spawn pose; local particles follow the emitter's current transform. Emitter scale is ignored.
- `ParticleEmitter::rain()`, `snow()`, `fire()`, `smoke()`, `sparks()`, and `dust()` provide authored presets. ATHAR attaches rain to its camera and uses the particle diagnostics count in its HUD.

| Preset | Shape | Typical use |
|---|---|---|
| `rain()` | Box | Camera-centered rain streaks; 147 particles/s, up to 512 |
| `snow()` | Sphere | Slow sparse snowfall |
| `fire()` | Cone | Additive warm flame |
| `smoke()` | Sphere | Expanding alpha smoke |
| `sparks()` | Point | Short additive streaks with gravity and ground response |
| `dust()` | Sphere | Short lived diffuse dust |

The simulator clamps `Time.delta` to 0.1 seconds, caps emitters at 100,000 particles, and uses a default global budget of 200,000. Alpha sorting is exposed in the batch contract and is limited to 20,000 instances per batch in the renderer. Extraction swaps the batch vector with the previous frame snapshot after that snapshot has been consumed by rendering; the simulation clears and reuses the returned batch/instance capacities. Empty trailing batches are retained as an allocation pool and are skipped by the renderer. World-space gravity and wind are rotated into emitter-local coordinates before local-space integration. Camera billboards use the camera world rotation basis (not inverse view-projection columns), and the renderer still processes particle-only scenes with no mesh draws. Removing an emitter component also removes its runtime simulation state.

## Not implemented

Textured sprites/flipbook atlases, soft particles or depth fade, GPU compute simulation, sub-emitters, trails/ribbons, mesh particles, lighting/shadows on particles, fog on particles, collision with physics colliders, and editor reflection of `ParticleEmitter` are not implemented. There is no particle frustum culling. The ignored GPU regressions cover the shadow-caster/color-pass interaction and an MSAA 4x particle readback; these still require a compatible adapter to be run. ATHAR's rain appearance needs manual visual verification.
