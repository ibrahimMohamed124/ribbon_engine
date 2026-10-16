//! STUDS application orchestration, independent of the desktop event loop.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use engine_core::app::{Ctx, Flow, WindowConfig, WindowRequest};
use engine_core::gfx::DrawList;
use engine_core::input::MouseSource;
use engine_scene::app::{App, FramePlan};
use engine_scene::material::MaterialTable;

use crate::audio::{Audio, AudioAmbience};
use crate::game::{Game, State, PAGES_TOTAL};
use crate::input::{Input, InputFrame};
use crate::settings::Settings;
use crate::surfaces::studs_material_table;
use crate::ui::{Screen, Ui, UiInput};

struct FrameMouse((f32, f32));

impl MouseSource for FrameMouse {
    fn delta(&mut self) -> (f32, f32) {
        std::mem::replace(&mut self.0, (0.0, 0.0))
    }
}

pub struct StudsApp {
    settings_path: PathBuf,
    settings: Settings,
    game: Game,
    input: Input,
    audio: Audio,
    ui: Ui,
    title_timer: f32,
    pending_input: InputFrame,
    render_frame: u32,
}

impl StudsApp {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        println!("STUDS - find all {PAGES_TOTAL} pages and don't get caught.");
        let settings_path = Settings::default_path();
        let settings = Settings::load_or_default(&settings_path);
        if !settings_path.exists() {
            let _ = settings.save(&settings_path);
        } else if fs::read_to_string(&settings_path)
            .map(|text| !text.contains("settings_version"))
            .unwrap_or(false)
        {
            let _ = fs::copy(&settings_path, settings_path.with_extension("cfg.bak"));
            let _ = settings.save(&settings_path);
        }

        let mut game = Game::new(seed_now());
        game.flashlight_on = settings.flashlight_on_start;
        let input = Input::with_mouse_look(
            settings.bindings.clone(),
            settings.mouse_sensitivity,
            settings.mouse_look_on_start,
            settings.invert_y,
        );
        let audio = match platform::audio::RodioBackend::new(crate::audio::sound_bank()) {
            Ok(backend) => Audio::with_engine_backend(Box::new(backend), settings.master_volume),
            Err(error) => {
                eprintln!("audio output unavailable; continuing silently: {error}");
                Audio::null(settings.master_volume)
            }
        };
        let mut app = Self {
            settings_path,
            settings,
            game,
            input,
            audio,
            ui: Ui::new(),
            title_timer: 0.0,
            pending_input: InputFrame::default(),
            render_frame: 0,
        };
        app.sync_audio_settings();
        Ok(app)
    }

    pub fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "STUDS".to_owned(),
            width: self.settings.render_width,
            height: self.settings.render_height,
            scale: self.settings.window_scale,
            fixed_hz: 120,
            max_steps: 10,
        }
    }

    fn sync_audio_settings(&mut self) {
        self.audio.set_effects_volume(self.settings.effects_volume);
        self.audio.set_ambient_volume(self.settings.ambient_volume);
        self.audio.set_muted(self.settings.mute_all);
    }

    fn update_title(&self, ctx: &mut Ctx<'_>) {
        let title = match self.ui.screen() {
            Screen::MainMenu => "STUDS | main menu".to_owned(),
            Screen::Difficulty => "STUDS | choose difficulty".to_owned(),
            Screen::Pause => "STUDS | paused".to_owned(),
            Screen::Settings => "STUDS | settings".to_owned(),
            Screen::Controls => "STUDS | controls".to_owned(),
            Screen::ConfirmMainMenu => "STUDS | confirm".to_owned(),
            Screen::Result | Screen::Playing => self.game.window_title(ctx.fps),
        };
        ctx.request_title(title);
    }
}

impl App for StudsApp {
    fn on_cpu_fallback(&mut self) {
        if self.settings.use_cpu_fallback_quality() {
            eprintln!(
                "CPU renderer fallback: selecting Medium quality automatically; choose a quality preset in Settings to override this."
            );
        }
    }

