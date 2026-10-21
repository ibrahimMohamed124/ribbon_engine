//! STUDS application setup and ECS plugin integration.
//!
//! Runtime-owned world state lives in ECS resources and entities: `RunState` and
//! `MazeResource` are resources, while `Player`, `Enemy`, and `PagePickup` are
//! independently addressable entities. The legacy `Game` type is used only to
//! generate a new run and as a compatibility/domain API for existing consumers.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use engine_app::{App, AppExit, Stage, Time, World};
use engine_events::WorldEventsExt;
use engine_math::Vect;
use engine_platform::input::{Key, MouseSource, ALL_KEYS};
use engine_platform::runtime::CursorGrabState;
use engine_platform::{Input as PlatformInput, PlatformConfig, WindowFrame, WindowHandle};
use engine_plugins::{AppPluginExt, Plugin};
use engine_render::{DrawList, FramePlan, Framebuffer, Renderer};
use plugin_audio::{AudioListener, PlaySound, SetAmbience, SoundId};
#[cfg(feature = "gpu")]
use plugin_render3d::{
    glam::{Mat4, Quat, Vec3},
    OffscreenCamera3D, OffscreenDraw3D, OffscreenVoxelRenderer, SunLight, VoxelBlockColor,
};
use plugin_rendering::cpu::renderer::CpuRenderer;
use plugin_rendering::ecs::{CpuBackend, DefaultShader, RenderBackend, RenderSettings};
use plugin_rendering::material::MaterialTable;
use plugin_rendering::Scene;
#[cfg(feature = "gpu")]
use plugin_voxel::ChunkedVoxelWorld;

use crate::audio::{ambiences, sounds, AudioEvent};
use crate::enemy::proximity_intensity;
use crate::enemy::Enemy;
use crate::game::{Game, GameReadView, GameUiState, State, PAGES_TOTAL};
use crate::game_ecs::{self, MazeResource, RunState, RunStep};
use crate::input::{Input as GameInput, InputFrame};
use crate::player::Player;
use crate::settings::Settings;
#[cfg(feature = "gpu")]
use crate::settings::VisualStyle;
use crate::surfaces::{studs_material_table, StudsSurface};
use crate::ui::{Screen, Ui, UiInput};

struct FrameMouse((f32, f32));

impl MouseSource for FrameMouse {
    fn delta(&mut self) -> (f32, f32) {
        std::mem::replace(&mut self.0, (0.0, 0.0))
    }
}

/// Input/UI/application preferences; this resource deliberately excludes gameplay state.
struct StudsState {
    settings_path: PathBuf,
    settings: Settings,
    input: GameInput,
    ui: Ui,
    title_timer: f32,
    pending_input: InputFrame,
    repeat_input: InputFrame,
    repeat_step: bool,
    render_frame: u32,
}

impl StudsState {
    fn new(start_in_game: bool) -> Self {
        println!("STUDS - find all {PAGES_TOTAL} pages and don't get caught.");
        let settings_path = Settings::default_path();
        let settings = load_settings(&settings_path);
        let input = GameInput::with_mouse_look(
            settings.bindings.clone(),
            settings.mouse_sensitivity,
            settings.mouse_look_on_start,
            settings.invert_y,
        );
        let mut ui = Ui::new();
        if start_in_game {
            ui.start_playing();
        }
        Self {
            settings_path,
            settings,
            input,
            ui,
            title_timer: 0.0,
            pending_input: InputFrame::default(),
            repeat_input: InputFrame::default(),
            repeat_step: false,
            render_frame: 0,
        }
    }
}

enum RendererImpl {
    Cpu(CpuRenderer<StudsSurface>),
    /// STUDS' GPU backend exercises the engine's native mesh renderer. The legacy voxel
    /// raycaster remains available in `plugin_rendering` for compatibility/rollback, but
    /// this game no longer selects it as its GPU implementation.
    #[cfg(feature = "gpu")]
    MeshGpu(Box<OffscreenVoxelRenderer>),
}

/// Per-game renderer. Both backends consume the same extracted scene and FramePlan.
struct StudsRenderer {
    renderer: RendererImpl,
    output: Framebuffer,
    backend: RenderBackend,
}

