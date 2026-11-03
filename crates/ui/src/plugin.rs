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
    pub fn with_fonts(mut self, fonts: Vec<Vec<u8>>) -> Self {
        self.fonts = fonts;
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
        app.insert_resource(UiContext {
            theme: self.theme.clone(),
            ..Default::default()
        })
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
    if let Some(frame) = world.try_resource::<ribbon_platform::WindowFrame>() {
        if frame.width > 0 && frame.height > 0 {
            screen.w = frame.width as f32;
            screen.h = frame.height as f32;
        }
    }
    if let Some(keys) = world.try_resource::<ribbon_platform::Input>() {
        use ribbon_platform::{Key, MouseButton};
        input.mouse = keys.mouse_position;
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
                input.nav.push(action);
            }
        }
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
        c.begin();
        c.screen = screen;
        c.input = input;
    }
}
fn end(world: &mut ribbon_app::World) {
    if world.try_resource::<UiContext>().is_some() {
        let c = world.resource_mut::<UiContext>();
        c.focus.previous = std::mem::take(&mut c.focus.current);
    }
}
