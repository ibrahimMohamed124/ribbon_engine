//! Immediate-mode editor UI and plugin shell over engine draw commands.
//!
//! The built-in UI avoids external widget frameworks; text entry is intentionally unsupported in v1.

use engine_app::App;
#[cfg(all(feature = "editor", debug_assertions))]
use engine_app::Stage;
use engine_ecs::{Entity, World};
use engine_platform::input::Key;
use engine_platform::Input;
use engine_plugins::Plugin;
use engine_render::{DrawCommand, DrawList};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Default)]
pub struct UiInput {
    pub pointer: Option<(f32, f32)>,
    pub left_down: bool,
    pub delta: (f32, f32),
    pub scroll: f32,
}

/// Immediate-mode layout and hit-testing state for one frame.
pub struct Ui {
    pub draw_list: DrawList,
    cursor: (i32, i32),
    row_height: i32,
    input: UiInput,
    clip_bottom: i32,
}
impl Ui {
    pub fn new(width: usize, height: usize, input: UiInput) -> Self {
        Self {
            draw_list: DrawList::new(width, height),
            cursor: (8, 8),
            row_height: 20,
            input,
            clip_bottom: height as i32,
        }
    }
    pub fn label(&mut self, text: &str) {
        if self.cursor.1 < self.clip_bottom {
            self.draw_list.commands.push(DrawCommand::Text {
                value: text.to_owned(),
                x: self.cursor.0,
                y: self.cursor.1,
                scale: 1,
                color: 0x00ff_ffff,
            });
        }
        self.cursor.1 += self.row_height;
    }
    pub fn button(&mut self, text: &str, width: i32) -> bool {
        let (x, y) = self.cursor;
        let hovered = self.hit(x, y, width, self.row_height);
        self.draw_list.commands.push(DrawCommand::Rect {
            x,
            y,
            w: width,
            h: self.row_height - 2,
            color: if hovered { 0x0040_6080 } else { 0x0028_3038 },
        });
        self.draw_list.commands.push(DrawCommand::Text {
            value: text.to_owned(),
            x: x + 4,
            y: y + 3,
            scale: 1,
            color: 0x00ff_ffff,
        });
        self.cursor.1 += self.row_height;
        hovered && self.input.left_down
    }
    pub fn checkbox(&mut self, label: &str, value: &mut bool) {
        let (x, y) = self.cursor;
        if self.hit(x, y, 180, self.row_height) && self.input.left_down {
            *value = !*value;
        }
        self.draw_list.commands.push(DrawCommand::Text {
            value: format!("[{}] {label}", if *value { "x" } else { " " }),
            x,
            y,
            scale: 1,
            color: 0x00ff_ffff,
        });
        self.cursor.1 += self.row_height;
    }
    /// Drag changes a value by horizontal pointer delta and clamps it to the supplied range.
    pub fn drag_value(&mut self, label: &str, value: &mut f64, range: (f64, f64), speed: f64) {
        let (x, y) = self.cursor;
        if self.hit(x, y, 180, self.row_height) && self.input.left_down {
            *value = (*value + f64::from(self.input.delta.0) * speed).clamp(range.0, range.1);
        }
        self.draw_list.commands.push(DrawCommand::Text {
            value: format!("{label}: {value:.3}"),
            x,
            y,
            scale: 1,
            color: 0x00ff_ffff,
        });
        self.cursor.1 += self.row_height;
    }
    pub fn separator(&mut self, width: i32) {
        self.draw_list.commands.push(DrawCommand::Rect {
            x: self.cursor.0,
            y: self.cursor.1 + 8,
            w: width,
            h: 1,
            color: 0x0080_8080,
        });
        self.cursor.1 += self.row_height;
    }
    pub fn begin_scroll(&mut self, height: i32) {
        self.clip_bottom = self.cursor.1 + height;
        self.cursor.1 -= (self.input.scroll * self.row_height as f32) as i32;
    }
    pub fn end_scroll(&mut self) {
        self.clip_bottom = self.draw_list.height as i32;
    }
    fn hit(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.input.pointer.is_some_and(|(px, py)| {
            px >= x as f32 && px < (x + w) as f32 && py >= y as f32 && py < (y + h) as f32
        })
    }
}