impl StudsRenderer {
    fn new(
        width: usize,
        height: usize,
        requested: RenderBackend,
    ) -> (Self, RenderBackend, Option<String>) {
        let width_u32 = width.max(1) as u32;
        let height_u32 = height.max(1) as u32;
        let output = Framebuffer::new(width_u32 as usize, height_u32 as usize);
        if requested == RenderBackend::Gpu {
            #[cfg(feature = "gpu")]
            {
                match OffscreenVoxelRenderer::new(width_u32, height_u32) {
                    Ok(renderer) => {
                        return (
                            Self {
                                renderer: RendererImpl::MeshGpu(Box::new(renderer)),
                                output,
                                backend: RenderBackend::Gpu,
                            },
                            RenderBackend::Gpu,
                            None,
                        );
                    }
                    Err(error) => {
                        return (
                            Self {
                                renderer: RendererImpl::Cpu(CpuRenderer::new(
                                    width_u32,
                                    height_u32,
                                    StudsSurface,
                                )),
                                output,
                                backend: RenderBackend::Cpu,
                            },
                            RenderBackend::Cpu,
                            Some(format!(
                                "GPU renderer initialization failed; continuing with CPU: {error}"
                            )),
                        );
                    }
                }
            }
            #[cfg(not(feature = "gpu"))]
            {
                return (
                    Self {
                        renderer: RendererImpl::Cpu(CpuRenderer::new(
                            width_u32,
                            height_u32,
                            StudsSurface,
                        )),
                        output,
                        backend: RenderBackend::Cpu,
                    },
                    RenderBackend::Cpu,
                    Some("GPU renderer requested, but STUDS was compiled without the `gpu` feature; using CPU".to_owned()),
                );
            }
        }
        (
            Self {
                renderer: RendererImpl::Cpu(CpuRenderer::new(width_u32, height_u32, StudsSurface)),
                output,
                backend: RenderBackend::Cpu,
            },
            RenderBackend::Cpu,
            None,
        )
    }

    fn resize(&mut self, width: usize, height: usize) {
        let width = width.max(1);
        let height = height.max(1);
        if self.output.width == width && self.output.height == height {
            return;
        }
        self.output = Framebuffer::new(width, height);
        match &mut self.renderer {
            RendererImpl::Cpu(renderer) => renderer.resize(width as u32, height as u32),
            #[cfg(feature = "gpu")]
            RendererImpl::MeshGpu(renderer) => renderer.resize(width as u32, height as u32),
        }
    }

    fn render_frame(
        &mut self,
        plan: &FramePlan<Scene<'_>>,
        materials: &MaterialTable,
    ) -> Option<String> {
        match &mut self.renderer {
            RendererImpl::Cpu(renderer) => {
                renderer.render_frame(plan, materials);
                None
            }
            #[cfg(feature = "gpu")]
            RendererImpl::MeshGpu(_) => {
                Some("mesh GPU backend must render through the engine voxel-scene API".to_owned())
            }
        }
    }

    fn present(&mut self) {
        match &mut self.renderer {
            RendererImpl::Cpu(renderer) => renderer.present(&mut self.output),
            #[cfg(feature = "gpu")]
            RendererImpl::MeshGpu(_) => {}
        }
    }

    #[cfg(feature = "gpu")]
    fn render_voxel_world(
        &mut self,
        world: &mut ChunkedVoxelWorld,
        camera: Option<OffscreenCamera3D>,
        palette: &[VoxelBlockColor],
        sun: SunLight,
        objects: &[OffscreenDraw3D],
        clear_color: [f64; 4],
    ) -> Result<(), String> {
        let rendered = match &mut self.renderer {
            RendererImpl::MeshGpu(renderer) => {
                renderer.render(world, camera, palette, sun, objects, clear_color)
            }
            RendererImpl::Cpu(_) => {
                return Err("mesh rendering was requested while the CPU backend is active".into())
            }
        }
        .map_err(|error| error.to_string())?;
        self.output = rendered;
        Ok(())
    }

    fn fall_back_to_cpu(&mut self) {
        self.renderer = RendererImpl::Cpu(CpuRenderer::new(
            self.output.width.max(1) as u32,
            self.output.height.max(1) as u32,
            StudsSurface,
        ));
        self.backend = RenderBackend::Cpu;
    }
}

/// Installs the game state and stage systems into the engine's ECS application.
pub struct StudsPlugin {
    seed: Option<u32>,
    start_in_game: bool,
    backend: RenderBackend,
}

impl Default for StudsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl StudsPlugin {
    pub fn new() -> Self {
        Self {
            seed: None,
            start_in_game: false,
            backend: default_backend(),
        }
    }

    pub fn with_backend(mut self, backend: RenderBackend) -> Self {
        self.backend = backend;
        self
    }

