# Spotlight shadow regression fixture adjustment (v4)

## Observed result

After forcing a clean rebuild, the spotlight-only corridor regression reported 12 changed pixels and difference energy 751. This establishes that the shadow path is active, but the current scene does not produce a large enough visible shadow for the existing assertion thresholds.

## Change

The regression fixture's spotlight direction now aims through the off-center blocker (`(-0.31, -0.32, -0.91)`) rather than down the corridor centerline. The blocker is deliberately off the camera centerline so the resulting floor shadow remains visible to the camera. Existing assertion thresholds and non-test engine behavior are unchanged.

## Verification status

This source package has not been compiled or GPU-tested in the packaging environment. Run the focused test on a machine with a usable wgpu adapter. If it still fails, inspect the newly generated `target/spotlight_corridor.png`; do not lower the assertion thresholds.
