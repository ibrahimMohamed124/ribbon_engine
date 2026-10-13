# STUDS

A monochrome first-person horror game built with a software raycaster. Explore a block maze, collect six pages, and keep the creature in sight: it moves when you look away.

> **Status:** Active development. Gameplay, assets, and platform support may change.

## Features

- Software-rendered, voxel-style world with a DDA raycaster
- Creature navigation with BFS pathfinding, line-of-sight pursuit, and a short search after losing the player
- Sprint stamina, flashlight, configurable controls, and saved settings
- Directional enemy footsteps, proximity-scaled ambience, and gameplay sound effects
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
| Restart (pause / result screens) | R |
| Toggle mouse look | Tab or M |
| Pause / Back | Esc |
| Quit | Q |

Bindings are configurable and saved in `settings.cfg` (`%APPDATA%\raycast_blocks\` on Windows, `~/.config/raycast_blocks/` on Linux/macOS, or `$XDG_CONFIG_HOME`). Mouse look starts enabled automatically when a run begins; press `Tab` or `M` only if you want to turn it off or back on. During active play, the cursor is hidden and recentered so you can turn continuously. Adjust `mouse_sensitivity` (default `0.0025` radians/pixel), `mouse_look_on_start`, and `invert_y` in `settings.cfg`.

The game opens on a main menu. Hover over an item and click to select it; the keyboard remains available. In settings, click the value area to change a value, and use the mouse wheel to move through long control lists. Press `Esc` during a run to pause it; simulation and gameplay timers stay frozen until you resume. Settings and controls are available from both menus and are saved when changed. The settings screen exposes the currently supported audio buses, mouse options, FOV, brightness, window scale, and render resolution. Window scale choices adjust to the render resolution and keep the initial window at or below 1280x720. Mouse sensitivity is shown as an easy-to-read 0-10 level.

Selecting **New Game** opens a difficulty screen. **Easy** gives more time before the enemy wakes and reduces its chase speed; **Normal** preserves the original balance; **Hard** wakes the enemy sooner and increases its speed. Looking at the enemy still slows it on every difficulty. New runs randomize the player and enemy's starting cells and place pages apart. The HUD teaches the objective and the enemy's sight rule, while a won run records the best time for its difficulty in `settings.cfg`.

Cursor recentering uses Enigo. On Wayland, cursor warping may be restricted or experimental depending on the compositor and Enigo backend. macOS may request Accessibility permission to control the cursor. If cursor warping is denied, use the keyboard look controls or disable mouse look.

## Project layout

```text
src/
  main.rs              Window and frame loop
  ui/
    menu.rs            Main menu entries
    difficulty.rs      Difficulty choices and descriptions
    pause.rs           Pause screen labels
    settings.rs        Supported option values and adjustments
    controls.rs        Control labels and rebind order
    dialogs.rs         Confirmation and result text
    layout.rs          Shared menu panel layout
    theme.rs           Pixel font, colors, and menu backgrounds
    mod.rs             UI navigation and drawing
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

WAV assets in `assets/audio/` are embedded in the game binary and played through Rodio. The menu ambience plays in menus; the enemy drone loops after the enemy wakes and grows louder with proximity. Enemy steps are panned left and right based on its direction relative to the player. Linux builds require ALSA development files (`libasound2-dev`).

The main menu starts each run explicitly. During play, `Esc` opens a pause screen; simulation and gameplay timers remain frozen until resume. Settings and controls can be edited in either menu. Supported options include audio buses and mute, mouse look, FOV, brightness, render resolution, and window scaling. There is no saved-run system, so the main menu does not show a Continue option.

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
- الإيقاف المؤقت: `Esc`، تشغيل/إيقاف النظر بالماوس: `Tab` أو `M`
- إعادة اللعب: `R`، الخروج: `Q`

يمكن تعديل الأزرار، وتُحفظ الإعدادات في `settings.cfg` (في `%APPDATA%\raycast_blocks\` على ويندوز، أو `~/.config/raycast_blocks/` على لينكس/ماك).

تظهر القائمة الرئيسية عند التشغيل. اضغط `Esc` أثناء اللعب للإيقاف المؤقت؛ تتوقف حركة اللاعب والوحش ومؤقتات اللعب حتى الاستئناف. يمكن تغيير الإعدادات وإعادة ربط الأزرار من القائمتين، وتُحفظ التعديلات مباشرة. شاشة الفوز تعرض وقت الجولة والصفحات التي جُمعت. لا يوجد نظام حفظ للجولات حاليًا، لذلك لا يظهر خيار «استكمال».

عند اختيار «لعبة جديدة» تظهر شاشة اختيار المستوى: السهل يمنحك وقتًا أطول قبل استيقاظ الوحش ويقلل سرعته، والعادي يحافظ على التوازن الحالي، والصعب يجعل الوحش يستيقظ أسرع ويزيد سرعته. يظل الوحش أبطأ عندما تنظر إليه في كل المستويات.

تظهر تعليمات الهدف في بداية الجولة، ثم تلميح إبقاء الوحش أمامك عندما يستيقظ. الوحش يبحث لفترة قصيرة عند فقدان رؤيتك، وخطواته تتجه يسارًا ويمينًا بحسب موقعه. تتغير أماكن البداية والصفحات مع كل جولة، مع الحفاظ على مسافة بينها. تُحفظ أفضل مدة فوز لكل مستوى في `settings.cfg`. إعداد السطوع يتراوح بين 50% و150%، وتُحد خيارات تكبير النافذة حسب دقة الرسم بحيث لا تتجاوز أبعاد البداية 1280×720.
