//! Bounded screen-space effects.

/// Bounded screen-ray warp controlled by a normalized effect strength.
pub fn screen_warp(u: f32, v: f32, phase: f32, intensity: f32) -> [f32; 2] {
    if !u.is_finite() || !v.is_finite() || !phase.is_finite() || !intensity.is_finite() {
        return [0.0; 2];
    }
    let amount = intensity.clamp(0.0, 1.0) * 0.003;
    [
        (v * 48.0 + phase).sin() * amount,
        (u * 32.0 - phase * 0.7).sin() * amount,
    ]
}
