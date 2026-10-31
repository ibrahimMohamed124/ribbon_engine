# Ribbon / ATHAR Physics Integration Audit

**Date:** 2026-10-29  
**Scope:** `plugins/physics` and `games/athar` only. This audit intentionally keeps the existing physics architecture and public compatibility API.

## Executive result

The intended gameplay path is now Rapier-backed: ATHAR creates a kinematic capsule for the player, submits one displacement intent per Ribbon fixed step, and resolves movement with Rapier's `KinematicCharacterController::move_shape`. ATHAR does not call the legacy `slide_capsule_xz` helper for player movement. `Aabb3` is still used as convenient authored bounds data for the village, but those bounds are converted into engine `Collider` entities and are not used to move the player.

This patch closes two concrete integration gaps found during the audit: scripted door movement previously queued a kinematic target that character shape-casts could not see until after the same step's query; and village tree colliders did not reflect their per-instance visual scale. It also includes a Rapier collider under the village ground and end-to-end backend/raycast regression coverage for the room and village.

## Findings by requested area

### 1. Player movement: Rapier vs. legacy helper

**Verified current path:**

- `games/athar/src/main.rs` registers `submit_character_movement` in `Stage::FixedUpdate` before adding `PhysicsPlugin`, then creates a player entity with `RigidBody::kinematic_position_based()`, `Collider::capsule_y(...)`, `CharacterController`, and `CharacterMovementIntent`.
- `games/athar/src/scripts/player.rs::submit_character_movement` converts WASD/Shift input and the engine fixed delta into a world-space X/Z intent. It does not call `slide_capsule_xz`.
- `plugins/physics/src/runtime.rs::PhysicsContext::apply_character_intents` consumes and clears that intent; `move_character` invokes Rapier's `KinematicCharacterController::move_shape` against the scene query pipeline.
- `games/athar/src/scripts/player.rs::sync_player_camera` follows the simulated capsule after the fixed steps.

**Legacy API status:** `slide_capsule_xz` and `Aabb3` remain public for compatibility. ATHAR only uses `Aabb3` to describe village bounds before converting them to Rapier colliders.

### 2. Environment, doors, interactables, and monster representations

- **Room shell/walls/floor:** the room-shell GLB is visual-only. `room.ron::static_colliders` is converted by `spawn_static_room_colliders` in `games/athar/src/main.rs` to fixed `Transform3D + Collider` entities. The room's floor and door-side wall segments are therefore in Rapier; there is no implicit triangle-mesh collision for the shell.
- **Door:** the rendered door and its collider are separate entities. `gameplay.rs::update_key_and_door_animation` updates the proxy transform with the door animation and despawns the proxy when the opening is clear. The proxy is now a fixed scripted obstacle, not a kinematic body: this makes its updated ECS pose and broad-phase AABB visible to the character query in the same pre-update/fixed-step cycle. ATHAR has no dynamic actor that relies on this door to impart kinematic push velocity.
- **Key:** the key model entity also has a sensor `Collider`. It is non-blocking for the character shape cast because that query excludes sensors. The current key pickup and prompt remain proximity-based (`gameplay.rs::choose_room_interaction`); they do not consume `SensorEntered` events.
- **Save point and monster placeholder:** their authored dimensions are represented by separate fixed cuboid proxies centered using the authored half-height. The monster is a *static placeholder*, not a moving monster body: `monster_spawn`/waypoints are present in the room data, but no monster AI/patrol simulation currently updates a collision body.
- **Village:** `VillageLayout::colliders` authors bounds for the 60 houses, hall, well, trees, ground, and world edges. `spawn_village_colliders` converts them into fixed Rapier collider entities, tracked in `AdventureState::village_entities` for cleanup. The village ground now has its own floor collider. Tree proxy footprint and height now follow each rendered tree's scale. Roads and lamps do not have separate collision volumes; the large ground plane supports walking on the roads.
- **Shape fidelity limitation:** room/village obstacles use primitive cuboids, not mesh-level collision. This is an intentional approximation, not verified triangle-mesh collision.

### 3. Creation, component changes/removal, and despawning

`plugins/physics/src/runtime.rs::PhysicsContext::reconcile` queries entities with `Collider`, validates their transform/shape/body/layers, creates backend objects when missing, and recreates entries when the body/collider/layer/scale/CCD descriptor changes. Entities that no longer have a valid desired descriptor are removed from the backend. `remove_entity` removes the Rapier body/collider handles, clears ownership, and emits stop/exit events for tracked overlaps. Removing `Collider` or despawning the ECS entity is reconciled during the next `PreUpdate`, not synchronously at the instant of the ECS mutation.

The broad-phase AABB for newly inserted colliders is now explicitly indexed during creation so an initial character shape-cast can see a wall in that same fixed step. External teleports for controlled characters update the current Rapier body pose and query AABB immediately so the same-step movement intent cannot overwrite the teleport.

### 4. Transform synchronization

