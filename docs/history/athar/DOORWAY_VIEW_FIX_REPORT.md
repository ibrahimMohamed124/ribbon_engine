# ATHAR doorway village visibility fix

## Verified source-level cause

The asynchronous CPU load and renderer GPU warmup made the village models resident, but did not create
`ModelInstance` entities for the village before the player crossed the doorway. `extract_scene_3d`
extracts visible model placements from ECS entities, so a warm cache alone gave the renderer no
village geometry to draw while the room was still active. In addition, ATHAR's room clear color was
nearly black and its room ambient light/fog settings were intentionally very dark. Those settings made
the empty doorway look like a black void.

## Changes in the overlay

- `games/athar/src/scripts/village.rs`: split `spawn_village_visuals` from `spawn_village_physics`.
  The preview stages 128 model placements plus eight point lights without creating colliders. The
  existing `spawn_village` helper remains available and composes both paths. The ground and road
  visuals are shifted by 2.0 cm and 1.4 cm down, respectively, to avoid coplanar overlap with the
  room's y=0 floor; the road surface remains 6 mm above the village ground.
- `games/athar/src/scripts/gameplay.rs`: after readiness is confirmed, stages the village view once
  the door reaches 70% open, before the 86% collision-clear threshold. The door-view clear color is
  changed to an outdoor slate rather than black. Once preview model entities exist, outdoor sunlight,
  fog, and the 260 m camera far plane are activated. On commit the already-staged entities are reused
  and only the 111 Rapier collider entities are added; the model instances are not duplicated.
  Failure/retry clears any stale preview entities.
- `games/athar/src/scripts/mod.rs`: tracks whether preview lighting has been activated to avoid
  repeatedly rewriting the environment.
- `docs/architecture/ATHAR_TRANSITION_WARMUP.md` and `tools/validate_athar_transition_warmup.py`:
  describe and check the new staging/handoff contract.

## Validation actually performed here

- `python3 tools/validate_athar_transition_warmup.py`: **45/45 static checks passed**.
- GLB container-header check: **12/12 packaged GLBs passed**, total compressed source files 24.59 MiB.
- Python syntax compilation for the validation/profiling scripts succeeded.
- ZIP integrity check passed and all expected overlay paths are present.

## Not run / needs confirmation on Windows

This environment does not have `cargo`, `rustc`, or `rustfmt`, so I did not run Rust compilation,
unit tests, workspace tests, or Clippy. I also did not launch the game or visually inspect a GPU frame.
Run the commands in `ATHAR_DOORWAY_VIEW_OVERLAY_README.txt`. In-game, open the key-unlocked door and
look through it before crossing: the staged village should already be visible against an outdoor
backdrop. Then cross to confirm physics proxies activate without duplicate models or progressive
GPU uploads. If the view is still black, capture a frame/render profile so the remaining possibility
of room-shell occlusion or render/surface output failure can be diagnosed separately.
