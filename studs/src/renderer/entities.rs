//! Box-based entity models and the ray tests used to draw them.

use crate::math::Vect;

/// (min corner, max corner, part id) in the entity's local space.
pub type Box3 = ([f32; 3], [f32; 3], u8);

/// Loose bounds around the whole enemy; used to skip the per-box tests for rays that miss.
pub const ENEMY_BBOX: ([f32; 3], [f32; 3]) = ([-0.5, -0.75, 0.0], [1.0, 0.75, 2.25]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntKind {
    Enemy,
    Page,
}

/// Slab test. Returns (t_entry, axis, sign_of_hit_normal). None if the origin is inside.
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

pub fn enemy_boxes(phase: f32) -> Vec<Box3> {
    let sw = phase.sin() * 0.28;
    vec![
        // legs swing fore/aft
        ([-0.15 + sw, -0.35, 0.0], [0.15 + sw, -0.05, 0.9], 0),
        ([-0.15 - sw, 0.05, 0.0], [0.15 - sw, 0.35, 0.9], 0),
        // torso
        ([-0.2, -0.4, 0.9], [0.2, 0.4, 1.7], 0),
        // arms stretched forward, zombie style
        ([-0.1, -0.65, 1.35], [0.85, -0.4, 1.7], 0),
        ([-0.1, 0.4, 1.35], [0.85, 0.65, 1.7], 0),
        // head (part 1 carries the face)
        ([-0.25, -0.25, 1.7], [0.25, 0.25, 2.2], 1),
    ]
}

pub fn page_boxes() -> Vec<Box3> {
    vec![([-0.2, -0.015, -0.25], [0.2, 0.015, 0.25], 0)]
}

/// An entity prepared for one frame: the eye is already expressed in the
/// entity's local space, so each pixel only has to rotate its ray direction.
#[derive(Clone)]
pub struct EntView {
    kind: EntKind,
    lo: [f32; 3],
    c: f32,
    s: f32,
    outside: bool,
    boxes: Vec<Box3>,
}

#[derive(Clone, Copy, Debug)]
pub struct EntHit {
    pub t: f32,
    pub gray: f32,
    pub emissive: bool,
    pub ndotl: f32,
}

impl EntView {
    pub fn new(kind: EntKind, pos: Vect, yaw: f32, eye: Vect, boxes: Vec<Box3>) -> EntView {
        let (s, c) = yaw.sin_cos();
        let dx = eye.x - pos.x;
        let dy = eye.y - pos.y;
        let lo = [c * dx + s * dy, -s * dx + c * dy, eye.z - pos.z];
        let inside = (0..3).all(|i| lo[i] >= ENEMY_BBOX.0[i] && lo[i] <= ENEMY_BBOX.1[i]);
        EntView {
            kind,
            lo,
            c,
            s,
            outside: !inside,
            boxes,
        }
    }

    /// The hunter, with its walk-cycle `phase`.
    pub fn enemy(pos: Vect, yaw: f32, phase: f32, eye: Vect) -> EntView {
        EntView::new(EntKind::Enemy, pos, yaw, eye, enemy_boxes(phase))
    }

    /// A collectible page floating at `pos`.
    pub fn page(pos: Vect, yaw: f32, eye: Vect) -> EntView {
        EntView::new(EntKind::Page, pos, yaw, eye, page_boxes())
    }

    /// Nearest hit of the world-space ray direction `d` (from the eye) with this entity.
    pub fn hit(&self, d: Vect) -> Option<EntHit> {
        let dl = [
            self.c * d.x + self.s * d.y,
            -self.s * d.x + self.c * d.y,
            d.z,
        ];
        if self.kind == EntKind::Enemy
            && self.outside
            && ray_box(self.lo, dl, ENEMY_BBOX.0, ENEMY_BBOX.1).is_none()
        {
            return None;
        }
        let mut best: Option<(f32, usize, f32, u8)> = None;
        for (mn, mx, part) in &self.boxes {
            if let Some((t, axis, sign)) = ray_box(self.lo, dl, *mn, *mx) {
                if best.is_none_or(|b| t < b.0) {
                    best = Some((t, axis, sign, *part));
                }
            }
        }
        let (t, axis, sign, part) = best?;
        let p = [
            self.lo[0] + dl[0] * t,
            self.lo[1] + dl[1] * t,
            self.lo[2] + dl[2] * t,
        ];
        let ndotl = dl[axis].abs();

        match self.kind {
            EntKind::Enemy => {
                // glowing eyes and mouth on the front of the head
                if part == 1 && axis == 0 && sign > 0.0 {
                    let u = p[1].abs();
                    let v = p[2];
                    let eyes = u > 0.05 && u < 0.17 && v > 1.93 && v < 2.06;
                    let mouth = u < 0.13 && v > 1.78 && v < 1.82;
                    if eyes || mouth {
                        return Some(EntHit {
                            t,
                            gray: 1.0,
                            emissive: true,
                            ndotl,
                        });
                    }
                }
                Some(EntHit {
                    t,
                    gray: 0.10,
                    emissive: false,
                    ndotl,
                })
            }
            EntKind::Page => {
                // a page of paper with dark "text" lines
                let mut g = 0.92;
                if axis == 1 {
                    let row = (p[2] + 0.25) * 9.0;
                    if row.fract() < 0.45 && p[0].abs() < 0.15 && row > 1.0 && row < 7.5 {
                        g = 0.35;
                    }
                }
                Some(EntHit {
                    t,
                    gray: g,
                    emissive: true,
                    ndotl,
                })
            }
        }
    }
}
