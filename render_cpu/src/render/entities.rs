//! CPU-side rotated-box intersection for data-only scene entities.

pub use super::lighting::EntitySurface;
use engine_core::math::Vect;
pub use engine_scene::entities::{Bounds3, Box3};
use engine_scene::entities::{EntityDefinition, EntityInstance, EntityKind, EntityRegistry, PAGE};

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
pub struct EntView<'a> {
    instance: EntityInstance,
    definition: &'a EntityDefinition,
    local_eye: [f32; 3],
    cos_yaw: f32,
    sin_yaw: f32,
    outside_bounds: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct EntHit {
    pub t: f32,
    pub point: [f32; 3],
    pub axis: usize,
    pub sign: f32,
    pub part: u8,
    pub kind: EntityKind,
    pub ndotl: f32,
}

impl<'a> EntView<'a> {
    pub fn new(
        mut instance: EntityInstance,
        eye: Vect,
        registry: &'a EntityRegistry,
    ) -> Option<Self> {
        let definition = registry.get(instance.kind)?;
        let mut yaw = instance.yaw;
        if instance.kind == PAGE {
            let time = instance.params[0];
            let page_index = instance.params[1];
            instance.pos.z += (time * 2.0 + page_index).sin() * 0.08;
            yaw = time * 1.2 + page_index;
        }
        let (sin_yaw, cos_yaw) = yaw.sin_cos();
        let dx = eye.x - instance.pos.x;
        let dy = eye.y - instance.pos.y;
        let local_eye = [
            cos_yaw * dx + sin_yaw * dy,
            -sin_yaw * dx + cos_yaw * dy,
            eye.z - instance.pos.z,
        ];
        let outside_bounds = definition.bounds.is_some_and(|(min, max)| {
            !(0..3).all(|i| local_eye[i] >= min[i] && local_eye[i] <= max[i])
        });
        Some(Self {
            instance,
            definition,
            local_eye,
            cos_yaw,
            sin_yaw,
            outside_bounds,
        })
    }

    pub fn hit(&self, direction: Vect) -> Option<EntHit> {
        let local_direction = [
            self.cos_yaw * direction.x + self.sin_yaw * direction.y,
            -self.sin_yaw * direction.x + self.cos_yaw * direction.y,
            direction.z,
        ];
        if self.outside_bounds
            && self.definition.bounds.is_some_and(|(min, max)| {
                ray_box(self.local_eye, local_direction, min, max).is_none()
            })
        {
            return None;
        }
        let mut best: Option<(f32, usize, f32, u8)> = None;
        for entity_box in self.definition.boxes {
            let sway = entity_box.phase_sway * self.instance.params[0].sin() * 0.28;
            let mut min = entity_box.min;
            let mut max = entity_box.max;
            min[0] += sway;
            max[0] += sway;
            if let Some((t, axis, sign)) = ray_box(self.local_eye, local_direction, min, max) {
                if best.is_none_or(|hit| t < hit.0) {
                    best = Some((t, axis, sign, entity_box.part));
                }
            }
        }
        let (t, axis, sign, part) = best?;
        let point = [
            self.local_eye[0] + local_direction[0] * t,
            self.local_eye[1] + local_direction[1] * t,
            self.local_eye[2] + local_direction[2] * t,
        ];
        Some(EntHit {
            t,
            point,
            axis,
            sign,
            part,
            kind: self.instance.kind,
            ndotl: local_direction[axis].abs(),
        })
    }
}
