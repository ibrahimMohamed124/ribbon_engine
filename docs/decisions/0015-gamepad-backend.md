# ADR 0015: Native gamepad input backend

`ribbon_input` owns backend-neutral button, axis, deadzone, hysteresis, and edge processing. `ribbon_platform` polls gilrs on a dedicated thread and forwards raw changes to the main thread. A virtual injection function exposes the same deterministic processing path for tests and replays.

The `gamepad` platform feature is enabled by default. gilrs default features are retained because disabling them fails to compile on Windows without selecting a Windows backend feature. Platform input tests do not require connected hardware.
