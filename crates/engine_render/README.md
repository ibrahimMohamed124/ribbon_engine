# engine_render
Provides backend-neutral draw data, a validated render graph, CPU presentation, and an optional wgpu context.
Frame plans are generic so rendering plugins can supply their own scene and material types.
Allowed workspace dependencies: `engine_app`, `engine_plugins`, `engine_platform`, `engine_assets`, and `engine_math`; `wgpu` is optional behind `gpu`.
It does not own scene extraction or game-specific rendering policy.
