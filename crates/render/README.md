# Ribbon Render (`ribbon_render`)
Owns the GPU context/device lifecycle, native-surface acquisition and presentation contracts, the validated render graph, and renderer-neutral frame plans.
Frame plans remain generic so rendering plugins can supply their own scene and material types.
Allowed workspace dependencies: `ribbon_app`, `ribbon_plugins`, `ribbon_platform`, `ribbon_assets`, and `ribbon_math`; `wgpu` is optional behind `gpu`.
It does not own scene extraction, mesh/PBR policy, or game-specific rendering policy.
