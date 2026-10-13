// STUDS - a monochrome, Roblox-flavoured horror game rendered by a software raycaster.
//
// You are trapped in an endless blocky maze (studded baseplate floor, brick walls).
// Find all 6 glowing pages. Something tall and blocky is hunting you:
// it crawls while you look at it, and runs the moment you look away.
//
// Default controls (rebind them in settings.cfg, see `Settings::default_path`)
//   i / k           walk forward / back        j / l   strafe left / right
//   w / s  (Up/Down)   look up / down          a / d   (Left/Right)  look left / right
//   Left Shift      sprint (limited stamina)   e       jump
//   f               flashlight on / off        r       restart (after dying / winning)
//   Esc / q         quit
//
// This file is only the OS glue: window, clock, and the frame loop.
// All game logic lives in the library crate (see lib.rs for the module map).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use minifb::{MouseMode, Scale, ScaleMode, Window, WindowOptions};

use raycast_blocks::audio::Audio;
use raycast_blocks::game::{Game, PAGES_TOTAL};
use raycast_blocks::input::Input;
use raycast_blocks::renderer::Framebuffer;
use raycast_blocks::settings::Settings;

const TARGET_FRAME: Duration = Duration::from_micros(16_000);

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("STUDS - find all {PAGES_TOTAL} pages and don't get caught.");
    println!("i/k/j/l move | w/s/a/d or arrows look | shift sprint | e jump | f flashlight | r restart | esc quit");

    // settings (write the defaults out once so the file is there to edit)
    let settings_path = Settings::default_path();
    let settings = Settings::load_or_default(&settings_path);
    if !settings_path.exists() {
        let _ = settings.save(&settings_path);
    }

    let mut window = Window::new(
        "STUDS",
        settings.render_width,
        settings.render_height,
        WindowOptions {
            scale: scale_from(settings.window_scale),
            scale_mode: ScaleMode::AspectRatioStretch,
            resize: true,
            ..WindowOptions::default()
        },
    )?;

    let mut game = Game::new(seed_now());
    game.flashlight_on = settings.flashlight_on_start;
    let mut fb = Framebuffer::new(settings.render_width, settings.render_height);
    let mut input = Input::new(settings.bindings.clone(), settings.mouse_sensitivity);
    let mut audio = Audio::null(settings.master_volume);
    let hfov = settings.hfov_radians();

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

        let frame_input = input.poll(&window, window.get_mouse_pos(MouseMode::Pass));
        if frame_input.quit {
            break;
        }
        if frame_input.restart && game.can_restart() {
            game.restart(seed_now());
        }

        game.update(&frame_input, dt);
        audio.play_all(&mut game.events);
        game.draw(&mut fb, frame_no, hfov);

        window.update_with_buffer(&fb.pixels, fb.width, fb.height)?;

        title_timer += dt;
        if title_timer > 0.4 {
            title_timer = 0.0;
            window.set_title(&game.window_title(fps));
        }

        let elapsed = frame_start.elapsed();
        if elapsed < TARGET_FRAME {
            std::thread::sleep(TARGET_FRAME - elapsed);
        }
    }
    Ok(())
}
