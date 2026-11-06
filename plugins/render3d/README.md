# `ribbon_plugin_render3d`

`ribbon_plugin_render3d` is the engine's native mesh/PBR rendering plugin. It owns ECS extraction for cameras, built-in meshes, glTF models, lights, fog, GPU mesh/material uploads, depth, shadows, HDR tone mapping, and renderer diagnostics. It does not own game world formats or game-specific mesh generation.

The generic renderer extension surface is exposed through `mesh`, `post`, `profiling`, and `renderer` modules. Game-specific render integrations may depend on those public APIs without making the engine depend on the game.

See [`../../docs/architecture/renderer-roles.md`](../../docs/architecture/renderer-roles.md) and [`../../docs/architecture/RENDERER_PLAN.md`](../../docs/architecture/RENDERER_PLAN.md).

## Public data contracts

Renderer-neutral camera, built-in mesh, light, fog, and transform-facing data types are re-exported
from `ribbon_render_contracts`. The plugin retains glTF-specific `ModelInstance`, renderer extraction
logic, GPU resource management, PBR pipelines, shadows, particles, and post-processing. This preserves existing
`ribbon_plugin_render3d::*` type paths while allowing reusable engine plugins such as particles to
consume public mesh/camera types without depending on render implementation.

The renderer has separate GPU vertex layouts: `StaticVertex` is 56 bytes (position, normal,
UV0, tangent plus handedness, and UV1); `SkinnedVertex` is 104 bytes and adds the existing
eight joint indices and weights. Unskinned glTF placements and the built-in cube use the static
layout; skinned placements retain all eight influences. A primitive referenced by both kinds
of node is uploaded once per required layout. The source-compatible `Vertex` and `MeshData`
CPU input remains 80 bytes. `MeshRenderer::upload_mesh` automatically uses the 56-byte static
layout when all eight legacy weights are zero, and preserves the 104-byte skinned layout when
weights are present or malformed. Both paths generate a tangent frame and use UV0 as the UV1
fallback; callers with authored tangent/UV1 data can use `upload_static_mesh` or
`upload_skinned_mesh` directly.

## Current rendering capability and known boundaries

