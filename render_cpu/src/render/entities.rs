//! CPU-side rotated-box intersection for data-only scene entities.

pub use super::lighting::EntitySurface;
use engine_core::math::Vect;
pub use engine_scene::entities::{Bounds3, Box3};
use engine_scene::entities::{EntityDefinition, EntityInstance, EntityKind, EntityRegistry, PAGE};
use engine_scene::texture::SpriteAtlas;

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

#[derive(Clone)]
pub struct SpriteView {
    instance: EntityInstance,
    center: [f32; 3],
    toward_camera: [f32; 2],
    right: [f32; 2],
    camera_angle: f32,
    plane_distance: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpriteHit {
    pub t: f32,
    pub u: f32,
    pub v: f32,
    pub view: usize,
    pub state: usize,
    pub frame: usize,
}

impl SpriteView {
    pub fn new(instance: EntityInstance, eye: Vect) -> Self {
        let dx = eye.x - instance.pos.x;
        let dy = eye.y - instance.pos.y;
        let length = (dx * dx + dy * dy).sqrt().max(1e-6);
        let toward_camera = [dx / length, dy / length];
        let right = [-toward_camera[1], toward_camera[0]];
        Self {
            instance,
            center: [instance.pos.x, instance.pos.y, instance.pos.z + 1.1],
            toward_camera,
            right,
            camera_angle: dy.atan2(dx) - instance.yaw,
            plane_distance: length,
            width: 0.92,
            height: 2.2,
        }
    }

    pub fn hit(&self, direction: Vect, atlas: &SpriteAtlas) -> Option<SpriteHit> {
        let denom = direction.x * self.toward_camera[0] + direction.y * self.toward_camera[1];
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = -self.plane_distance / denom;
        if !t.is_finite() || t <= 0.0 {
            return None;
        }
        let point = [
            self.instance.pos.x + direction.x * t,
            self.instance.pos.y + direction.y * t,
            self.instance.pos.z + direction.z * t,
        ];
        let rel = [point[0] - self.center[0], point[1] - self.center[1]];
        let u = 0.5 + (rel[0] * self.right[0] + rel[1] * self.right[1]) / self.width;
        let v = (point[2] - self.instance.pos.z) / self.height;
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            return None;
        }
        let wrapped = self.camera_angle.rem_euclid(std::f32::consts::TAU);
        let view =
            (wrapped / std::f32::consts::TAU * atlas.views as f32).floor() as usize % atlas.views;
        let state = self.instance.params[1]
            .round()
            .clamp(0.0, (atlas.states - 1) as f32) as usize;
        let frame_phase =
            self.instance.params[0].abs() / std::f32::consts::TAU * atlas.frames_per_state as f32;
        let frame = frame_phase.floor() as usize % atlas.frames_per_state;
        Some(SpriteHit {
            t,
            u,
            v,
            view,
            state,
            frame,
        })
    }

    pub fn floor_shadow_strength(&self, point: [f32; 3]) -> f32 {
        if (point[2] - self.instance.pos.z).abs() > 0.08 {
            return 0.0;
        }
        let dx = (point[0] - self.instance.pos.x) / 0.62;
        let dy = (point[1] - self.instance.pos.y) / 0.34;
        let edge = (1.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
        0.38 * edge * edge * (3.0 - 2.0 * edge)
    }
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

#[cfg(test)]
mod sprite_tests {
    use super::SpriteView;
    use engine_core::{math::Vect, voxel::VoxelWorld};
    use engine_scene::{
        entities::{EntityInstance, WARDEN},
        texture::SpriteAtlas,
    };

    fn atlas() -> SpriteAtlas {
        SpriteAtlas {
            width: 16,
            height: 40,
            cell_width: 1,
            cell_height: 1,
            views: 16,
            states: 5,
            frames_per_state: 8,
            albedo: vec![[255; 4]; 640],
            normal: vec![[128, 128, 255, 255]; 640],
            emissive: vec![0; 640],
        }
    }

    #[test]
    fn sprite_selects_relative_view_animation_state_and_frame() {
        let instance = EntityInstance {
            kind: WARDEN,
            pos: Vect::new(4.5, 0.5, 0.0),
            yaw: 0.0,
            params: [std::f32::consts::TAU * 0.30, 3.0, 0.0, 0.0],
        };
        let view = SpriteView::new(instance, Vect::new(0.5, 0.5, 1.0));
        let hit = view.hit(Vect::new(1.0, 0.0, 0.0), &atlas()).unwrap();
        assert_eq!((hit.view, hit.state, hit.frame), (8, 3, 2));
    }

    #[test]
    fn wall_dda_hit_occludes_sprite_behind_it() {
        let mut blocks = vec![0; 6 * 2 * 3];
        let idx = 14;
        blocks[idx] = 1;
        let world = VoxelWorld::new(6, 2, 3, blocks);
        let eye = Vect::new(0.5, 0.5, 1.1);
        let dir = Vect::new(1.0, 0.0, 0.0);
        let wall = world.cast(eye, dir, 10.0, 0).unwrap();
        let instance = EntityInstance {
            kind: WARDEN,
            pos: Vect::new(4.5, 0.5, 0.0),
            yaw: 0.0,
            params: [0.0; 4],
        };
        let sprite = SpriteView::new(instance, eye).hit(dir, &atlas()).unwrap();
        assert!(
            wall.dist < sprite.t,
            "wall at {} must occlude sprite at {}",
            wall.dist,
            sprite.t
        );
    }

    #[test]
    fn floor_contact_shadow_is_soft_and_local() {
        let instance = EntityInstance {
            kind: WARDEN,
            pos: Vect::new(4.5, 0.5, 0.0),
            yaw: 0.0,
            params: [0.0; 4],
        };
        let view = SpriteView::new(instance, Vect::new(0.5, 0.5, 1.0));
        let center = view.floor_shadow_strength([4.5, 0.5, 0.0]);
        let edge = view.floor_shadow_strength([4.9, 0.5, 0.0]);
        let far = view.floor_shadow_strength([6.0, 0.5, 0.0]);
        assert!(center > edge && edge > far);
        assert!(center <= 0.38 && far == 0.0);
        assert_eq!(view.floor_shadow_strength([4.5, 0.5, 0.2]), 0.0);
    }
}
