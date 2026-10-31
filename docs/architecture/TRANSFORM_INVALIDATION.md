# Transform invalidation

Date: 2026-10-22

## Problem

`ScenePlugin` propagates two independent transform conventions through the shared `Parent` graph:

- `Transform` / `GlobalTransform` for the existing 2D angle convention.
- `Transform3D` / `GlobalTransform3D` for quaternion rotation and per-axis scale.

Before this change, every `PostUpdate` reconstructed the local/parent maps, resolved every transform-bearing entity, and inserted every global component whether its value changed or not. A change to one entity therefore repeated composition work for unrelated branches.

## Contract implemented

Each propagation path owns a private resource cache in `ribbon_scene`. It stores the last observed local values, the previous parent edges, and the last propagated globals. The cache is moved out of the world during its system and reinserted after processing, avoiding a full cache clone every frame.

At each pass:

1. Snapshot current local transform components and current `Parent` links.
2. Seed invalidation with added, removed, or changed locals and parent relationships.
3. Also invalidate a transform-bearing entity when its output global is missing or no longer matches the cached output. This repairs externally removed/changed global components.
4. Propagate invalidation to descendants over both the previous and current parent graphs. Keeping old edges covers reparenting and local-transform removal.
5. Resolve dirty transforms parent-first. A clean ancestor reuses its cached global; dirty ancestors are resolved recursively first. The existing cycle fallback remains in place.
6. Write a global component only when the resolved result differs from the component currently on the entity. Remove the derived global when that entity loses its corresponding local transform.
7. Replace the cache snapshots with current input and resolved-output state.

The 2D and 3D caches are separate, while both use the same `Parent` hierarchy. The 3D chain continues to include only direct ancestors that also carry `Transform3D`; the 2D and quaternion composition rules are not mixed.

## Complexity and limitations

This is incremental **resolution**, not yet incremental **change discovery**. `Transform` and `Transform3D` fields remain public and existing systems freely mutate them through `query_mut`, so correctly detecting arbitrary field edits requires reading current component values each frame. Snapshot building and comparison remain O(N) for the number of local transforms and parent components. Composition and global writes can now be limited to dirty branches, but no performance benchmark has yet been run to quantify the benefit.

Comparisons use the component types' existing `PartialEq`; values containing NaNs will compare unequal and may keep a component dirty. That is conservative for correctness but could cause repeated recomputation for invalid numeric inputs. Transform shear is not newly represented: the 3D transform remains decomposed TRS as before.

The hierarchy's existing cycle safeguard is retained for invalid/corrupted parent graphs, while the public `World::set_parent` path rejects newly introduced cycles.

## Verification and next action

No tests were added and no Rust commands were run in the assistant environment, following the project's explicit constraints. The archive's earlier successful Windows test/lint results predate this code change and are not evidence that the new implementation compiles or passes.

In the Windows checkout, review the diff and run the existing checks (without adding tests):

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -j 1
cargo test --workspace --no-default-features -j 1
python tools/check_deps.py
```

Then profile two representative scenes: (a) a large mostly-static hierarchy where a small leaf/branch changes and (b) a scene where all local transforms change. Record entity count, dirty entity count (using temporary profiling instrumentation), time spent collecting snapshots, time spent resolving globals, and total propagation time. A result should not be described as a performance win unless it is measured against the previous implementation on the same machine and scene.

If snapshot scanning dominates, the next design decision is whether to add ECS component change ticks or controlled transform mutation APIs. Because public fields and current `query_mut` callers are part of today's usage, that redesign should be a separate compatibility-reviewed milestone rather than silently breaking those callers.
