# STUDS

A monochrome first-person horror game built with a software raycaster. Explore a block maze, collect six pages, and keep the creature in sight: it moves when you look away.

> **Status:** Active development. Gameplay, assets, and platform support may change.

## Features

- Software-rendered, voxel-style world with a DDA raycaster
- Creature navigation with BFS pathfinding and line-of-sight behavior
- Sprint stamina, flashlight, configurable controls, and saved settings
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
| Look | W / A / S / D or arrow keys |
| Move | I / J / K / L |
| Jump | E |
| Sprint | Left or right Shift |
| Flashlight | F |
| Restart | R |
| Quit | Esc or Q |

Bindings are configurable and saved in `settings.cfg` in the application's working directory.

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

- النظر: `W / A / S / D` أو الأسهم
- الحركة: `I / J / K / L`
- القفز: `E`، الجري: `Shift`، الكشاف: `F`
- إعادة اللعب: `R`، الخروج: `Esc` أو `Q`

يمكن تعديل الأزرار، وتُحفظ الإعدادات في `settings.cfg` داخل مجلد التشغيل.
