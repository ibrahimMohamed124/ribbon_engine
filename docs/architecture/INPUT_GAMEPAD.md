# Gamepad input

`ribbon_input::GamepadSet` stores up to four pads and applies radial stick deadzones, trigger deadzones, trigger press hysteresis, and frame edges. Axis bindings support positive and negative thresholds; `Bindings::axis_value` returns the strongest signed bound axis.

`ribbon_platform` owns gilrs on a polling thread. The main `Stage::First` system transfers raw changes into `Input` and emits `GamepadEvent`; window focus loss does not clear pad state. `inject_raw_gamepad_event` feeds the same processor for deterministic tools and tests. Rumble requests are forwarded on `Stage::Last`.

The native backend is optional through the `gamepad` feature, enabled by default. With default features disabled, the pure `GamepadSet` and virtual injection path remain available. Hardware behavior and per-device mapping differences have not been verified on this machine.
