# STUDS

A monochrome first-person horror game built with a software raycaster. Explore a block maze, collect six pages, and keep the creature in sight: it moves when you look away.

> **Status:** Active development. Gameplay, assets, and platform support may change.

## Features

- Software-rendered, voxel-style world with a DDA raycaster
- Creature navigation with BFS pathfinding, line-of-sight pursuit, and a short search after losing the player
- Sprint stamina, flashlight, configurable controls, and saved settings
- Directional enemy footsteps, proximity-scaled ambience, and gameplay sound effects
- Rust unit and integration tests, with cross-platform CI builds
- Classic grayscale and Moody cold-tinted rendering styles, plus a photo-textured Realistic option

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

The **Visual Style** setting defaults to **Realistic** (photo-textured PBR surfaces with a horror color grade), with **Moody** (cold green-gray palette, tone mapping, subtle procedural texture, and voxel ambient occlusion) and **Classic** (the original grayscale look) also available. Quality presets are Low, Medium, High, and Realistic; Low and Medium skip PBR map sampling. High and Realistic cast one flashlight DDA shadow ray per world hit. The Realistic monster is a CC0 baked animated sprite with a retained box fallback when its atlas is missing. Proximity adds bounded grain, vignette, a mild ray warp, and capped flashlight pulses. `reduce_flashing` replaces those pulses with steady dimming capped at 12% and disables grain/warp. GPU presentation is the default; select the CPU reference with `--renderer cpu` or `STUDS_RENDERER=cpu`. For a deterministic live GPU scene, run `cargo run --locked -p studs --example gpu_preview -- gpu` (use `cpu` to compare the reference). CPU sample captures remain available with `cargo run --locked -p render_cpu --example screenshot`; PNGs are written under `target/shots/`. The CPU Realistic game-scene benchmark is `cargo run --release --locked -p studs --example realistic_bench -- 30` (320×180; reports each style).

### Replacing the monster source

Keep authoring downloads out of git under `assets/monster/source/`. Add a `[[monsters]]` entry in `assets/manifest.toml` with the model name, source and download URLs, author/provenance, approved license, source SHA-256, local archive path, selected FBX filename, output directory, and processed-map SHA-256 values. Run `python tools/fetch_assets.py` to fetch and verify the archive, unpack it under that ignored source directory, then run `cargo run --release --locked -p bake_sprites -- bake "assets/monster/source/unpacked/<model>.fbx" studs/assets/monster [optional-albedo.png]`. The baker writes `albedo.png`, `normal.png`, `emissive.png`, and `metadata.txt`. Update the three processed SHA-256 values in the same manifest entry and generated `assets/CREDITS.md`, then run `python tools/fetch_assets.py --check`. `bake_sprites inspect <model.fbx>` prints rig and animation information before choosing a model. The source archive and unpacked model are ignored and are never part of the game build.

Selecting **New Game** opens a difficulty screen. **Easy** gives more time before the enemy wakes and reduces its chase speed; **Normal** preserves the original balance; **Hard** wakes the enemy sooner and increases its speed. Looking at the enemy still slows it on every difficulty. New runs randomize the player and enemy's starting cells and place pages apart. The HUD teaches the objective and the enemy's sight rule, while a won run records the best time for its difficulty in `settings.cfg`.

Cursor recentering uses Enigo. On Wayland, cursor warping may be restricted or experimental depending on the compositor and Enigo backend. macOS may request Accessibility permission to control the cursor. If cursor warping is denied, use the keyboard look controls or disable mouse look.

## Project layout

```text
Cargo.toml              Workspace; `studs` is the default member
engine_core/             Platform-independent math, voxel, audio, app context, and framebuffer
engine_scene/            Renderer-independent Scene, camera, materials, textures, and styles
render_cpu/              Rayon CPU raycaster, lighting, post-processing, and entity intersection
render_gpu/              wgpu world raycaster, overlay compositor, and presentation shaders
platform/                Desktop window, input mapping, cursor, timer, audio, and generic runner
  examples/walkabout.rs  Standalone room demo using the CPU renderer
studs/
  src/main.rs           Small application entry point
  src/lib.rs            Game library and module map
  src/app.rs             Game-specific App implementation and orchestration
  src/ui/               Menus, controls, settings, and pixel UI theme
  src/game.rs           Game state and update loop
  src/player.rs         Movement, physics, and stamina
  src/enemy.rs          Enemy behavior and pathfinding
  src/level.rs          Maze generation and game-specific block layout
  src/renderer/         HUD and game-over overlays
  src/input.rs          Input actions and key bindings
  src/audio.rs          Audio events and swappable backend
  src/settings.rs       Settings parsing and persistence
  src/math.rs           Compatibility re-exports for shared math
  tests/                Integration tests
  assets/               Runtime assets
```

WAV assets in `studs/assets/audio/` are embedded in the game binary and played through Rodio. The menu ambience plays in menus; the enemy drone loops after the enemy wakes and grows louder with proximity. Enemy steps are panned left and right based on its direction relative to the player. Linux builds require ALSA development files (`libasound2-dev`).

The main menu starts each run explicitly. During play, `Esc` opens a pause screen; simulation and gameplay timers remain frozen until resume. Settings and controls can be edited in either menu. Supported options include audio buses and mute, mouse look, FOV, brightness, render resolution, and window scaling. There is no saved-run system, so the main menu does not show a Continue option.

The workspace separates `engine_core`, `engine_scene`, `render_cpu`, `render_gpu`, `platform`, and `studs`. `engine_core` has no external dependencies; the scene crate contains plain renderer-independent data; CPU rendering owns Rayon and the reference raycaster; the platform runner accepts a renderer implementation. `studs` remains the default member, so `cargo run --release` from the repository root continues to launch `raycast_blocks`. See [ARCHITECTURE.md](ARCHITECTURE.md) for dependency rules and the GPU port checklist.

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

يبدأ خيار **Visual Style** بنمط **Realistic** ذي الخامات المصورة، مع إمكانية اختيار **Moody** أو الرجوع إلى **Classic** الرمادي الأصلي. لإنشاء صور مقارنة ثابتة شغّل `cargo run --locked -p render_cpu --example screenshot`؛ تحفظ الصور في `target/shots/`.

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

Renderer reference levels can be checked with `cargo run --release --locked -p render_cpu --example level_probe`; the command fails if the Moody lighting targets are missed.
