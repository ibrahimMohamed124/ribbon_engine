//! Headless-friendly input snapshot and UI navigation actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAction {
    Up,
    Down,
    Left,
    Right,
    Next,
    Prev,
    Accept,
    Cancel,
}
#[derive(Clone, Debug, Default)]
pub struct UiInput {
    pub mouse: Option<(f32, f32)>,
    pub mouse_down: bool,
    pub mouse_pressed: bool,
    pub mouse_released: bool,
    pub scroll: f32,
    pub nav: Vec<NavAction>,
    pub committed_text: Vec<String>,
    pub preedit: Option<String>,
    pub left: bool,
    pub right: bool,
    pub home: bool,
    pub end: bool,
    pub backspace: bool,
    pub delete: bool,
    pub enter: bool,
    pub escape: bool,
    pub dt: f32,
}
