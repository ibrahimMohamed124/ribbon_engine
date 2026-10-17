use engine_math::{forward_vec, hash4, Angles, Rng, Vect};
use plugin_rendering::cpu::render::entities::EntitySurface;
use plugin_rendering::cpu::render::lighting::SurfaceShader;
use plugin_rendering::cpu::render::RenderStyle;
use plugin_rendering::cpu::render::Scene as RenderScene;
use plugin_rendering::entities::{
    EntityAnimation, EntityBox, EntityDefinition, EntityInstance, EntityKind, EntityRegistry,
    EntityRenderMode, EntityShadow,
};
use plugin_rendering::texture::Texture;
use plugin_rendering::{Palette, Tonemap};
use plugin_voxel::{face_uv, Hit, VoxelWorld};

pub fn graded_style() -> RenderStyle {
    RenderStyle {
        tonemap: Tonemap::Exp { exposure: 2.0 },
        gamma: 1.6,
        palette: Palette::ramp(0x05_08_06, 0xcf_e3_d4),
        texture_strength: 0.6,
        ao_strength: 0.45,
        ambient: 0.010,
        ..RenderStyle::default()
    }
}

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 180;
pub const EMPTY: u8 = 0;
const PREVIEW_KIND: EntityKind = EntityKind(1);
const PREVIEW_BOXES: [EntityBox; 4] = [
    EntityBox {
        min: [-0.36, -0.36, 0.0],
        max: [0.30, 0.36, 1.8],
        part: 0,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [-0.30, -0.30, 1.55],
        max: [0.30, 0.30, 2.1],
        part: 0,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [0.30, -0.20, 1.82],
        max: [0.36, -0.06, 1.92],
        part: 1,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [0.30, 0.06, 1.82],
        max: [0.36, 0.20, 1.92],
        part: 1,
        phase_sway: 0.0,
    },
];
const PREVIEW_DEFINITIONS: [EntityDefinition; 1] = [EntityDefinition {
    kind: PREVIEW_KIND,
    bounds: None,
    boxes: &PREVIEW_BOXES,
    render_mode: EntityRenderMode::Boxes,
    animation: EntityAnimation::None,
    shadow: EntityShadow::None,
}];
static PREVIEW_REGISTRY: EntityRegistry = EntityRegistry::new(&PREVIEW_DEFINITIONS);
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
    noise: Texture,
}

#[derive(Clone, Copy)]
pub struct Viewpoint {
    pub name: &'static str,
    pub eye: Vect,
    pub angles: Angles,
    pub light_level: f32,
    pub actor: bool,
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

    pub fn viewpoints(&self) -> [Viewpoint; 6] {
        [
            Viewpoint {
                name: "corridor",
                eye: Vect::new(3.5, 3.5, 1.55),
                angles: Angles { psi: 0.0, phi: 0.0 },
                light_level: 0.0,
                actor: false,
            },
            Viewpoint {
                name: "corner",
                eye: Vect::new(10.5, 8.5, 1.55),
                angles: Angles {
                    psi: 0.0,
                    phi: std::f32::consts::FRAC_PI_2,
                },
                light_level: 0.0,
                actor: false,
            },
            Viewpoint {
                name: "open-room",
                eye: Vect::new(26.0, 11.5, 1.55),
                angles: Angles {
                    psi: 0.0,
                    phi: std::f32::consts::PI,
                },
                light_level: 0.0,
                actor: false,
            },
            Viewpoint {
                name: "directed-source",
                eye: Vect::new(26.0, 11.5, 1.55),
                angles: Angles {
                    psi: 0.0,
                    phi: std::f32::consts::PI,
                },
                light_level: 1.0,
                actor: false,
            },
            Viewpoint {
                name: "actor-beam",
                eye: Vect::new(26.0, 11.5, 1.55),
                angles: Angles {
                    psi: 0.0,
                    phi: std::f32::consts::PI,
                },
                light_level: 1.0,
                actor: true,
            },
            Viewpoint {
                name: "actor-dark",
                eye: Vect::new(26.0, 11.5, 1.55),
                angles: Angles {
                    psi: 0.0,
                    phi: std::f32::consts::PI,
                },
                light_level: 0.0,
                actor: true,
            },
        ]
    }

