# Ribbon Render (`ribbon_render`)
Provides backend-neutral draw data, a validated render graph, CPU presentation, and an optional wgpu context.
Frame plans are generic so rendering plugins can supply their own scene and material types.
Allowed workspace dependencies: `ribbon_app`, `ribbon_plugins`, `ribbon_platform`, `ribbon_assets`, and `ribbon_math`; `wgpu` is optional behind `gpu`.
It does not own scene extraction or game-specific rendering policy.
