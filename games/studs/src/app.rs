//! STUDS application setup and ECS plugin integration.
//!
//! Gameplay is advanced by engine schedule systems. The desktop event loop, input snapshot,
//! and native window are owned by `engine_platform`; rendering uses `plugin_rendering`'s CPU
//! renderer until the game's scene is migrated to the fully ECS-native render extraction path.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use engine_app::{App, AppExit, Stage, Time, World};
use engine_events::WorldEventsExt;
use engine_math::Vect;
use engine_platform::input::{Key, KeyState, MouseSource, ALL_KEYS};
use engine_platform::runtime::CursorGrabState;
use engine_platform::{Input as PlatformInput, PlatformConfig, WindowFrame, WindowHandle};
use engine_plugins::{AppPluginExt, Plugin};
use engine_render::{DrawList, FramePlan, Framebuffer, Renderer};
use plugin_audio::{AudioListener, AudioOutput, PlaySound, SetAmbience, SoundId};
use plugin_rendering::cpu::renderer::CpuRenderer;
use plugin_rendering::ecs::{RenderBackend, RenderSettings};
use plugin_rendering::material::MaterialTable;

use crate::audio::{ambiences, sounds, AudioEvent};
use crate::game::{Game, State, PAGES_TOTAL};
use crate::input::{Input as GameInput, InputFrame};
use crate::settings::Settings;
use crate::surfaces::{studs_material_table, StudsSurface};
use crate::ui::{Screen, Ui, UiInput};

struct FrameMouse((f32, f32));

impl MouseSource for FrameMouse {
    fn delta(&mut self) -> (f32, f32) {
        std::mem::replace(&mut self.0, (0.0, 0.0))
    }
}

/// Game-owned state shared by the registered update/render systems.
struct StudsState {
    settings_path: PathBuf,
    settings: Settings,
    game: Game,
    input: GameInput,
    ui: Ui,
    title_timer: f32,
    pending_input: InputFrame,
    repeat_input: InputFrame,
    repeat_step: bool,
    render_frame: u32,
}

impl StudsState {
    fn new(seed: Option<u32>, start_in_game: bool) -> Self {
        println!("STUDS - find all {PAGES_TOTAL} pages and don't get caught.");
        let settings_path = Settings::default_path();
        let settings = load_settings(&settings_path);

        let mut game = Game::new(seed.unwrap_or_else(seed_now));
        game.flashlight_on = settings.flashlight_on_start;
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
            game,
            input,
            ui,
            title_timer: 0.0,
            pending_input: InputFrame::default(),
            repeat_input: InputFrame::default(),
            repeat_step: false,
            render_frame: 0,
        }
    }

    fn title(&self, fps: f32) -> String {
        match self.ui.screen() {
            Screen::MainMenu => "STUDS | main menu".to_owned(),
            Screen::Difficulty => "STUDS | choose difficulty".to_owned(),
            Screen::Pause => "STUDS | paused".to_owned(),
            Screen::Settings => "STUDS | settings".to_owned(),
            Screen::Controls => "STUDS | controls".to_owned(),
            Screen::ConfirmMainMenu => "STUDS | confirm".to_owned(),
            Screen::Result | Screen::Playing => self.game.window_title(fps),
        }
    }
}

/// Per-game CPU renderer state. The platform runner only receives the neutral `WindowFrame`.
struct StudsRenderer {
    renderer: CpuRenderer<StudsSurface>,
    output: Framebuffer,
}

impl StudsRenderer {
    fn new(width: usize, height: usize) -> Self {
        Self {
            renderer: CpuRenderer::new(width as u32, height as u32, StudsSurface),
            output: Framebuffer::new(width, height),
        }
    }
}

