pub mod controls;
pub mod dialogs;
pub mod layout;
pub mod menu;
pub mod pause;
pub mod settings;
pub mod theme;

use self::settings as settings_page;

use crate::game::{Game, State};
use crate::input::{key_name, Action, Key};
use crate::renderer::hud::fill_rect;
use crate::renderer::Framebuffer;
use crate::settings::Settings;

use self::layout::centered_panel;
use self::theme::{ACCENT, MUTED, PANEL_EDGE, SELECTED, TEXT};

const SETTINGS_PARENT_DEFAULT: Screen = Screen::MainMenu;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    MainMenu,
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
}

impl UiInput {
    pub fn from_keys(pressed: Vec<Key>) -> Self {
        Self { pressed }
    }

    fn has(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }

    fn up(&self) -> bool {
        self.has(Key::Up) || self.has(Key::W)
    }

    fn down(&self) -> bool {
        self.has(Key::Down) || self.has(Key::S)
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
}

pub struct Ui {
    screen: Screen,
    selected: usize,
    settings_return: Screen,
    controls_return: Screen,
    rebinding: Option<Action>,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    pub fn new() -> Self {
        Self {
            screen: Screen::MainMenu,
            selected: 0,
            settings_return: SETTINGS_PARENT_DEFAULT,
            controls_return: SETTINGS_PARENT_DEFAULT,
            rebinding: None,
        }
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn is_playing(&self) -> bool {
        self.screen == Screen::Playing
    }

    pub fn show_result(&mut self) {
        self.screen = Screen::Result;
        self.selected = 0;
    }

    pub fn update(&mut self, input: &UiInput, settings: &mut Settings) -> UiOutcome {
        let mut outcome = UiOutcome::default();

        if let Some(action) = self.rebinding {
            if input.back() {
                self.rebinding = None;
            } else if let Some(key) = input
                .pressed
                .iter()
                .copied()
                .find(|key| *key != Key::Enter && *key != Key::Space)
            {
                settings.bindings.set(action, vec![key]);
                self.rebinding = None;
                outcome.settings_changed = true;
            }
            return outcome;
        }

        if settings
            .bindings
            .keys(Action::Quit)
            .iter()
            .any(|&key| input.has(key))
        {
            outcome.quit = true;
            return outcome;
        }

        let restart_pressed = settings
            .bindings
            .keys(Action::Restart)
            .iter()
            .any(|&key| input.has(key));
        if restart_pressed && matches!(self.screen, Screen::Pause | Screen::Result) {
            outcome.restart_game = self.screen == Screen::Pause;
            outcome.start_game = self.screen == Screen::Result;
            self.screen = Screen::Playing;
            self.selected = 0;
            return outcome;
        }

        match self.screen {
            Screen::Playing => {
                if input.back() {
                    self.screen = Screen::Pause;
                    self.selected = 0;
                }
            }
            Screen::MainMenu => {
                if input.back() {
                    outcome.quit = true;
                } else {
                    self.navigate(input, menu::MAIN_ITEMS.len());
                    if input.confirm() {
                        match self.selected {
                            0 => {
                                self.screen = Screen::Playing;
                                outcome.start_game = true;
                            }
                            1 => self.open_settings(Screen::MainMenu),
                            2 => self.open_controls(Screen::MainMenu),
                            _ => outcome.quit = true,
                        }
                    }
                }
            }
            Screen::Pause => {
                if input.back() {
                    self.screen = Screen::Playing;
                } else {
                    self.navigate(input, menu::PAUSE_ITEMS.len());
                    if input.confirm() {
                        match self.selected {
                            0 => self.screen = Screen::Playing,
                            1 => {
                                self.screen = Screen::Playing;
                                outcome.restart_game = true;
                            }
                            2 => self.open_settings(Screen::Pause),
                            3 => self.open_controls(Screen::Pause),
                            _ => {
                                self.screen = Screen::ConfirmMainMenu;
                                self.selected = 0;
                            }
                        }
                    }
                }
            }
            Screen::Settings => {
                if input.back()
                    || (self.selected == settings_page::ITEM_COUNT - 1 && input.confirm())
                {
                    self.screen = self.settings_return;
                    self.selected = 0;
                } else {
                    self.navigate(input, settings_page::ITEM_COUNT);
                    let direction = if input.right() {
                        1
                    } else if input.left() {
                        -1
                    } else {
                        0
                    };
                    if direction != 0 {
                        let (changed, window_changed) =
                            settings_page::adjust(settings, self.selected, direction);
                        outcome.settings_changed |= changed;
                        outcome.window_changed |= window_changed;
                    } else if input.confirm() && matches!(self.selected, 3 | 5 | 7 | 8) {
                        let (changed, window_changed) =
                            settings_page::adjust(settings, self.selected, 1);
                        outcome.settings_changed |= changed;
                        outcome.window_changed |= window_changed;
                    }
                }
            }
            Screen::Controls => {
                if input.back() || (self.selected == controls::ACTIONS.len() && input.confirm()) {
                    self.screen = self.controls_return;
                    self.selected = 0;
                } else {
                    self.navigate(input, controls::ACTIONS.len() + 1);
                    if input.confirm() {
                        if self.selected < controls::ACTIONS.len() {
                            self.rebinding = Some(controls::ACTIONS[self.selected]);
                        } else {
                            self.screen = self.controls_return;
                            self.selected = 0;
                        }
                    }
                }
            }
            Screen::ConfirmMainMenu => {
                if input.back() {
                    self.screen = Screen::Pause;
                    self.selected = 0;
                } else {
                    self.navigate(input, menu::CONFIRM_ITEMS.len());
                    if input.confirm() {
                        if self.selected == 1 {
                            self.screen = Screen::MainMenu;
                            self.selected = 0;
                            outcome.return_to_menu = true;
                        } else {
                            self.screen = Screen::Pause;
                            self.selected = 0;
                        }
                    }
                }
            }
            Screen::Result => {
                self.navigate(input, menu::RESULT_ITEMS.len());
                if input.confirm() {
                    match self.selected {
                        0 => {
                            self.screen = Screen::Playing;
                            outcome.start_game = true;
                        }
                        1 => {
                            self.screen = Screen::MainMenu;
                            self.selected = 0;
                            outcome.return_to_menu = true;
                        }
                        _ => outcome.quit = true,
                    }
                }
            }
        }
        outcome
    }

    fn navigate(&mut self, input: &UiInput, count: usize) {
        if count == 0 {
            self.selected = 0;
        } else if input.up() {
            self.selected = (self.selected + count - 1) % count;
        } else if input.down() {
            self.selected = (self.selected + 1) % count;
        }
    }

    fn open_settings(&mut self, parent: Screen) {
        self.settings_return = parent;
        self.screen = Screen::Settings;
        self.selected = 0;
    }

    fn open_controls(&mut self, parent: Screen) {
        self.controls_return = parent;
        self.screen = Screen::Controls;
        self.selected = 0;
    }

    pub fn draw(&self, fb: &mut Framebuffer, frame: u32, game: &Game, settings: &Settings) {
        if self.screen == Screen::Playing {
            return;
        }
        match self.screen {
            Screen::MainMenu => theme::fog_background(fb, frame),
            Screen::Settings if self.settings_return == Screen::MainMenu => {
                theme::fog_background(fb, frame)
            }
            Screen::Controls if self.controls_return == Screen::MainMenu => {
                theme::fog_background(fb, frame)
            }
            Screen::ConfirmMainMenu => theme::dim(fb, 0.46),
            Screen::Result => theme::dim(fb, 0.58),
            _ => theme::dim(fb, 0.64),
        }

        match self.screen {
            Screen::MainMenu => self.draw_main(fb),
            Screen::Pause => self.draw_menu(fb, pause::TITLE, menu::PAUSE_ITEMS, pause::HINT),
            Screen::Settings => self.draw_settings(fb, settings),
            Screen::Controls => self.draw_controls(fb, settings),
            Screen::ConfirmMainMenu => self.draw_confirm(fb),
            Screen::Result => self.draw_result(fb, game),
            Screen::Playing => {}
        }
    }

    fn draw_main(&self, fb: &mut Framebuffer) {
        theme::centered_text(fb, "STUDS", 14, 3, TEXT);
        theme::centered_text(fb, "THE MAZE IS LISTENING", 41, 1, MUTED);
        self.draw_menu_at(
            fb,
            "MAIN MENU",
            menu::MAIN_ITEMS,
            "ARROWS MOVE   ENTER SELECT",
            0.63,
        );
        theme::centered_text(
            fb,
            "WASD MOVE   ARROWS LOOK   Q QUIT",
            fb.height as i32 - 10,
            1,
            MUTED,
        );
    }

    fn draw_menu(&self, fb: &mut Framebuffer, title: &str, items: &[&str], hint: &str) {
        self.draw_menu_at(fb, title, items, hint, 0.5);
    }

    fn draw_menu_at(
        &self,
        fb: &mut Framebuffer,
        title: &str,
        items: &[&str],
        hint: &str,
        y_ratio: f32,
    ) {
        let panel = centered_panel(fb, 226, 22 + items.len() as i32 * 15 + 24);
        let y = ((fb.height as f32 * y_ratio) as i32 - panel.height / 2)
            .clamp(4, fb.height as i32 - panel.height - 4);
        let panel = layout::Rect { y, ..panel };
        theme::panel(fb, panel.x, panel.y, panel.width, panel.height);
        theme::text(fb, title, panel.x + 20, panel.y + 12, 1, TEXT);
        fill_rect(
            fb,
            panel.x + 16,
            panel.y + 24,
            panel.width - 32,
            1,
            PANEL_EDGE,
        );
        for (index, item) in items.iter().enumerate() {
            let row_y = panel.y + 30 + index as i32 * 15;
            self.draw_row(fb, panel, index, item, row_y);
        }
        theme::text(
            fb,
            hint,
            panel.x + 20,
            panel.y + panel.height - 13,
            1,
            MUTED,
        );
    }

    fn draw_row(
        &self,
        fb: &mut Framebuffer,
        panel: layout::Rect,
        index: usize,
        label: &str,
        y: i32,
    ) {
        if self.selected == index {
            fill_rect(fb, panel.x + 14, y - 3, panel.width - 28, 13, SELECTED);
            fill_rect(fb, panel.x + 14, y - 3, 2, 13, ACCENT);
        }
        theme::text(
            fb,
            label,
            panel.x + 23,
            y,
            1,
            if self.selected == index { TEXT } else { MUTED },
        );
        if self.selected == index {
            theme::text(fb, ">", panel.x + panel.width - 21, y, 1, ACCENT);
        }
    }

    fn draw_settings(&self, fb: &mut Framebuffer, settings: &Settings) {
        let panel = centered_panel(fb, 266, 170);
        theme::panel(fb, panel.x, panel.y, panel.width, panel.height);
        theme::text(fb, settings_page::TITLE, panel.x + 18, panel.y + 9, 1, TEXT);
        fill_rect(
            fb,
            panel.x + 14,
            panel.y + 20,
            panel.width - 28,
            1,
            PANEL_EDGE,
        );
        for index in 0..settings_page::ITEM_COUNT {
            let y = panel.y + 27 + index as i32 * 11;
            if index == 4 || index == 9 {
                fill_rect(fb, panel.x + 16, y - 4, panel.width - 32, 1, PANEL_EDGE);
            }
            if self.selected == index {
                fill_rect(fb, panel.x + 12, y - 2, panel.width - 24, 10, SELECTED);
                fill_rect(fb, panel.x + 12, y - 2, 2, 10, ACCENT);
            }
            theme::label_value(
                fb,
                settings_page::label(index),
                &settings_page::value(settings, index),
                panel.x + 20,
                y,
                panel.width - 40,
            );
        }
        theme::text(
            fb,
            settings_page::HINT,
            panel.x + 16,
            panel.y + panel.height - 11,
            1,
            MUTED,
        );
    }

    fn draw_controls(&self, fb: &mut Framebuffer, settings: &Settings) {
        let panel = centered_panel(fb, 278, 162);
        theme::panel(fb, panel.x, panel.y, panel.width, panel.height);
        theme::text(fb, controls::HEADER, panel.x + 18, panel.y + 9, 1, TEXT);
        fill_rect(
            fb,
            panel.x + 14,
            panel.y + 20,
            panel.width - 28,
            1,
            PANEL_EDGE,
        );
        if let Some(action) = self.rebinding {
            theme::centered_text(fb, "PRESS A KEY", panel.y + 63, 1, TEXT);
            theme::centered_text(fb, controls::label(action), panel.y + 78, 1, MUTED);
            theme::centered_text(fb, "ESC CANCEL", panel.y + 101, 1, MUTED);
            return;
        }

        const VISIBLE: usize = 10;
        let scroll = self.selected.saturating_sub(VISIBLE - 1);
        let total = controls::ACTIONS.len() + 1;
        for index in scroll..(scroll + VISIBLE).min(total) {
            let row_y = panel.y + 25 + (index - scroll) as i32 * 10;
            if self.selected == index {
                fill_rect(fb, panel.x + 12, row_y - 2, panel.width - 24, 9, SELECTED);
                fill_rect(fb, panel.x + 12, row_y - 2, 2, 9, ACCENT);
            }
            let (label, keys) = if index < controls::ACTIONS.len() {
                let action = controls::ACTIONS[index];
                let keys = settings
                    .bindings
                    .keys(action)
                    .iter()
                    .filter_map(|key| key_name(*key))
                    .collect::<Vec<_>>()
                    .join("/");
                (controls::label(action), keys)
            } else {
                ("BACK", String::new())
            };
            theme::label_value(fb, label, &keys, panel.x + 20, row_y, panel.width - 40);
        }
        theme::text(
            fb,
            controls::HINT,
            panel.x + 16,
            panel.y + panel.height - 21,
            1,
            MUTED,
        );
        theme::text(
            fb,
            controls::MOUSE_NOTE,
            panel.x + 16,
            panel.y + panel.height - 11,
            1,
            MUTED,
        );
    }

    fn draw_confirm(&self, fb: &mut Framebuffer) {
        let panel = centered_panel(fb, 250, 90);
        theme::panel(fb, panel.x, panel.y, panel.width, panel.height);
        theme::centered_text(fb, dialogs::MAIN_MENU_TITLE, panel.y + 14, 1, TEXT);
        theme::centered_text(fb, dialogs::MAIN_MENU_BODY, panel.y + 29, 1, MUTED);
        for (index, item) in menu::CONFIRM_ITEMS.iter().enumerate() {
            let x = panel.x + 40 + index as i32 * 100;
            if self.selected == index {
                fill_rect(fb, x - 8, panel.y + 57, 88, 16, SELECTED);
                fill_rect(fb, x - 8, panel.y + 57, 2, 16, ACCENT);
            }
            theme::text(fb, item, x, panel.y + 62, 1, TEXT);
        }
    }

    fn draw_result(&self, fb: &mut Framebuffer, game: &Game) {
        let won = game.state == State::Won;
        let (title, subtitle) = dialogs::result_copy(won);
        let panel = centered_panel(fb, 238, 125);
        theme::panel(fb, panel.x, panel.y, panel.width, panel.height);
        theme::centered_text(fb, title, panel.y + 13, 1, if won { TEXT } else { ACCENT });
        theme::centered_text(fb, subtitle, panel.y + 27, 1, MUTED);
        theme::centered_text(
            fb,
            &format!("PAGES {}/{}", game.collected, game.pages.len()),
            panel.y + 43,
            1,
            TEXT,
        );
        let elapsed = game.time.max(0.0) as u32;
        theme::centered_text(
            fb,
            &format!("TIME {:02}:{:02}", elapsed / 60, elapsed % 60),
            panel.y + 55,
            1,
            MUTED,
        );
        fill_rect(
            fb,
            panel.x + 20,
            panel.y + 69,
            panel.width - 40,
            1,
            PANEL_EDGE,
        );
        for (index, item) in menu::RESULT_ITEMS.iter().enumerate() {
            let y = panel.y + 77 + index as i32 * 13;
            self.draw_row(fb, panel, index, item, y);
        }
    }
}
