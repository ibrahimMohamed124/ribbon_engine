# Temporary compatibility shims

The legacy `voxelworks-core` crate temporarily re-exports `engine_math` as `math`, `plugin_voxel::voxel` as `voxel`, and `plugin_physics` as `physics`. These preserve the old import paths for games and remaining crates while leaf code moves. Phase 9 removes them when the legacy crate is deleted.