/// A fixed-layout editor panel.
pub trait Panel: Send + Sync + 'static {
    fn title(&self) -> &str;
    fn ui(&mut self, ui: &mut Ui, world: &mut World);
}
#[derive(Default)]
pub struct Selection(pub HashSet<Entity>);
#[derive(Default)]
pub struct EditorShell {
    pub visible: bool,
    pub panels: Vec<Box<dyn Panel>>,
}

/// Draw current panels and place the resulting overlay in a `DrawList` resource for the renderer.
pub fn editor_system(world: &mut World) {
    let input = world
        .try_resource::<Input>()
        .map(|input| UiInput {
            pointer: input.mouse_position,
            left_down: input.mouse_buttons.left,
            delta: input.mouse_delta,
            scroll: input.mouse_scroll,
        })
        .unwrap_or_default();
    let dimensions = world
        .try_resource::<engine_platform::PlatformConfig>()
        .map(|c| (c.width as usize, c.height as usize))
        .unwrap_or((320, 200));
    let overlay = world.resource_scope::<EditorShell, _>(|world, shell| {
        if world
            .try_resource::<Input>()
            .is_some_and(|i| i.just_pressed(Key::F1))
        {
            shell.visible = !shell.visible;
        }
        if !shell.visible {
            return None;
        }
        let mut ui = Ui::new(dimensions.0, dimensions.1, input);
        for panel in &mut shell.panels {
            ui.label(panel.title());
            panel.ui(&mut ui, world);
            ui.separator((dimensions.0 as i32 / 3).max(40));
            ui.cursor.0 = 8;
        }
        Some(ui.draw_list)
    });
    if let Some(overlay) = overlay {
        world.insert_resource(overlay);
    }
}

/// Install editor shell, selection, command history, and the PostUpdate UI pass.
pub struct EditorPlugin;
impl Plugin for EditorPlugin {
    fn name(&self) -> &'static str {
        "EditorPlugin"
    }
    fn dependencies(&self) -> &'static [&'static str] {
        &["PlatformPlugin", "ScenePlugin"]
    }
    fn build(&self, app: &mut App) {
        #[cfg(all(feature = "editor", debug_assertions))]
        {
            app.world.insert_resource(EditorShell::default());
            app.world.insert_resource(Selection::default());
            app.world
                .insert_resource(engine_commands::CommandStack::default());
            app.add_system(Stage::PostUpdate, editor_system);
        }
        #[cfg(not(all(feature = "editor", debug_assertions)))]
        let _ = app;
    }
}

#[cfg(test)]
mod tests {
    use super::{Ui, UiInput};
    use engine_render::DrawCommand;

    #[test]
    fn button_records_draw_commands_and_hits_pointer() {
        let mut ui = Ui::new(
            100,
            80,
            UiInput {
                pointer: Some((12.0, 12.0)),
                left_down: true,
                ..UiInput::default()
            },
        );
        assert!(ui.button("Run", 40));
        assert!(matches!(ui.draw_list.commands[0], DrawCommand::Rect { .. }));
        assert!(matches!(ui.draw_list.commands[1], DrawCommand::Text { .. }));
    }

    #[test]
    fn drag_widget_respects_numeric_bounds() {
        let mut ui = Ui::new(
            100,
            80,
            UiInput {
                pointer: Some((12.0, 12.0)),
                left_down: true,
                delta: (100.0, 0.0),
                ..UiInput::default()
            },
        );
        let mut value = 0.5;
        ui.drag_value("Speed", &mut value, (0.0, 1.0), 0.1);
        assert_eq!(value, 1.0);
    }
}
