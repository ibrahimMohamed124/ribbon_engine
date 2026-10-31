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
