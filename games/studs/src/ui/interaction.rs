use super::*;

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

    pub(super) fn menu_panel(
        size: (usize, usize),
        item_count: usize,
        y_ratio: f32,
    ) -> layout::Rect {
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
}
