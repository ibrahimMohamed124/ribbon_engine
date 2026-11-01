# Horror Slice — visual baseline and material pass

Date: 2026-10-20

> This report records the sequence of visual tuning passes. Its earlier sections describe
> intermediate states and are not a description of the current texture bundle. The current
> manifest records all nine downloaded 1K Poly Haven maps embedded in the GLBs; the procedural
> `PLACEHOLDER` PNGs are retained as explicit fallback inputs. The target Windows visual review
> is still pending.

## Scope of this pass

`examples/horror_slice` now loads one RON-authored room scene on the native GPU surface using `GpuWindowPlugin` + `Render3dPlugin`; it does not use STUDS or the offscreen/readback bridge. The room data names the player spawn, closed exit door, key, lantern/save-point prop, monster spawn/patrol waypoints, and room collision boxes. The render pass has an eye-level camera, a slightly lowered/right-biased initial flashlight spot with a perspective shadow map, low ambient fill, and restrained green-gray exponential fog. The native renderer's reusable lighting pass includes one shadow-casting spotlight, up to eight non-shadowed point lights, PCF filtering, slope-scaled raster bias, normal/AO/emissive texture sampling and bright-pass bloom before tone mapping. Spot-shadow depth comparison applies a normal/light-angle-dependent bias to reduce acne (this is shader comparison bias, not a `DepthBiasState` on the raster pipeline).

A visual reference generated for this example is checked in at `examples/horror_slice/assets/reference/horror_room_concept_reference.png`. It is a design reference only; the game does not load it at runtime.

## External texture sources and procedural fallback

Official Poly Haven CC0 materials are selected for the target look:

