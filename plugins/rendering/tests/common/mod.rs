//! Small renderer fixtures that use no game assets.
//!
//! Dev-only: lives in the test tree of `ribbon_plugin_rendering`, so no production
//! crate can depend on it. Every test that includes this module is gated on
//! the `cpu` feature.

use ribbon_math::{Angles, Vect};
use ribbon_plugin_rendering::cpu::render::lighting::{EntitySurface, SurfaceShader};
use ribbon_plugin_rendering::cpu::renderer::CpuRenderer;
use ribbon_plugin_rendering::entities::{
    EntityAnimation, EntityBox, EntityDefinition, EntityInstance, EntityKind, EntityRegistry,
    EntityRenderMode, EntityShadow,
};
use ribbon_plugin_rendering::material::{BaseAlbedoRule, Material, MaterialTable, TextureKind};
use ribbon_plugin_rendering::scene::{Camera, LightingParams, RenderScene};
use ribbon_plugin_rendering::style::{Palette, RenderStyle, Tonemap};
use ribbon_plugin_rendering::texture::SpriteAtlas;
use ribbon_plugin_voxel::{Hit, VoxelWorld};
use ribbon_render::{DrawCommand, DrawList, Effect, FramePlan, Framebuffer, Renderer};
use std::sync::OnceLock;

pub const WIDTH: usize = 64;
pub const HEIGHT: usize = 40;

static BOXES: [EntityBox; 2] = [
    EntityBox {
        min: [-0.32, -0.32, 0.0],
        max: [0.32, 0.32, 0.72],
        part: 0,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [-0.18, -0.18, 0.72],
        max: [0.18, 0.18, 1.0],
        part: 1,
        phase_sway: 0.0,
    },
];

static DEFINITIONS: [EntityDefinition; 2] = [
    EntityDefinition {
        kind: EntityKind(101),
        bounds: None,
        boxes: &BOXES,
        render_mode: EntityRenderMode::Boxes,
        animation: EntityAnimation::None,
        shadow: EntityShadow::None,
    },
    EntityDefinition {
        kind: EntityKind(102),
        bounds: None,
        boxes: &[],
        render_mode: EntityRenderMode::Billboard {
            atlas_slot: 0,
            width: 0.85,
            height: 1.4,
        },
        animation: EntityAnimation::None,
        shadow: EntityShadow::None,
    },
];

static MATERIALS: [Material; 2] = [
    Material {
        block_id: 1,
        base_rule: BaseAlbedoRule::Constant,
        constant_albedo: 0.62,
        texture: TextureKind::None,
        pbr_texture: None,
        uv_scale: [1.0, 1.0],
        cell_variation: false,
    },
    Material {
        block_id: 2,
        base_rule: BaseAlbedoRule::Constant,
        constant_albedo: 0.82,
        texture: TextureKind::None,
        pbr_texture: None,
        uv_scale: [1.0, 1.0],
        cell_variation: false,
    },
];

static SPRITE_ATLAS: OnceLock<SpriteAtlas> = OnceLock::new();

pub struct Fixture {
    pub world: VoxelWorld,
    pub registry: EntityRegistry,
    pub entities: Vec<EntityInstance>,
    pub materials: MaterialTable,
}

impl Fixture {
    pub fn new() -> Self {
        let (w, h, d) = (10, 8, 4);
        let mut blocks = vec![0; w * h * d];
        let mut set = |x: usize, y: usize, z: usize, id: u8| {
            blocks[(z * h + y) * w + x] = id;
        };
        for y in 0..h {
            for x in 0..w {
                set(x, y, 0, 1);
            }
        }
        for y in 0..h {
            for z in 1..3 {
                set(8, y, z, if y < 4 { 1 } else { 2 });
            }
        }
        for z in 1..3 {
            set(6, 2, z, 2);
        }

        let atlas = SPRITE_ATLAS.get_or_init(|| SpriteAtlas {
            width: 4,
            height: 1,
            cell_width: 1,
            cell_height: 1,
            views: 4,
            states: 1,
            frames_per_state: 1,
            albedo: vec![
                [230, 95, 55, 255],
                [235, 170, 55, 255],
                [80, 180, 100, 255],
                [65, 125, 220, 255],
            ],
            normal: vec![[128, 128, 255, 255]; 4],
            emissive: vec![0; 4],
        });

        Self {
            world: VoxelWorld::new(w, h, d, blocks),
            registry: EntityRegistry::new(&DEFINITIONS),
            entities: vec![
                EntityInstance {
                    kind: EntityKind(101),
                    pos: Vect::new(4.5, 2.8, 0.0),
                    yaw: 0.0,
                    params: [0.0; 4],
                },
                EntityInstance {
                    kind: EntityKind(102),
                    pos: Vect::new(5.2, 5.2, 0.0),
                    yaw: 0.0,
                    params: [0.0; 4],
                },
            ],
            materials: MaterialTable::with_pbr_and_sprite(&MATERIALS, &[], Some(atlas)),
        }
    }