/// Installs the game state and stage systems into the engine's ECS application.
pub struct StudsPlugin {
    seed: Option<u32>,
    start_in_game: bool,
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
        }
    }

    /// Deterministic in-game preview used by the example target.
    pub fn preview(seed: u32) -> Self {
        Self {
            seed: Some(seed),
            start_in_game: true,
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
        let state = StudsState::new(self.seed, self.start_in_game);
        let width = state.settings.render_width.max(1);
        let height = state.settings.render_height.max(1);
        let scale = state.settings.window_scale.max(1);

        app.insert_resource(PlatformConfig {
            title: "STUDS".to_owned(),
            width: (width as u32).saturating_mul(scale),
            height: (height as u32).saturating_mul(scale),
        });
        app.insert_resource(RenderSettings {
            backend: RenderBackend::Cpu,
            width: width as u32,
            height: height as u32,
        });
        app.insert_resource(StudsRenderer::new(width, height));
        app.insert_resource(state);
        app.world.spawn((AudioListener::default(),));

        #[cfg(feature = "audio")]
        match plugin_audio::rodio_backend::RodioBackend::new(crate::audio::sound_bank()) {
            Ok(backend) => {
                app.world.resource_mut::<AudioOutput>().0 = Some(Box::new(backend));
            }
            Err(error) => app.world.resource_mut::<engine_app::LogBuffer>().push(
                engine_app::LogLevel::Warn,
                format!("audio output unavailable; continuing silently: {error}"),
            ),
        }

        app.add_system(Stage::PreUpdate, update_ui_and_collect_input);
        app.add_system(Stage::FixedUpdate, fixed_game_update);
        app.add_system(Stage::PostUpdate, finish_game_state);
        app.add_system(Stage::PostUpdate, emit_audio_events);
        app.add_system(Stage::PostUpdate, update_window_title);
        app.add_system(Stage::Render, render_game_frame);
    }
}

/// Build the complete desktop app using the ECS app, platform runner, and rendering/audio plugins.
pub fn build_app(seed: Option<u32>, preview: bool) -> Result<App, Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.insert_resource(PlatformConfig::default());
    app.insert_resource(RenderSettings {
        backend: RenderBackend::Cpu,
        width: 320,
        height: 180,
    });
    app.add_plugin(engine_platform::PlatformPlugin)?;
    app.add_plugin(engine_scene::ScenePlugin)?;
    app.add_plugin(plugin_rendering::ecs::RenderingPlugin)?;
    app.add_plugin(plugin_audio::AudioPlugin)?;
    if preview {
        app.add_plugin(StudsPlugin::preview(seed.unwrap_or(42)))?;
    } else {
        app.add_plugin(StudsPlugin { seed, start_in_game: false })?;
    }
    Ok(app.build()?)
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
                    world.try_resource::<PlatformConfig>().map(|config| {
                        (config.width.max(1), config.height.max(1))
                    })
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
                world.insert_resource(RenderSettings {
                    backend: RenderBackend::Cpu,
                    width: width as u32,
                    height: height as u32,
                });
            }
            if outcome.start_game {
                state.pending_input = InputFrame::default();
                state.game = Game::new_with_difficulty(
                    seed_now(),
                    outcome.difficulty.unwrap_or(state.game.difficulty),
                );
                state.game.flashlight_on = state.settings.flashlight_on_start;
            } else if outcome.restart_game {
                state.pending_input = InputFrame::default();
                state.game.restart(seed_now());
            }

            if state.ui.is_playing() && state.game.state == State::Playing {
                let capture_mouse = state.input.mouse_look_active() && platform_input.focused;
                let mut mouse = FrameMouse(platform_input.mouse_delta);
                let frame_input = state.input.poll_with_mouse_source(
                    platform_input,
                    &mut mouse,
                    capture_mouse,
                );
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

fn fixed_game_update(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    world.resource_scope::<StudsState, _>(|_, state| {
        if state.ui.is_playing() && state.game.state == State::Playing {
            let input = if state.repeat_step {
                state.repeat_input
            } else {
                let first = std::mem::take(&mut state.pending_input);
                state.repeat_input = first.level_only();
                state.repeat_step = true;
                first
            };
            state.game.update(&input, dt);
        } else if state.game.state != State::Playing {
            state.game.update(&InputFrame::default(), dt);
        }
    });
}

fn finish_game_state(world: &mut World) {
    world.resource_scope::<StudsState, _>(|_, state| {
        if state.ui.is_playing() && state.game.state != State::Playing {
            if state.game.state == State::Won
                && state
                    .settings
                    .record_best_time(state.game.difficulty, state.game.time)
            {
                let _ = state.settings.save(&state.settings_path);
            }
            state.ui.show_result();
        }
        // `FixedUpdate` may have run multiple times this frame; the next frame starts with the
        // buffered edge/delta input again while held controls are repeated only within this frame.
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
            let title = state.title(fps);
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
            if renderer.output.width != width || renderer.output.height != height {
                renderer.renderer.resize(width as u32, height as u32);
                renderer.output = Framebuffer::new(width, height);
            }

            let has_game_scene = matches!(
                state.ui.screen(),
                Screen::Playing
                    | Screen::Result
                    | Screen::Pause
                    | Screen::Settings
                    | Screen::Controls
                    | Screen::ConfirmMainMenu
            );
            let scene = if has_game_scene {
                let mut style = state
                    .settings
                    .visual_style
                    .render_style(state.settings.quality_preset);
                style.limit_flicker = state.settings.reduce_flashing;
                state.game.scene_styled(
                    state.render_frame,
                    state.settings.hfov_radians(),
                    style,
                )
            } else {
                None
            };
            let apply_brightness = state.ui.is_playing()
                || matches!(
                    state.ui.screen(),
                    Screen::Result
                        | Screen::Pause
                        | Screen::Settings
                        | Screen::Controls
                        | Screen::ConfirmMainMenu
                );
            let mut world_layer = DrawList::new(width, height);
            if apply_brightness {
                state.game.draw_overlay(&mut world_layer, state.render_frame);
            }
            let mut ui_layer = DrawList::new(width, height);
            if !state.ui.is_playing() {
                state.ui.draw(
                    &mut ui_layer,
                    state.render_frame,
                    &state.game,
                    &state.settings,
                );
            } else {
                state.ui.draw_game_hints(&mut ui_layer, &state.game);
            }
            let plan = FramePlan {
                scene,
                world_layer,
                brightness: if apply_brightness {
                    state.settings.brightness
                } else {
                    1.0
                },
                ui_layer,
            };
            renderer.renderer.render_frame(&plan, studs_material_table());
            renderer.renderer.present(&mut renderer.output);
            world
                .resource_mut::<WindowFrame>()
                .set_pixels(width, height, &renderer.output.pixels);
        });
    });
}