- **Fixed body:** externally changed `Transform3D` pose is applied directly to Rapier, followed by broad-phase refresh.
- **Dynamic body:** Rapier owns the simulated world pose and `write_back_simulated_poses` writes it back into ECS transforms. An actual ECS pose edit/parent change is treated as a teleport; the prior ECS pose is not blindly reapplied every frame.
- **Kinematic velocity body:** simulation pose is written back, while explicit external pose changes are synchronized.
- **Kinematic position body:** external target changes use Rapier's next-kinematic pose, except a `CharacterController` body teleport uses immediate `set_position` because its shape query runs before `PhysicsPipeline::step`.
- **Parenting:** `reconcile` propagates `GlobalTransform3D` before validation. Parent chains with invalid/cyclic transforms or non-uniform ancestor scale are rejected. Dynamic write-back converts world pose to local space. Existing tests cover a parented dynamic body and rejection of non-uniform ancestor scale.

General engine limitation/risk: an externally animated *non-character* kinematic body uses a next-kinematic target. A character shape-cast that runs before `PhysicsPipeline::step` sees that body's previous simulated pose for that one query. ATHAR's scripted door now uses the fixed-proxy path to avoid this ordering issue. Future games that need dynamic actors pushed by a moving kinematic obstacle should add an explicit same-step query/kinematic-motion contract rather than silently changing kinematic velocity semantics.

### 5. Fixed-step lifecycle and collision/trigger events

`plugins/physics/src/plugin.rs` registers `PhysicsContext::reconcile` in `Stage::PreUpdate` and `PhysicsContext::step` in `Stage::FixedUpdate`. `crates/app/src/lib.rs::App::update` advances its existing `FixedTime` accumulator and runs `Stage::FixedUpdate` once for every due fixed step, setting `Time::delta` to the fixed delta for each iteration. The physics plugin does not maintain a second clock or accumulator. The plugin's fixed system calls `step` once per fixed-stage execution.

`flush_collision_events` maps Rapier handles back to Ribbon entity IDs and writes `CollisionStarted`, `CollisionStopped`, `SensorEntered`, and `SensorExited` into Ribbon's typed event storage. The physics tests exercise start/stop and sensor enter/exit delivery. **ATHAR currently has no event consumer for these physics events.** That is intentional for the key: gameplay interactions are currently distance-based; sensor events are exposed for future gameplay but do not drive the current pickup logic.

### 6. Filtering, raycasts, and coordinate spaces

`move_character` queries the current world-space Rapier scene, excludes the character's own rigid body and all sensor colliders, and applies its `CollisionLayers` interaction groups. `raycast` normalizes the supplied world-space direction, applies collision groups, optional sensor exclusion, and entity exclusion, then maps the hit handle back to the owning Ribbon entity; returned position/normal/distance are world-space values. Existing tests exercise raycast entity mapping, layer filtering, and sensor non-blocking behavior. ATHAR does not currently use raycasts for interaction targeting.

### 7. Legacy helper status

`Aabb3` and `slide_capsule_xz` are retained and exported for API compatibility. ATHAR's player movement no longer uses `slide_capsule_xz`; the village's `Aabb3` values are now only layout descriptors that become real `Collider` components. There is no reason to remove the compatibility API.

## Changes in this audit patch

1. Connected ATHAR's player intent/capsule to `PhysicsPlugin` and synchronized the camera from the simulated capsule.
2. Connected room static geometry, key sensor, door proxy, save point/monster placeholder proxies, and village obstacles to engine `Collider` components.
3. Added a village ground collider and scaled each tree collider to match the tree's visual instance scale.
4. Made the animated door a fixed scripted collider proxy so its current pose is queryable by the player in the same fixed step.
5. Refreshed broad-phase bounds for newly created colliders and relevant teleports; ignored sensors during character shape casts.
6. Added regression coverage for first-step colliders, moving fixed obstacles, character teleport, sensor non-blocking movement, actual room/village backend registration, and raycastability.

## Validation results available in this environment

| Check | Result |
|---|---|
| Static source guard checks (plugin registration/order, no legacy movement call in ATHAR player, proxy creation, regression test presence) | **PASS — 12 checks** |
| `python3 games/athar/tools/validate_horror_assets.py` | **PASS — 12 self-contained GLBs validated (24.59 MiB)** |
| `cargo fmt --all -- --check` | **Not run here** — this environment has no `cargo`/`rustfmt` executable |
| Targeted Rust tests / `cargo test --workspace --no-fail-fast` | **Not run here** — this environment has no Rust toolchain |
| Clippy / dependency-policy script | **Not rerun here** — this environment has no `cargo`; this patch does not change Cargo manifests or lockfiles |
| In-game visual gameplay test | **Not performed** — no claim of visual verification is made |

These results do not imply that the newly added Rust tests have compiled on this machine. Please run the commands below in the Windows project checkout to validate the final patch.

## Commands to run on the Windows checkout

```powershell
cargo fmt --all -- --check
cargo test -p ribbon_plugin_physics character_controller_sees_new_colliders_on_its_first_fixed_step
cargo test -p ribbon_plugin_physics moved_fixed_obstacle_is_visible_to_character_query_in_the_same_fixed_step
cargo test -p ribbon_plugin_physics character_controller_teleport_is_not_overwritten_by_same_step_intent
cargo test -p athar room_static_geometry_is_registered_in_rapier_and_raycastable
cargo test -p athar village_bounds_are_reconciled_into_rapier_and_ground_is_queryable
cargo test -p athar village_colliders_include_all_houses_trees_landmarks_and_bounds
cargo test --workspace --no-fail-fast
cargo clippy -p ribbon_plugin_physics --all-targets -- -D warnings
cargo clippy -p athar --all-targets -- -D warnings
py -3 tools/check_deps.py
```