| Feature | Status in this source tree | Boundary |
| --- | --- | --- |
| Metallic/roughness PBR | Implemented | Cook-Torrance/GGX; glTF base color and packed metallic-roughness textures. |
| Static/skinned vertex uploads | Implemented, partial | Static glTF placements and the built-in cube use a 56-byte position/normal/UV0/tangent/UV1 stream; skinned placements use a 104-byte stream with eight influences. Legacy CPU `Vertex` remains 80 bytes; `upload_mesh` selects the 56-byte static layout for zero-weight data and the 104-byte skinned layout for weighted data. Other producers that need authored UV1/tangents should use the explicit upload methods. |
| Tangents and normal maps | Implemented, limited | glTF `TANGENT` is imported and normalized; missing/invalid tangent frames use an area-weighted indexed-triangle/UV0 fallback with a deterministic degenerate-UV basis. This is not a full MikkTSpace implementation; provide authored tangents when exact baker-compatible seams matter. Normal maps using UV1 derive a per-fragment tangent frame from UV1 derivatives because glTF `TANGENT` is defined against UV0. |
| Multiple texture-coordinate sets | Implemented, limited | `TEXCOORD_0` and `TEXCOORD_1` are imported, and each base-color, metallic-roughness, normal, occlusion, and emissive texture retains its own `texCoord` selector. Higher UV sets fall back to UV0; missing UV1 aliases UV0. `KHR_texture_transform` offset/rotation/scale is not applied. |
| Material AO and emissive maps | Implemented | Material texture sampling only; independent from screen-space AO. |
| Fog and bloom | Implemented, basic | Exponential height/distance fog; one quarter-resolution bright-pass/blur bloom target. |
| HDR post-processing | Implemented, configurable | `PostProcessingSettings` updates uniform data without rebuilding pipelines; neutral defaults preserve the existing image and `PostProcessingSettings::cinematic()` exercises the full display-effects path. |
| Auto exposure and color grading | Implemented, limited | GPU 16x16 average-log-luminance compute adaptation, exposure compensation (EV), contrast, saturation, temperature and tint; no CPU pixel readback, histogram or local exposure. |
| Display effects | Implemented, configurable | Vignette, animated monochrome film grain and radial chromatic aberration; values are sanitized and bounded before upload. |
| MSAA | Implemented, limited | Select `None`, `Msaa2x`, `Msaa4x`, or `Msaa8x` through `AntiAliasingSettings`; the renderer negotiates a common sample count for `Rgba16Float` HDR resolve and `Depth32Float`, resolves MSAA before bloom/tone mapping, and falls back to the highest supported lower count. Configure it before the first GPU mesh/model upload. Runtime MSAA sample-count changes are not applied because existing renderer-owned GPU handles/pipelines must remain valid; resize recreates the color/depth targets at the active sample count. |
| FXAA | Implemented, basic | `AntiAliasingMode::Fxaa` selects a fullscreen post-tone-map FXAA pass; it can be toggled at runtime. It is conventional spatial FXAA, not temporal AA. |
| Directional shadows | Four-cascade implementation; RTX 2050/Vulkan split, blend, sampled transition, texel-stability, and static/skinned caster checks passed | Four overlapped 1024² `Depth32Float` layers in one depth array; practical 65% logarithmic / 35% linear camera-depth splits, independently fitted and texel-snapped light matrices, 3x3 PCF, and a 10% transition blend. Bounds from the accepted opaque/masked draw set extend lower cascade depth for overlapping upstream static/current-pose skinned casters within the configured extrusion limit; receiver XY fitting remains unchanged. The camera far plane bounds the cascades; per-cascade caster culling and configurable shadow distance/resolution remain deferred. |
| Spot-light shadows | Implemented, limited | One shadow-casting spotlight at a time, selected by a deterministic camera-aware contribution score with value-based tie-breaking; no multi-caster atlas is allocated. |
| Point lights | Implemented, limited | Up to eight lights ranked deterministically by estimated camera-space contribution (distance, range, intensity, and color); attenuation only, no point-light shadow visibility test. |
| glTF OPAQUE / MASK / BLEND | Implemented, limited | OPAQUE ignores alpha; MASK performs base-color alpha discard in color and shadow passes; BLEND uses alpha blending, depth testing without depth writes, and camera-space back-to-front object sorting. No OIT; intersecting transparent surfaces can sort imperfectly. |
| glTF `doubleSided` | Implemented | Per-material no-cull pipeline variants; back-face normals are flipped for lighting. Ordinary materials keep back-face culling. |
| Screen-space ambient occlusion | Implemented, spatial GTAO-style baseline | Full-resolution depth-reconstructed horizon sampling; single-sample and multisampled depth paths; Low/Medium/High quality. Visibility multiplies the scene HDR contribution (not ambient-only light); there is no temporal reprojection, history denoising or G-buffer normal input. The full-resolution Rgba8 target remains allocated while disabled, but compute dispatch is skipped. |
| IBL / reflection probes / runtime lightmaps | Not implemented | No environment irradiance, prefiltered specular, BRDF LUT, runtime probe, or lightmap sampling pipeline exists. |

### glTF alpha and sidedness

`OPAQUE` uses the opaque pipeline and ignores alpha. `MASK` compares the product of base-color factor alpha and base-color texture alpha against `alphaCutoff` in both color and shadow passes. `BLEND` uses standard straight-alpha blending with coverage from base-color factor alpha multiplied by base-texture alpha, remains depth-tested, does not write depth, and is rendered back-to-front by camera-space transformed mesh bounds center. Transparent materials do not cast the current directional or spotlight depth shadows. `doubleSided` selects per-material no-cull variants and flips back-face normals; it does not disable culling globally. Sorting is object-center based and is not order-independent transparency, so intersecting transparent geometry may still require asset authoring care.

The directional-light path has four overlapped camera-frustum cascades in a `Depth32Float` array, with practical splits, 3x3 PCF and transition blending. The base CSM GPU smoke test was confirmed on NVIDIA RTX 2050/Vulkan; cascade selection and per-layer depth have GPU diagnostic captures; actual boundary crossing and per-pixel PCF blending passed a GPU camera-sweep diagnostic on that adapter. Texel snapping has per-cascade CPU matrix tests and a 1024² GPU depth-layer translation test, both passed on the target host; these discrete checks do not claim continuous stability under arbitrary camera motion. The renderer does not claim multiple spotlight shadow maps, point-light occlusion, order-independent transparency, temporal anti-aliasing, or image-based lighting. MSAA uses a multisampled HDR color attachment and matching depth attachment, resolving into the single-sample HDR target before post-processing. FXAA runs in the fullscreen tone-mapping pass after the HDR+bloom image is converted to display-referred color. MSAA sample count is negotiated from the active adapter's HDR resolve and depth-format capabilities and is fixed when the renderer first creates GPU resources; changing that count at runtime is deferred to avoid invalidating live mesh/material handles. FXAA selection may be toggled at runtime; selecting FXAA after an MSAA renderer has initialized layers FXAA over the existing MSAA sample count rather than replacing it. Material AO maps remain independent of screen-space occlusion. The optional `AmbientOcclusionSettings` resource adds a deterministic depth-reconstructed horizon pass (GTAO-style) before bloom/tone mapping; it reads the main `Depth32Float` attachment, writes a full-resolution visibility target, and has separate single-sample and multisampled depth pipelines. Its default is disabled. It is a spatial screen-space approximation, not temporally reprojected GTAO; disocclusion history, geometric normals, denoising and a separate GPU timestamp metric remain future refinements. Auto exposure samples a fixed 16x16 grid rather than a histogram or local-exposure model; current post-process GPU timestamp queries cover bloom and tone/display passes, while AO and exposure command encoding are listed as CPU phases rather than GPU execution durations.