    /// Deterministic in-game preview used by the example target.
    pub fn preview(seed: u32) -> Self {
        Self {
            seed: Some(seed),
            start_in_game: true,
            backend: default_backend(),
        }
    }
}

impl Plugin for StudsPlugin {
    fn name(&self) -> &'static str {
        "StudsPlugin"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["PlatformPlugin", "RenderingPlugin", "AudioPlugin"]
    }

    fn build(&self, app: &mut App) {
        let state = StudsState::new(self.start_in_game);
        let width = state.settings.render_width.max(1);
        let height = state.settings.render_height.max(1);
        let scale = state.settings.window_scale.max(1);
        let mut initial_game = Game::new(self.seed.unwrap_or_else(seed_now));
        initial_game.flashlight_on = state.settings.flashlight_on_start;
        let (renderer, backend, warning) = StudsRenderer::new(width, height, self.backend);

        app.insert_resource(PlatformConfig {
            title: "STUDS".to_owned(),
            width: (width as u32).saturating_mul(scale),
            height: (height as u32).saturating_mul(scale),
        });
        app.insert_resource(RenderSettings {
            backend,
            width: width as u32,
            height: height as u32,
        });
        // RenderingPlugin provides the generic ECS renderer; STUDS supplies a specialized
        // renderer because its CPU and GPU voxel paths share custom materials and overlays.
        // Remove the unused generic instance so there is exactly one active frame renderer.
        app.world.remove_resource::<CpuBackend<DefaultShader>>();
        #[cfg(feature = "gpu")]
        app.world
            .remove_resource::<plugin_rendering::ecs::GpuBackend>();
        app.insert_resource(renderer);
        app.insert_resource(state);
        game_ecs::install_game(&mut app.world, initial_game);
        app.world.spawn((AudioListener::default(),));

        if let Some(warning) = warning {
            app.world
                .resource_mut::<engine_app::LogBuffer>()
                .push(engine_app::LogLevel::Warn, warning);
        }

        #[cfg(feature = "audio")]
        match plugin_audio::rodio_backend::RodioBackend::new(crate::audio::sound_bank()) {
            Ok(backend) => {
                app.world.resource_mut::<plugin_audio::AudioOutput>().0 = Some(Box::new(backend));
            }
            Err(error) => app.world.resource_mut::<engine_app::LogBuffer>().push(
                engine_app::LogLevel::Warn,
                format!("audio output unavailable; continuing silently: {error}"),
            ),
        }

        app.add_system(Stage::PreUpdate, update_ui_and_collect_input);
        app.add_system(Stage::FixedUpdate, begin_fixed_step);
        app.add_system(Stage::FixedUpdate, game_ecs::update_player);
        app.add_system(Stage::FixedUpdate, game_ecs::collect_pages);
        app.add_system(Stage::FixedUpdate, game_ecs::update_enemy);
        app.add_system(Stage::FixedUpdate, game_ecs::resolve_run);
        app.add_system(Stage::PostUpdate, finish_game_state);
        app.add_system(Stage::PostUpdate, emit_audio_events);
        app.add_system(Stage::PostUpdate, update_window_title);
        app.add_system(Stage::Render, render_game_frame);
    }
}

/// Build the desktop app using the ECS app, platform runner, and rendering/audio plugins.
pub fn build_app(seed: Option<u32>, preview: bool) -> Result<App, Box<dyn std::error::Error>> {
    build_app_with_renderer(seed, preview, default_backend())
}

/// Build STUDS while explicitly choosing the renderer backend.
pub fn build_app_with_renderer(
    seed: Option<u32>,
    preview: bool,
    backend: RenderBackend,
) -> Result<App, Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.insert_resource(PlatformConfig::default());
    app.insert_resource(RenderSettings {
        backend,
        width: 320,
        height: 180,
    });
    app.add_plugin(engine_platform::PlatformPlugin)?;
    app.add_plugin(engine_scene::ScenePlugin)?;
    app.add_plugin(plugin_rendering::ecs::RenderingPlugin)?;
    app.add_plugin(plugin_audio::AudioPlugin)?;
    if preview {
        app.add_plugin(StudsPlugin {
            seed: Some(seed.unwrap_or(42)),
            start_in_game: true,
            backend,
        })?;
    } else {
        app.add_plugin(StudsPlugin {
            seed,
            start_in_game: false,
            backend,
        })?;
    }
    Ok(app.build()?)
}

