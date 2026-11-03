# Crate map and dependency direction

## Layers

```text
core: math / ecs / events / schedule / reflect / profiling / input
  -> app and plugin composition (typed app states)
  -> platform (native gamepad polling) / assets / scene / render contracts / render / commands
  -> editor and editor tools
  -> rendering, particles, physics, audio, animation, glTF plugins
  -> games and examples
```

Dependencies point downward only. Core crates must not depend on app, platform, plugins, or games. Engine crates must never depend on `games/*`. `ribbon_scene` does not depend on assets or renderer crates. `ribbon_render_contracts` is the renderer-neutral data/metadata contract crate for camera, mesh, light, fog, and particle batches. The particle plugin depends on this contracts crate rather than on `ribbon_plugin_render3d`. `ribbon_input` and `ribbon_profiling` are leaf crates with no workspace-crate dependencies. Game-owned crates and showcases may depend on engine public APIs and other packages owned by the same game. Cross-game dependencies are forbidden: `games/athar` must not depend on `games/studs`, and vice versa. Engine crates, plugins, and engine-only examples must never depend on any `games/*` package.

`ribbon_save` provides game-owned persistence above the core world and plugin services. `ribbon_ui` and `ribbon_plugin_ai` occupy application services and gameplay-services layers respectively; their implemented scope is documented in their architecture pages.