## Cascaded directional-shadow diagnostics

`Render3dPlugin` installs `DirectionalShadowDebugOptions` as a world resource. It defaults to
`Disabled`, so normal PBR output is unchanged. Change the resource at runtime to inspect the
active renderer without rebuilding its pipelines:

```rust
use ribbon_plugin_render3d::{DirectionalShadowDebugMode, DirectionalShadowDebugOptions};

world.insert_resource(DirectionalShadowDebugOptions {
    mode: DirectionalShadowDebugMode::CascadeIndex,
    depth_layer: 0,
});
```

Code that owns a standalone `MeshRenderer` can also call
`renderer.set_directional_shadow_debug(options)` directly.

Supported modes:

- `Disabled`: normal material, lighting, and shadow rendering (default).
- `CascadeIndex`: visible mesh fragments use fixed red/green/blue/yellow colors for cascades 0–3.
- `BlendWeight`: visible mesh fragments show the current-to-next cascade transition weight; green is
  zero, red is one, and the intermediate transition ramps through yellow.
- `DepthLayer`: replaces the scene output with a full-screen grayscale preview of one depth-array
  layer. Clear/far depth is black; nearer caster depth is brighter. `depth_layer` is clamped to 0–3.
- `ShadowVisibility`: grayscale output of the actual directional visibility function after both
  3×3 PCF samples and the current-to-next cascade blend have executed. This is the mode to compare
  during camera movement; it is not a replacement for the normal PBR pass.
- `ShadowVisibilityComponents`: RGB shows current-cascade PCF visibility, next-cascade PCF
  visibility, and the exact blended result respectively. This mode is intended for validating the
  interpolation itself at shadow edges, not for normal scene presentation.

The diagnostic views still render the four directional shadow passes first. `DepthLayer` then
samples the selected depth-array layer directly. `CascadeIndex` and `BlendWeight` expose the same
selection data used by the normal shadow visibility function. `ShadowVisibility` shows its final
result, while `ShadowVisibilityComponents` exposes the two PCF inputs and the final interpolation
so the test can reject degenerate probes where both inputs are identically lit or shadowed. Particle
rendering is suppressed in mesh-color debug modes so particles cannot obscure diagnostic output.
Spotlight shadow resources and selection remain unchanged.

On a GPU-capable machine, run:

```powershell
cargo test --locked -p ribbon_plugin_render3d directional_shadow_debug_modes_write_high_resolution_pngs -- --ignored --nocapture
```

The test writes six 1280×720 images under `target/`:

- `csm_debug_cascade_index.png`
- `csm_debug_blend_weight.png`
- `csm_debug_depth_layer_0.png` through `csm_debug_depth_layer_3.png`

These images help inspect cascade selection, blend regions, and individual depth-array layers. They do
not by themselves prove temporal stability. For the sampled dynamic transition check, run:

```powershell
cargo test --locked -p ribbon_plugin_render3d directional_cascade_transition_camera_sweep_writes_gpu_and_numeric_diagnostics -- --ignored --nocapture
```

The sweep solves camera distance from the measured center-pixel receiver-plane view depth, rather
than assuming that camera-to-target distance is identical to the actual pixel depth. For every
boundary it samples before the blend, at blend start, at 25/50/75/95%, immediately before SplitFar,
and immediately after SplitFar. The test asserts that the final sample selects the next cascade in
both the CPU reference and GPU CascadeIndex output.