fn default_backend() -> RenderBackend {
    #[cfg(feature = "gpu")]
    {
        RenderBackend::Gpu
    }
    #[cfg(not(feature = "gpu"))]
    {
        RenderBackend::Cpu
    }
}

fn load_settings(settings_path: &std::path::Path) -> Settings {
    let settings = Settings::load_or_default(settings_path);
    if !settings_path.exists() {
        let _ = settings.save(settings_path);
    } else if fs::read_to_string(settings_path)
        .map(|text| !text.contains("settings_version"))
        .unwrap_or(false)
    {
        let _ = fs::copy(settings_path, settings_path.with_extension("cfg.bak"));
        let _ = settings.save(settings_path);
    }
    settings
}

fn update_ui_and_collect_input(world: &mut World) {
    world.resource_scope::<StudsState, _>(|world, state| {
        world.resource_scope::<PlatformInput, _>(|world, platform_input| {
            let pressed_keys: Vec<Key> = ALL_KEYS
                .iter()
                .copied()
                .filter(|key| platform_input.just_pressed(*key))
                .collect();
            let framebuffer_size = (
                state.settings.render_width.max(1),
                state.settings.render_height.max(1),
            );
            let window_size = world
                .try_resource::<WindowHandle>()
                .map(|window| {
                    let size = window.0.inner_size();
                    (size.width, size.height)
                })
                .or_else(|| {
                    world
                        .try_resource::<PlatformConfig>()
                        .map(|config| (config.width.max(1), config.height.max(1)))
                })
                .unwrap_or((framebuffer_size.0 as u32, framebuffer_size.1 as u32));
            let viewport = engine_platform::window::viewport(window_size, framebuffer_size);
            let mouse_position = platform_input.mouse_position.and_then(|position| {
                engine_platform::window::window_to_framebuffer(
                    position,
                    &viewport,
                    framebuffer_size,
                )
            });
            let ui_input = UiInput::from_keys(pressed_keys.clone()).with_mouse(
                mouse_position,
                platform_input.mouse_buttons.left,
                platform_input.mouse_scroll,
                framebuffer_size,
            );
            let outcome = state.ui.update(&ui_input, &mut state.settings);
            if outcome.quit {
                world.send_event(AppExit);
                return;
            }
            if outcome.settings_changed {
                let _ = state.settings.save(&state.settings_path);
                state.input.set_bindings(state.settings.bindings.clone());
                state
                    .input
                    .set_mouse_sensitivity(state.settings.mouse_sensitivity);
                state.input.set_invert_y(state.settings.invert_y);
            }
            if outcome.window_changed {
                let width = state.settings.render_width.max(1);
                let height = state.settings.render_height.max(1);
                let scale = state.settings.window_scale.max(1);
                world.insert_resource(PlatformConfig {
                    title: "STUDS".to_owned(),
                    width: (width as u32).saturating_mul(scale),
                    height: (height as u32).saturating_mul(scale),
                });
                let backend = world
                    .try_resource::<RenderSettings>()
                    .map_or(RenderBackend::Cpu, |settings| settings.backend);
                world.insert_resource(RenderSettings {
                    backend,
                    width: width as u32,
                    height: height as u32,
                });
            }
            if outcome.start_game {
                state.pending_input = InputFrame::default();
                let difficulty = outcome
                    .difficulty
                    .unwrap_or_else(|| world.resource::<RunState>().difficulty);
                let mut game = Game::new_with_difficulty(seed_now(), difficulty);
                game.flashlight_on = state.settings.flashlight_on_start;
                game_ecs::install_game(world, game);
            } else if outcome.restart_game {
                state.pending_input = InputFrame::default();
                let (difficulty, flashlight_on) = {
                    let run = world.resource::<RunState>();
                    (run.difficulty, run.flashlight_on)
                };
                let mut game = Game::new_with_difficulty(seed_now(), difficulty);
                game.flashlight_on = flashlight_on;
                game_ecs::install_game(world, game);
            }

            let is_playing =
                state.ui.is_playing() && world.resource::<RunState>().state == State::Playing;
            if is_playing {
                let capture_mouse = state.input.mouse_look_active() && platform_input.focused;
                let mut mouse = FrameMouse(platform_input.mouse_delta);
                let frame_input =
                    state
                        .input
                        .poll_with_mouse_source(platform_input, &mut mouse, capture_mouse);
                if frame_input.quit {
                    world.send_event(AppExit);
                    return;
                }
                state.pending_input.merge(frame_input);
                platform_input.cursor_grab = if capture_mouse {
                    CursorGrabState::Locked
                } else {
                    CursorGrabState::Released
                };
            } else {
                state.pending_input = InputFrame::default();
                state.repeat_input = InputFrame::default();
                state.input.synchronize_keys(platform_input);
                platform_input.cursor_grab = CursorGrabState::Released;
            }
            state.render_frame = world.resource::<Time>().frame as u32;
        });
    });
}

