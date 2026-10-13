// STUDS - a monochrome, Roblox-flavoured horror game rendered by a software raycaster.
//
// You are trapped in an endless blocky maze (studded baseplate floor, brick walls).
// Find all 6 glowing pages. Something tall and blocky is hunting you:
// it crawls while you look at it, and runs the moment you look away.
//
// Default controls (rebind them in settings.cfg, see `Settings::default_path`)
//   w / s           walk forward / back        a / d   strafe left / right
//   Up / Down       look up / down             Left / Right  look left / right
//   Left Shift      sprint (limited stamina)   e       jump
//   f               flashlight on / off        r       restart (after dying / winning)
//   Esc             pause menu              q       quit
//
// This file is only the OS glue: window, clock, and the frame loop.
// All game logic lives in the library crate (see lib.rs for the module map).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use enigo::{Coordinate, Enigo, Mouse, Settings as EnigoSettings};
use minifb::{KeyRepeat, MouseButton, MouseMode, Scale, ScaleMode, Window, WindowOptions};

use raycast_blocks::audio::{Audio, AudioAmbience, AudioBackend, AudioEvent};
use raycast_blocks::game::{Game, PAGES_TOTAL};
use raycast_blocks::input::{Input, InputFrame, MouseSource};
use raycast_blocks::renderer::Framebuffer;
use raycast_blocks::settings::Settings;
use raycast_blocks::ui::{Screen as UiScreen, Ui, UiInput};

const TARGET_FRAME: Duration = Duration::from_micros(16_000);

struct RodioBackend {
    output: rodio::MixerDeviceSink,
    ambience: Option<(AudioAmbience, rodio::Player)>,
}

impl RodioBackend {
    fn new() -> Result<Self, rodio::DeviceSinkError> {
        Ok(Self {
            output: rodio::DeviceSinkBuilder::open_default_sink()?,
            ambience: None,
        })
    }
}

impl AudioBackend for RodioBackend {
    fn play(&mut self, event: AudioEvent, volume: f32) {
        use rodio::source::Source;
        use std::io::Cursor;

        let (data, gain): (&'static [u8], f32) = match event {
            AudioEvent::Footstep => (include_bytes!("../assets/audio/footstep.wav"), 0.20),
            AudioEvent::Jump => (include_bytes!("../assets/audio/jump.wav"), 0.48),
            AudioEvent::Land => (include_bytes!("../assets/audio/footstep.wav"), 0.28),
            AudioEvent::PageCollected => (include_bytes!("../assets/audio/page_collect.wav"), 0.62),
            AudioEvent::EnemySting => (include_bytes!("../assets/audio/enemy_sting.wav"), 0.68),
            AudioEvent::Caught => (include_bytes!("../assets/audio/death.wav"), 0.72),
            AudioEvent::Won => (include_bytes!("../assets/audio/victory.wav"), 0.72),
        };

        match rodio::Decoder::try_from(Cursor::new(data)) {
            Ok(source) => self.output.mixer().add(source.amplify(volume * gain)),
            Err(error) => eprintln!("could not decode {event:?} audio: {error}"),
        }
    }

    fn set_ambience(&mut self, ambience: Option<AudioAmbience>, volume: f32) {
        use rodio::source::Source;
        use std::io::Cursor;

        let Some(ambience) = ambience.filter(|_| volume > 0.0) else {
            if let Some((_, player)) = self.ambience.take() {
                player.stop();
            }
            return;
        };

        if let Some((current, player)) = &self.ambience {
            if *current == ambience {
                player.set_volume(volume);
                return;
            }
        }
        if let Some((_, player)) = self.ambience.take() {
            player.stop();
        }

        let data: &'static [u8] = match ambience {
            AudioAmbience::Menu => include_bytes!("../assets/audio/menu_ambience.wav"),
            AudioAmbience::Enemy => include_bytes!("../assets/audio/enemy_ambient.wav"),
        };
        let player = rodio::Player::connect_new(self.output.mixer());
        player.set_volume(volume);
        match rodio::Decoder::try_from(Cursor::new(data)) {
            Ok(source) => {
                player.append(source.repeat_infinite());
                self.ambience = Some((ambience, player));
            }
            Err(error) => eprintln!("could not decode {ambience:?} audio: {error}"),
        }
    }
}

