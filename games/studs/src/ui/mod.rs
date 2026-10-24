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
use ribbon_plugin_rendering::cpu::canvas::Canvas;

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

impl Ui {
    pub fn new() -> Self {
        Self {
            screen: Screen::MainMenu,
            selected: 0,
            settings_return: SETTINGS_PARENT_DEFAULT,
            controls_return: SETTINGS_PARENT_DEFAULT,
            rebinding: None,
            prev_mouse_down: false,
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

    /// Start directly in gameplay for rendering previews and example applications.
    pub fn start_playing(&mut self) {
        self.screen = Screen::Playing;
        self.selected = 0;
        self.rebinding = None;
    }

    pub fn update(&mut self, input: &UiInput, settings: &mut Settings) -> UiOutcome {
        let mut outcome = UiOutcome::default();
        let mouse_clicked = input.mouse_down && !self.prev_mouse_down;
        self.prev_mouse_down = input.mouse_down;
        let mouse_target = if self.rebinding.is_none() && input.mouse_scroll == 0.0 {
            self.mouse_target(input)
        } else {
            None
        };
        if let Some((index, _)) = mouse_target {
            self.selected = index;
        }
        let confirm = input.confirm() || (mouse_clicked && mouse_target.is_some());

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
                    if confirm {
                        match self.selected {
                            0 => {
                                self.screen = Screen::Difficulty;
                                self.selected = 0;
                            }
                            1 => self.open_settings(Screen::MainMenu),
                            2 => self.open_controls(Screen::MainMenu),
                            _ => outcome.quit = true,
                        }
                    }
                }
            }
            Screen::Difficulty => {
                if input.back() {
                    self.screen = Screen::MainMenu;
                    self.selected = 0;
                } else {
                    self.navigate(input, difficulty::ITEMS.len());
                    if confirm {
                        match self.selected {
                            0..=2 => {
                                outcome.difficulty = Some(difficulty::OPTIONS[self.selected]);
                                outcome.start_game = true;
                                self.screen = Screen::Playing;
                                self.selected = 0;
                            }
                            _ => {
                                self.screen = Screen::MainMenu;
                                self.selected = 0;
                            }
                        }
                    }
                }
            }
            Screen::Pause => {
                if input.back() {
                    self.screen = Screen::Playing;
                } else {
                    self.navigate(input, menu::PAUSE_ITEMS.len());
                    if confirm {
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
                if input.back() || (self.selected == settings_page::ITEM_COUNT - 1 && confirm) {
                    self.screen = self.settings_return;
                    self.selected = 0;
                } else {
                    self.navigate(input, settings_page::ITEM_COUNT);
                    let direction = if input.right() {
                        1
                    } else if input.left() {
                        -1
                    } else if mouse_clicked {
                        match mouse_target {
                            Some((3 | 5 | 7 | 8 | 12, _)) => 1,
                            Some((index, row)) if index < settings_page::ITEM_COUNT - 1 => {
                                if let Some((x, _)) = input.mouse_position {
                                    let value_width =
                                        settings_page::value(settings, index).chars().count()
                                            as i32
                                            * 6
                                            - 1;
                                    let value_left = row.x + row.width - 8 - value_width;
                                    if x >= value_left as f32
                                        && x < (value_left + value_width) as f32
                                    {
                                        if x < (value_left + value_width / 2) as f32 {
                                            -1
                                        } else {
                                            1
                                        }
                                    } else {
                                        0
                                    }
                                } else {
                                    0
                                }
                            }
                            _ => 0,
                        }
                    } else {
                        0
                    };
                    if direction != 0 {
                        let (changed, window_changed) =
                            settings_page::adjust(settings, self.selected, direction);
                        outcome.settings_changed |= changed;
                        outcome.window_changed |= window_changed;
                    } else if confirm && matches!(self.selected, 3 | 5 | 7 | 8 | 12) {
                        let (changed, window_changed) =
                            settings_page::adjust(settings, self.selected, 1);
                        outcome.settings_changed |= changed;
                        outcome.window_changed |= window_changed;
                    }
                }
            }
            Screen::Controls => {
                if input.back() || (self.selected == controls::ACTIONS.len() && confirm) {
                    self.screen = self.controls_return;
                    self.selected = 0;
                } else {
                    self.navigate(input, controls::ACTIONS.len() + 1);
                    if confirm {
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
                    if confirm {
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
                if confirm {
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

    fn mouse_target(&self, input: &UiInput) -> Option<(usize, layout::Rect)> {
        let (x, y) = input.mouse_position?;
        let (width, height) = input.framebuffer_size;
        if width == 0 || height == 0 {
            return None;
        }
        let size = (width, height);
        let row_target = |panel: layout::Rect,
                          first_y: i32,
                          spacing: i32,
                          row_height: i32,
                          count: usize,
                          left: i32,
                          right: i32| {
            (0..count).find_map(|index| {
                let row = layout::Rect {
                    x: panel.x + left,
                    y: first_y + index as i32 * spacing,
                    width: panel.width - left - right,
                    height: row_height,
                };
                row.contains(x, y).then_some((index, row))
            })
        };

        match self.screen {
            Screen::MainMenu => {
                let panel = Self::menu_panel(size, menu::MAIN_ITEMS.len(), 0.63);
                row_target(panel, panel.y + 27, 15, 13, menu::MAIN_ITEMS.len(), 14, 14)
            }
            Screen::Difficulty => {
                let panel = Self::menu_panel(size, difficulty::ITEMS.len(), 0.5);
                row_target(panel, panel.y + 27, 15, 13, difficulty::ITEMS.len(), 14, 14)
            }
            Screen::Pause => {
                let panel = Self::menu_panel(size, menu::PAUSE_ITEMS.len(), 0.5);
                row_target(panel, panel.y + 27, 15, 13, menu::PAUSE_ITEMS.len(), 14, 14)
            }
            Screen::Settings => {
                let panel = layout::centered_panel_for_size(width, height, 266, 178);
                row_target(
                    panel,
                    panel.y + 25,
                    10,
                    10,
                    settings_page::ITEM_COUNT,
                    12,
                    12,
                )
            }
            Screen::Controls => {
                const VISIBLE: usize = 10;
                let panel = layout::centered_panel_for_size(width, height, 278, 162);
                let scroll = self.selected.saturating_sub(VISIBLE - 1);
                let total = controls::ACTIONS.len() + 1;
                (scroll..(scroll + VISIBLE).min(total)).find_map(|index| {
                    let row_y = panel.y + 23 + (index - scroll) as i32 * 10;
                    let row = layout::Rect {
                        x: panel.x + 12,
                        y: row_y,
                        width: panel.width - 24,
                        height: 9,
                    };
                    row.contains(x, y).then_some((index, row))
                })
            }
            Screen::ConfirmMainMenu => {
                let panel = layout::centered_panel_for_size(width, height, 250, 90);
                (0..menu::CONFIRM_ITEMS.len()).find_map(|index| {
                    let row = layout::Rect {
                        x: panel.x + 32 + index as i32 * 100,
                        y: panel.y + 57,
                        width: 88,
                        height: 16,
                    };
                    row.contains(x, y).then_some((index, row))
                })
            }
            Screen::Result => {
                let panel = layout::centered_panel_for_size(width, height, 238, 158);
                row_target(
                    panel,
                    panel.y + 94,
                    13,
                    12,
                    menu::RESULT_ITEMS.len(),
                    14,
                    14,
                )
            }
            Screen::Playing => None,
        }
    }

    fn menu_panel(size: (usize, usize), item_count: usize, y_ratio: f32) -> layout::Rect {
        let panel =
            layout::centered_panel_for_size(size.0, size.1, 226, 22 + item_count as i32 * 15 + 24);
        let y = ((size.1 as f32 * y_ratio) as i32 - panel.height / 2)
            .clamp(4, size.1 as i32 - panel.height - 4);
        layout::Rect { y, ..panel }
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

    pub fn draw(&self, fb: &mut impl Canvas, frame: u32, game: &Game, settings: &Settings) {
        self.draw_model(
            fb,
            frame,
            GameReadView::from_game(game).ui_state(),
            settings,
        );
    }

    /// Draw UI from the value model extracted by ECS systems. Runtime rendering does not need
    /// to reconstruct or retain the legacy aggregate `Game` object.
    pub fn draw_model(
        &self,
        fb: &mut impl Canvas,
        frame: u32,
        game: GameUiState,
        settings: &Settings,
    ) {
        if self.screen == Screen::Playing {
            return;
        }
        match self.screen {
            Screen::MainMenu => theme::fog_background(fb, frame),
            Screen::Difficulty => theme::fog_background(fb, frame),
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
            Screen::Difficulty => self.draw_menu(
                fb,
                difficulty::TITLE,
                difficulty::ITEMS,
                difficulty::hint(self.selected),
            ),
            Screen::Pause => self.draw_menu(fb, pause::TITLE, menu::PAUSE_ITEMS, pause::HINT),
            Screen::Settings => self.draw_settings(fb, settings),
            Screen::Controls => self.draw_controls(fb, settings),
            Screen::ConfirmMainMenu => self.draw_confirm(fb),
            Screen::Result => self.draw_result(fb, game, settings),
            Screen::Playing => {}
        }
    }

    /// Draw short, timed gameplay instructions above the player HUD.
    pub fn draw_game_hints(&self, fb: &mut impl Canvas, game: &Game) {
        self.draw_game_hints_model(fb, GameReadView::from_game(game).ui_state());
    }

    /// Draw the gameplay hints using a compact ECS-extracted model.
    pub fn draw_game_hints_model(&self, fb: &mut impl Canvas, game: GameUiState) {
        if self.screen != Screen::Playing {
            return;
        }
        let mut y = 22;
        if game.time < 7.0 {
            fill_rect(fb, 0, y - 3, fb.width() as i32, 12, 0x0007_090b);
            theme::centered_text(fb, "GOAL: FIND ALL 6 PAGES", y, 1, TEXT);
            y += 12;
        }
        if game.enemy_tip_active {
            fill_rect(fb, 0, y - 3, fb.width() as i32, 12, 0x0007_090b);
            theme::centered_text(fb, "KEEP THE CREATURE IN SIGHT", y, 1, ACCENT);
        }
    }

    fn draw_main(&self, fb: &mut impl Canvas) {
        theme::centered_text(fb, "STUDS", 14, 3, TEXT);
        theme::centered_text(fb, "THE MAZE IS LISTENING", 41, 1, MUTED);
        self.draw_menu_at(
            fb,
            "MAIN MENU",
            menu::MAIN_ITEMS,
            "HOVER + CLICK   ARROWS / ENTER",
            0.63,
        );
        theme::centered_text(
            fb,
            "WASD MOVE   ARROWS LOOK   Q QUIT",
            fb.height() as i32 - 10,
            1,
            MUTED,
        );
    }

    fn draw_menu(&self, fb: &mut impl Canvas, title: &str, items: &[&str], hint: &str) {
        self.draw_menu_at(fb, title, items, hint, 0.5);
    }

    fn draw_menu_at(
        &self,
        fb: &mut impl Canvas,
        title: &str,
        items: &[&str],
        hint: &str,
        y_ratio: f32,
    ) {
        let panel = Self::menu_panel((fb.width(), fb.height()), items.len(), y_ratio);
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
        fb: &mut impl Canvas,
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

    fn draw_settings(&self, fb: &mut impl Canvas, settings: &Settings) {
        let panel = layout::centered_panel(fb, 266, 178);
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
            let y = panel.y + 27 + index as i32 * 9;
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

    fn draw_controls(&self, fb: &mut impl Canvas, settings: &Settings) {
        let panel = layout::centered_panel(fb, 278, 162);
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

    fn draw_confirm(&self, fb: &mut impl Canvas) {
        let panel = layout::centered_panel(fb, 250, 90);
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

    fn draw_result(&self, fb: &mut impl Canvas, game: GameUiState, settings: &Settings) {
        let won = game.state == State::Won;
        let (title, subtitle) = dialogs::result_copy(won);
        let panel = layout::centered_panel(fb, 238, 158);
        theme::panel(fb, panel.x, panel.y, panel.width, panel.height);
        theme::centered_text(fb, title, panel.y + 13, 1, if won { TEXT } else { ACCENT });
        theme::centered_text(fb, subtitle, panel.y + 27, 1, MUTED);
        theme::centered_text(
            fb,
            &format!("DIFFICULTY {}", game.difficulty.label()),
            panel.y + 41,
            1,
            TEXT,
        );
        theme::centered_text(
            fb,
            &format!("PAGES {}/{}", game.collected, game.page_count),
            panel.y + 53,
            1,
            TEXT,
        );
        let elapsed = game.time.max(0.0) as u32;
        theme::centered_text(
            fb,
            &format!("TIME {:02}:{:02}", elapsed / 60, elapsed % 60),
            panel.y + 65,
            1,
            MUTED,
        );
        let best = settings
            .best_time(game.difficulty)
            .map(format_time)
            .unwrap_or_else(|| "--:--".to_owned());
        theme::centered_text(fb, &format!("BEST {best}"), panel.y + 77, 1, MUTED);
        fill_rect(
            fb,
            panel.x + 20,
            panel.y + 88,
            panel.width - 40,
            1,
            PANEL_EDGE,
        );
        for (index, item) in menu::RESULT_ITEMS.iter().enumerate() {
            let y = panel.y + 96 + index as i32 * 13;
            self.draw_row(fb, panel, index, item, y);
        }
        theme::centered_text(fb, "R PLAY AGAIN   ESC MENU", panel.y + 145, 1, MUTED);
    }
}

fn format_time(seconds: f32) -> String {
    let elapsed = seconds.max(0.0) as u32;
    format!("{:02}:{:02}", elapsed / 60, elapsed % 60)
}
