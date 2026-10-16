use studs::game::{Difficulty, Game};
use studs::input::Action;
use studs::input::Key;
use studs::settings::Settings;
use studs::ui::{Screen, Ui, UiInput};
use voxelworks_api::Framebuffer;

fn press(key: Key) -> UiInput {
    UiInput::from_keys(vec![key])
}

fn pointer(x: f32, y: f32, down: bool, scroll: f32) -> UiInput {
    UiInput::from_keys(vec![]).with_mouse(Some((x, y)), down, scroll, (320, 180))
}

fn start_normal(ui: &mut Ui, settings: &mut Settings) {
    ui.update(&press(Key::Enter), settings);
    assert_eq!(ui.screen(), Screen::Difficulty);
    ui.update(&press(Key::Down), settings);
    ui.update(&press(Key::Enter), settings);
}

#[test]
fn main_menu_starts_a_new_run() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    let opened = ui.update(&press(Key::Enter), &mut settings);
    assert!(!opened.start_game);
    assert_eq!(ui.screen(), Screen::Difficulty);
    ui.update(&press(Key::Down), &mut settings);
    let outcome = ui.update(&press(Key::Enter), &mut settings);
    assert!(outcome.start_game);
    assert_eq!(outcome.difficulty, Some(Difficulty::Normal));
    assert_eq!(ui.screen(), Screen::Playing);
}

#[test]
fn difficulty_screen_selects_modes_and_can_return_to_main_menu() {
    for (down_count, expected) in [
        (0, Difficulty::Easy),
        (1, Difficulty::Normal),
        (2, Difficulty::Hard),
    ] {
        let mut ui = Ui::new();
        let mut settings = Settings::default();
        ui.update(&press(Key::Enter), &mut settings);
        for _ in 0..down_count {
            ui.update(&press(Key::Down), &mut settings);
        }
        let selected = ui.update(&press(Key::Enter), &mut settings);
        assert!(selected.start_game);
        assert_eq!(selected.difficulty, Some(expected));
    }

    let mut ui = Ui::new();
    let mut settings = Settings::default();
    ui.update(&press(Key::Enter), &mut settings);
    ui.update(&press(Key::Escape), &mut settings);
    assert_eq!(ui.screen(), Screen::MainMenu);
}

#[test]
fn mouse_can_select_a_difficulty() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    ui.update(&pointer(160.0, 90.0, false, 0.0), &mut settings);
    ui.update(&pointer(160.0, 90.0, true, 0.0), &mut settings);
    assert_eq!(ui.screen(), Screen::Difficulty);

    ui.update(&pointer(160.0, 100.0, false, 0.0), &mut settings);
    let hard = ui.update(&pointer(160.0, 100.0, true, 0.0), &mut settings);
    assert!(hard.start_game);
    assert_eq!(hard.difficulty, Some(Difficulty::Hard));
}

#[test]
fn escape_pauses_and_resumes_without_a_restart() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    start_normal(&mut ui, &mut settings);
    ui.update(&press(Key::Escape), &mut settings);
    assert_eq!(ui.screen(), Screen::Pause);
    ui.update(&press(Key::Escape), &mut settings);
    assert_eq!(ui.screen(), Screen::Playing);
}

#[test]
fn restart_key_restarts_from_pause_and_result_screens() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    start_normal(&mut ui, &mut settings);
    ui.update(&press(Key::Escape), &mut settings);
    let paused = ui.update(&press(Key::R), &mut settings);
    assert!(paused.restart_game);
    assert_eq!(ui.screen(), Screen::Playing);

    ui.show_result();
    let result = ui.update(&press(Key::R), &mut settings);
    assert!(result.start_game);
    assert_eq!(ui.screen(), Screen::Playing);
}

#[test]
fn settings_are_changed_immediately_and_request_window_rebuilds() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    ui.update(&press(Key::Down), &mut settings);
    ui.update(&press(Key::Enter), &mut settings);
    assert_eq!(ui.screen(), Screen::Settings);

    let volume_before = settings.master_volume;
    let changed = ui.update(&press(Key::Right), &mut settings);
    assert!(changed.settings_changed);
    assert!(!changed.window_changed);
    assert!(settings.master_volume > volume_before);

    // Window scale is row nine in the settings screen.
    for _ in 0..9 {
        ui.update(&press(Key::Down), &mut settings);
    }
    let resized = ui.update(&press(Key::Right), &mut settings);
    assert!(resized.window_changed);
}

#[test]
fn visual_style_can_be_changed_from_the_settings_screen() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    ui.update(&press(Key::Down), &mut settings);
    ui.update(&press(Key::Enter), &mut settings);
    for _ in 0..12 {
        ui.update(&press(Key::Down), &mut settings);
    }
    assert_eq!(studs::ui::settings::value(&settings, 12), "REALISTIC");
    let changed = ui.update(&press(Key::Right), &mut settings);
    assert!(changed.settings_changed);
    assert_eq!(studs::ui::settings::value(&settings, 12), "CLASSIC");
    ui.update(&press(Key::Right), &mut settings);
    assert_eq!(studs::ui::settings::value(&settings, 12), "MOODY");
}