    pub fn scene(&self, style: RenderStyle) -> RenderScene<'_> {
        RenderScene {
            world: &self.world,
            empty_voxel: 0,
            camera: Camera {
                eye: Vect::new(2.5, 4.0, 1.55),
                view: Angles { psi: 0.0, phi: 0.0 },
                hfov: 1.0,
            },
            lighting: LightingParams {
                light_level: 0.85,
                intensity: 1.0,
                frame: 17,
                style,
            },
            entities: self.entities.clone(),
            entity_registry: &self.registry,
        }
    }

    pub fn frame(&self, style: RenderStyle, variant: usize) -> FramePlan<RenderScene<'_>> {
        let mut world_layer = DrawList::new(WIDTH, HEIGHT);
        let mut ui_layer = DrawList::new(WIDTH, HEIGHT);
        if variant > 0 {
            world_layer.commands.push(DrawCommand::Rect {
                x: 2,
                y: 2,
                w: 7,
                h: 3,
                color: 0x00_35_80_c0,
            });
            world_layer
                .commands
                .push(DrawCommand::Effect(Effect::FogBackground { frame: 17 }));
            ui_layer.commands.push(DrawCommand::Rect {
                x: 4,
                y: 32,
                w: 56,
                h: 5,
                color: 0x00_20_30_40,
            });
            ui_layer.commands.push(DrawCommand::Text {
                value: "ENGINE FIXTURE".to_owned(),
                x: 6,
                y: 33,
                scale: 1,
                color: 0x00_ff_ff_ff,
            });
        }
        FramePlan {
            scene: Some(self.scene(style)),
            world_layer,
            brightness: if variant == 2 { 0.72 } else { 1.0 },
            ui_layer,
        }
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

pub fn styles() -> [RenderStyle; 3] {
    [
        RenderStyle::default(),
        RenderStyle {
            tonemap: Tonemap::Exp { exposure: 1.35 },
            gamma: 1.1,
            palette: Palette::ramp(0x10_18_28, 0xe0_d8_c0),
            ..RenderStyle::default()
        },
        RenderStyle {
            ambient: 0.07,
            ambient_fill: 0.05,
            ao_strength: 0.25,
            fog_distance: 12.0,
            ..RenderStyle::default()
        },
    ]
}

/// Flat shader matching the fixture's two constant-albedo materials.
pub struct FixtureShader;

impl SurfaceShader for FixtureShader {
    fn albedo(&self, block: u8, _hit: &Hit, _direction: Vect, _style: &RenderStyle) -> f32 {
        match block {
            1 => 0.62,
            2 => 0.82,
            _ => 0.0,
        }
    }

    fn entity_surface(
        &self,
        _kind: EntityKind,
        _point: [f32; 3],
        _axis: usize,
        _sign: f32,
        part: u8,
    ) -> EntitySurface {
        EntitySurface {
            gray: if part == 0 { 0.32 } else { 0.48 },
            emissive: false,
        }
    }
}

/// Render one fixture variant through the CPU reference renderer.
pub fn render_cpu(fixture: &Fixture, variant: usize) -> Framebuffer {
    let mut renderer = CpuRenderer::new(WIDTH as u32, HEIGHT as u32, FixtureShader);
    let mut output = Framebuffer::new(WIDTH, HEIGHT);
    renderer.render_frame(
        &fixture.frame(styles()[variant % 3], variant),
        &fixture.materials,
    );
    renderer.present(&mut output);
    output
}
