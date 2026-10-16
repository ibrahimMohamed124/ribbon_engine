use raycast_blocks::{
    game::Game,
    renderer::RenderStyle,
    settings::{QualityPreset, VisualStyle},
};

#[test]
fn reduce_flashing_uses_steady_proximity_dimming_without_random_drops() {
    let mut game = Game::new(42);
    game.time = 100.0;
    let style = RenderStyle {
        reduce_flashing: true,
        ..RenderStyle::realistic()
    };
    let first = game
        .scene_styled(10, 1.0, style)
        .unwrap()
        .lighting
        .illumination;
    game.time += 0.1;
    let next = game
        .scene_styled(11, 1.0, style)
        .unwrap()
        .lighting
        .illumination;
    assert_eq!(first, next);
    assert!((0.88..=1.0).contains(&first));
}

#[test]
fn expensive_flashlight_shadows_are_gated_by_quality() {
    assert!(
        !VisualStyle::Realistic
            .render_style(QualityPreset::Low)
            .flashlight_shadows
    );
    assert!(
        !VisualStyle::Realistic
            .render_style(QualityPreset::Medium)
            .flashlight_shadows
    );
    assert!(
        VisualStyle::Realistic
            .render_style(QualityPreset::High)
            .flashlight_shadows
    );
    assert!(
        !VisualStyle::Classic
            .render_style(QualityPreset::Realistic)
            .flashlight_shadows
    );
}
