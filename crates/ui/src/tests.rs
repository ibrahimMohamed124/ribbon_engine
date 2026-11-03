//! Public-surface, headless UI integration tests.
use super::*;
use ribbon_app::App;

#[test]
fn ui_scope_builds_a_panel_with_widgets_and_focus_targets() {
    let mut app = App::new();
    app.insert_resource(UiContext::synthetic(
        Rect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        },
        UiInput::default(),
    ));
    app.insert_resource(TextSystem::new());

    ui_scope(&mut app.world, |ui| {
        ui.panel("Main", Anchor::Center, PanelSize::Width(360.0), |ui| {
            ui.heading("Ribbon UI");
            ui.label("مرحبا بك");
            ui.button("Continue");
        });
    });

    let context = app.world.resource::<UiContext>();
    assert!(
        !context.draw.batches.is_empty(),
        "panel and button shapes should be queued"
    );
    assert_eq!(
        context.focus.current.len(),
        1,
        "the button should be keyboard-focusable"
    );
}

#[test]
fn public_ui_settings_have_stable_safe_defaults() {
    let settings = UiSettings::default();
    assert_eq!(settings.scale, 1.0);
    assert!(settings.respect_dpi);
}

#[test]
fn context_theme_constructor_preserves_custom_theme_and_default_frame_state() {
    let mut theme = Theme::dark();
    theme.font_size = 19.0;
    let context = UiContext::with_theme(theme);

    assert_eq!(context.theme.font_size, 19.0);
    assert_eq!(context.frame, 0);
    assert!(context.draw.batches.is_empty());
}
