# Ribbon Platform (`ribbon_platform`)

Provides the winit window/event boundary, physical keyboard and mouse state, Unicode text/IME events, runtime display preferences, cursor control, timing, and desktop runners. Rendering and audio playback remain separate plugins. Deterministic scripted input remains available through `VirtualInput`.

## Current platform surface

- `PlatformConfig` keeps the source-compatible title and physical width/height settings; changing width/height at runtime requests a windowed resize and the renderer receives the native resize event to recreate size-dependent targets.
- `DisplaySettings` independently controls `WindowMode` (`Windowed`, `BorderlessFullscreen`, `ExclusiveFullscreen`), an optional physical `DisplayResolution`, and user-facing `VSyncMode` (`On`, `Adaptive`, `Off`). `set_resolution` rejects zero dimensions; clearing the override follows legacy `PlatformConfig` width/height. Borderless mode uses the monitor's native size, while exclusive mode requests a matching supported video mode and falls back to borderless when unavailable. `DisplaySettings::vsync = None` preserves (and can restore after a runtime override) the environment-selected backend mode. wgpu negotiates FIFO, FIFO Relaxed, Mailbox, or Immediate against active surface capabilities; runtime settings update the active swapchain without rebuilding the device.
- Keyboard input maps all 194 standard `winit 0.30.13` `KeyCode` variants to physical `Key` values, including A-Z, digit rows, F1-F35, international/language keys, numpad operations, media/browser/system controls, navigation/editing keys, and left/right modifiers. Unidentified native scan codes are preserved as `Key::Native(NativeKeyCode)` for local rebinding; those raw identities are OS-specific and are not portable default bindings. Focus loss releases held keys and mouse buttons. Stable names cover all standard keys through `key_name` / `parse_key`; native keys require application-specific persistence.
- Committed Unicode text and IME pre-edit/enable/disable events are exposed through `TextInput` and `TextInputEvent`; they are kept separate from physical gameplay key bindings. Pre-edit cursor/selection offsets remain byte-indexed and are clamped to valid UTF-8 boundaries. `WindowHandle` exposes the native IME candidate/caret rectangle API.
- Mouse input includes button edges, relative motion, pointer position, and accumulated scroll.
- `GamepadSet` is a platform-neutral four-pad state container with button edges, analog axes, radial stick and trigger deadzones, trigger hysteresis, and axis bindings. With the default `gamepad` feature, a dedicated gilrs thread discovers/polls devices and the `Stage::First` system transfers raw events into `Input`; rumble requests are forwarded on `Stage::Last`. The backend can be disabled with `--no-default-features`, while deterministic virtual injection remains available.

## Explicit limitations

- `DisplaySettings` is a runtime resource; application-specific persistence/serialization is still the responsibility of the settings owner.
- The native gamepad backend is implemented but device mapping, hot-plug, and rumble behavior still require testing with physical controller hardware. Mocked injection tests do not prove controller-specific behavior.
- The platform layer exposes committed/pre-edit text, byte-indexed cursor/selection offsets, IME lifecycle, and a native candidate/caret rectangle hook. A higher-level widget focus/ownership policy is still the responsibility of the UI/editor layer.
