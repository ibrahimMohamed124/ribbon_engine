use blocks_engine::gfx::texture::Texture;
use blocks_engine::math::{Angles, Rng, Vect};
use blocks_engine::render::lighting::SurfaceShader;
use blocks_engine::render::{Frame, RenderStyle};
use blocks_engine::voxel::{face_uv, Hit, VoxelWorld};

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 180;
pub const EMPTY: u8 = 0;
pub const FLOOR: u8 = 1;
pub const WALL: u8 = 2;
const WORLD_W: usize = 32;
const WORLD_H: usize = 24;
const WORLD_D: usize = 6;

pub struct Scene {
    pub world: VoxelWorld,
}

pub struct SceneShader {
    bricks: Texture,
    tiles: Texture,
}

#[derive(Clone, Copy)]
pub struct Viewpoint {
    pub name: &'static str,
    pub eye: Vect,
    pub angles: Angles,
    pub illumination: f32,
}

impl Scene {
    pub fn seeded(seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        let mut blocks = vec![EMPTY; WORLD_W * WORLD_H * WORLD_D];
        let mut set = |x: usize, y: usize, z: usize, block: u8| {
            blocks[VoxelWorld::idx(WORLD_W, WORLD_H, z, y, x)] = block;
        };

        for y in 0..WORLD_H {
            for x in 0..WORLD_W {
                set(x, y, 0, FLOOR);
                set(x, y, WORLD_D - 1, WALL);
                if x == 0 || y == 0 || x + 1 == WORLD_W || y + 1 == WORLD_H {
                    for z in 1..WORLD_D - 1 {
                        set(x, y, z, WALL);
                    }
                }
            }
        }

        for x in 2..WORLD_W - 2 {
            if ![7, 15, 23].contains(&x) {
                for z in 1..WORLD_D - 1 {
                    set(x, 6, z, WALL);
                }
            }
            if ![5, 18, 27].contains(&x) {
                for z in 1..WORLD_D - 1 {
                    set(x, 16, z, WALL);
                }
            }
        }
        for y in 2..WORLD_H - 2 {
            if ![4, 11, 19].contains(&y) {
                for z in 1..WORLD_D - 1 {
                    set(12, y, z, WALL);
                }
            }
            if ![7, 12, 19].contains(&y) {
                for z in 1..WORLD_D - 1 {
                    set(21, y, z, WALL);
                }
            }
        }

        for y in 2..WORLD_H - 2 {
            for x in 2..WORLD_W - 2 {
                let in_open_room = (22..30).contains(&x) && (8..16).contains(&y);
                let is_corridor = x == 3 || y == 3 || x == 10 || y == 10;
                if !in_open_room && !is_corridor && rng.chance(0.035) {
                    for z in 1..4 {
                        set(x, y, z, WALL);
                    }
                }
            }
        }

        Self {
            world: VoxelWorld::new(WORLD_W, WORLD_H, WORLD_D, blocks),
        }
    }

    pub fn viewpoints(&self) -> [Viewpoint; 4] {
        [
            Viewpoint {
                name: "corridor",
                eye: Vect::new(3.5, 3.5, 1.55),
                angles: Angles { psi: 0.0, phi: 0.0 },
                illumination: 0.0,
            },
            Viewpoint {
                name: "corner",
                eye: Vect::new(10.5, 8.5, 1.55),
                angles: Angles {
                    psi: 0.0,
                    phi: std::f32::consts::FRAC_PI_2,
                },
                illumination: 0.0,
            },
            Viewpoint {
                name: "open-room",
                eye: Vect::new(26.0, 11.5, 1.55),
                angles: Angles {
                    psi: 0.0,
                    phi: std::f32::consts::PI,
                },
                illumination: 0.0,
            },
            Viewpoint {
                name: "flashlight",
                eye: Vect::new(26.0, 11.5, 1.55),
                angles: Angles {
                    psi: 0.0,
                    phi: std::f32::consts::PI,
                },
                illumination: 1.0,
            },
        ]
    }

    pub fn frame<'a>(&'a self, view: &Viewpoint, style: RenderStyle, frame: u32) -> Frame<'a> {
        Frame {
            world: &self.world,
            eye: view.eye,
            view: view.angles,
            hfov: 1.35,
            illumination: view.illumination,
            empty_voxel: EMPTY,
            ents: Vec::new(),
            frame,
            intensity: 0.0,
            style,
        }
    }
}

impl Default for SceneShader {
    fn default() -> Self {
        Self {
            bricks: Texture::bricks(64, 64, 101),
            tiles: Texture::tiles(64, 64, 103),
        }
    }
}

impl SurfaceShader for SceneShader {
    fn albedo(&self, block: u8, hit: &Hit, _direction: Vect, style: &RenderStyle) -> f32 {
        let base = if block == FLOOR { 0.37 } else { 0.46 };
        let strength = style.texture_strength.clamp(0.0, 1.0);
        if strength == 0.0 {
            return base;
        }
        let (u, v) = face_uv(hit);
        let texture = if block == FLOOR {
            &self.tiles
        } else {
            &self.bricks
        };
        let detail = texture.sample_faded(u, v, hit.dist, 7.0, 17.0);
        base * (1.0 - strength) + detail * strength
    }
}
