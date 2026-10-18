# `engine/` — legacy crates (deprecated)

These are the original `voxelworks-*` crates. They exist only because
`games/studs` still depends on them. **Do not add features or new dependents here.**

- The current engine is in [`../crates`](../crates) and [`../plugins`](../plugins);
  see [`../ARCHITECTURE.md`](../ARCHITECTURE.md).
- The removal plan is [`../docs/PHASE9.md`](../docs/PHASE9.md).
- The compatibility re-exports linking old to new are listed in
  [`../docs/SHIMS.md`](../docs/SHIMS.md).
- `tools/check_deps.py` fails if any crate outside `engine/` and `games/studs`
  depends on a `voxelworks-*` crate.

`crates/render-gpu` is the only complete GPU voxel renderer today and must be
ported (PHASE9 step 1) before this directory can be deleted.