    pub fn frame<'a>(
        &'a self,
        view: &Viewpoint,
        style: RenderStyle,
        frame: u32,
    ) -> RenderScene<'a> {
        let ents = if view.actor {
            let forward = forward_vec(view.angles);
            let pos = Vect::new(
                view.eye.x + forward.x * 3.0,
                view.eye.y + forward.y * 3.0,
                0.0,
            );
            vec![EntityInstance {
                kind: PREVIEW_KIND,
                pos,
                yaw: view.angles.phi,
                params: [0.0; 4],
            }]
        } else {
            Vec::new()
        };
        RenderScene {
            world: &self.world,
            empty_voxel: EMPTY,
            camera: plugin_rendering::Camera {
                eye: view.eye,
                view: view.angles,
                hfov: 1.35,
            },
            lighting: plugin_rendering::LightingParams {
                light_level: view.light_level,
                intensity: 0.0,
                frame,
                style,
            },
            entities: ents,
            entity_registry: &PREVIEW_REGISTRY,
        }
    }
}

impl Default for SceneShader {
    fn default() -> Self {
        Self {
            bricks: Texture::bricks(64, 32, 101),
            noise: Texture::noise(64, 64, 103),
        }
    }
}

impl SurfaceShader for SceneShader {
    fn albedo(&self, block: u8, hit: &Hit, _direction: Vect, style: &RenderStyle) -> f32 {
        let base = procedural_albedo(block, hit);
        let strength = style.texture_strength.clamp(0.0, 1.0);
        if strength == 0.0 {
            return base;
        }
        if block == WALL {
            let axis = if hit.normal[0] != 0 { 0 } else { 1 };
            let horizontal = (if axis == 0 { hit.point.y } else { hit.point.x })
                + if ((hit.point.z * 2.0).floor() as i32) & 1 == 1 {
                    0.5
                } else {
                    0.0
                };
            let row = (hit.point.z * 2.0).floor() as i32;
            let texel = self.bricks.sample_faded(
                horizontal.fract(),
                (hit.point.z * 2.0).fract(),
                hit.dist,
                7.0,
                17.0,
            );
            let cell = 0.92
                + 0.16
                    * hash4(
                        horizontal.floor() as i32,
                        row,
                        axis as i32 + hit.normal[axis],
                        101,
                    );
            let factor =
                (1.0 + (texel / self.bricks.average() * cell - 1.0) * strength).clamp(0.7, 1.3);
            return base * factor;
        }
        let (u, v) = face_uv(hit);
        let texel = self.noise.sample_faded(u, v, hit.dist, 5.0, 14.0);
        let factor = (1.0 + (texel / self.noise.average() - 1.0) * strength).clamp(0.7, 1.3);
        base * factor
    }

    fn entity_surface(
        &self,
        _kind: plugin_rendering::entities::EntityKind,
        _point: [f32; 3],
        _axis: usize,
        _sign: f32,
        part: u8,
    ) -> EntitySurface {
        preview_actor_surface(part)
    }
}

fn procedural_albedo(block: u8, hit: &Hit) -> f32 {
    let axis = if hit.normal[0] != 0 {
        0
    } else if hit.normal[1] != 0 {
        1
    } else {
        2
    };
    if axis == 2 {
        let fx = hit.point.x.fract();
        let fy = hit.point.y.fract();
        let distance = ((fx - 0.5).powi(2) + (fy - 0.5).powi(2)).sqrt();
        let mut base = if block == FLOOR { 0.40 } else { 0.30 };
        if distance < 0.27 {
            base *= 1.25;
        } else if distance < 0.34 {
            base *= 0.6;
        }
        if fx < 0.025 || fx > 0.975 || fy < 0.025 || fy > 0.975 {
            base *= 0.65;
        }
        base * (1.0 + 0.05 * hash4(hit.cell.0, hit.cell.1, hit.cell.2, 7))
    } else {
        let height = if axis == 0 { hit.point.y } else { hit.point.x };
        let row_fraction = hit.point.z * 2.0;
        let row = row_fraction.floor() as i32;
        let offset = if row & 1 == 1 { 0.5 } else { 0.0 };
        let horizontal = height + offset;
        if row_fraction.fract() < 0.10 || horizontal.fract() < 0.06 {
            0.20
        } else {
            0.52 + 0.10 * hash4(horizontal.floor() as i32, row, axis, hit.cell.0)
        }
    }
}

fn preview_actor_surface(part: u8) -> EntitySurface {
    if part == 1 {
        EntitySurface {
            gray: 1.0,
            emissive: true,
        }
    } else {
        EntitySurface {
            gray: 0.10,
            emissive: false,
        }
    }
}
