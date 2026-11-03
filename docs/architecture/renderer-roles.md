# Renderer roles

- `ribbon_render` owns GPU context and device lifecycle, surface acquisition/presentation, render graph/frame planning, and renderer-neutral frame contracts.
- `ribbon_plugin_render3d` owns mesh/PBR rendering, scene extraction, lights, depth, shadows, and post-processing.
- `ribbon_render_contracts` owns renderer-neutral camera, mesh, light, fog, and particle batch data; `ribbon_plugin_particles` writes particle batches without depending on render3d implementation details.
- STUDS owns its voxel data, chunk meshing, CPU raycaster, and the offscreen GPU bridge that adapts chunk meshes to the public mesh renderer API.