/// Select the buffered input for this fixed step and advance run-level clocks.
fn begin_fixed_step(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    world.resource_scope::<StudsState, _>(|world, state| {
        let ui_playing = state.ui.is_playing();
        let step = world.resource_scope::<RunState, _>(|_, run| {
            if ui_playing && run.state == State::Playing {
                let input = if state.repeat_step {
                    state.repeat_input
                } else {
                    let first = std::mem::take(&mut state.pending_input);
                    state.repeat_input = first.level_only();
                    state.repeat_step = true;
                    first
                };
                let enemy_awake_before_time_advance = run.enemy_awake();
                if input.toggle_flashlight {
                    run.flashlight_on = !run.flashlight_on;
                }
                run.time += dt;
                RunStep {
                    active: true,
                    input,
                    enemy_awake_before_time_advance,
                }
            } else {
                if run.state != State::Playing {
                    run.state_t += dt;
                }
                RunStep::default()
            }
        });
        world.insert_resource(step);
    });
}

fn finish_game_state(world: &mut World) {
    world.resource_scope::<StudsState, _>(|world, state| {
        world.resource_scope::<RunState, _>(|_, run| {
            if state.ui.is_playing() && run.state != State::Playing {
                if run.state == State::Won
                    && state.settings.record_best_time(run.difficulty, run.time)
                {
                    let _ = state.settings.save(&state.settings_path);
                }
                state.ui.show_result();
            }
        });
        // FixedUpdate may execute more than once per displayed frame. Repeat held controls
        // within that frame, then let the next PreUpdate supply fresh input edges and deltas.
        state.repeat_step = false;
    });
}

fn update_window_title(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    let fps = if dt > 0.000_01 { 1.0 / dt } else { 0.0 };
    world.resource_scope::<StudsState, _>(|world, state| {
        state.title_timer += dt;
        if state.title_timer > 0.4 {
            state.title_timer = 0.0;
            let title = match state.ui.screen() {
                Screen::MainMenu => "STUDS | main menu".to_owned(),
                Screen::Difficulty => "STUDS | choose difficulty".to_owned(),
                Screen::Pause => "STUDS | paused".to_owned(),
                Screen::Settings => "STUDS | settings".to_owned(),
                Screen::Controls => "STUDS | controls".to_owned(),
                Screen::ConfirmMainMenu => "STUDS | confirm".to_owned(),
                Screen::Result | Screen::Playing => world.resource::<RunState>().window_title(fps),
            };
            if let Some(config) = world.try_resource::<PlatformConfig>().cloned() {
                world.insert_resource(PlatformConfig { title, ..config });
            }
        }
    });
}

