# ADR 0019: Immediate-mode game UI

## Decision

Add `ribbon_ui` as a game-facing immediate-mode UI crate. It uses cosmic-text 0.12 for advanced Unicode shaping, bundles Noto Sans and Noto Sans Arabic under SIL OFL 1.1, shapes and rasterizes glyphs into an `R8Unorm` atlas, and registers a wgpu frame pass after the scene renderer. `ui_showcase` is the runnable integration example.

The frame builder uses vertical/horizontal stacking, anchored panels, scroll clipping, per-widget IDs, mouse interaction, keyboard/gamepad focus navigation, single-line text editing and the native IME candidate rectangle hook. Glyphs and shapes retain their authored draw order. Atlas updates are uploaded as dirty regions after initial creation.

## Consequences and limits

The layout model is intentionally small rather than flexbox/grid; `PanelSize::Fit` settles from prior-frame measurement. Caret movement is Unicode-scalar based, not grapheme based. Selection, clipboard, undo, multiline text, color emoji, accessibility, drag/drop, world-space UI, animation and full RTL layout mirroring remain out of scope. GPU visual validation remains an explicit host-side check because the offscreen GPU test is ignored by default.
