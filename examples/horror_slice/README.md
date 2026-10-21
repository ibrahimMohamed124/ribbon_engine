# Horror Slice (single-room visual slice)

This is the native-surface horror acceptance example for Voxelworks. It uses
`GpuWindowPlugin` + `Render3dPlugin`; it does not invoke STUDS or the offscreen/readback
bridge.

## Visual target

![Generated room concept reference](assets/reference/horror_room_concept_reference.png)

This generated concept image is bundled as a layout/lighting target only; it is not loaded or drawn at runtime. The room implementation is intentionally approximated using native mesh rendering and the texture/model assets below. Exact visual parity still requires a fresh Windows GPU screenshot review after these lighting/material corrections.

## Current visual content and controls

- One enclosed, hand-authored concrete room from `assets/room.ron` with cracked/damp wall
  and floor materials, a low ceiling, and one closed metal exit door with visible lock hardware.
- Native first-person camera with WASD movement, mouse look, Shift sprint, mouse capture,
  Escape to release the pointer, and left-click to recapture it. The camera moves at a fixed
  eye height and collides against the room's authored AABB colliders, including wall sliding;
  collision math is a reusable `plugin_physics::slide_capsule_xz` helper.
- Flashlight spotlight and perspective shadow map follow the camera's position and orientation
  every frame. Point lighting/emissive lantern keeps the save point readable, while distance/
  height fog and low fill light preserve a dark horror mood.
- Brass-colored key mesh, save-point lantern/pedestal, and a tall static monster silhouette
  at the far end of the room.
- Embedded glTF models include the nine downloaded Poly Haven CC0 diffuse, AO/roughness/metallic,
  and OpenGL-normal maps. Deterministic procedural PBR maps remain available as fallbacks and are
  marked `PLACEHOLDER` in `assets/asset_manifest.json`.

The camera now moves and is constrained by the static room/prop colliders. The door/key are not
interactive yet; the monster is not skeletal-animated or AI-controlled; positional audio and save
serialization remain future phases and are not claimed as implemented.

## Run

```powershell
cargo run -p horror_slice
```

A bounded smoke run is available:

```powershell
cargo run -p horror_slice -- --frames 120
```

## Texture provenance and replacement

The bundled GLBs currently contain all nine downloaded 1K source maps, whose hashes and
attribution are recorded in the manifest. Procedural `PLACEHOLDER` maps are also kept in the
repository as deterministic fallback assets; they are not represented as Poly Haven photographs.
Recommended CC0 replacements are Poly Haven's [Concrete Wall 003](https://polyhaven.com/a/concrete_wall_003),
[Concrete Floor Damaged 01](https://polyhaven.com/a/concrete_floor_damaged_01), and
[Rusty Metal 02](https://polyhaven.com/a/rusty_metal_02); creator/licence credits are in
[`assets/CREDITS.md`](assets/CREDITS.md).

To refresh the original 1K diffuse, AO/roughness/metallic, and OpenGL-normal maps and regenerate
the embedded GLBs:

```powershell
python tools/fetch_horror_textures.py
python tools/make_test_models.py
python tools/check_deps.py
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -j 1
cargo test --workspace --no-default-features -j 1
cargo run -p horror_slice
```

`plugin_render3d` also includes a headless GPU test that saves
`target/spotlight_corridor.png` and prints the path when a suitable adapter exists;
if no adapter is available, it prints a clear skip message. That screenshot is a test
fixture, not proof that the interactive Windows composition matches the concept art.
