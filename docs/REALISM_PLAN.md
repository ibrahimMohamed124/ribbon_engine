# STUDS realism upgrade plan

## Audit and baseline (2026-10-16)

The workspace has a dependency-light scene model, a CPU reference renderer and a wgpu renderer. `engine_scene::Texture` currently stores only periodic grayscale values; `MaterialTable` describes block albedo rules and procedural texture kinds. CPU material sampling and shading live in `render_cpu`, while `render_gpu/src/world.wgsl` mirrors the voxel traversal and shading. Built-in entities are static boxes (`WARDEN` and `PAGE`), with entity intersection implemented independently by the two renderers. Settings currently expose Classic and Moody styles. The existing exact Classic and Moody goldens are protected and must not be updated.

Baseline on this Windows machine, release build, corridor scene, 320x180, 30 frames/style (`cargo run --release --locked -p render_cpu --example render_bench -- 30`): Classic 0.852 ms/frame; Moody 1.320 ms/frame. This is CPU reference time, not a 1080p GPU measurement.

## Concrete design

1. Keep `engine_core` dependency-free and keep all scene additions as owned/plain data. Put image/glTF decoding, fetching and sprite baking in tooling or `studs`; never add a decoder to `engine_core` or `engine_scene`.
2. Extend `engine_scene` with byte-based RGBA/PBR texture mip levels and explicit stable texture IDs. Add a new Realistic style while leaving Classic and Moody code paths intact. The CPU sampler will use wrapped bilinear/trilinear sampling; the GPU receives the same mip data through a texture array/atlas. Add deterministic tests around sampling and fallback.
3. Add a sprite entity descriptor and renderer-side sprite frame data. A software bake tool will convert a licensed animated GLB into view-angle/frame atlases; CPU and GPU composite sprites after world depth is known, using per-pixel depth and alpha cutout. Keep the box warden as the asset-missing fallback.
4. Extend Realistic-only shading with color lighting, roughness/specular, fog, filmic grade and accessibility-controlled effects. Keep legacy shading branches unchanged. Quality levels gate expensive effects; Low skips shadow rays and costly post effects.
5. Add manifest-driven fetch/processing and generated credits. Keep source downloads outside version control; ship only compact derived assets and verify every manifest hash. Asset loading failures must report a warning and preserve procedural/box fallbacks.
6. Integrate settings and game material assignments after both renderer paths accept the data. Add deterministic tests and document CPU/GPU comparison using `studs/examples/gpu_preview.rs` if CI has no headless adapter.

## Asset choices and licensing

- Concrete wall: Poly Haven, “Concrete Wall 004”, `https://polyhaven.com/a/concrete_wall_004`, author Rob Tuytel, CC0. Its page advertises diffuse, normal, roughness and AO maps and a 1K download tier. Use as the primary damaged wall set.
- Concrete brick/plaster variant: Poly Haven, “Concrete Wall 002”, `https://polyhaven.com/a/t_concrete_wall_002`, author Dimitrios Savva, CC0. Use for deterministic secondary wall cells.
- Floor: Poly Haven, “Concrete Floor Worn 001”, `https://polyhaven.com/a/concrete_floor_worn_001`, author Dimitrios Savva, CC0. Use for the cracked/worn floor.
- Ceiling: Poly Haven, “Ceiling Interior”, `https://polyhaven.com/a/ceiling_interior`, author Dimitrios Savva, CC0. Use only if its available maps meet the same PBR requirements; otherwise retain the procedural fallback and record the omission.
- Monster: no qualifying realistic, emaciated humanoid with clean redistribution terms was verified in this audit. Do not substitute an unrelated cartoon monster or protected character. Continue the box fallback until a suitable CC0/CC-BY asset is verified. Quaternius Universal Animation Library is CC0 and may provide compatible animation clips, but is not itself the requested monster model.

Asset page references reviewed: Poly Haven pages list the named map channels and 1K tiers; Poly Haven assets are CC0. Quaternius lists its Universal Animation Library as CC0.

## Risks and verification

- Existing CPU/GPU shaders are separate implementations; the legacy golden invariant makes branching essential. New visual tests must use a separate Realistic fixture/style.
- Sprite atlases multiply texture storage by angles and frames. Cap output resolution and retain compact shipping mip levels under the 40 MB budget.
- Network access or upstream URLs may fail on a local machine. The fetch tool must be repeatable, hash-checked, and explain failures without making fake asset files.
- Integrated GPU performance and headless adapter support are environment-dependent. Measure CPU baseline before/after with the same benchmark; report GPU measurements only when collected on an actual adapter. Use Low quality on CPU fallback.
- Monster appearance and animation cannot be claimed complete until a redistribution-safe model is found and baked. Procedural boxes remain a supported fallback.
