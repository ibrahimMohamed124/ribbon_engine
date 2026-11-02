# Ribbon Input (`ribbon_input`)

`ribbon_input` currently provides platform-independent key and mouse state, scripted/virtual input, and a small action-to-binding map.

## Implemented

- The `Key` enum currently covers A–Z, arrow keys, left/right Shift, Space, Escape, Enter, Tab, and F1.
- `InputState` tracks held, just-pressed, and just-released keys. `focus_lost` releases held keys so they cannot remain stuck.
- `MouseInputState` tracks mouse-button edges, pointer delta/position, scroll, and focus-loss cleanup.
- `Bindings` / `ActionMap` retain the key-only compatibility API and can also store multiple key or mouse-button sources. Runtime rebinding is available through `set_sources` / `apply_rebinding`.

## Not implemented

- The `GamepadButton` binding variant is only a reserved data extension point. There is no device discovery, button/axis polling, deadzone processing, multiple-controller backend, or haptics implementation.
- Gamepad axes are not yet binding sources.
- There is no built-in binding serialization format or persisted rebinding settings resource.
- The action helper currently answers held/active state; a complete per-action pressed/released edge abstraction is not provided.
- Text entry and IME composition are platform/UI concerns and are not implemented by this crate.

The platform integration maps a limited physical-key subset into this API; it does not infer typed Unicode text from physical keys. Do not treat the placeholder gamepad variant as working gamepad support.
