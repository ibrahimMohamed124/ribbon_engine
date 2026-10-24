# Ribbon Voxel Rendering Plugin (`ribbon_plugin_rendering`)
Owns rendering scene/material data, voxel extraction, and the CPU/GPU rendering backends (`gpu::voxel` is the frozen voxel raymarching GPU backend).
It uses the shared `ribbon_render` frame contracts and `ribbon_plugin_voxel` world data.
The `cpu` and `gpu` features are enabled by default and can be disabled independently.
Allowed workspace dependencies are listed in `tools/allowed_deps.toml`.
It does not own window creation or the engine's general scene hierarchy.