Outputs are written to `target/csm_transition_diagnostics/`: four 24-tile contact sheets, twelve
full-resolution midpoint captures (four modes per boundary), `numeric_trace.csv`, and a README with
per-boundary measurements. `ShadowVisibilityComponents` exposes current PCF in R, next PCF in G,
and final blended visibility in B. The test checks the mix equation for every eligible overlap
pixel within 3/255 8-bit quantization tolerance, verifies that the grayscale `ShadowVisibility`
mode matches the component output, and requires a per-boundary probe with a meaningful current/next
visibility difference. This proves the sampled blend operation rather than merely observing a
smooth weight. It remains a deterministic depth sweep, not a continuous temporal camera recording
or proof of texel stability under arbitrary motion; review the images and CSV before changing shadow
splits, bias, PCF, or resolution.


## CSM texel-snapping and camera-stability diagnostics

The runtime cascade fit uses a rotation-invariant sphere, a shared light-space basis, and a world-space
texel grid derived from the actual 1024² shadow allocation. The fitted sphere radius is quantized to
the nearest 1/16 world-unit step with a 1/32 world-unit conservative guard before deriving texel
size; the existing 1.5-texel projection guard is then added. The same quantized radius drives both
the depth padding and the conservative light-space Z half-extent. The Z interval is centered on the
fitted sphere center projected onto the light direction, with the quantized sphere radius as its
half-extent. Because the sphere encloses every cascade corner, this remains conservative while
avoiding independent min/max corner quantization boundaries that could jump by one depth quantum
under tiny f32 reconstruction noise. This can use a somewhat wider Z interval than a tight corner
min/max fit, trading a small amount of depth precision for stable coverage. Cascade splits,
transition width, bias, PCF, and map resolution remain unchanged.

CPU regressions now cover all four runtime cascades:

- The complete shadow matrix must remain unchanged within the test's strict matrix tolerance while
  light-space camera translation stays inside the same texel cell. The test aligns each camera pose
  iteratively with a damped correction against the measured runtime center, then records the actual
  f32 cell fractions as its baseline. A bounded residual of 0.02 texel fraction is allowed; all later
  probes are relative to the measured baseline. The quantized depth span and padding must remain
  identical; the light-space depth center may differ only by the tiny f32 error caught by the matrix
  comparison.
- In-cell probes target 0.25 and 0.42 texel fractions, leaving at least 0.08 texel before the
  half-texel snap boundary. Crossing to 0.65 texel must move a fixed world point by exactly one
  shadow-map texel.
- Small camera yaw and pitch rotations that remain inside the current snap cell must preserve the
  snapped XY origin and texel scale.
- The far cascade must continue to cover its overlapped slice through the camera far plane under
  several yaw/pitch orientations.

The GPU regression renders each of the four `Depth32Float` array layers to a 1024² depth preview.
For each layer it compares a base frame with a second camera pose moved from 0.1 to 0.49 texel in
light-space X/Y; the preview must remain pixel-identical (allowing at most one 8-bit level). A pose
at 0.51 texel in X must move the fitted depth image by the one integer pixel predicted by the matrix.
This is a deterministic sub-texel capture, not a continuous-time camera recording; it does not prove
arbitrary rotation stability, animated-object stability, or frame-time performance.

Run the CPU tests with:

```powershell
cargo test --locked -p ribbon_plugin_render3d directional_shadow_depth_range_is_conservative_and_stable_under_small_f32_noise
cargo test --locked -p ribbon_plugin_render3d directional_shadow_depth_padding_uses_the_same_quantized_radius_as_xy_extent
cargo test --locked -p ribbon_plugin_render3d directional_shadow_extent_keeps_texel_scale_stable_under_small_radius_noise
cargo test --locked -p ribbon_plugin_render3d every_cascade_shadow_matrix_is_stable_inside_its_sub_texel_cell
cargo test --locked -p ribbon_plugin_render3d cascade_texel_grid_remains_stable_for_small_camera_yaw_and_pitch
cargo test --locked -p ribbon_plugin_render3d far_cascade_covers_far_plane_under_camera_rotation
```

Run the actual GPU depth-layer sweep on the target adapter with:

```powershell
cargo test --locked -p ribbon_plugin_render3d directional_cascade_depth_layers_are_stable_under_sub_texel_camera_motion -- --ignored --nocapture
```

The GPU test writes twelve images and `texel_stability.csv` under
`target/csm_texel_stability/`. Compare each `cascade_N_base.png` against
`cascade_N_within_cell.png` pixel-for-pixel; compare `cascade_N_after_one_texel.png` against the
base after the integer `(dx, dy)` shift recorded in the CSV. Camera rotation is covered by CPU
matrix tests here; continuous GPU camera rotation, sub-texel video capture, and GPU timing remain
future validation work.
