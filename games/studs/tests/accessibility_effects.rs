use ribbon_plugin_rendering::style::RenderStyle;
use studs::{
    game::Game,
    settings::{QualityPreset, VisualStyle},
};

#[test]
fn reduce_flashing_uses_steady_proximity_dimming_without_random_drops() {
    let mut game = Game::new(42);
    game.time = 100.0;
    let style = RenderStyle {
        limit_flicker: true,
        ..studs::styles::realistic()
    };
    let first = game
        .scene_styled(10, 1.0, style)
        .unwrap()
        .lighting
        .light_level;
    game.time += 0.1;
    let next = game
        .scene_styled(11, 1.0, style)
        .unwrap()
        .lighting
        .light_level;
    assert_eq!(first, next);
    assert!((0.88..=1.0).contains(&first));
}

#[test]
fn expensive_flashlight_shadows_are_gated_by_quality() {
    assert!(
        !VisualStyle::Realistic
            .render_style(QualityPreset::Low)
            .spot_light_shadows
    );
    assert!(
        !VisualStyle::Realistic
            .render_style(QualityPreset::Medium)
            .spot_light_shadows
    );
    assert!(
        VisualStyle::Realistic
            .render_style(QualityPreset::High)
            .spot_light_shadows
    );
    assert!(
        !VisualStyle::Classic
            .render_style(QualityPreset::Realistic)
            .spot_light_shadows
    );
}
