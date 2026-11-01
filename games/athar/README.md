# Horror Slice — The Lost Key / Wet Creek Village

This is a playable first-person horror slice using Ribbon's native `GpuWindowPlugin` + `Render3dPlugin` path. It does not call STUDS and does not use the retired legacy GPU voxel raycaster.

## Play the sequence

```powershell
cargo run --release -p athar
```

1. Explore the dark room and find the brass key lying on the floor near the lantern/save point.
2. Walk close to the key and press **E**. The key disappears from the floor and the HUD confirms it is carried.
3. Walk to the metal exit door and press **E**. Without the key, it reports that the door is locked. With the key, the lock opens and the door swings on its hinge.
4. Walk through the doorway. The room is unloaded and the exterior village scene is spawned.
5. Explore **Wet Creek**, a roughly 260-by-260-unit village with 60 houses, a landmark hall, tree-lined edges, cobbled streets, lamps, a stone well, distance fog, and 220 animated rain streaks that follow the player.

## Controls

- **W / A / S / D** — walk and strafe.
- **Mouse** — look around while the pointer is captured.
- **Shift** — sprint.
- **E** — pick up the key or unlock the door when within reach.
- **Escape** — release the pointer; left-click the window to capture it again.

The in-window HUD shows the current objective, interaction prompts, and the village's scale/weather status. Walking is driven by the engine `ribbon_plugin_physics::PhysicsPlugin`: a kinematic capsule receives one movement intent per `FixedUpdate`, while Rapier resolves it against authored room and village collider entities. The key has a sensor collider, the scripted door uses a fixed collider proxy whose pose is reconciled before character queries, and the save-point/monster placeholder and village obstacles use engine-owned fixed colliders. The legacy `Aabb3` / `slide_capsule_xz` helpers remain available to older callers but do not resolve ATHAR movement. Quest progression, door animation, and village placement are scripts in `src/scripts/`. The engine-owned `ribbon_plugin_weather` plugin manages rain particles; ATHAR only enables rain when entering the village. Mesh rendering, glTF loading, frame presentation, and component storage stay in engine crates.

## Village and weather implementation

The outdoor scene is deterministic and uses shared glTF asset handles for repeated house/tree/lamp instances. The room's static Rapier collider entities are despawned on transition and replaced with a village collider set. The village ground now has a fixed Rapier collider; the main roads remain open, each house has a collision volume, trees have trunk collision, and world-edge colliders prevent walking off the ground plane. Rain streaks are lightweight built-in cube meshes with deterministic randomized offsets and fall speeds. The engine weather plugin recycles them through the standard mesh renderer without adding a separate particle renderer.

The scene uses the engine's native-surface mesh renderer, PBR glTF model support, directional lighting, spotlight, point lights, distance/height fog, and a reusable `WindowTextOverlayPass`. The text overlay is an engine-level rendering utility in `ribbon_render`; quest rules and all Horror Slice-specific content remain in this game.

## Textures and attribution

The shipped village GLBs are self-contained. They currently embed the **already-downloaded CC0 Poly Haven texture maps** used by the room scene as fallbacks, so the village works offline. Dedicated road/roof/timber maps are listed for optional replacement in [`assets/CREDITS.md`](assets/CREDITS.md):

- [Cobblestone Floor 001](https://polyhaven.com/a/cobblestone_floor_001) — Rob Tuytel, CC0 1.0.
- [Roof Tiles](https://polyhaven.com/a/roof_tiles) — Stephan Seeliger, CC0 1.0.
- [Wood Planks](https://polyhaven.com/a/wood_planks) — Amal Kumar, CC0 1.0.

The current packaged models report `dedicated_source_maps_present: 0/9` for the three village-specific map sets; they do not pretend those optional dedicated maps were downloaded in the offline build environment. The GLBs currently embed the existing downloaded CC0 maps, so the game has real texture data rather than depending on network access at run time. When online, fetch official Poly Haven URLs and rebuild the self-contained GLBs:

```powershell
python games/athar/tools/fetch_horror_textures.py
python games/athar/tools/horror_room_assets.py; python games/athar/tools/horror_village_assets.py
python games/athar/tools/validate_horror_assets.py
```

The fetcher checks downloaded bytes and only writes the set after all requested maps validate. If the API/network is unavailable, keep the packaged models; they already contain the downloaded CC0 fallback maps. Procedural maps are explicitly marked `PLACEHOLDER` and are not credited as external photographs. See [`assets/asset_manifest.json`](assets/asset_manifest.json) and [`assets/CREDITS.md`](assets/CREDITS.md).

## Validation

```powershell
cargo fmt --all -- --check
cargo test --locked -p athar
cargo clippy --locked -p athar --all-targets -- -D warnings
cargo run --release -p athar
python games/athar/tools/validate_horror_assets.py
```

For a bounded smoke run:

```powershell
cargo run --release -p athar -- --frames 120
```

For a render profile, preserve `target/profiles` and use a finite run:

```powershell
cargo run --release -p athar -- --frames 1800 --profile-csv target/profiles/horror-slice.csv
python tools/perf/render_profile_report.py target/profiles/horror-slice.csv --output target/profiles/horror-slice.md
```

## Scope and remaining limitations

- The monster is still a static silhouette; skeletal animation, monster AI, combat, and scripted scares are not part of this slice.
- The key/door quest currently transitions one-way into the village; there is no return door or persistence/save serialization.
- Rain is a visual weather effect; raindrop collision, wet-surface simulation, thunder audio, and dynamic puddles are not implemented.
- The village layout is a deliberate hand-authored grid, not procedural terrain streaming. All village assets are loaded as a finite scene.
- `SurfaceShader` callbacks remain CPU-only; this game's 3D scene uses the renderer's glTF PBR material data.
