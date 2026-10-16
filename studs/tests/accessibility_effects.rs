use raycast_blocks::{game::Game, renderer::RenderStyle};

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
