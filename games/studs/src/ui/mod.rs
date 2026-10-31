pub mod controls;
pub mod dialogs;
pub mod difficulty;
pub mod hud;
pub mod layout;
pub mod menu;
pub mod pause;
pub mod settings;
pub mod theme;

use self::settings as settings_page;

use crate::game::{Difficulty, Game, GameReadView, GameUiState, State};
use crate::input::{key_name, Action, Key};
use crate::settings::Settings;
use crate::ui::hud::fill_rect;
use studs_cpu_renderer::cpu::canvas::Canvas;

use self::theme::{ACCENT, MUTED, PANEL_EDGE, SELECTED, TEXT};

const SETTINGS_PARENT_DEFAULT: Screen = Screen::MainMenu;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    MainMenu,
    Difficulty,
    Playing,
    Pause,
    Settings,
    Controls,
    ConfirmMainMenu,
    Result,
}

#[derive(Clone, Debug, Default)]
pub struct UiInput {
    pub pressed: Vec<Key>,
    /// Cursor coordinates in framebuffer pixels, with the top-left as origin.
    pub mouse_position: Option<(f32, f32)>,
    pub mouse_down: bool,
    pub mouse_scroll: f32,
    pub framebuffer_size: (usize, usize),
}

impl UiInput {
    pub fn from_keys(pressed: Vec<Key>) -> Self {
        Self {
            pressed,
            mouse_position: None,
            mouse_down: false,
            mouse_scroll: 0.0,
            framebuffer_size: (0, 0),
        }
    }

    pub fn with_mouse(
        mut self,
        mouse_position: Option<(f32, f32)>,
        mouse_down: bool,
        mouse_scroll: f32,
        framebuffer_size: (usize, usize),
    ) -> Self {
        self.mouse_position = mouse_position;
        self.mouse_down = mouse_down;
        self.mouse_scroll = mouse_scroll;
        self.framebuffer_size = framebuffer_size;
        self
    }

    fn has(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }

    fn up(&self) -> bool {
        self.has(Key::Up) || self.has(Key::W) || self.mouse_scroll > 0.0
    }

    fn down(&self) -> bool {
        self.has(Key::Down) || self.has(Key::S) || self.mouse_scroll < 0.0
    }

    fn left(&self) -> bool {
        self.has(Key::Left) || self.has(Key::A)
    }

    fn right(&self) -> bool {
        self.has(Key::Right) || self.has(Key::D)
    }

    fn confirm(&self) -> bool {
        self.has(Key::Enter) || self.has(Key::Space)
    }

    fn back(&self) -> bool {
        self.has(Key::Escape)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UiOutcome {
    pub start_game: bool,
    pub restart_game: bool,
    pub return_to_menu: bool,
    pub quit: bool,
    pub settings_changed: bool,
    pub window_changed: bool,
    pub difficulty: Option<Difficulty>,
}

pub struct Ui {
    screen: Screen,
    selected: usize,
    settings_return: Screen,
    controls_return: Screen,
    rebinding: Option<Action>,
    prev_mouse_down: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

mod draw;
mod interaction;

fn format_time(seconds: f32) -> String {
    let elapsed = seconds.max(0.0) as u32;
    format!("{:02}:{:02}", elapsed / 60, elapsed % 60)
}
