# Voxelworks Engine Architecture

The workspace is organized as a small engine core, optional plugins, sample
applications, and the STUDS game. Engine crates do not depend on game code.
`tools/allowed_deps.toml` is the machine-checked source for direct dependency
policy; run `python tools/check_deps.py` after changing a manifest.

## Layers and responsibilities

| Layer | Packages | Does not do |
| --- | --- | --- |
| Foundations | `engine_math`, `engine_ecs`, `engine_events`, `engine_schedule` | Platform I/O, rendering, or gameplay |
| Runtime | `engine_app`, `engine_plugins`, `engine_platform`, `engine_assets` | Game-specific rules or GPU scene extraction |
| Scene and rendering contracts | `engine_reflect`, `engine_scene`, `engine_render`, `engine_commands`, `engine_editor` | Game assets, gameplay, or automatic parallelism |
| Plugins | `plugin_voxel`, `plugin_physics`, `plugin_animation`, `plugin_audio`, `plugin_rendering`, `plugin_editor_tools` | Owning the app loop or game state |
| Applications | `examples/*`, `games/studs` | Being dependencies of engine crates |

The permitted package edges are listed in `tools/allowed_deps.toml`. Math and
ECS are leaves; schedule/events build on ECS; app builds on those foundations;
platform, assets, scene, and rendering build on app contracts; plugins add
reusable domain behavior; games compose plugins.

## Frame order and plugins

`App::update` advances `Time`, then runs `First`, `PreUpdate`, zero or more
`FixedUpdate` steps, `Update`, `PostUpdate`, `Render`, and `Last`, in that
order. The fixed-step loop is capped at eight steps per frame. `Startup` and
`Shutdown` run once. Headless runs use a virtual 60 Hz delta.

Plugins are added in dependency order. Adding a plugin validates its declared
dependencies and unique name, calls `build`, and records it in `Plugins`;
`finish` hooks run when the app is built. Systems run serially in deterministic
stage order. A plugin should declare its dependencies, install resources, and
register systems in the appropriate stages.

## Writing a plugin or game

A plugin implements `engine_plugins::Plugin`, names its required plugins, and
adds reusable engine behavior. A game owns its content and gameplay, installs
the needed plugins, and adds game-specific systems or a compatibility bridge.
Keep dependencies from engine/plugin packages to game packages out of the
workspace.

## Current limits and migration state

- Systems run serially; system parameter injection is not implemented.
- Asset loading is synchronous and handles are weak.
- The render graph is an ordered, validated pass list, not a resource allocator.
- The ECS rendering bridge currently renders through its CPU backend. Selecting
  its GPU backend only records a warning; the old standalone GPU renderer is
  still under `engine/crates/render-gpu`.
- The editor has no text input and is compiled only with its `editor` feature
  in debug builds. The glyph table limits available UI characters.
- Scene transforms are recomputed each frame; dirty tracking is not implemented.
- STUDS still uses the old trait-based runner and compatibility shims. The
  migration bridge and deletion of the old `engine/` tree are not complete.
- The sample voxel room's `--renderer gpu` option currently falls back to CPU
  because the ECS GPU backend is pending.

See `docs/SHIMS.md` for transitional imports and `docs/DECISIONS.md` for
choices made where the refactor plan left behavior unspecified.
