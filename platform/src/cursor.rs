//! Cursor hiding, recentering, and relative motion via enigo.

use enigo::{Coordinate, Enigo, Mouse, Settings as EnigoSettings};
use minifb::{MouseMode, Window};

use engine_core::input::MouseSource;

pub struct CursorMouse {
    enigo: Option<Enigo>,
    unavailable: bool,
    anchor: (i32, i32),
    reference: (i32, i32),
    delta: (f32, f32),
    captured: bool,
}

impl CursorMouse {
    pub fn new() -> Self {
        Self {
            enigo: None,
            unavailable: false,
            anchor: (0, 0),
            reference: (0, 0),
            delta: (0.0, 0.0),
            captured: false,
        }
    }

    pub fn update(&mut self, window: &mut Window, should_capture: bool) {
        self.delta = (0.0, 0.0);
        if !should_capture || self.unavailable {
            self.release(window);
            return;
        }
        if let Err(error) = self.capture_step(window) {
            eprintln!("mouse look unavailable (cursor control failed): {error}");
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
            self.anchor = match window.get_unscaled_mouse_pos(MouseMode::Discard) {
                Some((local_x, local_y)) => {
                    let (width, height) = window.get_size();
                    (
                        x + (width as f32 / 2.0 - local_x).round() as i32,
                        y + (height as f32 / 2.0 - local_y).round() as i32,
                    )
                }
                None => (x, y),
            };
            self.captured = true;
        } else {
            self.delta = ((x - self.reference.0) as f32, (y - self.reference.1) as f32);
        }

        enigo.move_mouse(self.anchor.0, self.anchor.1, Coordinate::Abs)?;
        let (px, py) = enigo.location()?;
        self.reference = if (px - self.anchor.0).abs() <= 2 && (py - self.anchor.1).abs() <= 2 {
            (px, py)
        } else {
            self.anchor
        };
        Ok(())
    }

    pub fn release(&mut self, window: &mut Window) {
        window.set_cursor_visibility(true);
        self.captured = false;
        self.delta = (0.0, 0.0);
    }
}

impl Default for CursorMouse {
    fn default() -> Self {
        Self::new()
    }
}

impl MouseSource for CursorMouse {
    fn delta(&mut self) -> (f32, f32) {
        std::mem::replace(&mut self.delta, (0.0, 0.0))
    }
}
