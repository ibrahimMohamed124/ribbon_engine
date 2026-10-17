# Temporary compatibility shims

The legacy `voxelworks-core` crate temporarily re-exports `engine_math` as `math`, `plugin_voxel::voxel` as `voxel`, `plugin_physics` as `physics`, `engine_app::time` as `time`, and `engine_platform::input` as `input`. The legacy API re-exports `MouseButtons`, `MouseInput`, and `WindowConfig` from `engine_platform`. The legacy platform crate re-exports the moved `cursor`, `keyboard`, `keys`, `timer`, and `window` modules. These preserve existing imports while code moves. Phase 9 removes them when the legacy crates are deleted.
