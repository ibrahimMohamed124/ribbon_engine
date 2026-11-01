//! STUDS application setup and ECS plugin integration.
//!
//! Runtime-owned world state lives in ECS resources and entities: `RunState` and
//! `MazeResource` are resources, while `Player`, `Enemy`, and `PagePickup` are
//! independently addressable entities. The legacy `Game` type is used only to
//! generate a new run and as a compatibility/domain API for existing consumers.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ribbon_app::{App, AppExit, Stage, Time, World};
use ribbon_events::WorldEventsExt;
use ribbon_math::Vect;
use ribbon_platform::input::{Key, MouseSource, ALL_KEYS};
use ribbon_platform::runtime::CursorGrabState;
use ribbon_platform::{Input as PlatformInput, PlatformConfig, WindowFrame, WindowHandle};
use ribbon_plugin_audio::{AudioListener, PlaySound, SetAmbience, SoundId};
#[cfg(feature = "gpu")]
use ribbon_plugin_render3d::{
    glam::{Mat4, Quat, Vec3},
    SunLight,
};
use ribbon_plugins::{AppPluginExt, Plugin};
use ribbon_render::{DrawList, FramePlan, Framebuffer, Renderer};
use studs_cpu_renderer::cpu::renderer::CpuRenderer;
use studs_cpu_renderer::ecs::{CpuBackend, DefaultShader, RenderSettings};
use studs_cpu_renderer::material::MaterialTable;
use studs_cpu_renderer::Scene;
#[cfg(feature = "gpu")]
use studs_voxel::ChunkedVoxelWorld;
#[cfg(feature = "gpu")]
use studs_voxel_gpu::{
    OffscreenCamera3D, OffscreenDraw3D, OffscreenVoxelRenderer, VoxelBlockColor,
};

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
use ribbon_platform::input::BufferedInput;

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
    pending_input: BufferedInput<InputFrame>,
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
            pending_input: BufferedInput::default(),
            repeat_input: InputFrame::default(),
            repeat_step: false,
            render_frame: 0,
        }
    }
}

/// Game-local renderer selection. This does not configure `studs_cpu_renderer`,
/// whose generic ECS integration is intentionally CPU-only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StudsRendererMode {
    #[cfg_attr(not(feature = "gpu"), default)]
    Cpu,
    #[cfg_attr(feature = "gpu", default)]
    Gpu,
}

enum RendererImpl {
    Cpu(CpuRenderer<StudsSurface>),
    /// STUDS GPU rendering uses the engine-owned mesh renderer; game mode selection is
    /// intentionally separate from the legacy CPU ECS renderer API.
    #[cfg(feature = "gpu")]
    MeshGpu(Box<OffscreenVoxelRenderer>),
}

/// Per-game renderer. CPU rendering consumes the game scene; GPU rendering consumes
/// the engine-owned chunk-mesh scene API.
struct StudsRenderer {
    renderer: RendererImpl,
    output: Framebuffer,
    #[cfg(feature = "gpu")]
    backend: StudsRendererMode,
}

impl StudsRenderer {
    fn new(width: usize, height: usize, requested: StudsRendererMode) -> (Self, Option<String>) {
        let width_u32 = width.max(1) as u32;
        let height_u32 = height.max(1) as u32;
        let output = Framebuffer::new(width_u32 as usize, height_u32 as usize);

        if requested == StudsRendererMode::Gpu {
            #[cfg(feature = "gpu")]
            {
                match OffscreenVoxelRenderer::new(width_u32, height_u32) {
                    Ok(renderer) => {
                        return (
                            Self {
                                renderer: RendererImpl::MeshGpu(Box::new(renderer)),
                                output,
                                backend: StudsRendererMode::Gpu,
                            },
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
                                backend: StudsRendererMode::Cpu,
                            },
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
                    },
                    Some(
                        "GPU renderer requested, but STUDS was compiled without the `gpu` feature; using CPU"
                            .to_owned(),
                    ),
                );
            }
        }

        (
            Self {
                renderer: RendererImpl::Cpu(CpuRenderer::new(width_u32, height_u32, StudsSurface)),
                output,
                #[cfg(feature = "gpu")]
                backend: StudsRendererMode::Cpu,
            },
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

    #[cfg(feature = "gpu")]
    fn fall_back_to_cpu(&mut self) {
        self.renderer = RendererImpl::Cpu(CpuRenderer::new(
            self.output.width.max(1) as u32,
            self.output.height.max(1) as u32,
            StudsSurface,
        ));
        self.backend = StudsRendererMode::Cpu;
    }
}

/// Installs the game state and stage systems into the engine's ECS application.
pub struct StudsPlugin {
    seed: Option<u32>,
    start_in_game: bool,
    backend: StudsRendererMode,
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

    pub fn with_backend(mut self, backend: StudsRendererMode) -> Self {
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
        let (renderer, warning) = StudsRenderer::new(width, height, self.backend);

        app.insert_resource(PlatformConfig {
            title: "STUDS".to_owned(),
            width: (width as u32).saturating_mul(scale),
            height: (height as u32).saturating_mul(scale),
        });
        app.insert_resource(RenderSettings {
            width: width as u32,
            height: height as u32,
        });
        // RenderingPlugin installs a generic CPU placeholder. STUDS owns its game frame
        // renderer; its GPU route uses STUDS-owned `studs_voxel_gpu::OffscreenVoxelRenderer`.
        app.world.remove_resource::<CpuBackend<DefaultShader>>();
        app.insert_resource(renderer);
        app.insert_resource(state);
        game_ecs::install_game(&mut app.world, initial_game);
        app.world.spawn((AudioListener::default(),));

        if let Some(warning) = warning {
            app.world
                .resource_mut::<ribbon_app::LogBuffer>()
                .push(ribbon_app::LogLevel::Warn, warning);
        }

        #[cfg(feature = "audio")]
        match ribbon_plugin_audio::threaded::ThreadedAudioSink::new(crate::audio::sound_bank()) {
            Ok(backend) => {
                app.world
                    .resource_mut::<ribbon_plugin_audio::AudioOutput>()
                    .0 = Some(Box::new(backend));
            }
            Err(error) => app.world.resource_mut::<ribbon_app::LogBuffer>().push(
                ribbon_app::LogLevel::Warn,
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
    backend: StudsRendererMode,
) -> Result<App, Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.insert_resource(PlatformConfig::default());
    // Bootstrap the CPU-only legacy ECS renderer with its placeholder. StudsPlugin owns the
    // game renderer and selects STUDS-owned `OffscreenVoxelRenderer` for GPU mode.
    app.insert_resource(RenderSettings {
        width: 320,
        height: 180,
    });
    app.add_plugin(ribbon_platform::PlatformPlugin)?;
    app.add_plugin(ribbon_scene::ScenePlugin)?;
    app.add_plugin(studs_cpu_renderer::ecs::RenderingPlugin)?;
    app.add_plugin(ribbon_plugin_audio::AudioPlugin)?;
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

fn default_backend() -> StudsRendererMode {
    StudsRendererMode::default()
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

mod audio;
mod render;
mod systems;
use audio::emit_audio_events;
use render::render_game_frame;
use systems::{
    begin_fixed_step, finish_game_state, update_ui_and_collect_input, update_window_title,
};

fn seed_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() ^ (duration.as_secs() as u32))
        .unwrap_or(12345)
}

#[cfg(test)]
#[path = "tests.rs"]
mod renderer_mode_tests;
