//! Rotated box geometry and ray intersection for renderable objects.

use engine_core::math::Vect;

pub type Box3 = ([f32; 3], [f32; 3], u8);
pub type Bounds3 = ([f32; 3], [f32; 3]);
pub type SurfaceFn = fn([f32; 3], usize, f32, u8) -> EntitySurface;

#[derive(Clone, Copy, Debug)]
pub struct EntitySurface {
    pub gray: f32,
    pub emissive: bool,
}

pub fn ray_box(o: [f32; 3], d: [f32; 3], mn: [f32; 3], mx: [f32; 3]) -> Option<(f32, usize, f32)> {
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;
    let mut axis = 0usize;
    let mut sign = 1.0f32;
    for i in 0..3 {
        if d[i].abs() < 1e-8 {
            if o[i] < mn[i] || o[i] > mx[i] {
                return None;
            }
        } else {
            let inv = 1.0 / d[i];
            let mut t1 = (mn[i] - o[i]) * inv;
            let mut t2 = (mx[i] - o[i]) * inv;
            let mut s = -1.0;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
                s = 1.0;
            }
            if t1 > tmin {
                tmin = t1;
                axis = i;
                sign = s;
            }
            if t2 < tmax {
                tmax = t2;
            }
            if tmin > tmax {
                return None;
            }
        }
    }
    if tmin < 0.0 {
        return None;
    }
    Some((tmin, axis, sign))
}

#[derive(Clone)]
pub struct EntView {
    local_eye: [f32; 3],
    cos_yaw: f32,
    sin_yaw: f32,
    outside_bounds: bool,
    bounds: Option<Bounds3>,
    boxes: Vec<Box3>,
    surface: SurfaceFn,
}

#[derive(Clone, Copy, Debug)]
pub struct EntHit {
    pub t: f32,
    pub gray: f32,
    pub emissive: bool,
    pub ndotl: f32,
}

impl EntView {
    pub fn new(
        pos: Vect,
        yaw: f32,
        eye: Vect,
        boxes: Vec<Box3>,
        bounds: Option<Bounds3>,
        surface: SurfaceFn,
    ) -> Self {
        let (sin_yaw, cos_yaw) = yaw.sin_cos();
        let dx = eye.x - pos.x;
        let dy = eye.y - pos.y;
        let local_eye = [
            cos_yaw * dx + sin_yaw * dy,
            -sin_yaw * dx + cos_yaw * dy,
            eye.z - pos.z,
        ];
        let outside_bounds = bounds.is_some_and(|(min, max)| {
            !(0..3).all(|i| local_eye[i] >= min[i] && local_eye[i] <= max[i])
        });
        Self {
            local_eye,
            cos_yaw,
            sin_yaw,
            outside_bounds,
            bounds,
            boxes,
            surface,
        }
    }

    pub fn hit(&self, direction: Vect) -> Option<EntHit> {
        let local_direction = [
            self.cos_yaw * direction.x + self.sin_yaw * direction.y,
            -self.sin_yaw * direction.x + self.cos_yaw * direction.y,
            direction.z,
        ];
        if self.outside_bounds
            && self.bounds.is_some_and(|(min, max)| {
                ray_box(self.local_eye, local_direction, min, max).is_none()
            })
        {
            return None;
        }
        let mut best: Option<(f32, usize, f32, u8)> = None;
        for (min, max, part) in &self.boxes {
            if let Some((t, axis, sign)) = ray_box(self.local_eye, local_direction, *min, *max) {
                if best.is_none_or(|hit| t < hit.0) {
                    best = Some((t, axis, sign, *part));
                }
            }
        }
        let (t, axis, sign, part) = best?;
        let point = [
            self.local_eye[0] + local_direction[0] * t,
            self.local_eye[1] + local_direction[1] * t,
            self.local_eye[2] + local_direction[2] * t,
        ];
        let appearance = (self.surface)(point, axis, sign, part);
        Some(EntHit {
            t,
            gray: appearance.gray,
            emissive: appearance.emissive,
            ndotl: local_direction[axis].abs(),
        })
    }
}
