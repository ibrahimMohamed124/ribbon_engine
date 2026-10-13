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
    println!("w/a/s/d move | arrows look | shift sprint | e jump | f flashlight | r restart | tab/m mouse look | esc/q quit");

    // settings (write the defaults out once so the file is there to edit)
    let settings_path = Settings::default_path();
    let settings = Settings::load_or_default(&settings_path);
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
    // Older settings files were written with `mouse_sensitivity = 0` (the old
    // default), which silently zeroes all mouse movement. Mouse look now has its
    // own toggle, so treat 0 as "use the default".
    let mouse_sensitivity = if settings.mouse_sensitivity > 0.0 {
        settings.mouse_sensitivity
    } else {
        eprintln!("mouse_sensitivity = 0 in settings.cfg; using 0.0025 instead");
        0.0025
    };
    let mut input = Input::with_mouse_look(
        settings.bindings.clone(),
        mouse_sensitivity,
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
        let frame_input = input.poll_with_mouse_source(&window, &mut mouse, capture_mouse);
        if frame_input.quit {
            break;
        }
        if frame_input.toggle_mouse_look {
            eprintln!(
                "mouse look {}",
                if input.mouse_look_enabled() {
                    "ON"
                } else {
                    "OFF"
                }
            );
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

        // minifb refreshes its cached mouse position during update_with_buffer.
        // Sample and recenter only after that event pump, then consume this
        // delta during the next frame's input poll.
        let capture_mouse = input.mouse_look_enabled()
            && window.is_active()
            && game.state == raycast_blocks::game::State::Playing;
        mouse.update(&mut window, capture_mouse);

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
