# Scene Hierarchy 3D

A runnable visual example for the shared 3D transforms in `engine_scene` and
`plugin_render3d`.

Run it from the workspace root:

```sh
cargo run -p scene_hierarchy_3d
```

To close automatically after a fixed number of frames (useful when collecting
logs), run the command below. The window closing after those frames is expected;
omit `--frames` for an interactive session that stays open until you close it.

```sh
cargo run -p scene_hierarchy_3d -- --frames 240
```

## What you should see

A 3D window with a perspective grid and a colored, articulated rig. The cyan
pivot is the parent. Its orange arm and yellow tip are descendants; a magenta
counterweight and green satellite are sibling children. The parent rotates the
whole rig while the arm joint independently swings the link and tip. The dark
pedestal and floor remain stationary.

The terminal should print two lines similar to:

```text
[HIERARCHY] PASS: root -> joint -> link -> tip transforms propagated.
[HIERARCHY] PASS: local translation, rotation, and scale compose into world transforms.
```

It also prints the resolved world positions of the root and tip. If propagation
is missing or inconsistent, the example prints `[HIERARCHY] FAIL` and the
expected versus actual positions instead of claiming success.

## Controls

- **Space**: pause or resume the animation.
- **R**: reset the parent and joint angles.
- `--frames N`: exit after N frames.

This example checks 3D hierarchy propagation visually and through runtime
console diagnostics. It does not implement mesh picking, collision, shadows, or
PBR materials.