    fn update(&mut self, ctx: &mut Ctx<'_>) -> Flow {
        self.render_frame = ctx.frame;
        let ui_input = UiInput::from_keys(ctx.pressed_keys.to_vec()).with_mouse(
            ctx.mouse_position,
            ctx.mouse_buttons.left,
            ctx.mouse_scroll,
            (self.settings.render_width, self.settings.render_height),
        );
        let outcome = self.ui.update(&ui_input, &mut self.settings);
        if outcome.quit {
            return Flow::Quit;
        }
        if outcome.settings_changed {
            let _ = self.settings.save(&self.settings_path);
            self.input.set_bindings(self.settings.bindings.clone());
            self.input
                .set_mouse_sensitivity(self.settings.mouse_sensitivity);
            self.input.set_invert_y(self.settings.invert_y);
            self.audio.set_volume(self.settings.master_volume);
            self.sync_audio_settings();
        }
        if outcome.window_changed {
            ctx.request_window(WindowRequest {
                width: self.settings.render_width,
                height: self.settings.render_height,
                scale: self.settings.window_scale,
            });
        }
        if outcome.start_game {
            self.pending_input = InputFrame::default();
            self.game = Game::new_with_difficulty(
                seed_now(),
                outcome.difficulty.unwrap_or(self.game.difficulty),
            );
            self.game.flashlight_on = self.settings.flashlight_on_start;
        } else if outcome.restart_game {
            self.pending_input = InputFrame::default();
            self.game.restart(seed_now());
        }

        if self.ui.is_playing() && self.game.state == State::Playing {
            let capture_mouse = self.input.mouse_look_active() && ctx.window_active;
            let mut mouse = FrameMouse(ctx.mouse_delta);
            let frame_input =
                self.input
                    .poll_with_mouse_source(ctx.keys, &mut mouse, capture_mouse);
            if frame_input.quit {
                return Flow::Quit;
            }
            if ctx.fixed_steps == 0 {
                self.pending_input.merge(frame_input);
            } else {
                self.pending_input.merge(frame_input);
                let first = std::mem::take(&mut self.pending_input);
                for step in 0..ctx.fixed_steps {
                    if self.game.state != State::Playing {
                        break;
                    }
                    let input = if step == 0 { first } else { first.level_only() };
                    self.game.update(&input, ctx.fixed_dt);
                }
            }
            if self.game.state != State::Playing {
                if self.game.state == State::Won
                    && self
                        .settings
                        .record_best_time(self.game.difficulty, self.game.time)
                {
                    let _ = self.settings.save(&self.settings_path);
                }
                self.ui.show_result();
            }
        } else {
            self.pending_input = InputFrame::default();
            if self.game.state != State::Playing {
                for _ in 0..ctx.fixed_steps {
                    self.game.update(&InputFrame::default(), ctx.fixed_dt);
                }
            }
            self.input.synchronize_keys(ctx.keys);
        }

        if self.ui.is_playing() && self.game.state == State::Playing {
            if self.input.mouse_look_active() && ctx.window_active {
                ctx.capture_cursor();
            } else {
                ctx.release_cursor();
            }
        } else {
            ctx.release_cursor();
        }

        let (ambience, ambience_volume) = match self.ui.screen() {
            Screen::Playing if self.game.state == State::Playing && self.game.enemy_awake() => (
                Some(AudioAmbience::Enemy),
                0.025 + self.game.proximity() * 0.11,
            ),
            Screen::MainMenu
            | Screen::Pause
            | Screen::Difficulty
            | Screen::Settings
            | Screen::Controls
            | Screen::ConfirmMainMenu => (Some(AudioAmbience::Menu), 0.12),
            _ => (None, 0.0),
        };
        self.audio.set_ambience(ambience, ambience_volume);
        self.audio.play_all(&mut self.game.events);
        self.audio.play_spatial_all(&mut self.game.spatial_events);

        self.title_timer += ctx.dt;
        if self.title_timer > 0.4 {
            self.title_timer = 0.0;
            self.update_title(ctx);
        }
        Flow::Continue
    }

    fn frame_plan(&self, ctx: &Ctx<'_>) -> FramePlan<'_> {
        let has_game_scene = matches!(
            self.ui.screen(),
            Screen::Playing
                | Screen::Pause
                | Screen::Settings
                | Screen::Controls
                | Screen::ConfirmMainMenu
        );
        let scene = if has_game_scene {
            let mut style = self
                .settings
                .visual_style
                .render_style(self.settings.quality_preset);
            style.reduce_flashing = self.settings.reduce_flashing;
            self.game
                .scene_styled(self.render_frame, self.settings.hfov_radians(), style)
        } else {
            None
        };
        let mut world_layer =
            DrawList::new(self.settings.render_width, self.settings.render_height);
        let apply_brightness = self.ui.is_playing()
            || matches!(
                self.ui.screen(),
                Screen::Result
                    | Screen::Pause
                    | Screen::Settings
                    | Screen::Controls
                    | Screen::ConfirmMainMenu
            );
        if apply_brightness {
            self.game.draw_overlay(&mut world_layer, ctx.frame);
        }
        let mut ui_layer = DrawList::new(self.settings.render_width, self.settings.render_height);
        if !self.ui.is_playing() {
            self.ui
                .draw(&mut ui_layer, ctx.frame, &self.game, &self.settings);
        } else {
            self.ui.draw_game_hints(&mut ui_layer, &self.game);
        }
        FramePlan {
            scene,
            world_layer,
            brightness: if apply_brightness {
                self.settings.brightness
            } else {
                1.0
            },
            ui_layer,
        }
    }

    fn materials(&self) -> &'static MaterialTable {
        studs_material_table()
    }
}

fn seed_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() ^ (duration.as_secs() as u32))
        .unwrap_or(12345)
}
