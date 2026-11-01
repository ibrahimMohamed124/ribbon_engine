# Engine weather: camera-relative rain

`ribbon_plugin_weather` owns the reusable rain-particle lifecycle. A game chooses when precipitation is enabled and may adjust the public `RainSettings` resource; it does not spawn or tick individual rain entities. The weather plugin depends on `ribbon_render_contracts`, not on `ribbon_plugin_render3d`, so it creates normal mesh components without reaching into pipelines, GPU queues, or render passes.

## Responsibilities

- **Weather plugin:** particle spawn/recycling, deterministic initial distribution, fall speed, radius, wind drift, and disabling/removing the active pool.
- **Render contracts:** `Camera3D`, `BuiltinMesh`, `MeshInstance`, shared lights/fog parameters, and the shared `Transform3D` alias used by render-facing systems.
- **3D renderer:** consumes ordinary ECS mesh and camera components through its normal scene extraction and mesh rendering path.
- **Game/level:** toggles `RainSettings.enabled` at a narrative or zone boundary and chooses authored values.

## Default settings

`RainSettings::default()` keeps the Wet Creek scene's current density and look: 220 drops, a 21-unit radius, randomized fall speed between 11 and 20 units per second, the existing blue-gray tint, 0.42-unit streak scale, and no wind drift. The effect starts disabled. `active_particles()` reports zero while disabled and the configured pool size while enabled.

```rust
app.add_plugin(ribbon_plugin_weather::WeatherPlugin)?;
// When entering an outdoor rain zone:
world.resource_mut::<ribbon_plugin_weather::RainSettings>().enabled = true;
```

The weather update should be registered after the game's transition/update systems when the level enables rain during that same update. In ATHAR, plugin registration is intentionally placed after `gameplay_update`, preserving the existing ordering of the room-to-village transition. The pool is created on the active camera's position, then reused as drops fall below the ground offset.

## Current limitations

Rain is a visual mesh effect. It does not collide with arbitrary scene geometry, produce surface splashes, simulate wet materials, spawn puddles, or generate thunder audio. The system currently follows the camera entity's local `Transform3D`; a future extension for hierarchical cameras should use resolved `GlobalTransform3D` after transform propagation without reading stale transforms.
