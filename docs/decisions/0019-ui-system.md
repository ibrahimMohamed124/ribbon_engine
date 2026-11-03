# ADR 0019: Immediate-mode game UI

## Decision

Add `ribbon_ui` as a game-facing immediate-mode UI crate. It uses cosmic-text 0.12.1 for advanced Unicode shaping, bundles Noto Sans and Noto Sans Arabic under SIL OFL 1.1, emits CPU-side widget geometry, and registers a wgpu frame pass after the scene renderer. `ui_showcase` is the runnable integration example.

## Consequences and limits

The initial layout uses a small stacking model. The UI renderer currently draws solid widget geometry; glyph rasterization and glyph-atlas sampling are not connected to the GPU pass. OS IME caret placement, full focus repeat timing, selections, and full RTL layout mirroring remain incomplete. The current immediate-mode API is intentionally separate from the editor CPU UI.
