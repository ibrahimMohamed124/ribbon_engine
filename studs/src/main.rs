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

use std::time::{SystemTime, UNIX_EPOCH};

use blocks_engine_desktop::cursor::CursorMouse;
use blocks_engine_desktop::timer::FrameTimer;
use blocks_engine_desktop::window::{
    create_window, left_mouse_down, mouse_position_in_framebuffer, pressed_keys, scroll_y,
    WindowConfig,
};
use raycast_blocks::audio::{Audio, AudioAmbience};
use raycast_blocks::game::{Game, PAGES_TOTAL};
use raycast_blocks::input::{Input, InputFrame};
use raycast_blocks::renderer::Framebuffer;
use raycast_blocks::settings::Settings;
use raycast_blocks::ui::{Screen as UiScreen, Ui, UiInput};

fn seed_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
        .unwrap_or(12345)
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

    let window_config = WindowConfig {
        title: "STUDS",
        width: settings.render_width,
        height: settings.render_height,
        scale: settings.window_scale,
    };
    let mut window = create_window(&window_config)?;
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
    let mut audio = match blocks_engine_desktop::audio::RodioBackend::new(
        raycast_blocks::audio::sound_bank(),
    ) {
        Ok(backend) => Audio::with_engine_backend(Box::new(backend), settings.master_volume),
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
    let mut timer = FrameTimer::new();
    let mut title_timer = 0.0f32;

    while window.is_open() {
        let sample = timer.begin_frame();
        let dt = sample.dt;
        let fps = sample.fps;
        frame_no = frame_no.wrapping_add(1);

        let ui_input = UiInput::from_keys(pressed_keys(&mut window)).with_mouse(
            mouse_position_in_framebuffer(&window, (fb.width, fb.height)),
            left_mouse_down(&window),
            scroll_y(&window),
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
            let window_config = WindowConfig {
                title: "STUDS",
                width: settings.render_width,
                height: settings.render_height,
                scale: settings.window_scale,
            };
            window = create_window(&window_config)?;
            window.set_cursor_visibility(true);
            fb = Framebuffer::new(settings.render_width, settings.render_height);
        }
        if ui_outcome.start_game {
            game = Game::new_with_difficulty(
                seed_now(),
                ui_outcome.difficulty.unwrap_or(game.difficulty),
            );
            game.flashlight_on = settings.flashlight_on_start;
        } else if ui_outcome.restart_game {
            game.restart(seed_now());
        }

        if ui.is_playing() && game.state == raycast_blocks::game::State::Playing {
            let capture_mouse = input.mouse_look_active() && window.is_active();
            let keys = blocks_engine_desktop::keys::WindowKeys(&window);
            let frame_input = input.poll_with_mouse_source(&keys, &mut mouse, capture_mouse);
            if frame_input.quit {
                break;
            }
            game.update(&frame_input, dt);
            if game.state != raycast_blocks::game::State::Playing {
                if game.state == raycast_blocks::game::State::Won
                    && settings.record_best_time(game.difficulty, game.time)
                {
                    let _ = settings.save(&settings_path);
                }
                ui.show_result();
            }
        } else {
            let keys = blocks_engine_desktop::keys::WindowKeys(&window);
            input.synchronize_keys(&keys);
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
            | UiScreen::Difficulty
            | UiScreen::Settings
            | UiScreen::Controls
            | UiScreen::ConfirmMainMenu => (Some(AudioAmbience::Menu), 0.12),
            _ => (None, 0.0),
        };
        audio.set_ambience(ambience, ambience_volume);
        audio.play_all(&mut game.events);
        audio.play_spatial_all(&mut game.spatial_events);
        if ui.is_playing() {
            game.draw(&mut fb, frame_no, settings.hfov_radians());
            blocks_engine::gfx::post::adjust_brightness(&mut fb, settings.brightness);
        } else {
            if ui.screen() == UiScreen::Result {
                game.update(&InputFrame::default(), dt);
                game.draw(&mut fb, frame_no, settings.hfov_radians());
                blocks_engine::gfx::post::adjust_brightness(&mut fb, settings.brightness);
            } else if matches!(
                ui.screen(),
                UiScreen::Pause
                    | UiScreen::Settings
                    | UiScreen::Controls
                    | UiScreen::ConfirmMainMenu
            ) {
                game.draw(&mut fb, frame_no, settings.hfov_radians());
                blocks_engine::gfx::post::adjust_brightness(&mut fb, settings.brightness);
            }
            ui.draw(&mut fb, frame_no, &game, &settings);
        }
        if ui.is_playing() {
            ui.draw_game_hints(&mut fb, &game);
        }

        window.update_with_buffer(&fb.pixels, fb.width, fb.height)?;

        // Window events refresh the cached mouse position during buffer update.
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
                UiScreen::Difficulty => "STUDS | choose difficulty".to_owned(),
                UiScreen::Pause => "STUDS | paused".to_owned(),
                UiScreen::Settings => "STUDS | settings".to_owned(),
                UiScreen::Controls => "STUDS | controls".to_owned(),
                UiScreen::ConfirmMainMenu => "STUDS | confirm".to_owned(),
                UiScreen::Result => game.window_title(fps),
                UiScreen::Playing => game.window_title(fps),
            };
            window.set_title(&title);
        }

        timer.cap_frame();
    }
    mouse.release(&mut window);
    Ok(())
}
