# Compatibility-shim retirement record

The `voxelworks-*` compatibility packages were retired from the workspace as part of Phase 9.
The active engine API is now provided by `crates/` and `plugins/`; games and examples import those
packages directly. There are no compatibility-shim packages in the workspace.

The prior shims re-exported `engine_math`, `plugin_voxel`, `engine_platform`, `engine_render`, and
`plugin_rendering` types for the immediate-mode runner. The old minimal runner example was removed
rather than maintaining a parallel app/window/rendering architecture. The last STUDS test and
benchmark references were migrated to `plugin_rendering::cpu` and `plugin_audio::rodio_backend`.

This file is kept as a short migration record for repository history. Do not add new shim crates; if
external compatibility ever becomes a product requirement, define a separately versioned adapter with
an explicit owner and a removal/deprecation policy.
