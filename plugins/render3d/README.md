# `ribbon_plugin_render3d`

`ribbon_plugin_render3d` is the engine's native mesh/PBR rendering plugin. It owns ECS extraction for cameras, built-in meshes, glTF models, lights, fog, GPU mesh/material uploads, depth, shadows, HDR tone mapping, and renderer diagnostics. It does not own game world formats or game-specific mesh generation.

The generic renderer extension surface is exposed through `mesh`, `post`, `profiling`, and `renderer` modules. Game-specific render integrations may depend on those public APIs without making the engine depend on the game.

See [`../../docs/architecture/renderer-roles.md`](../../docs/architecture/renderer-roles.md) and [`../../docs/architecture/RENDERER_PLAN.md`](../../docs/architecture/RENDERER_PLAN.md).

## Public data contracts

Renderer-neutral camera, built-in mesh, light, fog, and transform-facing data types are re-exported
from `ribbon_render_contracts`. The plugin retains glTF-specific `ModelInstance`, renderer extraction
logic, GPU resource management, PBR pipelines, shadows, and post-processing. This preserves existing
`ribbon_plugin_render3d::*` type paths while allowing reusable engine plugins such as weather to
consume public mesh/camera types without depending on render implementation.


## Current rendering capability and known boundaries

| Feature | Status in this source tree | Boundary |
| --- | --- | --- |
| Metallic/roughness PBR | Implemented | Cook-Torrance/GGX; glTF base color and packed metallic-roughness textures. |
| Normal maps | Implemented | TBN reconstructed from fragment derivatives; imported glTF tangents are not stored by the current mesh format. |
| Material AO and emissive maps | Implemented | Material texture sampling only; this is not SSAO. |
| Fog and bloom | Implemented, basic | Exponential height/distance fog; one quarter-resolution bright-pass/blur bloom target. |
| Directional shadows | Implemented, limited | One 1024² depth map fitted to the active camera frustum, with a rotation-invariant light-space bound, light-space texel snapping, and frustum-derived depth range; no cascades or user-configurable shadow distance/resolution yet. |
| Spot-light shadows | Implemented, limited | One selected shadow-casting spotlight at a time; selection is still extraction-order based and no atlas is allocated. |
| Point lights | Implemented, limited | Up to eight lights ranked deterministically by estimated camera-space contribution (distance, range, intensity, and color); attenuation only, no point-light shadow visibility test. |
| glTF alpha modes / double-sided | Not implemented | OPAQUE/MASK/BLEND and `doubleSided` are not currently represented in the GPU material pipeline. |
| MSAA / FXAA, SSAO, IBL, probes, lightmaps | Not implemented | No user-facing setting should claim these features. |

The present shader and target configuration intentionally do not claim cascaded shadows, multiple spotlight shadow maps, point-light occlusion, order-independent transparency, temporal anti-aliasing, automatic exposure, SSAO, or image-based lighting. Directional shadow fitting uses the current camera's full projection frustum; there is not yet a configurable shadow-distance clamp, cascade blending, or runtime shadow-quality resource. Material AO maps are sampled in the material shader but do not compute screen-space occlusion.
