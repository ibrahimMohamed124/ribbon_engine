//! Per-frame UI context, stacking layouts, widgets, focus and text editing.
use crate::{Align, FocusState, Id, Padding, Rect, Theme, UiDrawList, UiInput};
use std::collections::{HashMap, HashSet};
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
#[derive(Clone, Copy, Debug)]
pub struct UiSettings {
    pub scale: f32,
    pub respect_dpi: bool,
}
impl Default for UiSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            respect_dpi: true,
        }
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayoutDirection {
    Vertical,
    Horizontal,
}
#[derive(Clone, Copy, Debug)]
struct LayoutFrame {
    rect: Rect,
    direction: LayoutDirection,
    padding: Padding,
    cursor_x: f32,
    cursor_y: f32,
    max_item_height: f32,
    max_item_width: f32,
    cross_align: Align,
    next_item_width: Option<f32>,
}
impl LayoutFrame {
    fn new(rect: Rect, direction: LayoutDirection, padding: Padding) -> Self {
        Self::new_aligned(rect, direction, padding, Align::Start)
    }
    fn new_aligned(
        rect: Rect,
        direction: LayoutDirection,
        padding: Padding,
        cross_align: Align,
    ) -> Self {
        Self {
            rect,
            direction,
            padding,
            cursor_x: rect.x + padding.left,
            cursor_y: rect.y + padding.top,
            max_item_height: 0.0,
            max_item_width: 0.0,
            cross_align,
            next_item_width: None,
        }
    }
    fn content_width(self) -> f32 {
        (self.rect.w - self.padding.left - self.padding.right).max(1.0)
    }
    fn used_height(self) -> f32 {
        let bottom = match self.direction {
            LayoutDirection::Vertical => self.cursor_y,
            LayoutDirection::Horizontal => self.cursor_y + self.max_item_height,
        };
        (bottom - self.rect.y + self.padding.bottom).max(0.0)
    }
    fn used_width(self) -> f32 {
        let width = match self.direction {
            LayoutDirection::Vertical => self.max_item_width,
            LayoutDirection::Horizontal => self.cursor_x - self.rect.x,
        };
        (width + self.padding.left + self.padding.right).max(0.0)
    }
}
fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.w).min(b.x + b.w);
    let bottom = (a.y + a.h).min(b.y + b.h);
    (right > left && bottom > top).then_some(Rect {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct TextEditState {
    caret: usize,
    editing: bool,
}

#[derive(Clone, Copy, Debug)]
struct HitRegion {
    id: Id,
    rect: Rect,
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
    layouts: Vec<LayoutFrame>,
    clips: Vec<Rect>,
    previous_hit_regions: Vec<HitRegion>,
    current_hit_regions: Vec<HitRegion>,
    pub(crate) previous_kinds: HashMap<Id, String>,
    pub(crate) current_kinds: HashMap<Id, String>,
    nav_repeat_timers: [f32; 4],
    nav_repeat_active: [bool; 4],
    panel_sizes: HashMap<Id, (f32, f32)>,
    scroll_offsets: HashMap<Id, f32>,
    text_edits: HashMap<Id, TextEditState>,
    used_ids: HashSet<Id>,
    request_focus_first: bool,
    anonymous_id_sequence: u64,
    text_direction: TextDirection,
    last_mouse: Option<(f32, f32)>,
    suppress_hover: bool,
    pub(crate) ime_active: bool,
    pub(crate) ime_rect: Option<Rect>,
}
impl UiContext {
    /// Creates a fresh UI context with the supplied theme and default frame state.
    pub fn with_theme(theme: Theme) -> Self {
        Self {
            theme,
            ..Default::default()
        }
    }

    pub fn synthetic(screen: Rect, input: UiInput) -> Self {
        Self {
            input,
            screen,
            ..Self::with_theme(Theme::dark())
        }
    }
    pub fn begin(&mut self) {
        self.draw.clear();
        self.cursor = (self.theme.padding, self.theme.padding);
        self.frame += 1;
        self.clicked = None;
        self.ids.clear();
        self.ids.push(0xcbf29ce484222325);
        self.layouts.clear();
        self.clips.clear();
        self.current_hit_regions.clear();
        self.focus.current.clear();
        self.current_kinds.clear();
        self.used_ids.clear();
        self.anonymous_id_sequence = 0;
        self.text_direction = TextDirection::Auto;
        self.layouts.push(LayoutFrame::new(
            self.screen,
            LayoutDirection::Vertical,
            Padding {
                left: self.theme.padding,
                top: self.theme.padding,
                right: self.theme.padding,
                bottom: self.theme.padding,
            },
        ));
        self.ime_active = false;
        self.ime_rect = None;
        if self.input.mouse != self.last_mouse {
            self.suppress_hover = false;
        }
        self.navigate_focus();
        if !self.input.nav.is_empty() {
            self.suppress_hover = true;
        }
        self.last_mouse = self.input.mouse;
    }
    pub(crate) fn finish_frame(&mut self) {
        let used_ids = &self.used_ids;
        self.text_edits.retain(|id, _| used_ids.contains(id));
        self.panel_sizes.retain(|id, _| used_ids.contains(id));
        self.scroll_offsets.retain(|id, _| used_ids.contains(id));
        self.previous_hit_regions = std::mem::take(&mut self.current_hit_regions);
        if self.down.is_some_and(|id| !used_ids.contains(&id)) {
            self.down = None;
        }
        if self
            .focus
            .focused
            .is_some_and(|focused| !self.focus.current.iter().any(|(id, _)| *id == focused))
        {
            self.focus.focused = self.focus.current.first().map(|(id, _)| *id);
        }
    }
    fn navigate_focus(&mut self) {
        if self.focus.previous.is_empty() {
            return;
        }
        let directions = [
            crate::NavAction::Up,
            crate::NavAction::Down,
            crate::NavAction::Left,
            crate::NavAction::Right,
        ];
        let mut actions = self.input.nav.clone();
        for (index, action) in directions.into_iter().enumerate() {
            let held = self.input.nav_held.contains(&action);
            if held {
                if !self.nav_repeat_active[index] {
                    if !actions.contains(&action) {
                        actions.push(action);
                    }
                    if !self.input.nav.contains(&action) {
                        self.input.nav.push(action);
                    }
                    self.nav_repeat_active[index] = true;
                    self.nav_repeat_timers[index] = 0.35;
                } else {
                    self.nav_repeat_timers[index] -= self.input.dt.max(0.0);
                    if self.nav_repeat_timers[index] <= 0.0 {
                        if !actions.contains(&action) {
                            actions.push(action);
                        }
                        if !self.input.nav.contains(&action) {
                            self.input.nav.push(action);
                        }
                        self.nav_repeat_timers[index] = 0.1;
                    }
                }
            } else {
                self.nav_repeat_active[index] = false;
                self.nav_repeat_timers[index] = 0.0;
            }
        }
        for action in actions {
            let len = self.focus.previous.len();
            let current = self.focus.focused.and_then(|id| {
                self.focus
                    .previous
                    .iter()
                    .position(|(candidate, _)| *candidate == id)
            });
            let focused_kind = self
                .focus
                .focused
                .and_then(|id| self.previous_kinds.get(&id));
            // Left/right are value/caret edits while a slider or text field owns focus.
            let text_is_editing = self
                .focus
                .focused
                .is_some_and(|id| self.text_edits.get(&id).is_some_and(|state| state.editing));
            if matches!(action, crate::NavAction::Left | crate::NavAction::Right)
                && (focused_kind.is_some_and(|kind| kind == "slider")
                    || (focused_kind.is_some_and(|kind| kind == "text") && text_is_editing))
            {
                continue;
            }
            let next = match action {
                crate::NavAction::Next => Some(current.map_or(0, |i| (i + 1) % len)),
                crate::NavAction::Prev => Some(current.map_or(len - 1, |i| (i + len - 1) % len)),
                crate::NavAction::Up
                | crate::NavAction::Down
                | crate::NavAction::Left
                | crate::NavAction::Right => {
                    if let Some(index) = current {
                        let origin = self.focus.previous[index].1.center();
                        let mut best: Option<(usize, f32)> = None;
                        for (candidate_index, (_, rect)) in self.focus.previous.iter().enumerate() {
                            if candidate_index == index {
                                continue;
                            }
                            let center = rect.center();
                            let (primary, perpendicular) = match action {
                                crate::NavAction::Up if center.1 < origin.1 => {
                                    (origin.1 - center.1, (center.0 - origin.0).abs())
                                }
                                crate::NavAction::Down if center.1 > origin.1 => {
                                    (center.1 - origin.1, (center.0 - origin.0).abs())
                                }
                                crate::NavAction::Left if center.0 < origin.0 => {
                                    (origin.0 - center.0, (center.1 - origin.1).abs())
                                }
                                crate::NavAction::Right if center.0 > origin.0 => {
                                    (center.0 - origin.0, (center.1 - origin.1).abs())
                                }
                                _ => continue,
                            };
                            let score = primary + 2.0 * perpendicular;
                            if best.is_none_or(|(_, old)| score < old) {
                                best = Some((candidate_index, score));
                            }
                        }
                        best.map(|(i, _)| i).or_else(|| {
                            // Directional navigation wraps to the opposite edge when
                            // there is no widget farther along the requested axis.
                            // This keeps a held gamepad direction useful at row/column
                            // boundaries instead of leaving focus permanently stuck.
                            self.focus
                                .previous
                                .iter()
                                .enumerate()
                                .filter(|entry| {
                                    let (candidate_index, (_, rect)) = entry;
                                    if *candidate_index == index {
                                        return false;
                                    }
                                    let center = rect.center();
                                    match action {
                                        crate::NavAction::Up => center.1 > origin.1,
                                        crate::NavAction::Down => center.1 < origin.1,
                                        crate::NavAction::Left => center.0 > origin.0,
                                        crate::NavAction::Right => center.0 < origin.0,
                                        _ => false,
                                    }
                                })
                                .min_by(|left, right| {
                                    let a = left.1 .1.center();
                                    let b = right.1 .1.center();
                                    let (a_edge, b_edge, a_perp, b_perp) = match action {
                                        crate::NavAction::Up => (
                                            -a.1,
                                            -b.1,
                                            (a.0 - origin.0).abs(),
                                            (b.0 - origin.0).abs(),
                                        ),
                                        crate::NavAction::Down => (
                                            a.1,
                                            b.1,
                                            (a.0 - origin.0).abs(),
                                            (b.0 - origin.0).abs(),
                                        ),
                                        crate::NavAction::Left => (
                                            -a.0,
                                            -b.0,
                                            (a.1 - origin.1).abs(),
                                            (b.1 - origin.1).abs(),
                                        ),
                                        crate::NavAction::Right => (
                                            a.0,
                                            b.0,
                                            (a.1 - origin.1).abs(),
                                            (b.1 - origin.1).abs(),
                                        ),
                                        _ => (0.0, 0.0, 0.0, 0.0),
                                    };
                                    a_edge
                                        .total_cmp(&b_edge)
                                        .then_with(|| a_perp.total_cmp(&b_perp))
                                })
                                .map(|(candidate_index, _)| candidate_index)
                        })
                    } else {
                        Some(0)
                    }
                }
                crate::NavAction::Accept | crate::NavAction::Cancel => None,
            };
            if let Some(index) = next {
                self.focus.focused = Some(self.focus.previous[index].0);
            }
        }
    }
}

pub struct Ui<'a> {
    pub(crate) ctx: &'a mut UiContext,
    text: &'a mut crate::TextSystem,
}
impl Ui<'_> {
    pub fn screen_rect(&self) -> Rect {
        self.ctx.screen
    }
    pub fn theme(&self) -> &Theme {
        &self.ctx.theme
    }
    fn id(&self, kind: &str, label: &str) -> Id {
        crate::Id(crate::hash_id(
            *self.ctx.ids.last().unwrap_or(&0),
            kind,
            label,
            0,
        ))
    }
    fn rounded_rect(&mut self, rect: Rect, color: crate::Color, radius: Option<f32>) {
        self.ctx
            .draw
            .rounded_rect(rect, color, radius.unwrap_or(self.ctx.theme.corner_radius));
    }
    fn rounded_border(&mut self, rect: Rect, color: crate::Color, width: f32) {
        self.ctx
            .draw
            .rounded_border(rect, color, self.ctx.theme.corner_radius, width);
    }
    fn note_intrinsic_width(&mut self, text: &str, size: f32, extra: f32) {
        if text.is_empty() {
            return;
        }
        let available = self
            .ctx
            .layouts
            .last()
            .map_or(self.ctx.screen.w, |frame| frame.content_width());
        let (width, _) = self.text.measure(text, size, available.max(1.0));
        if let Some(frame) = self.ctx.layouts.last_mut() {
            let preferred = (width + extra).clamp(1.0, frame.content_width());
            frame.max_item_width = frame.max_item_width.max(preferred);
            frame.next_item_width = Some(preferred);
        }
    }
    fn item(&mut self, kind: &str, label: &str, height: f32, focusable: bool) -> Response {
        let id = self.id(kind, label);
        self.ctx.used_ids.insert(id);
        let Some(frame) = self.ctx.layouts.last_mut() else {
            return Response::default();
        };
        let rect = match frame.direction {
            LayoutDirection::Vertical => {
                let available = frame.content_width();
                let preferred = frame.next_item_width.take().unwrap_or(available);
                let width = if frame.cross_align == Align::Start {
                    available
                } else {
                    preferred.min(available)
                };
                let left = frame.rect.x + frame.padding.left;
                let right = frame.rect.x + frame.rect.w - frame.padding.right;
                let x = match frame.cross_align {
                    Align::Start => left,
                    Align::Center => left + (available - width) * 0.5,
                    Align::End => right - width,
                };
                // Derive the end-aligned width from the same right edge used to
                // position it. This avoids a one-ULP overflow from x + width.
                let width = if frame.cross_align == Align::End {
                    (right - x).max(0.0)
                } else {
                    width
                };
                Rect {
                    x,
                    y: frame.cursor_y,
                    w: width,
                    h: height,
                }
            }
            LayoutDirection::Horizontal => {
                let available_width =
                    (frame.rect.x + frame.rect.w - frame.padding.right - frame.cursor_x).max(1.0);
                let preferred = frame.next_item_width.take().unwrap_or(120.0);
                let width = preferred.min(available_width);
                let available_height =
                    (frame.rect.h - frame.padding.top - frame.padding.bottom).max(height);
                let y = match frame.cross_align {
                    Align::Start => frame.rect.y + frame.padding.top,
                    Align::Center => {
                        frame.rect.y + frame.padding.top + (available_height - height) * 0.5
                    }
                    Align::End => frame.rect.y + frame.padding.top + available_height - height,
                };
                Rect {
                    x: frame.cursor_x,
                    y,
                    w: width,
                    h: height,
                }
            }
        };
        match frame.direction {
            LayoutDirection::Vertical => frame.cursor_y += height + self.ctx.theme.spacing,
            LayoutDirection::Horizontal => {
                frame.cursor_x += rect.w + self.ctx.theme.spacing;
                frame.max_item_height = frame.max_item_height.max(height);
            }
        }
        let visible_rect = self
            .ctx
            .clips
            .iter()
            .try_fold(rect, |visible, clip| intersect_rect(visible, *clip));
        if focusable {
            if let Some(visible) = visible_rect {
                self.ctx
                    .current_hit_regions
                    .push(HitRegion { id, rect: visible });
            }
        }
        let hovered = !self.ctx.suppress_hover
            && self.ctx.input.mouse.is_some_and(|p| {
                let inside_visible = visible_rect.is_some_and(|visible| visible.contains(p));
                let topmost_previous = self
                    .ctx
                    .previous_hit_regions
                    .iter()
                    .rev()
                    .find(|region| region.rect.contains(p));
                inside_visible
                    && (self.ctx.previous_hit_regions.is_empty()
                        || topmost_previous.is_some_and(|region| region.id == id))
            });
        if self.ctx.input.mouse_pressed && hovered {
            self.ctx.down = Some(id);
        }
        let mut clicked = false;
        if self.ctx.input.mouse_released && self.ctx.down == Some(id) {
            clicked = hovered;
            self.ctx.down = None;
        }
        if clicked {
            self.ctx.clicked = Some(id);
            self.ctx.focus.focused = Some(id);
        }
        if focusable && self.ctx.request_focus_first {
            self.ctx.focus.focused = Some(id);
            self.ctx.request_focus_first = false;
        }
        let focused = self.ctx.focus.focused == Some(id);
        if focusable {
            self.ctx.focus.current.push((id, rect));
            self.ctx.current_kinds.insert(id, kind.to_owned());
        }
        Response {
            hovered,
            pressed: self.ctx.down == Some(id) && self.ctx.input.mouse_down,
            clicked,
            focused,
            rect,
            ..Default::default()
        }
    }
    pub fn label(&mut self, text: &str) -> Response {
        self.note_intrinsic_width(text, self.ctx.theme.font_size, 8.0);
        let r = self.item("label", text, 24.0, false);
        self.ctx.draw.text(
            text,
            Rect {
                x: r.rect.x + 4.0,
                y: r.rect.y + 1.0,
                w: (r.rect.w - 8.0).max(1.0),
                h: r.rect.h,
            },
            self.ctx.theme.font_size,
            self.ctx.theme.text,
            self.ctx.text_direction,
        );
        r
    }
    pub fn heading(&mut self, text: &str) -> Response {
        self.note_intrinsic_width(text, self.ctx.theme.heading_size, 8.0);
        let r = self.item("heading", text, 36.0, false);
        self.ctx.draw.text(
            text,
            Rect {
                x: r.rect.x + 4.0,
                y: r.rect.y + 1.0,
                w: (r.rect.w - 8.0).max(1.0),
                h: r.rect.h,
            },
            self.ctx.theme.heading_size,
            self.ctx.theme.text,
            self.ctx.text_direction,
        );
        r
    }
    pub fn button(&mut self, text: &str) -> Response {
        self.note_intrinsic_width(text, self.ctx.theme.font_size, 20.0);
        let mut r = self.item("button", text, 38.0, true);
        if r.focused && self.ctx.input.nav.contains(&crate::NavAction::Accept) {
            r.clicked = true;
        }
        self.rounded_rect(r.rect, self.ctx.theme.panel, None);
        if r.focused {
            self.rounded_border(r.rect, self.ctx.theme.focus, 2.0);
        } else if r.hovered {
            self.rounded_border(r.rect, self.ctx.theme.accent, 1.0);
        }
        self.ctx.draw.text(
            text,
            Rect {
                x: r.rect.x + 10.0,
                y: r.rect.y + 7.0,
                w: (r.rect.w - 20.0).max(1.0),
                h: 24.0,
            },
            self.ctx.theme.font_size,
            self.ctx.theme.text,
            self.ctx.text_direction,
        );
        r
    }
    pub fn checkbox(&mut self, value: &mut bool, text: &str) -> Response {
        self.note_intrinsic_width(text, self.ctx.theme.font_size, 34.0);
        let mut r = self.item("checkbox", text, 32.0, true);
        if r.clicked || (r.focused && self.ctx.input.nav.contains(&crate::NavAction::Accept)) {
            *value = !*value;
            r.changed = true;
        }
        self.rounded_rect(
            Rect {
                w: 22.0,
                h: 22.0,
                y: r.rect.y + 5.0,
                ..r.rect
            },
            if *value {
                self.ctx.theme.accent
            } else {
                self.ctx.theme.panel
            },
            Some(3.0),
        );
        if r.focused {
            self.rounded_border(r.rect, self.ctx.theme.focus, 2.0);
        }
        self.ctx.draw.text(
            text,
            Rect {
                x: r.rect.x + 30.0,
                y: r.rect.y + 4.0,
                w: (r.rect.w - 34.0).max(1.0),
                h: 24.0,
            },
            self.ctx.theme.font_size,
            self.ctx.theme.text,
            self.ctx.text_direction,
        );
        r
    }
    pub fn slider(&mut self, value: &mut f32, range: RangeInclusive<f32>, text: &str) -> Response {
        self.note_intrinsic_width(text, self.ctx.theme.font_size, 8.0);
        let mut r = self.item("slider", text, 36.0, true);
        let min = *range.start();
        let max = *range.end();
        if min.is_finite() && max.is_finite() && max > min {
            if self.ctx.down == Some(self.id("slider", text)) && self.ctx.input.mouse_down {
                if let Some(position) = self.ctx.input.mouse {
                    let fraction = ((position.0 - r.rect.x) / r.rect.w.max(1.0)).clamp(0.0, 1.0);
                    let next = min + (max - min) * fraction;
                    r.changed |= (*value - next).abs() > f32::EPSILON;
                    *value = next;
                }
            }
            if r.focused {
                let step = (max - min) * 0.05;
                if self.ctx.input.nav.contains(&crate::NavAction::Left) {
                    let next = (*value - step).max(min);
                    r.changed |= (*value - next).abs() > f32::EPSILON;
                    *value = next;
                }
                if self.ctx.input.nav.contains(&crate::NavAction::Right) {
                    let next = (*value + step).min(max);
                    r.changed |= (*value - next).abs() > f32::EPSILON;
                    *value = next;
                }
            }
            self.rounded_rect(
                Rect {
                    x: r.rect.x,
                    y: r.rect.y + 18.0,
                    w: r.rect.w,
                    h: 8.0,
                },
                self.ctx.theme.panel,
                Some(4.0),
            );
            let fraction = ((*value - min) / (max - min)).clamp(0.0, 1.0);
            self.rounded_rect(
                Rect {
                    x: r.rect.x,
                    y: r.rect.y + 18.0,
                    w: r.rect.w * fraction,
                    h: 8.0,
                },
                self.ctx.theme.accent,
                Some(4.0),
            );
        }
        if r.focused {
            self.rounded_border(r.rect, self.ctx.theme.focus, 2.0);
        } else if r.hovered {
            self.rounded_border(r.rect, self.ctx.theme.accent, 1.0);
        }
        self.ctx.draw.text(
            text,
            Rect {
                x: r.rect.x,
                y: r.rect.y - 1.0,
                w: r.rect.w,
                h: 18.0,
            },
            self.ctx.theme.font_size,
            self.ctx.theme.text,
            self.ctx.text_direction,
        );
        r
    }
    pub fn progress_bar(&mut self, fraction: f32, text: Option<&str>) -> Response {
        if let Some(text) = text {
            self.note_intrinsic_width(text, self.ctx.theme.font_size, 10.0);
        }
        let r = self.item("progress", text.unwrap_or(""), 28.0, false);
        self.rounded_rect(r.rect, self.ctx.theme.panel, None);
        let mut fill = r.rect;
        fill.w *= if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.rounded_rect(fill, self.ctx.theme.accent, None);
        if let Some(label) = text {
            self.ctx.draw.text(
                label,
                Rect {
                    x: r.rect.x + 5.0,
                    y: r.rect.y + 2.0,
                    w: (r.rect.w - 10.0).max(1.0),
                    h: 22.0,
                },
                self.ctx.theme.font_size,
                self.ctx.theme.text,
                self.ctx.text_direction,
            );
        }
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
    pub fn text_input_with(&mut self, text: &mut String, options: TextInputOptions) -> Response {
        let intrinsic = if text.is_empty() {
            options.hint.as_str()
        } else {
            text.as_str()
        };
        self.note_intrinsic_width(intrinsic, self.ctx.theme.font_size, 16.0);
        let id = self.id("text", &options.hint);
        let mut response = self.item("text", &options.hint, 38.0, true);
        let mut state = self.ctx.text_edits.get(&id).copied().unwrap_or_default();
        state.caret = clamp_char_boundary(text, state.caret);
        let began_editing = response.clicked
            || (response.focused
                && self.ctx.input.nav.contains(&crate::NavAction::Accept)
                && !state.editing);
        if response.clicked
            || (response.focused && self.ctx.input.nav.contains(&crate::NavAction::Accept))
        {
            state.editing = true;
            if response.clicked {
                if let Some(mouse) = self.ctx.input.mouse {
                    let display_for_caret = if options.password {
                        "•".repeat(text.chars().count())
                    } else {
                        text.clone()
                    };
                    let text_rect = Rect {
                        x: response.rect.x + 8.0,
                        y: response.rect.y + 7.0,
                        w: (response.rect.w - 16.0).max(1.0),
                        h: 24.0,
                    };
                    let layout = self.text.layout(
                        &display_for_caret,
                        self.ctx.theme.font_size,
                        text_rect.w,
                        self.ctx.text_direction,
                    );
                    let align = if layout.is_rtl {
                        (text_rect.w - layout.width).max(0.0)
                    } else {
                        0.0
                    };
                    let visual_x = mouse.0 - text_rect.x - align;
                    let display_byte = self.text.byte_for_x(
                        &display_for_caret,
                        self.ctx.theme.font_size,
                        visual_x,
                    );
                    let display_chars = display_for_caret[..display_byte].chars().count();
                    state.caret = byte_at_char(text, display_chars);
                }
            }
        }
        if response.focused && state.editing {
            if self.ctx.input.escape || self.ctx.input.nav.contains(&crate::NavAction::Cancel) {
                state.editing = false;
            } else {
                if self.ctx.input.home {
                    state.caret = 0;
                }
                if self.ctx.input.end {
                    state.caret = text.len();
                }
                let rtl = self
                    .text
                    .layout(
                        text,
                        self.ctx.theme.font_size,
                        16_384.0,
                        TextDirection::Auto,
                    )
                    .is_rtl;
                if self.ctx.input.left || self.ctx.input.nav.contains(&crate::NavAction::Left) {
                    state.caret = if rtl {
                        next_char_boundary(text, state.caret)
                    } else {
                        previous_char_boundary(text, state.caret)
                    };
                }
                if self.ctx.input.right || self.ctx.input.nav.contains(&crate::NavAction::Right) {
                    state.caret = if rtl {
                        previous_char_boundary(text, state.caret)
                    } else {
                        next_char_boundary(text, state.caret)
                    };
                }
                if self.ctx.input.backspace && state.caret > 0 {
                    let previous = previous_char_boundary(text, state.caret);
                    text.replace_range(previous..state.caret, "");
                    state.caret = previous;
                    response.changed = true;
                }
                if self.ctx.input.delete && state.caret < text.len() {
                    let next = next_char_boundary(text, state.caret);
                    text.replace_range(state.caret..next, "");
                    response.changed = true;
                }
                for committed in &self.ctx.input.committed_text {
                    for character in committed.chars().filter(|c| !c.is_control()) {
                        if options
                            .max_len
                            .is_none_or(|limit| text.chars().count() < limit)
                        {
                            text.insert(state.caret, character);
                            state.caret += character.len_utf8();
                            response.changed = true;
                        }
                    }
                }
                response.submitted = !began_editing
                    && (self.ctx.input.enter
                        || self.ctx.input.nav.contains(&crate::NavAction::Accept));
                if response.submitted {
                    state.editing = false;
                }
            }
        }
        let display = if options.password {
            "•".repeat(text.chars().count())
        } else if text.is_empty() {
            options.hint.clone()
        } else {
            text.clone()
        };
        let color = if text.is_empty() {
            self.ctx.theme.muted
        } else {
            self.ctx.theme.text
        };
        self.rounded_rect(response.rect, self.ctx.theme.panel, None);
        if response.focused {
            self.rounded_border(response.rect, self.ctx.theme.focus, 2.0);
        } else if response.hovered {
            self.rounded_border(response.rect, self.ctx.theme.accent, 1.0);
        }
        let text_rect = Rect {
            x: response.rect.x + 8.0,
            y: response.rect.y + 7.0,
            w: (response.rect.w - 16.0).max(1.0),
            h: 24.0,
        };
        self.ctx.draw.text(
            &display,
            text_rect,
            self.ctx.theme.font_size,
            color,
            self.ctx.text_direction,
        );
        if response.focused {
            self.ctx.ime_active = state.editing;
            let display_for_caret = if options.password {
                "•".repeat(text.chars().count())
            } else {
                text.clone()
            };
            let display_caret =
                byte_at_char(&display_for_caret, text[..state.caret].chars().count());
            let layout = self.text.layout(
                &display_for_caret,
                self.ctx.theme.font_size,
                text_rect.w,
                self.ctx.text_direction,
            );
            let align = if layout.is_rtl {
                (text_rect.w - layout.width).max(0.0)
            } else {
                0.0
            };
            let caret_x = text_rect.x
                + align
                + self.text.caret_x_for_byte(
                    &display_for_caret,
                    self.ctx.theme.font_size,
                    display_caret,
                );
            let caret = Rect {
                x: caret_x.min(text_rect.x + text_rect.w - 2.0),
                y: response.rect.y + 6.0,
                w: 2.0,
                h: 25.0,
            };
            self.ctx.ime_rect = Some(caret);
            if state.editing {
                self.ctx.draw.rect(caret, self.ctx.theme.text);
                if let Some(preedit) = self.ctx.input.preedit.as_deref().filter(|v| !v.is_empty()) {
                    let preedit_rect = Rect {
                        x: caret.x + 2.0,
                        y: caret.y,
                        w: (preedit.chars().count() as f32 * self.ctx.theme.font_size * 0.65)
                            .max(8.0),
                        h: caret.h,
                    };
                    self.ctx.draw.text(
                        preedit,
                        preedit_rect,
                        self.ctx.theme.font_size,
                        self.ctx.theme.text,
                        self.ctx.text_direction,
                    );
                    self.ctx.draw.rect(
                        Rect {
                            x: preedit_rect.x,
                            y: preedit_rect.y + preedit_rect.h - 2.0,
                            w: preedit_rect.w,
                            h: 1.0,
                        },
                        self.ctx.theme.text,
                    );
                }
            }
        }
        self.ctx.text_edits.insert(id, state);
        response
    }
    pub fn spacing(&mut self, amount: f32) {
        if let Some(frame) = self.ctx.layouts.last_mut() {
            match frame.direction {
                LayoutDirection::Vertical => frame.cursor_y += amount,
                LayoutDirection::Horizontal => frame.cursor_x += amount,
            }
        }
    }
    pub fn separator(&mut self) {
        let r = self.item("separator", "", 1.0, false);
        self.ctx.draw.rect(r.rect, self.ctx.theme.muted);
    }
    pub fn panel(
        &mut self,
        title: &str,
        anchor: Anchor,
        size: PanelSize,
        add: impl FnOnce(&mut Ui<'_>),
    ) {
        let id = self.id("panel", title);
        self.ctx.used_ids.insert(id);
        let title_height = if title.is_empty() { 0.0 } else { 30.0 };
        let (width, height) = match size {
            PanelSize::Fixed(w, h) => (w.max(1.0), h.max(1.0)),
            PanelSize::Width(w) => (
                w.max(1.0),
                self.ctx.panel_sizes.get(&id).map_or(240.0, |v| v.1),
            ),
            PanelSize::Fit => self
                .ctx
                .panel_sizes
                .get(&id)
                .copied()
                .unwrap_or((360.0, 240.0)),
        };
        let width = width.min(self.ctx.screen.w.max(1.0));
        let height = height.min(self.ctx.screen.h.max(1.0));
        let (x, y) = match anchor {
            Anchor::Center => (
                (self.ctx.screen.w - width) * 0.5,
                (self.ctx.screen.h - height) * 0.5,
            ),
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::TopRight => (self.ctx.screen.w - width, 0.0),
            Anchor::BottomLeft => (0.0, self.ctx.screen.h - height),
            Anchor::BottomRight => (self.ctx.screen.w - width, self.ctx.screen.h - height),
            Anchor::Top => ((self.ctx.screen.w - width) * 0.5, 0.0),
            Anchor::Bottom => (
                (self.ctx.screen.w - width) * 0.5,
                self.ctx.screen.h - height,
            ),
        };
        let rect = Rect {
            x: x.max(0.0),
            y: y.max(0.0),
            w: width,
            h: height,
        };
        self.rounded_rect(rect, self.ctx.theme.background, None);
        if !title.is_empty() {
            self.rounded_rect(
                Rect {
                    h: title_height,
                    ..rect
                },
                self.ctx.theme.panel,
                None,
            );
            self.ctx.draw.text(
                title,
                Rect {
                    x: rect.x + self.ctx.theme.padding,
                    y: rect.y + 5.0,
                    w: (rect.w - self.ctx.theme.padding * 2.0).max(1.0),
                    h: 22.0,
                },
                self.ctx.theme.font_size,
                self.ctx.theme.text,
                self.ctx.text_direction,
            );
        }
        self.ctx.ids.push(id.0);
        self.ctx.layouts.push(LayoutFrame::new(
            Rect {
                x: rect.x,
                y: rect.y + title_height,
                w: rect.w,
                h: (rect.h - title_height).max(1.0),
            },
            LayoutDirection::Vertical,
            Padding {
                left: self.ctx.theme.padding,
                top: self.ctx.theme.padding,
                right: self.ctx.theme.padding,
                bottom: self.ctx.theme.padding,
            },
        ));
        add(self);
        if let Some(completed) = self.ctx.layouts.pop() {
            let title_width = if title.is_empty() {
                0.0
            } else {
                self.text
                    .measure(title, self.ctx.theme.font_size, self.ctx.screen.w)
                    .0
                    + self.ctx.theme.padding * 2.0
            };
            let measured_width = completed.used_width().max(title_width).max(1.0);
            self.ctx
                .panel_sizes
                .insert(id, (measured_width, completed.used_height() + title_height));
        }
        self.ctx.ids.pop();
    }
    pub fn horizontal(&mut self, add: impl FnOnce(&mut Ui<'_>)) {
        self.horizontal_aligned(Align::Start, add);
    }
    /// Run a horizontal row with cross-axis alignment for its children.
    pub fn horizontal_aligned(&mut self, align: Align, add: impl FnOnce(&mut Ui<'_>)) {
        let Some(parent) = self.ctx.layouts.last().copied() else {
            return;
        };
        let (x, y) = match parent.direction {
            LayoutDirection::Vertical => (parent.rect.x + parent.padding.left, parent.cursor_y),
            LayoutDirection::Horizontal => (parent.cursor_x, parent.rect.y + parent.padding.top),
        };
        let available_width = match parent.direction {
            LayoutDirection::Vertical => parent.content_width(),
            LayoutDirection::Horizontal => {
                (parent.rect.x + parent.rect.w - parent.padding.right - x).max(1.0)
            }
        };
        let rect = Rect {
            x,
            y,
            w: available_width,
            h: (parent.rect.y + parent.rect.h - y).max(1.0),
        };
        self.ctx.layouts.push(LayoutFrame::new_aligned(
            rect,
            LayoutDirection::Horizontal,
            Padding::default(),
            align,
        ));
        add(self);
        let Some(completed) = self.ctx.layouts.pop() else {
            return;
        };
        if let Some(parent) = self.ctx.layouts.last_mut() {
            match parent.direction {
                LayoutDirection::Vertical => {
                    parent.cursor_y += completed.max_item_height + self.ctx.theme.spacing;
                    parent.max_item_width = parent.max_item_width.max(completed.used_width());
                }
                LayoutDirection::Horizontal => {
                    let used_width = (completed.cursor_x - completed.rect.x).max(0.0);
                    parent.cursor_x += used_width + self.ctx.theme.spacing;
                    parent.max_item_height = parent.max_item_height.max(completed.max_item_height);
                }
            }
        }
    }
    pub fn vertical(&mut self, add: impl FnOnce(&mut Ui<'_>)) {
        self.vertical_aligned(Align::Start, add);
    }
    /// Run a vertical column with cross-axis alignment for its children.
    pub fn vertical_aligned(&mut self, align: Align, add: impl FnOnce(&mut Ui<'_>)) {
        let Some(parent) = self.ctx.layouts.last().copied() else {
            return;
        };
        let (x, y) = match parent.direction {
            LayoutDirection::Vertical => (parent.rect.x + parent.padding.left, parent.cursor_y),
            LayoutDirection::Horizontal => (parent.cursor_x, parent.rect.y + parent.padding.top),
        };
        let available_width = match parent.direction {
            LayoutDirection::Vertical => parent.content_width(),
            // A vertical group inside a horizontal row is a column. Give it
            // an intrinsic-width slot instead of consuming the entire row.
            LayoutDirection::Horizontal => {
                120.0_f32.min((parent.rect.x + parent.rect.w - parent.padding.right - x).max(1.0))
            }
        };
        let rect = Rect {
            x,
            y,
            w: available_width,
            h: (parent.rect.y + parent.rect.h - y).max(1.0),
        };
        self.ctx.layouts.push(LayoutFrame::new_aligned(
            rect,
            LayoutDirection::Vertical,
            Padding::default(),
            align,
        ));
        add(self);
        let Some(completed) = self.ctx.layouts.pop() else {
            return;
        };
        if let Some(parent) = self.ctx.layouts.last_mut() {
            let used = completed.used_height();
            match parent.direction {
                LayoutDirection::Vertical => {
                    parent.cursor_y += used + self.ctx.theme.spacing;
                    parent.max_item_width = parent.max_item_width.max(completed.used_width());
                }
                LayoutDirection::Horizontal => {
                    parent.cursor_x += completed.rect.w + self.ctx.theme.spacing;
                    parent.max_item_height = parent.max_item_height.max(used);
                }
            }
        }
    }
    pub fn scroll_area(&mut self, max_height: f32, add: impl FnOnce(&mut Ui<'_>)) {
        let parent = *self.ctx.ids.last().unwrap_or(&0);
        let sequence = self.ctx.anonymous_id_sequence;
        self.ctx.anonymous_id_sequence = self.ctx.anonymous_id_sequence.wrapping_add(1);
        let id = Id(crate::hash_id(parent, "scroll", "area", sequence));
        self.ctx.used_ids.insert(id);
        let Some(parent) = self.ctx.layouts.last().copied() else {
            return;
        };
        let (x, y) = match parent.direction {
            LayoutDirection::Vertical => (parent.rect.x + parent.padding.left, parent.cursor_y),
            LayoutDirection::Horizontal => (parent.cursor_x, parent.rect.y + parent.padding.top),
        };
        let available_width = match parent.direction {
            LayoutDirection::Vertical => parent.content_width(),
            LayoutDirection::Horizontal => {
                (parent.rect.x + parent.rect.w - parent.padding.right - x).max(1.0)
            }
        };
        let rect = Rect {
            x,
            y,
            w: available_width,
            h: max_height.max(1.0),
        };
        let hovered = !self.ctx.suppress_hover
            && self.ctx.input.mouse.is_some_and(|p| {
                rect.contains(p) && self.ctx.clips.iter().all(|clip| clip.contains(p))
            });
        let mut offset = self.ctx.scroll_offsets.get(&id).copied().unwrap_or(0.0);
        if hovered {
            offset = (offset - self.ctx.input.scroll * 24.0).max(0.0);
        }
        self.ctx.draw.clip_push(rect);
        self.ctx.clips.push(rect);
        self.ctx.layouts.push(LayoutFrame::new(
            Rect {
                y: rect.y - offset,
                ..rect
            },
            LayoutDirection::Vertical,
            Padding::default(),
        ));
        add(self);
        let content_height = self
            .ctx
            .layouts
            .pop()
            .map_or(rect.h, LayoutFrame::used_height);
        offset = offset.min((content_height - rect.h).max(0.0));
        self.ctx.scroll_offsets.insert(id, offset);
        self.ctx.clips.pop();
        self.ctx.draw.clip_pop();
        if let Some(parent) = self.ctx.layouts.last_mut() {
            match parent.direction {
                LayoutDirection::Vertical => {
                    parent.cursor_y += rect.h + self.ctx.theme.spacing;
                    parent.max_item_width = parent.max_item_width.max(rect.w);
                }
                LayoutDirection::Horizontal => {
                    parent.cursor_x += rect.w + self.ctx.theme.spacing;
                    parent.max_item_height = parent.max_item_height.max(rect.h);
                }
            }
        }
    }
    pub fn push_id(&mut self, salt: impl std::hash::Hash, add: impl FnOnce(&mut Ui<'_>)) {
        let mut hasher = FnvHasher::default();
        std::hash::Hash::hash(&salt, &mut hasher);
        let parent = *self.ctx.ids.last().unwrap_or(&0);
        self.ctx.ids.push(crate::hash_id(
            parent,
            "scope",
            "",
            std::hash::Hasher::finish(&hasher),
        ));
        add(self);
        self.ctx.ids.pop();
    }
    pub fn request_focus_first(&mut self) {
        self.ctx.request_focus_first = true;
    }
    pub fn set_text_direction(&mut self, direction: TextDirection) {
        self.ctx.text_direction = direction;
    }
}

#[derive(Clone)]
struct FnvHasher(u64);
impl Default for FnvHasher {
    fn default() -> Self {
        Self(0xcbf29ce484222325)
    }
}
impl std::hash::Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

fn clamp_char_boundary(text: &str, index: usize) -> usize {
    let end = index.min(text.len());
    (0..=end)
        .rev()
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0)
}
fn byte_at_char(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}
fn previous_char_boundary(text: &str, index: usize) -> usize {
    text[..clamp_char_boundary(text, index)]
        .char_indices()
        .last()
        .map_or(0, |(i, _)| i)
}
fn next_char_boundary(text: &str, index: usize) -> usize {
    let index = clamp_char_boundary(text, index);
    text[index..]
        .chars()
        .next()
        .map_or(text.len(), |c| index + c.len_utf8())
}

pub fn ui_scope(world: &mut ribbon_app::World, build: impl FnOnce(&mut Ui<'_>)) {
    if world.try_resource::<UiContext>().is_none()
        || world.try_resource::<crate::TextSystem>().is_none()
    {
        return;
    }
    world.resource_scope::<UiContext, _>(|world, context| {
        world.resource_scope::<crate::TextSystem, _>(|_, text| {
            let mut ui = Ui { ctx: context, text };
            build(&mut ui);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vertical_layout_applies_center_and_end_cross_axis_alignment() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 220.0,
            },
            UiInput::default(),
        );
        let expected_right = context.screen.x + context.screen.w - context.theme.padding;
        let mut text = crate::TextSystem::new();
        context.begin();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.vertical_aligned(Align::Center, |ui| {
                let centered = ui.button("center");
                assert!(centered.rect.x > 16.0);
            });
            ui.vertical_aligned(Align::End, |ui| {
                let end = ui.button("end");
                let actual_right = end.rect.x + end.rect.w;
                assert!(end.rect.x > 16.0);
                assert!(
                    actual_right <= expected_right + 0.001,
                    "End-aligned widget overflowed: actual_right={actual_right}, expected_right={expected_right}, rect={:?}",
                    end.rect
                );
                assert!(
                    (expected_right - actual_right).abs() <= 0.001,
                    "End-aligned widget is not flush with the content edge: actual_right={actual_right}, expected_right={expected_right}"
                );
            });
        }
    }

    #[test]
    fn horizontal_layout_applies_vertical_cross_axis_alignment() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 220.0,
            },
            UiInput::default(),
        );
        let mut text = crate::TextSystem::new();
        context.begin();
        let mut ui = Ui {
            ctx: &mut context,
            text: &mut text,
        };
        let mut center_y = 0.0;
        ui.horizontal_aligned(Align::Center, |ui| {
            center_y = ui.button("center row").rect.y;
        });
        assert!(center_y > 16.0);
    }

    #[test]
    fn button_click_requires_press_then_release_inside() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 300.0,
            },
            UiInput {
                mouse: Some((20.0, 20.0)),
                mouse_pressed: true,
                mouse_down: true,
                ..Default::default()
            },
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        let _ = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .button("go");
        context.input.mouse_pressed = false;
        context.input.mouse_down = false;
        context.input.mouse_released = true;
        context.begin();
        assert!(
            Ui {
                ctx: &mut context,
                text: &mut text
            }
            .button("go")
            .clicked
        );
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
    #[test]
    fn focus_moves_to_next_previous_and_nearest_directional_widget() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 300.0,
            },
            UiInput::default(),
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.button("a");
            ui.button("b");
            ui.button("c");
        }
        let first = context.focus.current[0].0;
        context.focus.previous = std::mem::take(&mut context.focus.current);
        context.focus.focused = Some(first);
        context.input.nav = vec![crate::NavAction::Next];
        context.begin();
        assert_eq!(context.focus.focused, Some(context.focus.previous[1].0));
        context.input.nav = vec![crate::NavAction::Prev];
        context.begin();
        assert_eq!(context.focus.focused, Some(first));
    }
    #[test]
    fn horizontal_layout_places_items_side_by_side_and_panel_is_anchored() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
            },
            UiInput::default(),
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.panel(
                "test",
                Anchor::Center,
                PanelSize::Fixed(400.0, 300.0),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.button("left");
                        ui.button("right");
                    });
                },
            );
        }
        let a = context.focus.current[0].1;
        let b = context.focus.current[1].1;
        assert!(b.x > a.x);
        assert_eq!((a.x, a.y), (212.0, 192.0));
    }
    #[test]
    fn text_input_caret_and_delete_operations_respect_utf8_boundaries() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 120.0,
            },
            UiInput::default(),
        );
        context.begin();
        let id = crate::Id(crate::hash_id(0xcbf29ce484222325, "text", "name", 0));
        context.text_edits.insert(
            id,
            TextEditState {
                caret: "aé".len(),
                editing: true,
            },
        );
        context.focus.focused = Some(id);
        context.input.backspace = true;
        let mut value = "aéx".to_owned();
        let mut text = crate::TextSystem::new();
        let response = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .text_input_with(
            &mut value,
            TextInputOptions {
                hint: "name".into(),
                ..Default::default()
            },
        );
        assert!(response.changed);
        assert_eq!(value, "ax");
    }

    #[test]
    fn accepting_a_text_field_starts_editing_before_a_later_accept_submits() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 120.0,
            },
            UiInput::default(),
        );
        context.begin();
        let mut value = String::from("name");
        let mut text = crate::TextSystem::new();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.request_focus_first();
            let _ = ui.text_input(&mut value, "name");
        }

        context.input.nav = vec![crate::NavAction::Accept];
        context.begin();
        let started = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .text_input(&mut value, "name");
        assert!(!started.submitted);
        let id = crate::Id(crate::hash_id(0xcbf29ce484222325, "text", "name", 0));
        assert!(context
            .text_edits
            .get(&id)
            .is_some_and(|state| state.editing));

        context.input.nav = vec![crate::NavAction::Accept];
        context.begin();
        let submitted = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .text_input(&mut value, "name");
        assert!(submitted.submitted);
        assert!(!context
            .text_edits
            .get(&id)
            .is_some_and(|state| state.editing));
    }

    #[test]
    fn text_input_honors_max_len_and_delete_at_the_caret() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 120.0,
            },
            UiInput::default(),
        );
        context.begin();
        let id = crate::Id(crate::hash_id(0xcbf29ce484222325, "text", "player", 0));
        context.text_edits.insert(
            id,
            TextEditState {
                caret: "aé".len(),
                editing: true,
            },
        );
        context.focus.focused = Some(id);
        context.input.delete = true;
        context.input.committed_text = vec!["🙂z".into()];
        let mut value = String::from("aéx");
        let mut text = crate::TextSystem::new();
        let response = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .text_input_with(
            &mut value,
            TextInputOptions {
                hint: "player".into(),
                max_len: Some(3),
                ..Default::default()
            },
        );
        assert!(response.changed);
        assert_eq!(value, "aé🙂");
        assert_eq!(value.chars().count(), 3);
        assert!(
            value.len() > 3,
            "max_len counts Unicode scalar values, not UTF-8 bytes"
        );
    }

    #[test]
    fn multiple_scroll_areas_get_independent_persistent_ids() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 480.0,
                h: 300.0,
            },
            UiInput::default(),
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.horizontal(|ui| {
                ui.scroll_area(60.0, |ui| {
                    for index in 0..6 {
                        ui.label(&format!("left {index}"));
                    }
                });
                ui.scroll_area(60.0, |ui| {
                    for index in 0..6 {
                        ui.label(&format!("right {index}"));
                    }
                });
            });
        }
        assert_eq!(context.scroll_offsets.len(), 2);
        assert_ne!(
            crate::hash_id(0xcbf29ce484222325, "scroll", "area", 0),
            crate::hash_id(0xcbf29ce484222325, "scroll", "area", 1),
        );
    }

    #[test]
    fn gamepad_directional_hold_repeats_after_delay_and_wraps_at_edge() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 200.0,
            },
            UiInput::default(),
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.horizontal(|ui| {
                ui.button("left");
                ui.button("right");
            });
        }
        context.focus.previous = std::mem::take(&mut context.focus.current);
        context.focus.focused = Some(context.focus.previous[0].0);
        context.input.nav_held = vec![crate::NavAction::Right];
        context.input.dt = 0.016;
        context.begin();
        assert_eq!(context.focus.focused, Some(context.focus.previous[1].0));

        for _ in 0..20 {
            context.input.nav.clear();
            context.begin();
        }
        assert_eq!(context.focus.focused, Some(context.focus.previous[1].0));
        for _ in 0..7 {
            context.input.nav.clear();
            context.begin();
        }
        assert_eq!(context.focus.focused, Some(context.focus.previous[0].0));
    }

    #[test]
    fn keyboard_navigation_suppresses_hover_until_the_mouse_moves() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 200.0,
            },
            UiInput {
                mouse: Some((20.0, 20.0)),
                ..Default::default()
            },
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        let _ = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .button("first");
        context.focus.previous = std::mem::take(&mut context.focus.current);
        context.focus.focused = Some(context.focus.previous[0].0);
        context.input.nav = vec![crate::NavAction::Next];
        context.begin();
        assert!(context.suppress_hover);
        let mut text = crate::TextSystem::new();
        let response = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .button("first");
        assert!(!response.hovered);
        context.input.nav.clear();
        context.input.mouse = Some((21.0, 20.0));
        context.begin();
        assert!(!context.suppress_hover);
    }

    #[test]
    fn per_id_widget_state_is_swept_when_widgets_disappear() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 200.0,
            },
            UiInput::default(),
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .panel("remembered", Anchor::Center, PanelSize::Fit, |_| {});
        assert!(!context.panel_sizes.is_empty());
        context.finish_frame();
        context.begin();
        context.finish_frame();
        assert!(context.panel_sizes.is_empty());
    }

    #[test]
    fn focused_slider_draws_a_focus_outline() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 160.0,
            },
            UiInput::default(),
        );
        context.begin();
        let id = crate::Id(crate::hash_id(0xcbf29ce484222325, "slider", "volume", 0));
        context.focus.focused = Some(id);
        let mut value = 0.5;
        let mut text = crate::TextSystem::new();
        let _ = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .slider(&mut value, 0.0..=1.0, "volume");
        assert!(context
            .draw
            .vertices
            .iter()
            .any(|vertex| vertex.border == 2.0));
    }

    #[test]
    fn slider_tracks_pointer_and_keyboard_adjustment() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 160.0,
            },
            UiInput {
                mouse: Some((150.0, 40.0)),
                mouse_pressed: true,
                mouse_down: true,
                ..Default::default()
            },
        );
        context.begin();
        let mut value = 0.0;
        let mut text = crate::TextSystem::new();
        let response = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .slider(&mut value, 0.0..=1.0, "volume");
        assert!(response.changed);
        assert!((value - 0.5).abs() < 0.05);

        context.focus.focused = Some(crate::Id(crate::hash_id(
            0xcbf29ce484222325,
            "slider",
            "volume",
            0,
        )));
        context.input.mouse_pressed = false;
        context.input.mouse_down = false;
        context.input.nav = vec![crate::NavAction::Right];
        context.begin();
        let before = value;
        let mut text = crate::TextSystem::new();
        let _ = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .slider(&mut value, 0.0..=1.0, "volume");
        assert!(value > before);
    }

    #[test]
    fn checkbox_accept_toggles_and_request_focus_first_selects_first_widget() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 160.0,
            },
            UiInput::default(),
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.request_focus_first();
            let mut checked = false;
            let _ = ui.checkbox(&mut checked, "enabled");
            assert_eq!(ui.ctx.focus.focused, Some(ui.ctx.focus.current[0].0));
        }
        let id = context.focus.focused;
        assert!(id.is_some());
        context.input.nav = vec![crate::NavAction::Accept];
        context.begin();
        let mut checked = false;
        let mut text = crate::TextSystem::new();
        let response = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .checkbox(&mut checked, "enabled");
        assert!(checked);
        assert!(response.changed);
    }

    #[test]
    fn scroll_area_clips_pointer_interaction_for_offscreen_widgets() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 240.0,
            },
            UiInput {
                mouse: Some((24.0, 60.0)),
                mouse_pressed: true,
                mouse_down: true,
                ..Default::default()
            },
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        let mut second_hovered = true;
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.scroll_area(40.0, |ui| {
                ui.button("visible");
                second_hovered = ui.button("clipped").hovered;
            });
        }
        assert!(
            !second_hovered,
            "a clipped widget must not receive pointer input"
        );
    }

    #[test]
    fn nested_vertical_columns_in_horizontal_layout_have_distinct_slots() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 500.0,
                h: 300.0,
            },
            UiInput::default(),
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.button("left");
                });
                ui.vertical(|ui| {
                    ui.button("right");
                });
            });
        }
        assert_eq!(context.focus.current.len(), 2);
        assert!(context.focus.current[1].1.x > context.focus.current[0].1.x);
    }

    #[test]
    fn scroll_area_clips_content_and_keeps_a_bounded_offset() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 240.0,
            },
            UiInput::default(),
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.scroll_area(40.0, |ui| {
                for index in 0..8 {
                    ui.label(&format!("row {index}"));
                }
            });
        }
        assert!(context
            .draw
            .texts
            .iter()
            .any(|draw| draw.clip == Some([12, 12, 296, 40])));
        context.input.mouse = Some((24.0, 24.0));
        context.input.scroll = -3.0;
        context.begin();
        let mut text = crate::TextSystem::new();
        Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .scroll_area(40.0, |ui| {
            for index in 0..8 {
                ui.label(&format!("row {index}"));
            }
        });
        let id = crate::Id(crate::hash_id(0xcbf29ce484222325, "scroll", "area", 0));
        assert!(context.scroll_offsets.get(&id).copied().unwrap_or(0.0) > 0.0);
    }

    #[test]
    fn directional_focus_uses_nearest_widget_in_the_requested_direction() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 400.0,
                h: 300.0,
            },
            UiInput {
                nav: vec![crate::NavAction::Right],
                ..Default::default()
            },
        );
        let first = crate::Id(1);
        let right_far = crate::Id(2);
        let right_near = crate::Id(3);
        context.focus.focused = Some(first);
        context.focus.previous = vec![
            (
                first,
                Rect {
                    x: 10.0,
                    y: 10.0,
                    w: 40.0,
                    h: 30.0,
                },
            ),
            (
                right_far,
                Rect {
                    x: 180.0,
                    y: 10.0,
                    w: 40.0,
                    h: 30.0,
                },
            ),
            (
                right_near,
                Rect {
                    x: 80.0,
                    y: 10.0,
                    w: 40.0,
                    h: 30.0,
                },
            ),
        ];
        context.begin();
        assert_eq!(context.focus.focused, Some(right_near));
    }

    #[test]
    fn password_text_is_masked_and_ime_preedit_sets_candidate_rectangle() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 120.0,
            },
            UiInput {
                preedit: Some("かな".into()),
                ..Default::default()
            },
        );
        context.begin();
        let id = crate::Id(crate::hash_id(0xcbf29ce484222325, "text", "password", 0));
        context.text_edits.insert(
            id,
            TextEditState {
                caret: 3,
                editing: true,
            },
        );
        context.focus.focused = Some(id);
        let mut password = "secret".to_owned();
        let mut text = crate::TextSystem::new();
        let _ = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .text_input_with(
            &mut password,
            TextInputOptions {
                hint: "password".into(),
                password: true,
                ..Default::default()
            },
        );
        assert!(context.ime_active);
        assert!(context.ime_rect.is_some());
        assert!(context.draw.texts.iter().any(|draw| draw.text == "••••••"));
        assert!(context.draw.texts.iter().any(|draw| draw.text == "かな"));
        assert!(!context.draw.texts.iter().any(|draw| draw.text == "secret"));
    }

    #[test]
    fn left_and_right_move_caret_visually_for_rtl_text() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 120.0,
            },
            UiInput {
                nav: vec![crate::NavAction::Left],
                ..Default::default()
            },
        );
        context.begin();
        let id = crate::Id(crate::hash_id(0xcbf29ce484222325, "text", "arabic", 0));
        context.text_edits.insert(
            id,
            TextEditState {
                caret: 0,
                editing: true,
            },
        );
        context.focus.focused = Some(id);
        let mut value = "مرحبا".to_owned();
        let mut text = crate::TextSystem::new();
        let _ = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .text_input_with(
            &mut value,
            TextInputOptions {
                hint: "arabic".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            context.text_edits.get(&id).map(|state| state.caret),
            Some("م".len())
        );
    }

    #[test]
    fn fit_panel_measures_content_width_on_the_following_frame() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 640.0,
                h: 360.0,
            },
            UiInput::default(),
        );
        let mut text = crate::TextSystem::new();
        context.begin();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.panel("Fit", Anchor::Center, PanelSize::Fit, |ui| {
                ui.label("short");
            });
        }
        context.finish_frame();
        context.begin();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.panel("Fit", Anchor::Center, PanelSize::Fit, |ui| {
                ui.label("short");
            });
        }
        let measured = context
            .draw
            .vertices
            .first()
            .map(|vertex| vertex.rect_size[0]);
        assert!(measured.is_some_and(|width| width < 360.0));
    }

    #[test]
    fn ui_settings_default_to_logical_scale_and_dpi_awareness() {
        let settings = UiSettings::default();
        assert_eq!(settings.scale, 1.0);
        assert!(settings.respect_dpi);
    }

    #[test]
    fn overlapping_widgets_only_the_last_drawn_widget_receives_mouse_hover() {
        let screen = Rect {
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 200.0,
        };
        let mut context = UiContext::synthetic(screen, UiInput::default());
        let mut text = crate::TextSystem::new();
        context.begin();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.panel(
                "under panel",
                Anchor::TopLeft,
                PanelSize::Fixed(200.0, 100.0),
                |ui| {
                    ui.button("under");
                },
            );
            ui.panel(
                "over panel",
                Anchor::TopLeft,
                PanelSize::Fixed(200.0, 100.0),
                |ui| {
                    ui.button("over");
                },
            );
        }
        context.finish_frame();
        context.input.mouse = Some((30.0, 65.0));
        context.begin();
        let mut ui = Ui {
            ctx: &mut context,
            text: &mut text,
        };
        let mut lower_response = Response::default();
        let mut upper_response = Response::default();
        ui.panel(
            "under panel",
            Anchor::TopLeft,
            PanelSize::Fixed(200.0, 100.0),
            |ui| {
                lower_response = ui.button("under");
            },
        );
        ui.panel(
            "over panel",
            Anchor::TopLeft,
            PanelSize::Fixed(200.0, 100.0),
            |ui| {
                upper_response = ui.button("over");
            },
        );
        assert!(!lower_response.hovered);
        assert!(upper_response.hovered);
    }

    #[test]
    fn disappeared_pressed_widget_releases_mouse_capture() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 200.0,
            },
            UiInput {
                mouse: Some((20.0, 20.0)),
                mouse_pressed: true,
                mouse_down: true,
                ..Default::default()
            },
        );
        let mut text = crate::TextSystem::new();
        context.begin();
        let response = Ui {
            ctx: &mut context,
            text: &mut text,
        }
        .button("temporary");
        assert!(response.pressed);
        context.finish_frame();
        context.input.mouse_pressed = false;
        context.input.mouse_down = false;
        context.begin();
        context.finish_frame();
        assert!(context.down.is_none());
    }

    #[test]
    fn scroll_area_does_not_capture_wheel_outside_parent_clip() {
        let mut context = UiContext::synthetic(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 200.0,
            },
            UiInput {
                mouse: Some((250.0, 180.0)),
                scroll: 2.0,
                ..Default::default()
            },
        );
        context.begin();
        let mut text = crate::TextSystem::new();
        {
            let mut ui = Ui {
                ctx: &mut context,
                text: &mut text,
            };
            ui.scroll_area(80.0, |ui| {
                ui.scroll_area(200.0, |ui| {
                    for index in 0..12 {
                        ui.label(&format!("long content {index}"));
                    }
                });
            });
        }
        assert!(context.scroll_offsets.values().all(|offset| *offset == 0.0));
    }
}
