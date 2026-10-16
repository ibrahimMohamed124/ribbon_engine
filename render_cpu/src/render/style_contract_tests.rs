use super::lighting::light_with_style;
use engine_scene::{RenderStyle, RENDER_DIST};

#[test]
fn moody_reference_surfaces_meet_the_lighting_targets() {
    let style = RenderStyle::moody();
    let level = |gray: f32| style.level_for(gray);
    let surface = |gray: f32, distance: f32, flashlight: f32| {
        let lighting = light_with_style(flashlight, distance, 1.0, flashlight, &style);
        level(gray * lighting)
    };
    let dark_1 = surface(0.55, 1.0, 0.0);
    let dark_3 = surface(0.55, 3.0, 0.0);
    let dark_12 = surface(0.55, 12.0, 0.0);
    let beam_3 = surface(0.55, 3.0, 1.0);
    let beam_6 = surface(0.55, 6.0, 1.0);
    let body_dark = surface(0.10, 3.0, 0.0);
    let body_beam = surface(0.10, 3.0, 1.0);
    let eyes = level(1.0 * (1.0 - 4.0 / (RENDER_DIST * 1.4)));
    let page = level(0.92 * (1.0 - 6.0 / (RENDER_DIST * 1.4)));

    assert!((2.0..=3.0).contains(&dark_1), "wall d=1: {dark_1}");
    assert!((0.8..=1.6).contains(&dark_3), "wall d=3: {dark_3}");
    assert!(dark_12 <= 0.6, "wall d=12: {dark_12}");
    assert!((8.0..=10.0).contains(&beam_3), "wall beam d=3: {beam_3}");
    assert!((5.0..=7.0).contains(&beam_6), "wall beam d=6: {beam_6}");
    assert!(body_dark <= 0.6, "enemy dark: {body_dark}");
    assert!((3.0..=4.5).contains(&body_beam), "enemy beam: {body_beam}");
    assert!(eyes >= 11.0, "enemy eyes: {eyes}");
    assert!(page >= 10.5, "page: {page}");
    assert!(
        beam_3 / dark_3 >= 6.0,
        "beam/dark ratio: {}",
        beam_3 / dark_3
    );
    assert!(level(1.7) >= 12.0, "upper levels: {}", level(1.7));
}