/// Cursor capture built on `enigo` (minifb has no pointer lock).
///
/// Everything is measured in *screen* pixels via `Enigo::location`, so it does
/// not depend on minifb's cached mouse position (which is divided by the window
/// scale factor and only refreshes on window events). Each frame we read the
/// cursor, compute `delta = cursor - reference`, then warp it back to a fixed
/// anchor inside the window.
struct CursorMouse {
    enigo: Option<Enigo>,
    unavailable: bool,
    anchor: (i32, i32),
    reference: (i32, i32),
    delta: (f32, f32),
    captured: bool,
}

impl CursorMouse {
    fn new() -> Self {
        Self {
            enigo: None,
            unavailable: false,
            anchor: (0, 0),
            reference: (0, 0),
            delta: (0.0, 0.0),
            captured: false,
        }
    }

    fn update(&mut self, window: &mut Window, should_capture: bool) {
        self.delta = (0.0, 0.0);
        if !should_capture || self.unavailable {
            self.release(window);
            return;
        }
        if let Err(err) = self.capture_step(window) {
            eprintln!("mouse look unavailable (cursor control failed): {err}");
            self.unavailable = true;
            self.release(window);
        }
    }

    fn capture_step(&mut self, window: &mut Window) -> Result<(), Box<dyn std::error::Error>> {
        if self.enigo.is_none() {
            self.enigo = Some(Enigo::new(&EnigoSettings::default())?);
        }
        window.set_cursor_visibility(false);
        let enigo = self.enigo.as_mut().expect("initialized above");
        let (x, y) = enigo.location()?;

        if !self.captured {
            // Anchor = window centre in screen space. `get_unscaled_mouse_pos`
            // and `get_size` are both in real window pixels (unlike
            // `get_mouse_pos`, which is divided by the scale factor).
            self.anchor = match window.get_unscaled_mouse_pos(MouseMode::Discard) {
                Some((lx, ly)) => {
                    let (w, h) = window.get_size();
                    (
                        x + (w as f32 / 2.0 - lx).round() as i32,
                        y + (h as f32 / 2.0 - ly).round() as i32,
                    )
                }
                None => (x, y),
            };
            self.captured = true;
        } else {
            self.delta = ((x - self.reference.0) as f32, (y - self.reference.1) as f32);
        }

        enigo.move_mouse(self.anchor.0, self.anchor.1, Coordinate::Abs)?;
        // Some platforms round the warp target by a pixel; use where the cursor
        // really landed as next frame's reference so that never drifts the camera.
        let (px, py) = enigo.location()?;
        self.reference = if (px - self.anchor.0).abs() <= 2 && (py - self.anchor.1).abs() <= 2 {
            (px, py)
        } else {
            self.anchor
        };
        Ok(())
    }

    fn release(&mut self, window: &mut Window) {
        // `capture_step` hides the cursor before the first fallible OS call.
        // Always restore visibility on release, including a partially failed
        // capture where `captured` was never set.
        window.set_cursor_visibility(true);
        self.captured = false;
        self.delta = (0.0, 0.0);
    }
}

impl MouseSource for CursorMouse {
    fn delta(&mut self) -> (f32, f32) {
        std::mem::replace(&mut self.delta, (0.0, 0.0))
    }
}

fn seed_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
        .unwrap_or(12345)
}

fn scale_from(factor: u32) -> Scale {
    match factor {
        1 => Scale::X1,
        2 => Scale::X2,
        8 => Scale::X8,
        16 => Scale::X16,
        _ => Scale::X4,
    }
}

fn create_window(settings: &Settings) -> Result<Window, minifb::Error> {
    Window::new(
        "STUDS",
        settings.render_width,
        settings.render_height,
        WindowOptions {
            scale: scale_from(settings.window_scale),
            scale_mode: ScaleMode::AspectRatioStretch,
            resize: true,
            ..WindowOptions::default()
        },
    )
}

