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
        self.dot(self).sqrt()
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
            forward: Vec3::new(0.0, 1.0, 0.0),
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
    pub pan: f32,
    pub attenuation: f32,
    pub gain: f32,
    pub doppler_ratio: f32,
}

/// Inverse-distance attenuation with near/far clamps and a conservative Doppler ratio.
pub fn calculate(listener: Listener3d, emitter: Emitter3d, speed_of_sound: f32) -> SpatialResult {
    let raw_delta = emitter.position - listener.position;
    let delta = if raw_delta.x.is_finite() && raw_delta.y.is_finite() && raw_delta.z.is_finite() {
        raw_delta
    } else {
        Vec3::ZERO
    };
    let distance = delta.length();
    let forward = listener.forward.normalized();
    let right = Vec3::new(forward.y, -forward.x, 0.0).normalized();
    let pan = if distance > 1.0e-5 {
        delta.normalized().dot(right).clamp(-1.0, 1.0)
    } else {
        0.0
    };
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
    let clamped_distance = distance.clamp(min_distance, max_distance);
    let rolloff = if emitter.rolloff.is_finite() {
        emitter.rolloff.clamp(0.0, 4.0)
    } else {
        1.0
    };
    let attenuation = 1.0 / (1.0 + rolloff * ((clamped_distance / min_distance) - 1.0).max(0.0));
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
    let relative_velocity = emitter.velocity - listener.velocity;
    let radial_raw = relative_velocity.dot(delta.normalized());
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
}
