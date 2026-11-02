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
| HDR post-processing | Implemented, configurable | `PostProcessingSettings` updates uniform data without rebuilding pipelines; neutral defaults preserve the existing image and `PostProcessingSettings::cinematic()` exercises the full display-effects path. |
| Auto exposure and color grading | Implemented, limited | GPU 16x16 average-log-luminance compute adaptation, exposure compensation (EV), contrast, saturation, temperature and tint; no CPU pixel readback, histogram or local exposure. |
| Display effects | Implemented, configurable | Vignette, animated monochrome film grain and radial chromatic aberration; values are sanitized and bounded before upload. |
| MSAA | Implemented, limited | Select `None`, `Msaa2x`, `Msaa4x`, or `Msaa8x` through `AntiAliasingSettings`; the renderer negotiates a common sample count for `Rgba16Float` HDR resolve and `Depth32Float`, resolves MSAA before bloom/tone mapping, and falls back to the highest supported lower count. Configure it before the first GPU mesh/model upload. Runtime MSAA sample-count changes are not applied because existing renderer-owned GPU handles/pipelines must remain valid; resize recreates the color/depth targets at the active sample count. |
| FXAA | Implemented, basic | `AntiAliasingMode::Fxaa` selects a fullscreen post-tone-map FXAA pass; it can be toggled at runtime. It is conventional spatial FXAA, not temporal AA. |
| Directional shadows | Implemented, limited | One 1024² depth map fitted to the active camera frustum, with a rotation-invariant light-space bound, light-space texel snapping, and frustum-derived depth range; no cascades or user-configurable shadow distance/resolution yet. |
| Spot-light shadows | Implemented, limited | One selected shadow-casting spotlight at a time; selection is still extraction-order based and no atlas is allocated. |
| Point lights | Implemented, limited | Up to eight lights ranked deterministically by estimated camera-space contribution (distance, range, intensity, and color); attenuation only, no point-light shadow visibility test. |
| glTF OPAQUE / MASK / BLEND | Implemented, limited | OPAQUE ignores alpha; MASK performs base-color alpha discard in color and shadow passes; BLEND uses alpha blending, depth testing without depth writes, and camera-space back-to-front object sorting. No OIT; intersecting transparent surfaces can sort imperfectly. |
| glTF `doubleSided` | Implemented | Per-material no-cull pipeline variants; back-face normals are flipped for lighting. Ordinary materials keep back-face culling. |
| SSAO | Not implemented | Material AO maps do not compute screen-space occlusion. |
| IBL / reflection probes / runtime lightmaps | Not implemented | No environment irradiance, prefiltered specular, BRDF LUT, runtime probe, or lightmap sampling pipeline exists. |

### glTF alpha and sidedness

`OPAQUE` uses the opaque pipeline and ignores alpha. `MASK` compares the product of base-color factor alpha and base-color texture alpha against `alphaCutoff` in both color and shadow passes. `BLEND` uses standard straight-alpha blending with coverage from base-color factor alpha multiplied by base-texture alpha, remains depth-tested, does not write depth, and is rendered back-to-front by camera-space transformed mesh bounds center. Transparent materials do not cast the current directional or spotlight depth shadows. `doubleSided` selects per-material no-cull variants and flips back-face normals; it does not disable culling globally. Sorting is object-center based and is not order-independent transparency, so intersecting transparent geometry may still require asset authoring care.

The present shader and target configuration intentionally do not claim cascaded shadows, multiple spotlight shadow maps, point-light occlusion, order-independent transparency, temporal anti-aliasing, SSAO, or image-based lighting. MSAA uses a multisampled HDR color attachment and matching depth attachment, resolving into the single-sample HDR target before post-processing. FXAA runs in the fullscreen tone-mapping pass after the HDR+bloom image is converted to display-referred color. MSAA sample count is negotiated from the active adapter's HDR resolve and depth-format capabilities and is fixed when the renderer first creates GPU resources; changing that count at runtime is deferred to avoid invalidating live mesh/material handles. FXAA selection may be toggled at runtime; selecting FXAA after an MSAA renderer has initialized layers FXAA over the existing MSAA sample count rather than replacing it. Material AO maps are sampled in the material shader but do not compute screen-space occlusion. Auto exposure samples a fixed 16x16 grid rather than a histogram or local-exposure model; the post-process GPU timestamp query currently measures bloom and tone/display passes, not the compute pass as a separate GPU timing value.