fn render_game_frame(world: &mut World) {
    world.resource_scope::<StudsState, _>(|world, state| {
        world.resource_scope::<StudsRenderer, _>(|world, renderer| {
            let width = state.settings.render_width.max(1);
            let height = state.settings.render_height.max(1);
            renderer.resize(width, height);

            let has_game_scene = matches!(
                state.ui.screen(),
                Screen::Playing
                    | Screen::Result
                    | Screen::Pause
                    | Screen::Settings
                    | Screen::Controls
                    | Screen::ConfirmMainMenu
            );
            let apply_brightness = state.ui.is_playing()
                || matches!(
                    state.ui.screen(),
                    Screen::Result
                        | Screen::Pause
                        | Screen::Settings
                        | Screen::Controls
                        | Screen::ConfirmMainMenu
                );

            world.resource_scope::<MazeResource, _>(|world, maze| {
                world.resource_scope::<RunState, _>(|world, run| {
                    let Some((player, enemy, (pages, got))) = game_ecs::read_view_parts(world)
                    else {
                        return;
                    };
                    let style = {
                        let mut style = state
                            .settings
                            .visual_style
                            .render_style(state.settings.quality_preset);
                        style.limit_flicker = state.settings.reduce_flashing;
                        style
                    };
                    let view = GameReadView {
                        difficulty: run.difficulty,
                        world: &maze.0,
                        player: &player,
                        enemy: &enemy,
                        pages: &pages,
                        got: &got,
                        collected: run.collected,
                        time: run.time,
                        state: run.state,
                        state_t: run.state_t,
                        flashlight_on: run.flashlight_on,
                        enemy_awake_since: run.enemy_awake_since,
                    };
                    let scene = if has_game_scene {
                        view.scene_styled(state.render_frame, state.settings.hfov_radians(), style)
                    } else {
                        None
                    };
                    let mut world_layer = DrawList::new(width, height);
                    if apply_brightness {
                        view.draw_overlay(&mut world_layer, state.render_frame);
                    }
                    let ui_model: GameUiState = view.ui_state();
                    let mut ui_layer = DrawList::new(width, height);
                    if !state.ui.is_playing() {
                        state.ui.draw_model(
                            &mut ui_layer,
                            state.render_frame,
                            ui_model,
                            &state.settings,
                        );
                    } else {
                        state.ui.draw_game_hints_model(&mut ui_layer, ui_model);
                    }
                    let brightness = if apply_brightness {
                        state.settings.brightness
                    } else {
                        1.0
                    };

                    #[cfg(feature = "gpu")]
                    if renderer.backend == RenderBackend::Gpu {
                        // Only copy presentation-neutral camera/light data from the legacy
                        // scene adapter. The voxel world, chunking, greedy meshing and GPU
                        // mesh lifecycle are all owned by engine crates/plugins.
                        let (camera, flashlight) = scene
                            .as_ref()
                            .map(|scene| {
                                (
                                    studs_camera(scene.camera, width, height),
                                    scene.lighting.light_level,
                                )
                            })
                            .map_or((None, 0.0), |(camera, light)| (Some(camera), light));
                        // `view` has no destructor; its borrows end at its final use above.
                        // Drop the scene adapter explicitly because it can retain the world borrow.
                        drop(scene);

                        let objects = if camera.is_some() {
                            studs_scene_objects(
                                &enemy,
                                &pages,
                                &got,
                                run.time,
                                run.enemy_awake(),
                            )
                        } else {
                            Vec::new()
                        };
                        // `SunLight::direction` describes where light travels; the shader
                        // reverses it to obtain the surface-to-light vector.
                        let light_direction = camera
                            .map(|camera| camera.forward)
                            .unwrap_or(Vec3::new(-0.35, -0.8, -0.45));
                        let sun = SunLight {
                            direction: light_direction.normalize_or_zero(),
                            color: Vec3::splat(flashlight.max(0.0) * 3.0),
                            ambient: Vec3::splat(match state.settings.visual_style {
                                VisualStyle::Moody => 0.025,
                                VisualStyle::Classic => 0.10,
                                VisualStyle::Realistic => 0.065,
                            }),
                        };
                        let clear_color = match state.settings.visual_style {
                            VisualStyle::Moody => [0.002, 0.006, 0.004, 1.0],
                            VisualStyle::Classic => [0.018, 0.018, 0.018, 1.0],
                            VisualStyle::Realistic => [0.008, 0.009, 0.008, 1.0],
                        };
                        let palette = studs_voxel_palette(state.settings.visual_style);
                        let error = renderer.render_voxel_world(
                            maze.0.chunked_world_mut(),
                            camera,
                            &palette,
                            sun,
                            &objects,
                            clear_color,
                        );
                        if let Err(error) = error {
                            world.resource_mut::<engine_app::LogBuffer>().push(
                                engine_app::LogLevel::Warn,
                                format!("STUDS mesh renderer frame failed: {error}; falling back to CPU"),
                            );
                            renderer.output.pixels.fill(0);
                            renderer.fall_back_to_cpu();
                            if let Some(settings) = world.try_resource::<RenderSettings>().copied()
                            {
                                world.insert_resource(RenderSettings {
                                    backend: RenderBackend::Cpu,
                                    width: settings.width.max(1),
                                    height: settings.height.max(1),
                                });
                            }
                        }
                        plugin_rendering::cpu::renderer::composite_frame_layers(
                            &mut renderer.output,
                            &world_layer,
                            brightness,
                            &ui_layer,
                        );
                    } else {
                        let error = render_cpu_frame(
                            renderer,
                            scene,
                            world_layer,
                            brightness,
                            ui_layer,
                        );
                        if let Some(error) = error {
                            world.resource_mut::<engine_app::LogBuffer>().push(
                                engine_app::LogLevel::Warn,
                                format!("STUDS renderer frame failed: {error}"),
                            );
                        }
                    }

                    #[cfg(not(feature = "gpu"))]
                    {
                        let error = render_cpu_frame(
                            renderer,
                            scene,
                            world_layer,
                            brightness,
                            ui_layer,
                        );
                        if let Some(error) = error {
                            world.resource_mut::<engine_app::LogBuffer>().push(
                                engine_app::LogLevel::Warn,
                                format!("STUDS renderer frame failed: {error}"),
                            );
                        }
                    }
                    if world.try_resource::<WindowFrame>().is_some() {
                        world.resource_mut::<WindowFrame>().set_pixels(
                            renderer.output.width,
                            renderer.output.height,
                            &renderer.output.pixels,
                        );
                    }
                });
            });
        });
    });
}

