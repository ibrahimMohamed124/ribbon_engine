# `ribbon_ui`

`ribbon_ui` is an immediate-mode UI with logical-pixel widgets, mouse activation, keyboard/gamepad action input, bundled Noto Sans font data and cosmic-text advanced shaping. `UiPlugin` installs a frame context and a wgpu surface pass; call `ui_scope(world, |ui| ...)` from an Update system. The pass overlays the completed scene. The font assets use SIL OFL 1.1.

The available widgets are labels/headings, buttons, checkboxes, sliders, progress bars, and single-line text inputs. Layout helpers currently stack widgets. `PanelSize::Fit` is intended to settle from prior-frame measurements. Text shaping/measurement is CPU-side; the current surface pass draws solid-color widget geometry, so glyph rasterization and text glyph rendering are not yet connected to the GPU pass. Input snapshots are headless-testable; full OS IME caret placement and repeated directional focus movement are not implemented.

Not implemented: retained widget trees, flexbox/grid layout, images/icons/9-slice, color emoji, selection/clipboard/undo, multi-line text editing, accessibility tree, drag-and-drop, in-world UI, animation/transitions, or full RTL layout mirroring. Combining-mark-aware caret movement is also out of scope.