fn emit_audio_events(world: &mut World) {
    let frame = world.resource_scope::<StudsState, _>(|world, state| {
        let listener = AudioListener {
            position: state.game.player.pos,
            yaw: state.game.player.view.psi,
        };
        for (_, current) in world.query_mut::<&mut AudioListener>() {
            *current = listener;
            break;
        }
        let ambience = match state.ui.screen() {
            Screen::Playing if state.game.state == State::Playing && state.game.enemy_awake() => {
                Some((ambiences::THREAT, 0.025 + state.game.proximity() * 0.11))
            }
            Screen::MainMenu
            | Screen::Pause
            | Screen::Difficulty
            | Screen::Settings
            | Screen::Controls
            | Screen::ConfirmMainMenu => Some((ambiences::MENU, 0.12)),
            _ => None,
        };
        AudioFrame {
            events: std::mem::take(&mut state.game.events),
            spatial_events: std::mem::take(&mut state.game.spatial_events),
            ambience,
            effects_gain: if state.settings.mute_all {
                0.0
            } else {
                state.settings.master_volume * state.settings.effects_volume
            },
            ambient_gain: if state.settings.mute_all {
                0.0
            } else {
                state.settings.master_volume * state.settings.ambient_volume
            },
            listener,
        }
    });

    for event in frame.events {
        let gain = frame.effects_gain;
        if gain > 0.0 {
            world.send_event(PlaySound {
                id: sound_id(event),
                position: None,
                gain,
            });
        }
    }
    for event in frame.spatial_events {
        let gain = frame.effects_gain;
        if gain <= 0.0 {
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
            gain,
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