fn render_cpu_frame(
    renderer: &mut StudsRenderer,
    scene: Option<Scene<'_>>,
    world_layer: DrawList,
    brightness: f32,
    ui_layer: DrawList,
) -> Option<String> {
    let plan = FramePlan {
        scene,
        world_layer,
        brightness,
        ui_layer,
    };
    let error = renderer.render_frame(&plan, studs_material_table());
    drop(plan);
    renderer.present();
    error
}

#[cfg(feature = "gpu")]
fn studs_camera(
    camera: plugin_rendering::Camera,
    width: usize,
    height: usize,
) -> OffscreenCamera3D {
    let forward = engine_math::forward_vec(camera.view);
    let aspect = width.max(1) as f32 / height.max(1) as f32;
    let fov_y = 2.0 * ((camera.hfov * 0.5).tan() / aspect.max(1.0e-4)).atan();
    OffscreenCamera3D {
        position: Vec3::new(camera.eye.x, camera.eye.z, -camera.eye.y),
        forward: Vec3::new(forward.x, forward.z, -forward.y),
        up: Vec3::Y,
        fov_y,
        near: 0.08,
        far: 160.0,
    }
}

#[cfg(feature = "gpu")]
fn studs_voxel_palette(style: VisualStyle) -> [VoxelBlockColor; 3] {
    use crate::level::{CEIL, FLOOR, WALL};
    let (floor, wall, ceiling) = match style {
        VisualStyle::Classic => ([0.26, 0.26, 0.26], [0.38, 0.38, 0.38], [0.24, 0.24, 0.24]),
        VisualStyle::Moody => (
            [0.025, 0.045, 0.032],
            [0.075, 0.13, 0.095],
            [0.04, 0.06, 0.045],
        ),
        VisualStyle::Realistic => ([0.19, 0.21, 0.19], [0.30, 0.31, 0.28], [0.22, 0.23, 0.21]),
    };
    [
        VoxelBlockColor {
            block: FLOOR,
            color: floor,
        },
        VoxelBlockColor {
            block: WALL,
            color: wall,
        },
        VoxelBlockColor {
            block: CEIL,
            color: ceiling,
        },
    ]
}

#[cfg(feature = "gpu")]
fn studs_scene_objects(
    enemy: &Enemy,
    pages: &[Vect],
    got: &[bool],
    time: f32,
    enemy_awake: bool,
) -> Vec<OffscreenDraw3D> {
    let mut objects = Vec::with_capacity(1 + pages.len());
    let enemy_position = Vec3::new(enemy.pos.x, enemy.pos.z + 0.67, -enemy.pos.y);
    let enemy_scale = if enemy_awake {
        Vec3::new(0.52, 1.34, 0.52)
    } else {
        Vec3::new(0.42, 1.08, 0.42)
    };
    objects.push(OffscreenDraw3D {
        model: Mat4::from_scale_rotation_translation(
            enemy_scale,
            Quat::from_rotation_y(enemy.yaw),
            enemy_position,
        ),
        tint: if enemy_awake {
            [0.9, 0.055, 0.035]
        } else {
            [0.16, 0.12, 0.10]
        },
    });

    for (index, page) in pages.iter().enumerate() {
        if got.get(index).copied().unwrap_or(true) {
            continue;
        }
        let phase = time * 0.9 + index as f32 * 1.7;
        let position = Vec3::new(page.x, page.z + 0.38 + phase.sin() * 0.08, -page.y);
        objects.push(OffscreenDraw3D {
            model: Mat4::from_scale_rotation_translation(
                Vec3::splat(0.22),
                Quat::from_rotation_y(phase),
                position,
            ),
            tint: [1.8, 1.35, 0.28],
        });
    }
    objects
}