fn mouse_position_in_framebuffer(
    window: &Window,
    framebuffer_size: (usize, usize),
) -> Option<(f32, f32)> {
    let (x, y) = window.get_unscaled_mouse_pos(MouseMode::Discard)?;
    let (window_width, window_height) = window.get_size();
    let (framebuffer_width, framebuffer_height) = framebuffer_size;
    if window_width == 0 || window_height == 0 || framebuffer_width == 0 || framebuffer_height == 0
    {
        return None;
    }

    // Match minifb's AspectRatioStretch viewport, including any letterbox area
    // introduced when the resizable window no longer matches the render aspect.
    let scale = (window_width as f32 / framebuffer_width as f32)
        .min(window_height as f32 / framebuffer_height as f32);
    let offset_x = (window_width as f32 - framebuffer_width as f32 * scale) * 0.5;
    let offset_y = (window_height as f32 - framebuffer_height as f32 * scale) * 0.5;
    let x = (x - offset_x) / scale;
    let y = (y - offset_y) / scale;
    (x >= 0.0 && y >= 0.0 && x < framebuffer_width as f32 && y < framebuffer_height as f32)
        .then_some((x, y))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("STUDS - find all {PAGES_TOTAL} pages and don't get caught.");

    // settings (write the defaults out once so the file is there to edit)
    let settings_path = Settings::default_path();
    let mut settings = Settings::load_or_default(&settings_path);
    if !settings_path.exists() {
        let _ = settings.save(&settings_path);
    } else if std::fs::read_to_string(&settings_path)
        .map(|t| !t.contains("settings_version"))
        .unwrap_or(false)
    {
        // Old settings file: back it up and rewrite it with the new default
        // movement/look bindings (WASD move, arrows look).
        let _ = std::fs::copy(&settings_path, settings_path.with_extension("cfg.bak"));
        let _ = settings.save(&settings_path);
    }

    let mut window = create_window(&settings)?;
    window.set_cursor_visibility(true);

    let mut game = Game::new(seed_now());
    game.flashlight_on = settings.flashlight_on_start;
    let mut fb = Framebuffer::new(settings.render_width, settings.render_height);
    let mut input = Input::with_mouse_look(
        settings.bindings.clone(),
        settings.mouse_sensitivity,
        settings.mouse_look_on_start,
        settings.invert_y,
    );
    let mut mouse = CursorMouse::new();
    let mut audio = match RodioBackend::new() {
        Ok(backend) => Audio::new(Box::new(backend), settings.master_volume),
        Err(error) => {
            eprintln!("audio output unavailable; continuing silently: {error}");
            Audio::null(settings.master_volume)
        }
    };
    audio.set_effects_volume(settings.effects_volume);
    audio.set_ambient_volume(settings.ambient_volume);
    audio.set_muted(settings.mute_all);
    let mut ui = Ui::new();

    let mut frame_no: u32 = 0;
    let mut last = Instant::now();
    let mut fps = 60.0f32;
    let mut title_timer = 0.0f32;

    while window.is_open() {
        let frame_start = Instant::now();
        let dt = frame_start.duration_since(last).as_secs_f32().min(0.05);
        last = frame_start;
        if dt > 0.0 {
            fps = fps * 0.9 + (1.0 / dt) * 0.1;
        }
        frame_no = frame_no.wrapping_add(1);

        let ui_input = UiInput::from_keys(window.get_keys_pressed(KeyRepeat::No)).with_mouse(
            mouse_position_in_framebuffer(&window, (fb.width, fb.height)),
            window.get_mouse_down(MouseButton::Left),
            window.get_scroll_wheel().map_or(0.0, |(_, y)| y),
            (fb.width, fb.height),
        );
        let ui_outcome = ui.update(&ui_input, &mut settings);
        if ui_outcome.quit {
            break;
        }
        if ui_outcome.settings_changed {
            let _ = settings.save(&settings_path);
            input.set_bindings(settings.bindings.clone());
            input.set_mouse_sensitivity(settings.mouse_sensitivity);
            input.set_invert_y(settings.invert_y);
            audio.set_volume(settings.master_volume);
            audio.set_effects_volume(settings.effects_volume);
            audio.set_ambient_volume(settings.ambient_volume);
            audio.set_muted(settings.mute_all);
        }
        if ui_outcome.window_changed {
            mouse.release(&mut window);
            window = create_window(&settings)?;
            window.set_cursor_visibility(true);
            fb = Framebuffer::new(settings.render_width, settings.render_height);
        }
        if ui_outcome.start_game {
            game = Game::new(seed_now());
            game.flashlight_on = settings.flashlight_on_start;
        } else if ui_outcome.restart_game {
            game.restart(seed_now());
        }

        if ui.is_playing() && game.state == raycast_blocks::game::State::Playing {
            let capture_mouse = input.mouse_look_active() && window.is_active();
            let frame_input = input.poll_with_mouse_source(&window, &mut mouse, capture_mouse);
            if frame_input.quit {
                break;
            }
            game.update(&frame_input, dt);
            if game.state != raycast_blocks::game::State::Playing {
                ui.show_result();
            }
        } else {
            input.synchronize_keys(&window);
        }
        if !ui.is_playing() || game.state != raycast_blocks::game::State::Playing {
            mouse.release(&mut window);
        }
        let (ambience, ambience_volume) = match ui.screen() {
            UiScreen::Playing
                if game.state == raycast_blocks::game::State::Playing && game.enemy_awake() =>
            {
                (Some(AudioAmbience::Enemy), 0.025 + game.proximity() * 0.11)
            }
            UiScreen::MainMenu
            | UiScreen::Pause
            | UiScreen::Settings
            | UiScreen::Controls
            | UiScreen::ConfirmMainMenu => (Some(AudioAmbience::Menu), 0.12),
            _ => (None, 0.0),
        };
        audio.set_ambience(ambience, ambience_volume);
        audio.play_all(&mut game.events);
        if ui.is_playing() {
            game.draw(&mut fb, frame_no, settings.hfov_radians());
        } else {
            if ui.screen() == UiScreen::Result {
                game.update(&InputFrame::default(), dt);
                game.draw(&mut fb, frame_no, settings.hfov_radians());
            } else if matches!(
                ui.screen(),
                UiScreen::Pause
                    | UiScreen::Settings
                    | UiScreen::Controls
                    | UiScreen::ConfirmMainMenu
            ) {
                game.draw(&mut fb, frame_no, settings.hfov_radians());
            }
            ui.draw(&mut fb, frame_no, &game, &settings);
        }

        window.update_with_buffer(&fb.pixels, fb.width, fb.height)?;

        // minifb refreshes its cached mouse position during update_with_buffer.
        // Sample and recenter only after that event pump, then consume this
        // delta during the next frame's input poll.
        let capture_mouse = ui.is_playing()
            && input.mouse_look_active()
            && window.is_active()
            && game.state == raycast_blocks::game::State::Playing;
        mouse.update(&mut window, capture_mouse);

        title_timer += dt;
        if title_timer > 0.4 {
            title_timer = 0.0;
            let title = match ui.screen() {
                UiScreen::MainMenu => "STUDS | main menu".to_owned(),
                UiScreen::Pause => "STUDS | paused".to_owned(),
                UiScreen::Settings => "STUDS | settings".to_owned(),
                UiScreen::Controls => "STUDS | controls".to_owned(),
                UiScreen::ConfirmMainMenu => "STUDS | confirm".to_owned(),
                UiScreen::Result => game.window_title(fps),
                UiScreen::Playing => game.window_title(fps),
            };
            window.set_title(&title);
        }

        let elapsed = frame_start.elapsed();
        if elapsed < TARGET_FRAME {
            std::thread::sleep(TARGET_FRAME - elapsed);
        }
    }
    mouse.release(&mut window);
    Ok(())
}
