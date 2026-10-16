use super::lighting::light_with_style;
use voxelworks_scene::RenderStyle;

#[test]
fn generic_lighting_response_stays_finite_and_fades_with_distance() {
    let style = RenderStyle {
        ambient: 0.01,
        ambient_fill: 0.12,
        ..RenderStyle::default()
    };
    let levels: Vec<_> = (1..=12)
        .map(|distance| light_with_style(1.0, distance as f32, 1.0, 1.0, &style))
        .collect();
    assert!(levels.iter().all(|level| level.is_finite()));
    assert!(levels.windows(2).all(|pair| pair[0] >= pair[1]));
}
