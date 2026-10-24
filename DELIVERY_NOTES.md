# Ribbon Engine overlay: merge bloom and tone-mapping submissions

## Change

- `plugins/render3d/src/post.rs`: `ToneMapper::render` now records the bloom extraction pass and the tone-mapping pass into one `wgpu::CommandEncoder`, and submits that command buffer once at the end.
- The pass order, attachments, clear/load operations, pipelines, bind groups, and draw calls are unchanged.
- No changes are made to the shared `MeshRenderer`, window clear behavior, HDR clear behavior, or offscreen call sites.

## Apply

Extract this ZIP at the root of the existing engine repository, allowing the path `plugins/render3d/src/post.rs` to overlay the existing file. Review the resulting diff before committing.

## Validation

No project build, app run, or tests are claimed by this overlay. After applying it locally, run `cargo fmt --check` and `cargo check --release -p graphics_showcase`; then capture the same benchmark configuration as the baseline and compare frame-wall P95/P99, graph CPU P95, and mean frame-wall time. Keep the old capture to make the comparison reproducible.
