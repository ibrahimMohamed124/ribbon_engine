use super::*;

impl Ui {
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
