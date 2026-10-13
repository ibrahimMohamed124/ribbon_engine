# STUDS

A monochrome first-person horror game built with a software raycaster. Explore a block maze, collect six pages, and keep the creature in sight: it moves when you look away.

> **Status:** Active development. Gameplay, assets, and platform support may change.

## Features

- Software-rendered, voxel-style world with a DDA raycaster
- Creature navigation with BFS pathfinding and line-of-sight behavior
- Sprint stamina, flashlight, configurable controls, and saved settings
- Spatially responsive horror ambience, footsteps, and gameplay sound effects
- Rust unit and integration tests, with cross-platform CI builds

## Requirements

- Rust stable (edition 2021)
- A desktop environment supported by [minifb](https://github.com/emoon/rust_minifb)

On Linux, the native windowing development libraries required by `minifb` may also be needed. The CI workflow installs the dependencies used by its Ubuntu runners.

## Run

```sh
cargo run --release
```

Run the test suite with:

```sh
cargo test --locked
```

## Controls

| Action | Default keys |
| --- | --- |
| Move | W / A / S / D |
| Look | Arrow keys (or mouse look) |
| Jump | E |
| Sprint | Left or right Shift |
| Flashlight | F |
| Restart | R |
| Toggle mouse look | Tab or M |
| Quit | Esc or Q |

Bindings are configurable and saved in `settings.cfg` (`%APPDATA%\raycast_blocks\` on Windows, `~/.config/raycast_blocks/` on Linux/macOS, or `$XDG_CONFIG_HOME`). Mouse look starts disabled. Press `Tab` or `M` to toggle it; while active, the cursor is hidden and recentered so you can turn continuously. Adjust `mouse_sensitivity` (default `0.0025` radians/pixel), `mouse_look_on_start`, and `invert_y` in `settings.cfg`.

Cursor recentering uses Enigo. On Wayland, cursor warping may be restricted or experimental depending on the compositor and Enigo backend. macOS may request Accessibility permission to control the cursor. If cursor warping is denied, use the keyboard look controls or disable mouse look.

## Project layout

```text
src/
  main.rs              Window and frame loop
  game.rs              Game state and update loop
  player.rs            Movement, physics, and stamina
  enemy.rs             Enemy behavior and pathfinding
  world.rs             Maze, voxel world, and raycasting
  renderer/            Camera, lighting, entities, post-processing, and HUD
  input.rs             Input actions and key bindings
  audio.rs             Audio events and swappable backend
  settings.rs          Settings parsing and persistence
  math.rs              Vector, angle, RNG, and hashing utilities
tests/                 Integration tests
assets/                Runtime assets
```

Gameplay WAV assets in `assets/audio/` are embedded in the game binary and played through Rodio. The optional menu ambience is reserved because the game has no title screen yet. Linux builds require ALSA development files (`libasound2-dev`).

Modules are kept independent: the world and math layers do not depend on rendering or player state. The executable entry point handles the window while the game logic remains in the library crate.

## Contributing

Issues and pull requests are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) before making a change. CI builds and tests on Linux, Windows, and macOS.

## License

No license has been added yet. Until the project owner selects one, all rights are reserved; please do not redistribute or reuse the project's code or assets.

---

## بالعربية

لعبة رعب بمنظور الشخص الأول، برسوم أحادية اللون ومحرك رسم برمجي. استكشف متاهة من المكعبات، واجمع ست صفحات، وأبقِ الوحش أمام عينيك لأنه يتحرك عندما تنظر بعيدًا.

### التشغيل

يتطلب المشروع إصدار Rust المستقر وبيئة سطح مكتب يدعمها `minifb`.

```sh
cargo run --release
cargo test --locked
```

### أزرار التحكم الافتراضية

- الحركة: `W / A / S / D`
- النظر: الأسهم أو الماوس
- القفز: `E`، الجري: `Shift`، الكشاف: `F`
- تشغيل/إيقاف النظر بالماوس: `Tab` أو `M` (يبدأ متوقفًا افتراضيًا)
- إعادة اللعب: `R`، الخروج: `Esc` أو `Q`

يمكن تعديل الأزرار، وتُحفظ الإعدادات في `settings.cfg` (في `%APPDATA%\raycast_blocks\` على ويندوز، أو `~/.config/raycast_blocks/` على لينكس/ماك).