- [Concrete Wall 003](https://polyhaven.com/a/concrete_wall_003) — worn painted plaster/concrete, authored by Dimitrios Savva (photography) and Rico Cilliers (processing).
- [Concrete Floor Damaged 01](https://polyhaven.com/a/concrete_floor_damaged_01) — cracked and scuffed concrete, authored by Rob Tuytel.
- [Rusty Metal 02](https://polyhaven.com/a/rusty_metal_02) — worn rusted metal, authored by Rob Tuytel.

All three official asset pages indicate CC0. The nine 1K binary maps have since been downloaded and are embedded in the bundled GLBs; source hashes and attribution are recorded in `assets/asset_manifest.json`. Deterministic procedural maps remain separately marked `PLACEHOLDER`, and no generated map is represented as a Poly Haven photograph. To refresh the source maps and regenerate the GLBs, run:

```powershell
python tools/fetch_horror_textures.py
python tools/make_test_models.py
```

The fetch script downloads 1K diffuse, AO/roughness/metalness, and OpenGL-normal JPEGs via the official Poly Haven file API. The generator embeds them in self-contained GLBs and refreshes the manifest/credits, recording downloaded-file SHA-256 hashes. It does not change or claim the license of procedural fallback files.

## Windows follow-up fixes and screenshot-driven tuning

The user-provided Windows screenshot showed a running window but almost no readable room geometry, no legible key/door/lantern, and only a faint monster silhouette. Reviewing the initial data showed the main reasons: placeholder albedo maps were already dark but were multiplied by another dark glTF base-color factor, the flashlight aimed too shallowly to place its pool near the key, and its range/intensity/cone were too conservative for the renderer's inverse-square attenuation. This update brightens the deterministic placeholder albedos (still explicitly `PLACEHOLDER`), makes the main wall/floor material factors near-neutral so downloaded albedo maps are not double-tinted, lowers and broadens the flashlight aim, reduces fog density, modestly lifts ambient visibility, moves the lantern into the left foreground and centers the key near the flashlight pool. This is a reasoned visual tuning pass; it is not claimed visually verified until a new Windows GPU screenshot is reviewed.

The first local launch after the initial visual pass exposed two integration issues:

- `room.ron` used list syntax (`[...]`) for Rust fixed-size `[f32; 3]` fields. RON/Serde represents fixed-size arrays as tuples, so every fixed-size vector is now written as `(...)`; `Vec` collections such as `entities`, `static_colliders`, and `monster_patrol_waypoints` remain lists using `[...]`.
- `plugins/render3d/src/renderer.rs` had an empty line between the spotlight render doc comment and `MeshRenderer::render`, which Clippy rejects under `-D warnings`. The empty line has been removed.

These are source/data corrections based on the user-provided Windows diagnostics. The Rust validation matrix must be rerun on Windows after applying this fix; no success is claimed here for that rerun.

## Follow-up: user screenshot shows floor-only beam (2026-10-20)

The next Windows screenshot confirmed that the beam was still too steep: the floor was strongly mottled while the side/back walls and door disappeared into darkness. The prior settings used a flashlight vertical direction component of `-0.43`; it is now `-0.22`, with the view pitch reduced to `-0.18` and the starting yaw moved to `0.32` so the right-wall door is more central without losing the left-side monster silhouette. The inner/outer cone is now 27°/42°. Fog density was lowered to `0.008`, ambient fill raised to `0.020`, and the placeholder wall/floor/metal albedo factors were lifted to avoid multiplying two very dark colors. The floor material is rougher. The generated normal-map placeholder now uses near-flat normals with subtle perturbations instead of high-frequency, large-amplitude hash noise; this specifically targets the speckled floor visible in the screenshot. Brass material metallic is reduced so it remains legible without environment lighting.

This is a targeted, screenshot-driven correction, not a visual acceptance claim. The screenshot after these changes still needs to be produced from the Windows GPU build.

## Files changed in this pass

- `examples/horror_slice/src/main.rs`, `assets/room.ron`, embedded placeholder GLBs, placeholder maps, manifest, credits and README.
- `tools/horror_room_assets.py`, `tools/make_test_models.py`, `tools/fetch_horror_textures.py`.
- `plugins/render3d/src/components.rs`, `extract.rs`, `renderer.rs`, `model.rs`, `post.rs`, `lib.rs`, `offscreen.rs`, and WGSL mesh/shadow/bloom/tone-map shaders.
- `plugins/gltf/src/lib.rs` (normal, occlusion and emissive material map extraction).
- `tools/allowed_deps.toml`, `../architecture/ARCHITECTURE.md`, `../guides/PROJECT_OVERVIEW.md`, `docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md` and this report.

No code was added under the legacy `engine/` directory and no Engine crate depends on the example or a game. STUDS golden constants and golden expectations were not altered by this pass.

## Verification status

### Static/file checks run in this generation environment

- `python tools/make_test_models.py` — passed; model files, brighter explicitly marked placeholder maps, and material manifest regenerated from deterministic geometry and the available local maps.
- Python TOML parsing for `Cargo.toml`, `examples/horror_slice/Cargo.toml`, and `plugins/render3d/Cargo.toml` — passed.
- Manifest file existence and SHA-256 checks — passed for the declared placeholder maps, generated GLBs and metadata records.
- Workspace membership check for `examples/horror_slice` — passed.
- The repository's `cargo`/`rustc`/`rustfmt` binaries are unavailable in this generation environment, and external host DNS failed. Rust formatting, Clippy, workspace tests, WGSL runtime compilation on the target adapter, and interactive presentation have therefore **not been run here**.

### Requires verification on Windows GPU

Run the workspace check matrix after applying this archive to the real checkout:

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\validate_horror_slice_visual_pass.ps1
```

The script sets `CARGO_INCREMENTAL=0`, runs the commands serially to reduce Windows memory pressure,
and ends by launching the interactive window. Its equivalent individual commands are:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -j 1
cargo test --workspace --no-default-features -j 1
python tools/check_deps.py
cargo run -p horror_slice
```

The new headless spotlight corridor test attempts to save `target/spotlight_corridor.png` and prints its path on success; when no GPU adapter/device is available it prints a clear skip message and returns. Its pixel checks establish that the rendered frame contains lit and dark regions; they do not by themselves prove that the exact interactive composition matches the reference image. Manually confirm the window, room orientation, wall/floor mapping, key, door, lantern, monster silhouette, spot cone, shadows, shadow acne/peter-panning and a short stability run before treating visual acceptance as complete.

## Deliberately not implemented in this pass

The monster is a static procedural silhouette (`monster_placeholder.glb`), not a skeletal-animated creature. Player movement and camera look are now implemented, and static room/prop AABBs constrain walking. Patrol waypoints do not drive AI yet. Door/key interactions, inventory, spatial sound, save/load and gameplay state are not implemented. These belong to later explicitly gated phases. This pass does not claim a complete playable horror vertical slice.


## Follow-up: interactive first-person camera and texture pass (2026-10-20)

The latest Windows screenshot confirmed that the room mesh assets were present but high-frequency
placeholder normal variation made the floor and wall highlights look like white pixel chips. This
pass uses 256x256 procedural placeholder maps with lower albedo contrast, subtle low-frequency
normal perturbations, and higher roughness. They remain `PLACEHOLDER` and are not presented as
Poly Haven downloads. The startup camera now faces between the key/door and monster; its spotlight
is updated from the camera's pose each frame rather than being a static world-space light, and the
view uses a lower, wider beam with less fog. A rear wall is generated and included in `room.ron` so
the room is enclosed during backtracking.

Interactive controls are now implemented in `horror_slice`: WASD walking, mouse look, Shift sprint,
Escape to release the cursor, and left-click to recapture it. Player movement uses the authored
static wall/floor/ceiling boxes plus the key, lantern, door and monster prop boxes. Reusable capsule
versus AABB collision and axis-separated wall sliding live in `ribbon_plugin_physics`; input mapping and the
camera/light-follow system remain example-owned. The camera eye stays at authored height; jumping,
gravity, dynamic rigid bodies, door interaction and AI are not implemented yet.

### Validation status for this follow-up

- `python tools/make_test_models.py` — passed during asset regeneration; placeholder maps,
  embedded GLBs, manifest hashes, and credits were regenerated from deterministic local code.
- TOML allowlist and dependency declaration were updated to permit `horror_slice` to depend on
  `ribbon_plugin_physics`.
- The Rust toolchain (`cargo`, `rustc`, `rustfmt`) is not installed in this generation environment,
  so `cargo fmt`, Clippy, Rust tests, and an interactive GPU run after this pass have not been run.
- Needs manual Windows GPU review: room readability, flashlight shadows, mouse capture/release,
  WASD bounds, corner sliding, and camera orientation after looking all the way around.


## Follow-up: reduce specular flecks and improve first-person framing (2026-10-20)

The last user screenshot showed the room was readable at a glance, but flashlight reflections on the floor and ceiling still looked like high-frequency white chips, and the lamp was close to the left edge while the locked exit door was nearly black. The fallback map generator now uses lower-contrast, lower-frequency albedo variation and near-flat normal maps; the floor, concrete and metal materials use higher roughness to keep the flashlight diffuse instead of glittering. The initial yaw was adjusted to place the door to the right, the key nearer the forward beam, and the save-point lantern further inside the left side of the frame. The spot stays camera-mounted and retains a strong cone; a slight ambient increase is only a fill and does not remove the dark corners.

Player controls in `horror_slice`: WASD walk, mouse look, Shift sprint, Escape release the cursor, left-click capture it again. The view is fixed at standing eye height and cannot jump, climb, open the door, pick up the key, or animate/chase the monster. The reusable movement helper in `ribbon_plugin_physics` resolves a vertical capsule against static axis-aligned boxes on XZ, stopping and sliding at walls; it is intentionally not a general rigid-body or ray-query system. The prior Rapier decision is revised in `docs/DECISIONS.md` to state why this smallest controller is the current fit and when to reconsider Rapier.

`python tools/make_test_models.py` has been rerun after the material changes; GLB model headers and asset-manifest SHA-256s were checked in the working container. Rust tools are unavailable in this environment, so formatting, clippy, workspace tests, and the Windows GPU walk-through remain unverified until run in the user's Windows checkout.

## Shadow regression follow-up

The first Windows GPU run of `spotlight_only_corridor_writes_a_headless_png` found no image difference between the same scene with the spotlight shadow map enabled and disabled. The spotlight used the same bias values as the directional shadow map even though perspective shadow depth is nonlinear and its useful depth differences differ across the light range. The spotlight now has a separately tuned, smaller minimum and slope-scaled bias; the directional shadow bias remains unchanged. The test writes `target/spotlight_corridor.png` before assertions so a failing render still provides a diagnostic artifact. This change is not considered verified until the test passes and the generated image is inspected.