#[test]
fn controls_can_rebind_and_return_to_their_parent() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    ui.update(&press(Key::Down), &mut settings);
    ui.update(&press(Key::Down), &mut settings);
    ui.update(&press(Key::Enter), &mut settings);
    assert_eq!(ui.screen(), Screen::Controls);
    ui.update(&press(Key::Enter), &mut settings);
    let rebound = ui.update(&press(Key::J), &mut settings);
    assert!(rebound.settings_changed);
    assert_eq!(settings.bindings.keys(Action::Forward), &[Key::J]);
    ui.update(&press(Key::Escape), &mut settings);
    assert_eq!(ui.screen(), Screen::MainMenu);
}

#[test]
fn leaving_a_paused_run_requires_confirmation() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    start_normal(&mut ui, &mut settings);
    ui.update(&press(Key::Escape), &mut settings);
    for _ in 0..4 {
        ui.update(&press(Key::Down), &mut settings);
    }
    ui.update(&press(Key::Enter), &mut settings);
    assert_eq!(ui.screen(), Screen::ConfirmMainMenu);
    ui.update(&press(Key::Down), &mut settings);
    let leave = ui.update(&press(Key::Enter), &mut settings);
    assert!(leave.return_to_menu);
    assert_eq!(ui.screen(), Screen::MainMenu);
}

#[test]
fn menu_layers_render_without_changing_the_game_framebuffer_size() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    let game = Game::new(4);
    let mut framebuffer = Framebuffer::new(320, 180);

    ui.draw(&mut framebuffer, 0, &game, &settings);
    assert!(framebuffer.pixels.iter().any(|&pixel| pixel != 0));

    start_normal(&mut ui, &mut settings);
    ui.update(&press(Key::Escape), &mut settings);
    game.draw(&mut framebuffer, 1, settings.hfov_radians());
    ui.draw(&mut framebuffer, 1, &game, &settings);
    assert_eq!(framebuffer.pixels.len(), 320 * 180);
}

#[test]
fn mouse_hover_and_click_open_settings_and_adjust_a_value() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();

    ui.update(&pointer(160.0, 105.0, false, 0.0), &mut settings);
    assert_eq!(ui.screen(), Screen::MainMenu);
    ui.update(&pointer(160.0, 105.0, true, 0.0), &mut settings);
    assert_eq!(ui.screen(), Screen::Settings);

    let before = settings.hfov_degrees;
    ui.update(&pointer(263.0, 98.0, false, 0.0), &mut settings);
    let changed = ui.update(&pointer(263.0, 98.0, true, 0.0), &mut settings);
    assert!(changed.settings_changed);
    assert_eq!(settings.hfov_degrees, before + 5.0);
}

#[test]
fn mouse_can_rebind_controls_and_scroll_to_back() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    ui.update(&pointer(160.0, 120.0, false, 0.0), &mut settings);
    ui.update(&pointer(160.0, 120.0, true, 0.0), &mut settings);
    assert_eq!(ui.screen(), Screen::Controls);

    ui.update(&pointer(160.0, 35.0, false, 0.0), &mut settings);
    ui.update(&pointer(160.0, 35.0, true, 0.0), &mut settings);
    let rebound = ui.update(&press(Key::J), &mut settings);
    assert!(rebound.settings_changed);
    assert_eq!(settings.bindings.keys(Action::Forward), &[Key::J]);

    ui.update(&press(Key::Escape), &mut settings);
    assert_eq!(ui.screen(), Screen::MainMenu);
    ui.update(&press(Key::Down), &mut settings);
    ui.update(&press(Key::Down), &mut settings);
    ui.update(&press(Key::Enter), &mut settings);
    assert_eq!(ui.screen(), Screen::Controls);

    for _ in 0..14 {
        ui.update(&pointer(0.0, 0.0, false, -1.0), &mut settings);
    }
    ui.update(&pointer(160.0, 125.0, false, 0.0), &mut settings);
    ui.update(&pointer(160.0, 125.0, true, 0.0), &mut settings);
    assert_eq!(ui.screen(), Screen::MainMenu);
}

#[test]
fn mouse_navigates_pause_confirmation_and_result_screens() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    start_normal(&mut ui, &mut settings);
    ui.update(&press(Key::Escape), &mut settings);
    assert_eq!(ui.screen(), Screen::Pause);

    ui.update(&pointer(160.0, 63.0, false, 0.0), &mut settings);
    ui.update(&pointer(160.0, 63.0, true, 0.0), &mut settings);
    assert_eq!(ui.screen(), Screen::Playing);

    ui.update(&press(Key::Escape), &mut settings);
    ui.update(&pointer(160.0, 123.0, false, 0.0), &mut settings);
    ui.update(&pointer(160.0, 123.0, true, 0.0), &mut settings);
    assert_eq!(ui.screen(), Screen::ConfirmMainMenu);

    ui.update(&pointer(210.0, 110.0, false, 0.0), &mut settings);
    let leave = ui.update(&pointer(210.0, 110.0, true, 0.0), &mut settings);
    assert!(leave.return_to_menu);
    assert_eq!(ui.screen(), Screen::MainMenu);

    ui.show_result();
    ui.update(&pointer(160.0, 112.0, false, 0.0), &mut settings);
    let replay = ui.update(&pointer(160.0, 112.0, true, 0.0), &mut settings);
    assert!(replay.start_game);
    assert_eq!(ui.screen(), Screen::Playing);
}
