# Ribbon Platform (`ribbon_platform`)

Provides winit window events, the current physical keyboard/mouse bridge, cursor control, timing, and desktop runners. It also supports deterministic scripted input through `VirtualInput`. It does not own rendering or audio playback.

## Current platform surface

- `PlatformConfig` configures the initial title, physical width, and physical height.
- The winit bridge maps the currently supported `ribbon_input::Key` subset and mouse state, including focus-loss cleanup.
- Window creation and drawable resize events are handled by the runtime, but there is not yet a complete persisted/runtime display-settings API for windowed/borderless/exclusive modes, user-facing VSync, or configurable present-mode negotiation.
- Gamepad discovery/polling, text input, and IME composition are not implemented. Physical keys are not a substitute for committed Unicode text.

## Not implemented

The current `PlatformConfig` is not a full display-preferences resource. It does not expose live resolution/window-mode switching or FIFO/relaxed FIFO/mailbox/immediate preference selection. The current input bridge does not provide a functional controller backend or full keyboard/IME coverage.
