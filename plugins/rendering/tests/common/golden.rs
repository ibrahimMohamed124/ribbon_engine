//! Golden-image support used only by `fixture_golden.rs`.
//!
//! Keeping these helpers out of the shared fixture module prevents other integration tests from
//! compiling dead-code-only render helpers under `clippy -D warnings`.

use crate::common::{Fixture, HEIGHT, WIDTH};
use ribbon_math::Vect;
use ribbon_plugin_rendering::cpu::render::lighting::{EntitySurface, SurfaceShader};
use ribbon_plugin_rendering::cpu::renderer::CpuRenderer;
use ribbon_plugin_rendering::entities::EntityKind;
use ribbon_plugin_rendering::scene::RenderScene;
use ribbon_plugin_rendering::style::{Palette, RenderStyle, Tonemap};
use ribbon_plugin_voxel::Hit;
use ribbon_render::{DrawCommand, DrawList, Effect, FramePlan, Framebuffer, Renderer};

impl Fixture {
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

fn styles() -> [RenderStyle; 3] {
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
struct FixtureShader;

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
