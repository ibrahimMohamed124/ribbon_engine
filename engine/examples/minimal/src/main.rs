use voxelworks_api::DrawList;
use voxelworks_api::WindowConfig;
use voxelworks_api::{App, Ctx, Flow, FramePlan};
use voxelworks_core::input::Key;
use voxelworks_core::math::{Angles, Vect};
use voxelworks_core::voxel::{Hit, VoxelWorld};
use voxelworks_platform::{run_with_mode, RendererMode};
use voxelworks_render_cpu::render::lighting::SurfaceShader;
use voxelworks_render_cpu::renderer::CpuRenderer;
use voxelworks_scene::entities::{
    EntityBox, EntityDefinition, EntityInstance, EntityKind, EntityRegistry,
};
use voxelworks_scene::{Camera, LightingParams, RenderStyle, Scene};

const EMPTY: u8 = 0;
const FLOOR: u8 = 1;
const WALL: u8 = 2;

struct RoomSurface;

impl SurfaceShader for RoomSurface {
    fn albedo(&self, block: u8, hit: &Hit, _direction: Vect, _style: &RenderStyle) -> f32 {
        if hit.normal[2] != 0 {
            if block == FLOOR {
                0.52
            } else {
                0.36
            }
        } else if hit.cell.2 % 2 == 0 {
            0.58
        } else {
            0.44
        }
    }
}

const WALKABOUT_BOXES: [EntityBox; 1] = [EntityBox {
    min: [-0.4, -0.4, 0.0],
    max: [0.4, 0.4, 1.4],
    part: 0,
    phase_sway: 0.0,
}];
const WALKABOUT_KIND: EntityKind = EntityKind(7);
const WALKABOUT_DEFINITIONS: [EntityDefinition; 1] = [EntityDefinition {
    kind: WALKABOUT_KIND,
    bounds: Some(([-0.5, -0.5, 0.0], [0.5, 0.5, 1.5])),
    boxes: &WALKABOUT_BOXES,
    render_mode: voxelworks_scene::entities::EntityRenderMode::Boxes,
    animation: voxelworks_scene::entities::EntityAnimation::None,
    shadow: voxelworks_scene::entities::EntityShadow::None,
}];
static WALKABOUT_REGISTRY: EntityRegistry = EntityRegistry::new(&WALKABOUT_DEFINITIONS);

struct Walkabout {
    world: VoxelWorld,
    eye: Vect,
    view: Angles,
    entity: (Vect, f32),
}

impl Walkabout {
    fn new() -> Self {
        let (width, height, depth) = (12, 9, 5);
        let mut blocks = vec![EMPTY; width * height * depth];
        for y in 0..height {
            for x in 0..width {
                blocks[VoxelWorld::idx(width, height, 0, y, x)] = FLOOR;
                blocks[VoxelWorld::idx(width, height, depth - 1, y, x)] = WALL;
                if x == 0 || y == 0 || x + 1 == width || y + 1 == height {
                    for z in 1..depth - 1 {
                        blocks[VoxelWorld::idx(width, height, z, y, x)] = WALL;
                    }
                }
            }
        }
        Self {
            world: VoxelWorld::new(width, height, depth, blocks),
            eye: Vect::new(2.5, 2.5, 1.55),
            view: Angles { psi: 0.0, phi: 0.0 },
            entity: (Vect::new(7.0, 5.0, 1.0), 0.4),
        }
    }
}

impl App for Walkabout {
    fn update(&mut self, ctx: &mut Ctx<'_>) -> Flow {
        if ctx.keys.is_down(Key::Escape) || ctx.keys.is_down(Key::Q) {
            return Flow::Quit;
        }
        ctx.capture_cursor();

        self.view.phi += ctx.mouse_delta.0 * 0.0025;
        self.view.psi = (self.view.psi - ctx.mouse_delta.1 * 0.0025).clamp(-1.2, 1.2);
        let forward = f32::from(ctx.keys.is_down(Key::W)) - f32::from(ctx.keys.is_down(Key::S));
        let strafe = f32::from(ctx.keys.is_down(Key::D)) - f32::from(ctx.keys.is_down(Key::A));
        let (sin, cos) = self.view.phi.sin_cos();
        let speed = 3.0 * ctx.dt;
        self.eye.x += (cos * forward - sin * strafe) * speed;
        self.eye.y += (sin * forward + cos * strafe) * speed;
        let (width, height, _) = self.world.dimensions();
        self.eye.x = self.eye.x.clamp(1.0, width as f32 - 1.0);
        self.eye.y = self.eye.y.clamp(1.0, height as f32 - 1.0);
        Flow::Continue
    }

    fn frame_plan(&self, _ctx: &Ctx<'_>) -> FramePlan<'_> {
        let (position, yaw) = &self.entity;
        let scene = Some(Scene {
            world: &self.world,
            empty_voxel: EMPTY,
            camera: Camera {
                eye: self.eye,
                view: self.view,
                hfov: 1.25,
            },
            lighting: LightingParams {
                light_level: 0.75,
                intensity: 0.0,
                frame: 0,
                style: RenderStyle::default(),
            },
            entities: vec![EntityInstance {
                kind: WALKABOUT_KIND,
                pos: *position,
                yaw: *yaw,
                params: [0.0; 4],
            }],
            entity_registry: &WALKABOUT_REGISTRY,
        });
        FramePlan {
            scene,
            world_layer: DrawList::new(640, 360),
            brightness: 1.0,
            ui_layer: DrawList::new(640, 360),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut selected = "gpu".to_owned();
    let mut headless = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => headless = true,
            "--renderer" => {
                selected = args.next().ok_or("--renderer expects cpu or gpu")?;
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    if headless {
        let app = Walkabout::new();
        let scene = Scene {
            world: &app.world,
            empty_voxel: EMPTY,
            camera: Camera {
                eye: app.eye,
                view: app.view,
                hfov: 1.25,
            },
            lighting: LightingParams {
                light_level: 0.75,
                intensity: 0.0,
                frame: 0,
                style: RenderStyle::default(),
            },
            entities: vec![EntityInstance {
                kind: WALKABOUT_KIND,
                pos: app.entity.0,
                yaw: app.entity.1,
                params: [0.0; 4],
            }],
            entity_registry: &WALKABOUT_REGISTRY,
        };
        let mut framebuffer = voxelworks_api::Framebuffer::new(160, 90);
        voxelworks_render_cpu::render::raycaster::render_with_materials_checked(
            &mut framebuffer,
            &scene,
            &RoomSurface,
            &voxelworks_scene::EMPTY_MATERIAL_TABLE,
        )?;
        assert!(framebuffer.pixels.iter().any(|pixel| *pixel != 0));
        println!("Minimal engine app rendered one headless CPU frame.");
        return Ok(());
    }
    let mode = match selected.as_str() {
        "cpu" => RendererMode::Cpu,
        "gpu" => RendererMode::Gpu,
        other => return Err(format!("expected --renderer cpu or gpu; got {other}").into()),
    };
    let config = WindowConfig {
        title: "Minimal Engine App".to_owned(),
        width: 640,
        height: 360,
        scale: 1,
        fixed_hz: 120,
        max_steps: 10,
    };
    run_with_mode(
        Walkabout::new(),
        "Minimal Engine App",
        config,
        CpuRenderer::new(640, 360, RoomSurface),
        mode,
    )
}
