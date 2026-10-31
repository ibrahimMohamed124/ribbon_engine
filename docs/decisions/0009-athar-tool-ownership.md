## Context

`horror_slice` was moved from `examples/` to `games/athar`, but the saved village asset-tool overlay still targeted the old tree. The Oct 28 source archive did not contain the complete earlier local working tree; a saved village overlay did preserve the later gameplay scripts and assets.

## Decision

Keep Athar-specific asset acquisition, model generation, and asset validation under `games/athar/tools/`. Use workspace-relative manifest paths and resolve the Athar asset root explicitly. Restore the village generation/validation tools together with the recoverable Wet Creek gameplay scripts, HUD pass, and village assets. Adapt paths to `games/athar` and preserve the room schema and room quest before the one-way transition.

## Rejected alternatives

Extracting the older pre-move game over `games/athar` would risk overwriting newer scene code. Inventing missing flashlight/audio generators would not recover the original behavior.

## Consequences

The saved village tool chain and playable Wet Creek transition are present, and the manifest validator covers the room and village GLBs. `generate_flashlight_asset.py` and `generate_horror_audio.py` remain unresolved because no copies were present in the saved project or available ZIP overlays.
