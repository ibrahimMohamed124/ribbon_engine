# STUDS realism upgrade plan

## Audit and baseline (2026-10-16)

The workspace has a dependency-light scene model, CPU and wgpu renderers, and a deterministic golden suite. Legacy Classic and Moody pixels remain protected. Realistic data is carried as plain RGBA mip chains and material texture IDs; asset decode/processing stays outside `engine_scene`. Built-in entities are still boxes (`WARDEN` and `PAGE`).

Baseline on this Windows machine, release build, corridor scene, 320x180, 30 frames/style (`cargo run --release --locked -p render_cpu --example render_bench -- 30`): Classic 0.852 ms/frame; Moody 1.320 ms/frame. This is CPU reference time, not a 1080p GPU measurement.

## Concrete design

1. Keep `engine_core` dependency-free and keep all scene additions as owned/plain data. Put image/glTF decoding, fetching and sprite baking in tooling or `studs`; never add a decoder to `engine_core` or `engine_scene`.
2. Extend `engine_scene` with byte-based RGBA/PBR texture mip levels and explicit stable texture IDs. Add a new Realistic style while leaving Classic and Moody code paths intact. The CPU sampler will use wrapped bilinear/trilinear sampling; the GPU receives the same mip data through a texture array/atlas. Add deterministic tests around sampling and fallback.
3. Add a sprite entity descriptor and renderer-side sprite frame data. A software bake tool will convert a licensed animated GLB into view-angle/frame atlases; CPU and GPU composite sprites after world depth is known, using per-pixel depth and alpha cutout. Keep the box warden as the asset-missing fallback.
4. Extend Realistic-only shading with color lighting, roughness/specular, fog, filmic grade and accessibility-controlled effects. Keep legacy shading branches unchanged. Quality levels gate expensive effects; Low skips shadow rays and costly post effects.
5. Add manifest-driven fetch/processing and generated credits. Keep source downloads outside version control; ship only compact derived assets and verify every manifest hash. Asset loading failures must report a warning and preserve procedural/box fallbacks.
6. Integrate settings and game material assignments after both renderer paths accept the data. Add deterministic tests and document CPU/GPU comparison using `studs/examples/gpu_preview.rs` if CI has no headless adapter.

## Asset choices and licensing

- Concrete wall: Poly Haven, “Concrete Wall 004”, `https://polyhaven.com/a/concrete_wall_004`, photography by Charlotte Baglioni and processing by Dario Barresi, CC0. Its page advertises diffuse, normal, roughness and AO maps and a 1K download tier. Use as the primary damaged wall set.
- Concrete brick/plaster variant: Poly Haven, “Concrete Wall 002”, `https://polyhaven.com/a/t_concrete_wall_002`, photography by Dimitrios Savva and processing by Rico Cilliers, CC0. Use for deterministic secondary wall cells.
- Floor: Poly Haven, “Concrete Floor Worn 001”, `https://polyhaven.com/a/concrete_floor_worn_001`, photography by Dimitrios Savva and processing by Rico Cilliers, CC0. Use for the worn floor.
- Ceiling: Poly Haven, “Ceiling Interior”, `https://polyhaven.com/a/ceiling_interior`, CC0. The page advertises diffuse, normal, roughness and AO maps; the credits generator records the listed creators.
- Monster candidates: Quaternius “Ultimate Monsters” is CC0, glTF-capable, and includes 50 animated creatures, but the available pack is stylized rather than the requested realistic, emaciated humanoid. OpenGameArt “Nightmare Monsters” by Teh_Bucket is CC0 and has four rigged, textured characters, but is FBX-only and does not list animation clips; the asset page credits textures to João Paulo via 3dtextures.me and says the Sonask concept artist approved public-domain release. Neither candidate was downloaded or visually validated in this environment. Do not substitute a protected character; keep the legacy box fallback until a model is verified and converted.

## Work completed and remaining

- Phase 0: repository audit, the original CPU baseline, design, candidate review, and risk notes are recorded here.
- Phase 1: four CC0 Poly Haven PBR sets are downloaded and processed: damaged concrete wall, weathered concrete wall, worn concrete floor, and interior ceiling. The manifest records source, creator, license, usage, local path, and SHA-256. `tools/fetch_assets.py` downloads/caches, converts to 256×256 raw RGBA8 sidecars and ZIPs, generates credits, and validates required metadata, license allowlist, hashes, archive contents, and a 40 MiB shipping cap. Source files are ignored by git. No suitable decal set was selected.
- Phase 2: `engine_scene` has plain RGBA mip chains, stable texture IDs and material PBR sets. CPU and WGSL GPU paths sample albedo, tangent normal, roughness, and AO. The Realistic path is separate; Classic and Moody pixel goldens were not changed. No per-cell random texture rotation/flip/tint has been added yet.
- Phase 3: Realistic has a horror color grade, mapped normals, roughness-driven flashlight response, and AO. Low/Medium presets skip PBR samples; High/Realistic enable them. Existing fog/film effects remain available in the renderer. Flashlight DDA shadows, inverse-square/cone lighting, battery degradation, height dust, bloom, chromatic aberration, breathing, monster-driven distortion, and meaningful reduce-flashing behavior remain outstanding.
- Phase 4: incomplete. The monster remains box geometry; there is no licensed animated sprite atlas, bake tool, depth-tested billboard compositor, or animation-state binding. The reviewed Quaternius pack is CC0 and glTF-capable but stylized. OpenGameArt “Nightmare Monsters” is CC0 and rigged but FBX-only with no animation clips listed. Neither was integrated because each misses key requirements. Existing enemy audio remains unchanged.
- Phase 5: visual style and quality choices are exposed and persisted; default settings now select Realistic. PBR maps are loaded once into the material table; missing or invalid assets warn and use the procedural fallback. UI/menu layers are unchanged. Automatic CPU fallback quality selection is not connected to presenter selection yet.
- Phase 6: Realistic deterministic CPU golden, asset-loading tests, manifest CI validation, architecture/readme updates, and GPU preview wiring are in place. Exact CPU/GPU parity and 1080p integrated-GPU performance are not established. A manual GPU preview run validated the shader on this host. The CPU Realistic sample render measured 7.391 ms/frame at 320×180 on this machine; this is not a 1080p or GPU measurement.

Asset page references reviewed: Poly Haven pages list the named map channels and 1K tiers; Poly Haven assets are CC0. Quaternius lists its Universal Animation Library as CC0.

## Risks and verification

- Existing CPU/GPU shaders are separate implementations; the legacy golden invariant makes branching essential. New visual tests must use a separate Realistic fixture/style.
- Sprite atlases multiply texture storage by angles and frames. Cap output resolution and retain compact shipping mip levels under the 40 MB budget.
- Network access or upstream URLs may fail on a local machine. The fetch tool is repeatable and hash-checked; it never fabricates images.
- Integrated GPU performance and headless adapter support are environment-dependent. The existing CPU corridor baseline (Classic 0.852 ms, Moody 1.320 ms at 320×180) and the new game-scene benchmark (Realistic 7.391 ms at 320×180) are different fixtures and must not be compared as an apples-to-apples before/after. No 1080p GPU timing has been collected.
- Monster appearance and animation cannot be claimed complete until a redistribution-safe model is found and baked. Procedural boxes remain a supported fallback.
