#![cfg(feature = "cpu")]

//! Contract tests for the deliberately CPU-only `SurfaceShader` extension API.
//!
//! GPU mesh rendering has a separate data-driven PBR/tint contract; it does not accept this trait.

mod common;

use common::{Fixture, HEIGHT, WIDTH};
use ribbon_math::{Angles, Vect};
use ribbon_plugin_rendering::cpu::render::lighting::{EntitySurface, SurfaceShader};
use ribbon_plugin_rendering::cpu::renderer::CpuRenderer;
use ribbon_plugin_rendering::entities::{EntityInstance, EntityKind};
use ribbon_plugin_rendering::scene::{Camera, LightingParams, RenderScene};
use ribbon_plugin_rendering::style::RenderStyle;
use ribbon_plugin_voxel::{Hit, VoxelWorld};
use ribbon_render::{Framebuffer, Renderer};

struct ContractShader {
    albedo: f32,
    entity: EntitySurface,
}

impl SurfaceShader for ContractShader {
    fn albedo(&self, _block: u8, _hit: &Hit, _direction: Vect, _style: &RenderStyle) -> f32 {
        self.albedo
    }

    fn entity_surface(
        &self,
        _kind: EntityKind,
        _point: [f32; 3],
        _axis: usize,
        _sign: f32,
        _part: u8,
    ) -> EntitySurface {
        self.entity
    }
}

fn render_with_shader<S: SurfaceShader + Sync>(
    fixture: &Fixture,
    scene: &RenderScene<'_>,
    shader: S,
) -> Framebuffer {
    let mut renderer = CpuRenderer::new(WIDTH as u32, HEIGHT as u32, shader);
    renderer.render_scene(Some(scene), &fixture.materials);
    let mut output = Framebuffer::new(WIDTH, HEIGHT);
    renderer.present(&mut output);
    output
}

fn shader(albedo: f32, gray: f32, emissive: bool) -> ContractShader {
    ContractShader {
        albedo,
        entity: EntitySurface { gray, emissive },
    }
}

#[test]
fn custom_albedo_callback_changes_cpu_voxel_shading() {
    let fixture = Fixture::new();
    let mut scene = fixture.scene(RenderStyle::default());
    scene.entities.clear();

    let dark = render_with_shader(&fixture, &scene, shader(0.0, 0.8, false));
    let bright = render_with_shader(&fixture, &scene, shader(1.0, 0.8, false));

    assert_ne!(
        dark.pixels, bright.pixels,
        "custom albedo must affect CPU voxel shading"
    );
    assert!(
        bright.pixels.iter().any(|pixel| *pixel != 0),
        "the test scene should visibly render voxel geometry"
    );
}

#[test]
fn custom_entity_surface_callback_controls_gray_and_emissive_behavior_on_cpu() {
    let fixture = Fixture::new();
    let world = VoxelWorld::new(10, 8, 4, vec![0; 10 * 8 * 4]);
    let style = RenderStyle {
        ambient: 0.0,
        ambient_fill: 0.0,
        billboards_enabled: false,
        ..RenderStyle::default()
    };
    let mut scene = RenderScene {
        world: &world,
        empty_voxel: 0,
        camera: Camera {
            eye: Vect::new(2.5, 4.0, 1.55),
            view: Angles { psi: 0.0, phi: 0.0 },
            hfov: 1.4,
        },
        lighting: LightingParams {
            light_level: 1.0,
            intensity: 0.0,
            frame: 17,
            style,
        },
        entities: vec![EntityInstance {
            kind: EntityKind(101),
            pos: Vect::new(5.2, 5.2, 0.0),
            yaw: 0.0,
            params: [0.0; 4],
        }],
        entity_registry: &fixture.registry,
    };

    let low_gray = render_with_shader(&fixture, &scene, shader(0.0, 0.15, false));
    let high_gray = render_with_shader(&fixture, &scene, shader(0.0, 0.90, false));
    assert_ne!(
        low_gray.pixels, high_gray.pixels,
        "the CPU entity callback's gray value must affect the resulting frame"
    );
    assert!(
        high_gray.pixels.iter().any(|pixel| *pixel != 0),
        "the entity test scene should visibly render the box primitive"
    );

    // With the direct light and ambient terms disabled, only the emissive branch can contribute
    // pixels. This isolates the emissive part of the public CPU callback contract.
    scene.lighting.light_level = 0.0;
    let unlit = render_with_shader(&fixture, &scene, shader(0.0, 0.75, false));
    let emissive = render_with_shader(&fixture, &scene, shader(0.0, 0.75, true));
    assert_ne!(
        unlit.pixels, emissive.pixels,
        "the CPU entity callback's emissive flag must affect the resulting frame"
    );
    assert!(
        emissive.pixels.iter().any(|pixel| *pixel != 0),
        "the visible entity should contribute emissive pixels"
    );
}

#[test]
fn default_material_aware_cpu_callback_delegates_to_custom_albedo() {
    let fixture = Fixture::new();
    let mut scene = fixture.scene(RenderStyle::default());
    scene.entities.clear();
    let first = render_with_shader(&fixture, &scene, shader(0.25, 0.0, false));
    let second = render_with_shader(&fixture, &scene, shader(0.75, 0.0, false));
    assert_ne!(
        first.pixels, second.pixels,
        "the default albedo_with_materials implementation must preserve albedo dispatch"
    );
}
