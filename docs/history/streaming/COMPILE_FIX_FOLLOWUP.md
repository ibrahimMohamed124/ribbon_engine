# World Streaming Compile Fix Follow-up

## Reported diagnostics addressed

| Diagnostic | Source of issue | Change |
|---|---|---|
| E0689 `saturating_add` ambiguous integer | Scene snapshot count inferred from untyped zero | Declare `prefab_entity_count: usize` |
| Unreachable statement | A trailing return followed a match whose paths all returned | Remove the redundant return |
| E0599 `PrefabLoadContext::remove` missing | Context API required to restore a temporarily owned `AssetServer` | Ensure typed removal API exists in `crates/scene/src/prefab.rs`; add regression test |
| E0599 `World::despawn_recursive` missing | `HierarchyExt` trait was not in scope for the coordinator | Import `ribbon_scene::HierarchyExt` |
| E0502 nested mutable/immutable World borrow | `StreamingAssetAdapters` was looked up inside `AssetServer` resource scope | Move the adapter registry out before `resource_scope`, release typed leases, restore registry afterward |
| unused `leases` warning | Lease-handle snapshot no longer used by the SceneDoc readiness path | Remove it from the tuple |

## Rebuild note

The earlier `cargo clean` commands removed only `ribbon_render` and `ribbon_plugin_render3d`. They did not invalidate the `ribbon_scene` artifact, which is relevant to the reported missing `PrefabLoadContext::remove` method. This follow-up explicitly requires cleaning `ribbon_scene` and `ribbon_plugin_streaming` before rebuilding.

## Scope

This is a compile-fix follow-up, not a claim that all production streaming acceptance criteria are complete. It does not establish runtime, GPU hardware, memory-pressure, or world-scale stress validation.


## v3 test-fixture correction

The v2 Windows run compiled the full workspace but eight StreamingCoordinator tests panicked because their hand-built `World` omitted `StreamingAssetAdapters`, a resource installed by `StreamingPlugin::build` at runtime. The v3 coordinator test module now centralizes world construction in `test_world()`, which inserts the adapter registry before each test tick. Seventeen test-world fixtures use this helper. Seven stale, unused prefab imports in test functions were reduced to the `Transform3D` import actually needed, removing the warnings shown by the prior run.

This is a test-fixture correction, not a change to production plugin behavior. Re-run `cargo test -p ribbon_plugin_streaming --all-targets`, then `cargo clippy -p ribbon_plugin_streaming --all-targets -- -D warnings` and the full validation script.
