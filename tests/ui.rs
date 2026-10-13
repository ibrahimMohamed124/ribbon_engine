use minifb::Key;
use raycast_blocks::game::Game;
use raycast_blocks::input::Action;
use raycast_blocks::renderer::Framebuffer;
use raycast_blocks::settings::Settings;
use raycast_blocks::ui::{Screen, Ui, UiInput};

fn press(key: Key) -> UiInput {
    UiInput::from_keys(vec![key])
}

#[test]
fn main_menu_starts_a_new_run() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    let outcome = ui.update(&press(Key::Enter), &mut settings);
    assert!(outcome.start_game);
    assert_eq!(ui.screen(), Screen::Playing);
}

#[test]
fn escape_pauses_and_resumes_without_a_restart() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    ui.update(&press(Key::Enter), &mut settings);
    ui.update(&press(Key::Escape), &mut settings);
    assert_eq!(ui.screen(), Screen::Pause);
    ui.update(&press(Key::Escape), &mut settings);
    assert_eq!(ui.screen(), Screen::Playing);
}

#[test]
fn restart_key_restarts_from_pause_and_result_screens() {
    let mut ui = Ui::new();
    let mut settings = Settings::default();
    ui.update(&press(Key::Enter), &mut settings);
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
    ui.update(&press(Key::Enter), &mut settings);
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

    ui.update(&press(Key::Enter), &mut settings);
    ui.update(&press(Key::Escape), &mut settings);
    game.draw(&mut framebuffer, 1, settings.hfov_radians());
    ui.draw(&mut framebuffer, 1, &game, &settings);
    assert_eq!(framebuffer.pixels.len(), 320 * 180);
}
