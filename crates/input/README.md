# Ribbon Input (`ribbon_input`)

`ribbon_input` provides platform-independent keyboard, mouse, gamepad, scripted/virtual input, and action-binding state. Native device access belongs to `ribbon_platform`; this crate contains the deterministic processed state and binding logic.

## Implemented

- `Key` covers the portable physical-key catalogue, preserves unidentified native scan-code identities, and keeps left/right modifiers distinct.
- `InputState` tracks held, just-pressed, and just-released keys. Focus loss releases held keyboard and mouse state.
- `MouseInputState` tracks mouse-button edges, pointer delta/position, scroll, and focus-loss cleanup.
- `GamepadSet` stores up to four pads, tracks connection and button edges, processes stick and trigger deadzones, sanitizes non-finite input, and derives digital trigger hysteresis.
- `BindingSource` supports keyboard, mouse, gamepad buttons, and thresholded positive/negative gamepad axes. `Bindings::axis_value` returns the strongest signed axis value for an action.
- Runtime rebinding is available through `set_sources` / `apply_rebinding`.

## Platform boundary and limitations

- `ribbon_platform` owns the optional gilrs polling thread and forwards platform-neutral raw events into `GamepadSet`. With the platform `gamepad` feature disabled, pure gamepad processing and virtual injection remain available.
- Rumble requests are exposed by the platform integration, but actual device support and mapping vary by controller and must be checked on hardware.
- There is no built-in binding serialization format or persisted rebinding settings resource.
- The action helper exposes active/axis queries; a complete per-action just-pressed/just-released edge abstraction is not provided.
- Text entry and IME composition are platform/UI concerns and are not implemented by this crate.

Physical keyboard input is separate from committed Unicode text; typed characters must come from the platform text-input stream, not by translating gameplay keys.
