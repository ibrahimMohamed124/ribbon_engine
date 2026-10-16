//! Player movement and physics.

use crate::input::Controls;
use crate::world::{World, FLOOR_TOP};
use voxelworks_core::math::{Angles, Vect};
use voxelworks_core::physics::{collides as engine_collides, slide_xy, BodyShape};

pub const EYE_HEIGHT: f32 = 1.5;
pub const HEAD_MARGIN: f32 = 0.1;
pub const BODY_RADIUS: f32 = 0.3;
pub const MOVE_SPEED: f32 = 4.2;
pub const SPRINT_MULT: f32 = 1.7;
pub const SPRINT_TIME: f32 = 4.0; // seconds of sprint from full stamina
pub const REGEN_TIME: f32 = 6.0; // seconds to refill
pub const TURN_SPEED: f32 = 2.2;
pub const GRAVITY: f32 = 20.0;
pub const JUMP_SPEED: f32 = 6.5;
pub const PSI_LIMIT: f32 = 1.4;
const FOOTSTEP_DISTANCE: f32 = 1.2;

fn body_shape() -> BodyShape {
    BodyShape::new(
        BODY_RADIUS,
        vec![-EYE_HEIGHT, -EYE_HEIGHT + 0.9, HEAD_MARGIN],
    )
}

pub struct Player {
    pub pos: Vect, // eye position
    pub view: Angles,
    pub vz: f32,
    pub on_ground: bool,
    pub stamina: f32,
    pub exhausted: bool,
    footstep_distance: f32,
}

/// Things that happened during one `Player::update`, for audio / effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerEvents {
    pub jumped: bool,
    pub landed: bool,
    pub footstep: bool,
}

impl Player {
    /// A player standing on the floor at tile position (x, y).
    pub fn new(x: f32, y: f32, view: Angles) -> Player {
        Player {
            pos: Vect::new(x, y, FLOOR_TOP + 0.001 + EYE_HEIGHT),
            view,
            vz: 0.0,
            on_ground: true,
            stamina: 1.0,
            exhausted: false,
            footstep_distance: 0.0,
        }
    }

    pub fn update(&mut self, world: &World, c: &Controls, dt: f32) -> PlayerEvents {
        let mut events = PlayerEvents::default();

        // ---- look
        if c.look_up {
            self.view.psi += TURN_SPEED * dt;
        }
        if c.look_down {
            self.view.psi -= TURN_SPEED * dt;
        }
        if c.look_right {
            self.view.phi += TURN_SPEED * dt;
        }
        if c.look_left {
            self.view.phi -= TURN_SPEED * dt;
        }
        self.view.phi += c.mouse_yaw;
        self.view.psi += c.mouse_pitch;
        self.view.psi = self.view.psi.clamp(-PSI_LIMIT, PSI_LIMIT);

        // ---- horizontal movement + stamina
        let (sin, cos) = self.view.phi.sin_cos();
        let was_grounded = self.on_ground;
        let previous_xy = (self.pos.x, self.pos.y);
        let mut mx = 0.0;
        let mut my = 0.0;
        if c.forward {
            mx += cos;
            my += sin;
        }
        if c.back {
            mx -= cos;
            my -= sin;
        }
        if c.right {
            mx -= sin;
            my += cos;
        }
        if c.left {
            mx += sin;
            my -= cos;
        }
        let len = (mx * mx + my * my).sqrt();
        let moving = len > 0.0;

        let mut speed = MOVE_SPEED;
        if c.sprint && moving && !self.exhausted && self.stamina > 0.0 {
            speed *= SPRINT_MULT;
            self.stamina -= dt / SPRINT_TIME;
            if self.stamina <= 0.0 {
                self.stamina = 0.0;
                self.exhausted = true;
            }
        } else {
            self.stamina = (self.stamina + dt / REGEN_TIME).min(1.0);
            if self.exhausted && self.stamina > 0.3 {
                self.exhausted = false;
            }
        }

        if moving {
            // axis-separated moves so the player slides along walls
            let k = speed * dt / len;
            self.pos = slide_xy(
                world,
                crate::world::AIR,
                self.pos,
                (mx * k, my * k),
                &body_shape(),
            );
        }

        let moved = (self.pos.x - previous_xy.0).hypot(self.pos.y - previous_xy.1);
        if was_grounded && moved > 0.0 {
            self.footstep_distance += moved;
            if self.footstep_distance >= FOOTSTEP_DISTANCE {
                events.footstep = true;
                self.footstep_distance -= FOOTSTEP_DISTANCE;
            }
        }

        // ---- vertical movement
        if c.jump && self.on_ground {
            self.vz = JUMP_SPEED;
            events.jumped = true;
        }
        self.vz = (self.vz - GRAVITY * dt).max(-30.0);
        let mut np = self.pos;
        np.z += self.vz * dt;
        if collides(np, world) {
            if self.vz < 0.0 {
                let feet = np.z - EYE_HEIGHT;
                np.z = feet.floor() + 1.001 + EYE_HEIGHT;
                self.on_ground = true;
            } else {
                let head = np.z + HEAD_MARGIN;
                np.z = head.floor() - HEAD_MARGIN - 0.001;
            }
            self.vz = 0.0;
            if !collides(np, world) {
                self.pos.z = np.z;
            }
        } else {
            self.pos.z = np.z;
            self.on_ground = false;
        }
        events.landed = !was_grounded && self.on_ground;
        events
    }
}

/// Does a body with its eyes at `eye` overlap any solid block?
pub fn collides(eye: Vect, world: &World) -> bool {
    engine_collides(world, crate::world::AIR, eye, &body_shape())
}
