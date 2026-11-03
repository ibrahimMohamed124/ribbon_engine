//! Per-frame UI context and immediate-mode widgets.
use crate::{FocusState, Id, Rect, Theme, UiDrawList, UiInput};
use std::ops::RangeInclusive;
#[derive(Clone, Copy, Debug, Default)]
pub struct Response {
    pub hovered: bool,
    pub pressed: bool,
    pub clicked: bool,
    pub changed: bool,
    pub focused: bool,
    pub submitted: bool,
    pub rect: Rect,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct UiSettings {
    pub scale: f32,
    pub respect_dpi: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub enum Anchor {
    #[default]
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Top,
    Bottom,
}
#[derive(Clone, Copy, Debug, Default)]
pub enum PanelSize {
    Fixed(f32, f32),
    Width(f32),
    #[default]
    Fit,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextDirection {
    #[default]
    Auto,
    Ltr,
    Rtl,
}
#[derive(Clone, Debug, Default)]
pub struct TextInputOptions {
    pub max_len: Option<usize>,
    pub password: bool,
    pub hint: String,
}
#[derive(Default)]
pub struct UiContext {
    pub input: UiInput,
    pub draw: UiDrawList,
    pub focus: FocusState,
    pub screen: Rect,
    pub theme: Theme,
    pub(crate) cursor: (f32, f32),
    pub(crate) frame: u64,
    pub(crate) ids: Vec<u64>,
    pub(crate) down: Option<Id>,
    pub(crate) clicked: Option<Id>,
}
impl UiContext {
    pub fn synthetic(screen: Rect, input: UiInput) -> Self {
        Self {
            input,
            screen,
            theme: Theme::dark(),
            ..Default::default()
        }
    }
    pub fn begin(&mut self) {
        self.draw.clear();
        self.cursor = (self.theme.padding, self.theme.padding);
        self.frame += 1;
        self.clicked = None;
        self.ids.clear();
        self.ids.push(0xcbf29ce484222325);
    }
}
pub struct Ui<'a> {
    pub(crate) ctx: &'a mut UiContext,
}
impl Ui<'_> {
    pub fn screen_rect(&self) -> Rect {
        self.ctx.screen
    }
    pub fn theme(&self) -> &Theme {
        &self.ctx.theme
    }
    fn id(&self, k: &str, label: &str) -> Id {
        crate::Id(crate::hash_id(
            *self.ctx.ids.last().unwrap_or(&0),
            k,
            label,
            0,
        ))
    }
    fn item(&mut self, k: &str, label: &str, h: f32, focusable: bool) -> Response {
        let id = self.id(k, label);
        let r = Rect {
            x: self.ctx.cursor.0,
            y: self.ctx.cursor.1,
            w: (self.ctx.screen.w - self.ctx.cursor.0 - self.ctx.theme.padding).max(40.0),
            h,
        };
        self.ctx.cursor.1 += h + self.ctx.theme.spacing;
        let hovered = self.ctx.input.mouse.is_some_and(|p| r.contains(p));
        let down = hovered && self.ctx.input.mouse_pressed;
        if down {
            self.ctx.down = Some(id);
        }
        let clicked = self.ctx.input.mouse_released && hovered && self.ctx.down == Some(id);
        if clicked {
            self.ctx.down = None;
            self.ctx.clicked = Some(id);
            self.ctx.focus.focused = Some(id);
        }
        let focused = self.ctx.focus.focused == Some(id);
        if focusable {
            self.ctx.focus.current.push((id, r));
        }
        Response {
            hovered,
            pressed: down,
            clicked,
            focused,
            rect: r,
            ..Default::default()
        }
    }
    pub fn label(&mut self, text: &str) -> Response {
        let r = self.item("label", text, 24.0, false);
        self.ctx.draw.rect(r.rect, self.ctx.theme.panel);
        r
    }
    pub fn heading(&mut self, text: &str) -> Response {
        let r = self.item("heading", text, 36.0, false);
        self.ctx.draw.rect(r.rect, self.ctx.theme.panel);
        r
    }
    pub fn button(&mut self, text: &str) -> Response {
        let mut r = self.item("button", text, 38.0, true);
        for a in self.ctx.input.nav.clone() {
            if a == crate::NavAction::Accept && r.focused {
                r.clicked = true;
            }
        }
        self.ctx.draw.rect(
            r.rect,
            if r.focused {
                self.ctx.theme.accent
            } else {
                self.ctx.theme.panel
            },
        );
        r
    }
    pub fn checkbox(&mut self, v: &mut bool, text: &str) -> Response {
        let mut r = self.item("checkbox", text, 32.0, true);
        if r.clicked || (r.focused && self.ctx.input.nav.contains(&crate::NavAction::Accept)) {
            *v = !*v;
            r.changed = true;
        }
        self.ctx.draw.rect(
            r.rect,
            if *v {
                self.ctx.theme.accent
            } else {
                self.ctx.theme.panel
            },
        );
        r
    }
    pub fn slider(&mut self, v: &mut f32, range: RangeInclusive<f32>, text: &str) -> Response {
        let mut r = self.item("slider", text, 32.0, true);
        if r.hovered && self.ctx.input.mouse_down {
            if let Some(p) = self.ctx.input.mouse {
                let lo = *range.start();
                let hi = *range.end();
                let n = ((p.0 - r.rect.x) / r.rect.w).clamp(0., 1.);
                let x = lo + (hi - lo) * n;
                r.changed = (*v - x).abs() > f32::EPSILON;
                *v = x;
            }
        }
        self.ctx.draw.rect(r.rect, self.ctx.theme.panel);
        r
    }
    pub fn progress_bar(&mut self, f: f32, text: Option<&str>) -> Response {
        let r = self.item("progress", text.unwrap_or(""), 24., false);
        self.ctx.draw.rect(r.rect, self.ctx.theme.panel);
        let mut fill = r.rect;
        fill.w *= f.clamp(0., 1.);
        self.ctx.draw.rect(fill, self.ctx.theme.accent);
        r
    }
    pub fn text_input(&mut self, text: &mut String, hint: &str) -> Response {
        self.text_input_with(
            text,
            TextInputOptions {
                hint: hint.to_owned(),
                ..Default::default()
            },
        )
    }
    pub fn text_input_with(&mut self, text: &mut String, opts: TextInputOptions) -> Response {
        let mut r = self.item("text", &opts.hint, 34., true);
        if r.clicked {
            self.ctx.focus.focused = Some(self.id("text", &opts.hint));
        }
        if r.focused {
            for s in &self.ctx.input.committed_text {
                for c in s.chars().filter(|c| !c.is_control()) {
                    if opts.max_len.is_none_or(|m| text.chars().count() < m) {
                        text.push(c);
                        r.changed = true;
                    }
                }
            }
            if self.ctx.input.backspace {
                r.changed = text.pop().is_some();
            }
            r.submitted = self.ctx.input.enter;
        }
        self.ctx.draw.rect(r.rect, self.ctx.theme.panel);
        r
    }
    pub fn spacing(&mut self, a: f32) {
        self.ctx.cursor.1 += a
    }
    pub fn separator(&mut self) {
        let r = self.item("sep", "", 1., false);
        self.ctx.draw.rect(r.rect, self.ctx.theme.muted)
    }
    pub fn panel(
        &mut self,
        _title: &str,
        _a: Anchor,
        _s: PanelSize,
        add: impl FnOnce(&mut Ui<'_>),
    ) {
        add(self)
    }
    pub fn horizontal(&mut self, add: impl FnOnce(&mut Ui<'_>)) {
        add(self)
    }
    pub fn vertical(&mut self, add: impl FnOnce(&mut Ui<'_>)) {
        add(self)
    }
    pub fn scroll_area(&mut self, _h: f32, add: impl FnOnce(&mut Ui<'_>)) {
        add(self)
    }
    pub fn push_id(&mut self, salt: impl std::hash::Hash, add: impl FnOnce(&mut Ui<'_>)) {
        use std::hash::Hasher;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&salt, &mut h);
        let x = h.finish();
        self.ctx.ids.push(x);
        add(self);
        self.ctx.ids.pop();
    }
    pub fn request_focus_first(&mut self) {}
    pub fn set_text_direction(&mut self, _d: TextDirection) {}
}
pub fn ui_scope(world: &mut ribbon_app::World, build: impl FnOnce(&mut Ui<'_>)) {
    world.resource_scope::<UiContext, _>(|_, ctx| {
        let mut u = Ui { ctx };
        build(&mut u);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn button_click_requires_press_then_release_inside() {
        let mut c = UiContext::synthetic(
            Rect {
                x: 0.,
                y: 0.,
                w: 300.,
                h: 300.,
            },
            UiInput {
                mouse: Some((20., 20.)),
                mouse_pressed: true,
                mouse_down: true,
                ..Default::default()
            },
        );
        c.begin();
        let _ = Ui { ctx: &mut c }.button("go");
        c.input.mouse_pressed = false;
        c.input.mouse_down = false;
        c.input.mouse_released = true;
        c.begin();
        assert!(Ui { ctx: &mut c }.button("go").clicked);
    }
    #[test]
    fn ids_are_stable_and_salted() {
        assert_eq!(
            crate::hash_id(3, "button", "ok", 1),
            crate::hash_id(3, "button", "ok", 1)
        );
        assert_ne!(
            crate::hash_id(3, "button", "ok", 1),
            crate::hash_id(4, "button", "ok", 1)
        );
    }
}
