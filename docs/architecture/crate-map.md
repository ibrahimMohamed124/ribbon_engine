# Crate map and dependency direction

## Layers

```text
core: math / ecs / events / schedule / reflect / profiling / input
  -> app and plugin composition (typed app states)
  -> platform (native gamepad polling) / assets / scene / render contracts / render / commands
  -> editor and editor tools
  -> rendering, weather, physics, audio, animation, glTF plugins
  -> games and examples
```

Dependencies point downward only. Core crates must not depend on app, platform, plugins, or games. Engine crates must never depend on `games/*`. `ribbon_scene` does not depend on assets or renderer crates. `ribbon_render_contracts` is the renderer-neutral data/metadata contract crate (camera, mesh, light, fog, mesh/camera extraction values, and their reflection metadata); it may depend on `ribbon_scene` and `ribbon_reflect` but never on renderer implementation or game crates. `ribbon_plugin_weather` depends on this contracts crate rather than on `ribbon_plugin_render3d`. `ribbon_input` and `ribbon_profiling` are leaf crates with no workspace-crate dependencies. Game-owned crates and showcases may depend on engine public APIs and other packages owned by the same game. Cross-game dependencies are forbidden: `games/athar` must not depend on `games/studs`, and vice versa. Engine crates, plugins, and engine-only examples must never depend on any `games/*` package.

Reserved future positions (not created here): `ribbon_ui` belongs beside editor/application services; `ribbon_save` belongs above scene/assets serialization; `ribbon_nav` or `ribbon_ai` belongs in a future gameplay-services layer above core and below games.
