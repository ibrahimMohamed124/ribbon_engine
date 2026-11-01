# STUDS GPU camera input note

The prior restructuring pass added a GPU-only input conversion in `games/studs/src/app/systems.rs`. It was a behavior change, not a structural requirement. The conversion reversed mouse yaw and swapped both look and strafe directions only for the mesh GPU backend. The REF `games/studs/src/player.rs` adds `mouse_yaw` directly to `view.phi`, and the REF input poller maps positive mouse X to positive yaw, so this backend-specific conversion was removed.

## Removed invocation and backend branch

```rust
            // The mesh renderer uses a right-handed view matrix, while the legacy CPU
            // raycaster's screen-right vector has the opposite horizontal convention. Adapt
            // only this game's collected intent for the active GPU backend so mouse-right,
            // arrow-key look, and A/D strafing agree with the rendered camera without altering
            // the CPU fallback's golden-output convention.
            let mesh_gpu_camera = world.resource::<StudsRenderer>().uses_mesh_gpu();
```

```rust
                if mesh_gpu_camera {
                    adapt_input_to_mesh_gpu_camera(&mut frame_input);
                }
```

## Removed helper

```rust
/// Match the conventional screen-right direction of the right-handed mesh camera.
/// The CPU raycaster keeps its original horizontal convention for reference/golden stability.
pub(super) fn adapt_input_to_mesh_gpu_camera(frame: &mut InputFrame) {
    frame.controls.mouse_yaw = -frame.controls.mouse_yaw;
    std::mem::swap(
        &mut frame.controls.look_left,
        &mut frame.controls.look_right,
    );
    std::mem::swap(&mut frame.controls.left, &mut frame.controls.right);
}
```

The same behavior-specific test was retained and changed to assert that mouse yaw, keyboard look, and strafing controls stay in the reference convention regardless of renderer selection. The structural use of `ribbon_input` / `ActionMap` remains untouched.
