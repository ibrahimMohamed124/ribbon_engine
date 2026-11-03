//! UI lifecycle and render pass registration.
use crate::{Theme, UiContext, UiSettings};
use ribbon_app::{App, Stage};
use ribbon_plugins::Plugin;
pub struct UiPlugin {
    fonts: Vec<Vec<u8>>,
    font_files: Vec<std::path::PathBuf>,
    theme: Theme,
}
impl Default for UiPlugin {
    fn default() -> Self {
        Self::new()
    }
}
impl UiPlugin {
    pub fn new() -> Self {
        Self {
            fonts: vec![
                include_bytes!("../assets/fonts/NotoSans-Regular.ttf").to_vec(),
                include_bytes!("../assets/fonts/NotoSansArabic-Regular.ttf").to_vec(),
            ],
            font_files: Vec::new(),
            theme: Theme::dark(),
        }
    }
    /// Register additional font faces after the bundled Latin and Arabic fonts.
    pub fn with_fonts(mut self, fonts: Vec<Vec<u8>>) -> Self {
        self.fonts.extend(fonts);
        self
    }
    pub fn with_font_files(mut self, files: Vec<std::path::PathBuf>) -> Self {
        self.font_files = files;
        self
    }
    pub fn with_theme(mut self, t: Theme) -> Self {
        self.theme = t;
        self
    }
}
impl Plugin for UiPlugin {
    fn name(&self) -> &'static str {
        "UiPlugin"
    }
    fn dependencies(&self) -> &'static [&'static str] {
        &["PlatformPlugin", "GpuWindowPlugin"]
    }
    fn build(&self, app: &mut App) {
        let mut fonts = self.fonts.clone();
        let mut failed_font = false;
        for path in &self.font_files {
            match std::fs::read(path) {
                Ok(data) => fonts.push(data),
                Err(_) => failed_font = true,
            }
        }
        if failed_font && app.world.try_resource::<ribbon_app::LogBuffer>().is_some() {
            app.world.resource_mut::<ribbon_app::LogBuffer>().push(
                ribbon_app::LogLevel::Warn,
                "one or more UI font files could not be read; using bundled fonts",
            );
        }
        app.insert_resource(crate::TextSystem::with_fonts(fonts));
        app.insert_resource(UiContext::with_theme(self.theme.clone()))
            .insert_resource(UiSettings {
                scale: 1.,
                respect_dpi: true,
            });
        app.add_system(Stage::PreUpdate, begin);
        app.add_system(Stage::PostUpdate, end);
        if app
            .world
            .try_resource::<ribbon_render::gpu_window::GpuWindow>()
            .is_some()
        {
            app.world
                .resource_mut::<ribbon_render::gpu_window::GpuWindow>()
                .add_pass(Box::new(crate::render::UiPass::default()));
        }
    }
}
fn begin(world: &mut ribbon_app::World) {
    let mut input = crate::UiInput::default();
    let mut screen = crate::Rect {
        x: 0.0,
        y: 0.0,
        w: 1280.0,
        h: 720.0,
    };
    let settings = world
        .try_resource::<UiSettings>()
        .copied()
        .unwrap_or_default();
    let dpi_scale = world
        .try_resource::<ribbon_platform::WindowHandle>()
        .map_or(1.0, |window| window.scale_factor());
    let scale_factor = effective_scale_factor(dpi_scale, settings);
    if let Some(frame) = world.try_resource::<ribbon_platform::WindowFrame>() {
        if frame.width > 0 && frame.height > 0 {
            screen.w = frame.width as f32 / scale_factor;
            screen.h = frame.height as f32 / scale_factor;
        }
    }
    input.dt = world
        .try_resource::<ribbon_app::Time>()
        .map_or(0.0, |time| time.delta.as_secs_f32().clamp(0.0, 0.1));
    if let Some(keys) = world.try_resource::<ribbon_platform::Input>() {
        use ribbon_platform::{Key, MouseButton};
        input.mouse = keys
            .mouse_position
            .map(|(x, y)| (x / scale_factor, y / scale_factor));
        input.mouse_down = keys.mouse_button_down(MouseButton::Left);
        input.mouse_pressed = keys.mouse_button_just_pressed(MouseButton::Left);
        input.mouse_released = keys.mouse_button_just_released(MouseButton::Left);
        input.scroll = keys.mouse_scroll;
        for (key, action) in [
            (Key::Up, crate::NavAction::Up),
            (Key::Down, crate::NavAction::Down),
            (Key::Left, crate::NavAction::Left),
            (Key::Right, crate::NavAction::Right),
            (Key::Tab, crate::NavAction::Next),
            (Key::Enter, crate::NavAction::Accept),
            (Key::Space, crate::NavAction::Accept),
            (Key::Escape, crate::NavAction::Cancel),
        ] {
            if keys.just_pressed(key) {
                if key == Key::Tab
                    && (keys.is_down(Key::LeftShift) || keys.is_down(Key::RightShift))
                {
                    input.nav.push(crate::NavAction::Prev);
                } else {
                    input.nav.push(action);
                }
            }
            if keys.is_down(key)
                && matches!(
                    action,
                    crate::NavAction::Up
                        | crate::NavAction::Down
                        | crate::NavAction::Left
                        | crate::NavAction::Right
                )
            {
                input.nav_held.push(action);
            }
        }
        input.enter = keys.just_pressed(Key::Enter);
        input.escape = keys.just_pressed(Key::Escape);
        if keys.just_pressed(Key::Backspace) {
            input.backspace = true;
        }
        if keys.just_pressed(Key::Delete) {
            input.delete = true;
        }
        if keys.just_pressed(Key::Home) {
            input.home = true;
        }
        if keys.just_pressed(Key::End) {
            input.end = true;
        }
        if keys.just_pressed(Key::Left) {
            input.left = true;
        }
        if keys.just_pressed(Key::Right) {
            input.right = true;
        }
        for pad in keys.gamepads().connected_pads() {
            for (button, action) in [
                (ribbon_platform::GamepadButton::DPadUp, crate::NavAction::Up),
                (
                    ribbon_platform::GamepadButton::DPadDown,
                    crate::NavAction::Down,
                ),
                (
                    ribbon_platform::GamepadButton::DPadLeft,
                    crate::NavAction::Left,
                ),
                (
                    ribbon_platform::GamepadButton::DPadRight,
                    crate::NavAction::Right,
                ),
                (
                    ribbon_platform::GamepadButton::South,
                    crate::NavAction::Accept,
                ),
                (
                    ribbon_platform::GamepadButton::East,
                    crate::NavAction::Cancel,
                ),
                (
                    ribbon_platform::GamepadButton::LeftBumper,
                    crate::NavAction::Prev,
                ),
                (
                    ribbon_platform::GamepadButton::RightBumper,
                    crate::NavAction::Next,
                ),
            ] {
                if keys.gamepads().just_pressed(pad, button) {
                    input.nav.push(action);
                }
                if keys.gamepads().button_down(pad, button)
                    && matches!(
                        action,
                        crate::NavAction::Up
                            | crate::NavAction::Down
                            | crate::NavAction::Left
                            | crate::NavAction::Right
                    )
                {
                    input.nav_held.push(action);
                }
            }
            let stick_x = keys
                .gamepads()
                .axis(pad, ribbon_platform::GamepadAxis::LeftStickX);
            let stick_y = keys
                .gamepads()
                .axis(pad, ribbon_platform::GamepadAxis::LeftStickY);
            if stick_x <= -0.5 {
                input.nav_held.push(crate::NavAction::Left);
            }
            if stick_x >= 0.5 {
                input.nav_held.push(crate::NavAction::Right);
            }
            if stick_y >= 0.5 {
                input.nav_held.push(crate::NavAction::Up);
            }
            if stick_y <= -0.5 {
                input.nav_held.push(crate::NavAction::Down);
            }
        }
    }
    if let Some(text) = world.try_resource::<ribbon_platform::TextInput>() {
        input
            .committed_text
            .extend(text.committed().iter().cloned());
        input.preedit = text.preedit().map(str::to_owned);
    }
    if world.try_resource::<UiContext>().is_some() {
        let c = world.resource_mut::<UiContext>();
        c.screen = screen;
        c.input = input;
        c.begin();
    }
}
fn end(world: &mut ribbon_app::World) {
    if world.try_resource::<UiContext>().is_some() {
        let (ime_active, ime_rect) = {
            let context = world.resource_mut::<UiContext>();
            context.finish_frame();
            if context.input.mouse_released {
                context.down = None;
            }
            context.focus.previous = std::mem::take(&mut context.focus.current);
            context.previous_kinds = std::mem::take(&mut context.current_kinds);
            (context.ime_active, context.ime_rect)
        };
        if let Some(window) = world.try_resource::<ribbon_platform::WindowHandle>() {
            window.set_ime_allowed(ime_active);
            if let Some(rect) = ime_rect {
                let settings = world
                    .try_resource::<UiSettings>()
                    .copied()
                    .unwrap_or_default();
                let scale = f64::from(effective_scale_factor(window.scale_factor(), settings));
                window.set_ime_cursor_area_pixels(
                    (
                        (rect.x as f64 * scale).round() as i32,
                        (rect.y as f64 * scale).round() as i32,
                    ),
                    (
                        (rect.w as f64 * scale).ceil().max(1.0) as u32,
                        (rect.h as f64 * scale).ceil().max(1.0) as u32,
                    ),
                );
            }
        }
    }
}

fn effective_scale_factor(dpi_scale: f64, settings: UiSettings) -> f32 {
    let dpi = if settings.respect_dpi && dpi_scale.is_finite() {
        dpi_scale.clamp(0.25, 8.0) as f32
    } else {
        1.0
    };
    let user = if settings.scale.is_finite() {
        settings.scale.clamp(0.5, 3.0)
    } else {
        1.0
    };
    (dpi * user).clamp(0.25, 8.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_ime_share_the_same_dpi_and_user_scale() {
        let settings = UiSettings {
            scale: 1.5,
            respect_dpi: true,
        };
        assert!((effective_scale_factor(2.0, settings) - 3.0).abs() < f32::EPSILON);
        let settings = UiSettings {
            scale: 1.5,
            respect_dpi: false,
        };
        assert!((effective_scale_factor(2.0, settings) - 1.5).abs() < f32::EPSILON);
        let settings = UiSettings {
            scale: f32::NAN,
            respect_dpi: true,
        };
        assert!((effective_scale_factor(2.0, settings) - 2.0).abs() < f32::EPSILON);
    }
}
