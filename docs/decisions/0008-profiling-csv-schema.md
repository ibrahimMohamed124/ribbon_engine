# ADR 0008: Keep game-specific resource counters out of engine profiling

## Context

The strict ownership boundary prohibits voxel data and voxel-specific counters in `crates/` and `plugins/`, while the previous renderer CSV had four chunk/voxel-specific columns.

## Decision

Move those four counters to `games/studs/crates/studs_voxel_gpu::VoxelRendererResourceStats` and remove them from the generic engine CSV. The profile report script detects optional columns instead of emitting fabricated zeros.

## Consequence

This intentionally changes the legacy CSV header and therefore does not satisfy the prompt's byte-identical CSV acceptance criterion. The deviation is explicit rather than retaining game-specific names in engine code.
