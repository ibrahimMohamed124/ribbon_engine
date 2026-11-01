# Shadow coordinate interpolation fix

## Symptom

On the Windows GPU, `spotlight_only_corridor_writes_a_headless_png` rendered the corridor but reported
`changed pixels=0, difference energy=0` when comparing the exact same frame with the spotlight shadow
map enabled and disabled. The PNG was saved before the failing assertion so the frame remains inspectable.

## Root cause in the shader path

The mesh vertex shader emitted directional and spotlight **light clip-space coordinates** as ordinary
varyings. Rasterization perspective-corrects ordinary varyings using the *camera's* clip-space `w`,
which is not the projection used by the light. The resulting interpolated coordinates can be wrong
for shadow-map lookup, even when the light projection matrix and shadow render pass are present.

## Change

The fragment shader now computes the directional and spotlight shadow coordinates from the
perspective-correct interpolated `world_position` using the matching light view-projection uniform.
This keeps projection consistent with the fragment's actual surface position. The shadow comparison,
PCF kernel, depth bias, and regression assertions have not been weakened.

## Verification status

The source change is made, but this document does not claim the regression test passes yet. On Windows,
run:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ribbon_plugin_render3d spotlight_only_corridor_writes_a_headless_png -- --nocapture
```

The test is expected to save `target/spotlight_corridor.png` before checking the shadow difference. If
it still fails, inspect that PNG and the reported pixel-difference measurements before making further
changes. Then run both full workspace test configurations.
