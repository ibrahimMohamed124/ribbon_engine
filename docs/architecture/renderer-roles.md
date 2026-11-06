# Renderer roles

- `ribbon_render` owns GPU context and device lifecycle, surface acquisition/presentation, render graph/frame planning, and renderer-neutral frame contracts.
- `ribbon_plugin_render3d` owns mesh/PBR rendering, scene extraction, lights, depth, shadows, and post-processing.
- `ribbon_render_contracts` owns renderer-neutral camera, mesh, light, fog, and particle batch data; `ribbon_plugin_particles` writes particle batches without depending on render3d implementation details.
- STUDS owns its voxel data, chunk meshing, CPU raycaster, and the offscreen GPU bridge that adapts chunk meshes to the public mesh renderer API.


The native mesh renderer uses a 56-byte static GPU vertex stream (position, normal, UV0,
tangent/handedness, UV1) for unskinned glTF placements and the built-in cube. Skinned glTF
placements use a 104-byte GPU stream and preserve all eight influences. The source-compatible
80-byte CPU `Vertex` input remains available; its upload adapter selects the compact static
stream when all weights are zero and the skinned stream when weights are present or malformed.
Both paths generate tangent data and fall back from UV1 to UV0. glTF imports `TANGENT`, `TEXCOORD_0`, and `TEXCOORD_1`, retaining
per-texture UV selectors for the five supported PBR texture roles. Higher UV sets currently
fall back to UV0, and UV1 normal maps derive their tangent frame from screen-space derivatives. The generated CPU tangent fallback is area-weighted rather than full MikkTSpace; authored glTF tangents remain preferred for exact normal-baker seam matching. `KHR_texture_transform` is not applied.
