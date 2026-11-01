//! Deterministic 3D spatial-audio calculations, independent of the output backend.
//! Kira receives listener-relative gain and stereo pan from these calculations. This baseline does not claim HRTF or binaural rendering.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }
    pub fn length(self) -> f32 {
        self.x.hypot(self.y).hypot(self.z)
    }
    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }
    pub fn normalized(self) -> Self {
        let length = self.length();
        if length > 1.0e-6 && length.is_finite() {
            Self::new(self.x / length, self.y / length, self.z / length)
        } else {
            Self::ZERO
        }
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl std::ops::Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl std::ops::Mul<f32> for Vec3 {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener3d {
    pub position: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub velocity: Vec3,
}

impl Default for Listener3d {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            // Matches ribbon_math::forward_vec(Angles::default()).
            forward: Vec3::new(1.0, 0.0, 0.0),
            up: Vec3::new(0.0, 0.0, 1.0),
            velocity: Vec3::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emitter3d {
    pub position: Vec3,
    pub velocity: Vec3,
    pub min_distance: f32,
    pub max_distance: f32,
    pub rolloff: f32,
    pub gain: f32,
    pub occlusion: f32,
}

impl Default for Emitter3d {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            min_distance: 1.0,
            max_distance: 100.0,
            rolloff: 1.0,
            gain: 1.0,
            occlusion: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialResult {
    pub distance: f32,
    /// Stereo pan from the listener's local right axis. This is not an HRTF renderer.
    pub pan: f32,
    pub attenuation: f32,
    pub gain: f32,
    pub doppler_ratio: f32,
    /// Normalized source direction in listener-local right/forward/up coordinates.
    pub local_direction: Vec3,
    /// Signed azimuth in radians; positive means listener-right.
    pub azimuth_radians: f32,
    /// Signed elevation in radians; positive means listener-up.
    pub elevation_radians: f32,
}

/// Supported distance attenuation curves. Sources beyond `max_distance` are silent for all models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceModel {
    /// Classic inverse-distance rolloff, normalized to unity at the near distance.
    Inverse,
    /// Linear interpolation from unity at the near distance to silence at the far distance.
    Linear,
    /// Exponential falloff, normalized to unity at the near distance.
    Exponential,
}

/// Inverse-distance attenuation with near/far limits and a conservative Doppler ratio.
pub fn calculate(listener: Listener3d, emitter: Emitter3d, speed_of_sound: f32) -> SpatialResult {
    calculate_with_model(listener, emitter, speed_of_sound, DistanceModel::Inverse)
}

/// Calculate listener-local 3D direction, a stereo-compatible pan approximation, attenuation,
/// and Doppler. Ribbon scenes use +Z up and `forward_vec(Angles::default())` = +X; listener
/// `forward` and `up` define orientation. Positive pan is listener-right.
pub fn calculate_with_model(
    listener: Listener3d,
    emitter: Emitter3d,
    speed_of_sound: f32,
    model: DistanceModel,
) -> SpatialResult {
    // Subtract in f64 first so large, finite f32 world coordinates cannot overflow to infinity.
    let delta = Vec3::new(
        ((emitter.position.x as f64) - (listener.position.x as f64))
            .clamp(-(f32::MAX as f64), f32::MAX as f64) as f32,
        ((emitter.position.y as f64) - (listener.position.y as f64))
            .clamp(-(f32::MAX as f64), f32::MAX as f64) as f32,
        ((emitter.position.z as f64) - (listener.position.z as f64))
            .clamp(-(f32::MAX as f64), f32::MAX as f64) as f32,
    );
    let distance = delta.length();
    let distance = if distance.is_finite() {
        distance
    } else {
        f32::MAX
    };

    let mut forward = listener.forward.normalized();
    if forward == Vec3::ZERO {
        forward = Vec3::new(1.0, 0.0, 0.0);
    }
    let mut up = listener.up;
    if !up.x.is_finite() || !up.y.is_finite() || !up.z.is_finite() {
        up = Vec3::new(0.0, 0.0, 1.0);
    }
    // Gram-Schmidt makes the listener basis orthonormal and stable when forward/up are not.
    up = (up - forward * up.dot(forward)).normalized();
    if up == Vec3::ZERO {
        let fallback = if forward.z.abs() < 0.9 {
            Vec3::new(0.0, 0.0, 1.0)
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        up = (fallback - forward * fallback.dot(forward)).normalized();
    }
    let right = forward.cross(up).normalized();
    let up = right.cross(forward).normalized();

    let direction = delta.normalized();
    let local_direction = Vec3::new(
        direction.dot(right),
        direction.dot(forward),
        direction.dot(up),
    );
    let horizontal = local_direction.x.hypot(local_direction.y);
    let pan = if distance > 1.0e-5 && horizontal > 1.0e-5 {
        (local_direction.x / horizontal).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let azimuth_radians = local_direction.x.atan2(local_direction.y);
    let elevation_radians = local_direction.z.atan2(horizontal);
    let min_distance = if emitter.min_distance.is_finite() {
        emitter.min_distance.max(0.01)
    } else {
        1.0
    };
    let max_distance = if emitter.max_distance.is_finite() {
        emitter.max_distance.max(min_distance)
    } else {
        100.0
    };
    let rolloff = if emitter.rolloff.is_finite() {
        emitter.rolloff.clamp(0.0, 4.0)
    } else {
        1.0
    };
    let attenuation = if distance > max_distance {
        0.0
    } else {
        let clamped_distance = distance.max(min_distance);
        match model {
            DistanceModel::Inverse => {
                1.0 / (1.0 + rolloff * ((clamped_distance / min_distance) - 1.0).max(0.0))
            }
            DistanceModel::Linear => {
                let span = (max_distance - min_distance).max(f32::EPSILON);
                (1.0 - rolloff.min(1.0) * ((clamped_distance - min_distance) / span))
                    .clamp(0.0, 1.0)
            }
            DistanceModel::Exponential => {
                (min_distance / clamped_distance.max(min_distance)).powf(rolloff)
            }
        }
    };
    let occlusion = if emitter.occlusion.is_finite() {
        emitter.occlusion.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let base_gain = if emitter.gain.is_finite() {
        emitter.gain.clamp(0.0, 4.0)
    } else {
        0.0
    };
    let gain = base_gain * attenuation * (1.0 - 0.85 * occlusion);
    let sound_speed = if speed_of_sound.is_finite() {
        speed_of_sound.clamp(100.0, 1000.0)
    } else {
        343.0
    };
    let finite_velocity = |velocity: Vec3| {
        if velocity.x.is_finite() && velocity.y.is_finite() && velocity.z.is_finite() {
            velocity
        } else {
            Vec3::ZERO
        }
    };
    let relative_velocity = finite_velocity(emitter.velocity) - finite_velocity(listener.velocity);
    let radial_raw = relative_velocity.dot(direction);
    let radial_velocity = if radial_raw.is_finite() {
        radial_raw
    } else {
        0.0
    };
    let doppler_ratio = (sound_speed
        / (sound_speed + radial_velocity.clamp(-0.8 * sound_speed, 4.0 * sound_speed)))
    .clamp(0.5, 2.0);
    SpatialResult {
        distance,
        pan: if pan.is_finite() { pan } else { 0.0 },
        attenuation,
        gain: gain.clamp(0.0, 4.0),
        doppler_ratio,
        local_direction,
        azimuth_radians: if azimuth_radians.is_finite() {
            azimuth_radians
        } else {
            0.0
        },
        elevation_radians: if elevation_radians.is_finite() {
            elevation_radians
        } else {
            0.0
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcousticZone {
    pub id: u32,
    pub wet_mix: f32,
    pub low_pass_hz: f32,
    pub reverb_seconds: f32,
    pub ambience_gain: f32,
}

impl AcousticZone {
    pub fn sanitized(self) -> Self {
        Self {
            wet_mix: finite_clamp(self.wet_mix, 0.0, 1.0, 0.0),
            low_pass_hz: finite_clamp(self.low_pass_hz, 20.0, 20_000.0, 20_000.0),
            reverb_seconds: finite_clamp(self.reverb_seconds, 0.0, 10.0, 0.0),
            ambience_gain: finite_clamp(self.ambience_gain, 0.0, 2.0, 1.0),
            ..self
        }
    }
}

fn finite_clamp(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spatial_result_is_finite_for_bad_inputs() {
        let result = calculate(
            Listener3d::default(),
            Emitter3d {
                position: Vec3::new(f32::NAN, 0.0, 0.0),
                gain: f32::INFINITY,
                occlusion: f32::NAN,
                ..Emitter3d::default()
            },
            343.0,
        );
        assert!(
            result.pan.is_finite() && result.gain.is_finite() && result.doppler_ratio.is_finite()
        );
    }

    #[test]
    fn occlusion_reduces_gain() {
        let clear = calculate(Listener3d::default(), Emitter3d::default(), 343.0);
        let blocked = calculate(
            Listener3d::default(),
            Emitter3d {
                occlusion: 1.0,
                ..Emitter3d::default()
            },
            343.0,
        );
        assert!(blocked.gain < clear.gain);
    }

    #[test]
    fn listener_basis_respects_rotation_and_elevation() {
        let listener = Listener3d {
            position: Vec3::ZERO,
            forward: Vec3::new(1.0, 0.0, 0.0),
            up: Vec3::new(0.0, 0.0, 1.0),
            velocity: Vec3::ZERO,
        };
        let right = calculate(
            listener,
            Emitter3d {
                position: Vec3::new(0.0, -10.0, 0.0),
                ..Emitter3d::default()
            },
            343.0,
        );
        assert!(right.pan > 0.99);
        assert!(right.azimuth_radians > 1.5);

        let above = calculate(
            listener,
            Emitter3d {
                position: Vec3::new(0.0, 0.0, 10.0),
                ..Emitter3d::default()
            },
            343.0,
        );
        assert!(above.elevation_radians > 1.5);
    }

    #[test]
    fn distance_models_are_distinct_and_far_cutoff_is_silent() {
        let emitter = Emitter3d {
            position: Vec3::new(0.0, 10.0, 0.0),
            min_distance: 1.0,
            max_distance: 20.0,
            rolloff: 1.0,
            ..Emitter3d::default()
        };
        let listener = Listener3d::default();
        let inverse = calculate_with_model(listener, emitter, 343.0, DistanceModel::Inverse);
        let linear = calculate_with_model(listener, emitter, 343.0, DistanceModel::Linear);
        let exponential =
            calculate_with_model(listener, emitter, 343.0, DistanceModel::Exponential);
        assert!((0.0..1.0).contains(&inverse.attenuation));
        assert!((0.0..1.0).contains(&linear.attenuation));
        assert!((0.0..1.0).contains(&exponential.attenuation));
        assert_ne!(inverse.attenuation, linear.attenuation);

        let beyond = calculate(
            listener,
            Emitter3d {
                position: Vec3::new(0.0, 21.0, 0.0),
                ..emitter
            },
            343.0,
        );
        assert_eq!(beyond.attenuation, 0.0);
        assert_eq!(beyond.gain, 0.0);
    }
}
