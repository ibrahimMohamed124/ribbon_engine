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

use enigo::{Coordinate, Enigo, Mouse, Settings as EnigoSettings};
use minifb::{MouseMode, Scale, ScaleMode, Window, WindowOptions};

use raycast_blocks::audio::Audio;
use raycast_blocks::game::{Game, PAGES_TOTAL};
use raycast_blocks::input::{Input, MouseSource};
use raycast_blocks::renderer::Framebuffer;
use raycast_blocks::settings::Settings;

const TARGET_FRAME: Duration = Duration::from_micros(16_000);

struct CursorMouse {
    enigo: Option<Enigo>,
    delta: (f32, f32),
    captured: bool,
}

impl CursorMouse {
    fn new() -> Self {
        Self {
            enigo: None,
            delta: (0.0, 0.0),
            captured: false,
        }
    }

    fn update(
        &mut self,
        window: &mut Window,
        should_capture: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.delta = (0.0, 0.0);
        if !should_capture {
            window.set_cursor_visibility(true);
            self.captured = false;
            return Ok(());
        }

        if self.enigo.is_none() {
            self.enigo = Some(Enigo::new(&EnigoSettings::default())?);
        }
        window.set_cursor_visibility(false);
        let (width, height) = window.get_size();
        let center = (width as f32 / 2.0, height as f32 / 2.0);
        let local_mouse = window.get_mouse_pos(MouseMode::Clamp).unwrap_or(center);
        let enigo = self.enigo.as_mut().expect("initialized above");
        let (screen_x, screen_y) = enigo.location()?;
        // Derive the client-area origin from the current pointer location so
        // window decorations do not offset the warp target.
        let target = (
            screen_x + (center.0 - local_mouse.0).round() as i32,
            screen_y + (center.1 - local_mouse.1).round() as i32,
        );

        if !self.captured {
            enigo.move_mouse(target.0, target.1, Coordinate::Abs)?;
            self.captured = true;
            return Ok(());
        }

        self.delta = (local_mouse.0 - center.0, local_mouse.1 - center.1);
        enigo.move_mouse(target.0, target.1, Coordinate::Abs)?;
        Ok(())
    }

    fn release(&mut self, window: &mut Window) {
        if self.captured {
            window.set_cursor_visibility(true);
        }
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("STUDS - find all {PAGES_TOTAL} pages and don't get caught.");
    println!("i/k/j/l move | w/s/a/d or arrows look | shift sprint | e jump | f flashlight | r restart | tab/m mouse look | esc/q quit");

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

        let capture_mouse = input.mouse_look_enabled()
            && window.is_active()
            && game.state == raycast_blocks::game::State::Playing;
        mouse.update(&mut window, capture_mouse)?;
        let frame_input = input.poll_with_mouse_source(&window, &mut mouse, capture_mouse);
        if frame_input.quit {
            break;
        }
        if frame_input.restart && game.can_restart() {
            game.restart(seed_now());
        }

        game.update(&frame_input, dt);
        if game.state != raycast_blocks::game::State::Playing {
            mouse.release(&mut window);
        }
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
    mouse.release(&mut window);
    Ok(())
}
