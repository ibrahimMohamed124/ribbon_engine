# `ribbon_ui`

`ribbon_ui` is an immediate-mode game UI with logical-pixel layout, a mouse/keyboard/gamepad input snapshot, and a GPU overlay `FramePass`. Add `UiPlugin` after `Render3dPlugin`, then call `ui_scope(world, |ui| ...)` from an `Update` system. The pass loads the already-rendered surface and draws on top of it.

## Implemented surface

- Widgets: labels, headings, buttons, checkboxes, sliders, progress bars, and single-line text inputs.
- Layout: vertical/horizontal stacking, anchored panels, nested scopes/IDs, separators, spacing, and clipped scroll areas. `PanelSize::Fit` uses the previous frame's measured content width and height and settles one frame later.
- Input: press/release mouse activation, slider dragging, scroll-wheel offsets, keyboard Tab/Shift+Tab and directional focus, gamepad D-pad/left-stick directional focus, South/Enter/Space accept, East/Escape cancel, bumper previous/next, and bounded repeat timing for held directions. Focus movement uses the previous frame's focusable rectangles.
- Text: cosmic-text advanced shaping, bundled Noto Sans and Noto Sans Arabic fonts, per-frame glyph rasterization into an `R8Unorm` shelf atlas, dirty-region uploads, UTF-8-safe caret movement by Unicode scalar value (visual Left/Right respects RTL runs), password masking, committed text, IME pre-edit underline, and OS IME candidate-rectangle placement. RTL runs are right-aligned; widget order is not mirrored.
- Rendering: ordered shape/text draw commands, rounded-rectangle SDF fills/borders, glyph-atlas sampling, alpha blending, clipping via scissor rectangles, logical-pixel to NDC conversion, growable vertex storage, and GPU-resource recreation after device loss.

## Limits

This is an immediate-mode UI, not a retained widget tree or a flexbox/grid engine. Panel `Fit` measurement is one frame late. Text caret movement is by Unicode scalar value rather than grapheme cluster. Text selection, clipboard, undo, multiline input, color emoji, accessibility/screen-reader tree, drag-and-drop, in-world UI, animated transitions, and full RTL layout mirroring are not implemented. The normal headless test suite validates CPU shaping/atlas preparation and WGSL; the offscreen GPU panel/glyph readback test is ignored and still requires a compatible adapter to run.

Font assets are Noto Sans and Noto Sans Arabic under SIL OFL 1.1; see `crates/ui/assets/fonts/OFL.txt`.