fn emit_audio_events(world: &mut World) {
    let frame = world.resource_scope::<RunState, _>(|world, run| {
        let player = world
            .query::<&Player>()
            .iter()
            .next()
            .map(|(_, player)| (player.pos, player.view.psi))
            .unwrap_or((Vect::new(0.0, 0.0, 0.0), 0.0));
        let enemy_pos = world
            .query::<&Enemy>()
            .iter()
            .next()
            .map(|(_, enemy)| enemy.pos)
            .unwrap_or(player.0);
        let listener = AudioListener {
            position: player.0,
            yaw: player.1,
        };
        if let Some((_, current)) = world.query_mut::<&mut AudioListener>().into_iter().next() {
            *current = listener;
        }
        let enemy_awake = run.enemy_awake();
        let distance = enemy_pos.dist_xy(player.0);
        let ambience = match world
            .try_resource::<StudsState>()
            .map(|state| state.ui.screen())
            .unwrap_or(Screen::MainMenu)
        {
            Screen::Playing if run.state == State::Playing && enemy_awake => Some((
                ambiences::THREAT,
                0.025 + proximity_intensity(distance, true) * 0.11,
            )),
            Screen::MainMenu
            | Screen::Pause
            | Screen::Difficulty
            | Screen::Settings
            | Screen::Controls
            | Screen::ConfirmMainMenu => Some((ambiences::MENU, 0.12)),
            _ => None,
        };
        AudioFrame {
            events: std::mem::take(&mut run.events),
            spatial_events: std::mem::take(&mut run.spatial_events),
            ambience,
            effects_gain: world
                .try_resource::<StudsState>()
                .map(|state| {
                    if state.settings.mute_all {
                        0.0
                    } else {
                        state.settings.master_volume * state.settings.effects_volume
                    }
                })
                .unwrap_or(0.0),
            ambient_gain: world
                .try_resource::<StudsState>()
                .map(|state| {
                    if state.settings.mute_all {
                        0.0
                    } else {
                        state.settings.master_volume * state.settings.ambient_volume
                    }
                })
                .unwrap_or(0.0),
            listener,
        }
    });

    for event in frame.events {
        if frame.effects_gain > 0.0 {
            world.send_event(PlaySound {
                id: sound_id(event),
                position: None,
                gain: frame.effects_gain,
            });
        }
    }
    for event in frame.spatial_events {
        if frame.effects_gain <= 0.0 {
            continue;
        }
        let pan = event.pan.clamp(-1.0, 1.0);
        let forward_amount = (1.0 - pan * pan).max(0.0).sqrt();
        let right = Vect::new(-frame.listener.yaw.sin(), frame.listener.yaw.cos(), 0.0);
        let forward = Vect::new(-frame.listener.yaw.cos(), -frame.listener.yaw.sin(), 0.0);
        let position = Vect::new(
            frame.listener.position.x + right.x * pan + forward.x * forward_amount,
            frame.listener.position.y + right.y * pan + forward.y * forward_amount,
            frame.listener.position.z,
        );
        world.send_event(PlaySound {
            id: sound_id(event.event),
            position: Some(position),
            gain: frame.effects_gain,
        });
    }
    let (id, gain) = frame
        .ambience
        .map(|(id, volume)| (Some(id), frame.ambient_gain * volume))
        .unwrap_or((None, 0.0));
    world.send_event(SetAmbience { id, gain });
}

struct AudioFrame {
    events: Vec<AudioEvent>,
    spatial_events: Vec<crate::audio::SpatialAudioEvent>,
    ambience: Option<(plugin_audio::AmbienceId, f32)>,
    effects_gain: f32,
    ambient_gain: f32,
    listener: AudioListener,
}

fn sound_id(event: AudioEvent) -> SoundId {
    match event {
        AudioEvent::Footstep => sounds::FOOTSTEP,
        AudioEvent::Jump => sounds::JUMP,
        AudioEvent::Land => sounds::LAND,
        AudioEvent::PageCollected => sounds::COLLECT,
        AudioEvent::EnemySting => sounds::STING,
        AudioEvent::EnemyFootstep => sounds::CHASE_FOOTSTEP,
        AudioEvent::Caught => sounds::CAUGHT,
        AudioEvent::Won => sounds::VICTORY,
    }
}

fn seed_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() ^ (duration.as_secs() as u32))
        .unwrap_or(12345)
}
