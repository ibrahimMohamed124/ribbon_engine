# plugin_rendering
Owns rendering scene/material data, voxel extraction, and the CPU/GPU rendering backends.
It uses the shared `engine_render` frame contracts and `plugin_voxel` world data.
Allowed workspace dependencies are listed in `tools/allowed_deps.toml`.
It does not own window creation or the engine's general scene hierarchy.
