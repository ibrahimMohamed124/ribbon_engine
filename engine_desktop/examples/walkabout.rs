use blocks_engine::app::{App, Ctx, Flow, WindowConfig};
use blocks_engine::gfx::Framebuffer;
use blocks_engine::input::Key;
use blocks_engine::math::{Angles, Vect};
use blocks_engine::render::entities::{Box3, EntView, EntitySurface};
use blocks_engine::render::lighting::SurfaceShader;
use blocks_engine::render::{render, Frame, RenderStyle};
use blocks_engine::voxel::{Hit, VoxelWorld};
use blocks_engine_desktop::run;

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

fn object_surface(_point: [f32; 3], _axis: usize, _sign: f32, _part: u8) -> EntitySurface {
    EntitySurface {
        gray: 0.9,
        emissive: false,
    }
}

struct Walkabout {
    world: VoxelWorld,
    eye: Vect,
    view: Angles,
    entity: (Vect, f32, Vec<Box3>),
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
            entity: (
                Vect::new(7.0, 5.0, 1.0),
                0.4,
                vec![([-0.4, -0.4, 0.0], [0.4, 0.4, 1.4], 0)],
            ),
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

    fn render(&mut self, framebuffer: &mut Framebuffer, ctx: &Ctx<'_>) {
        let (position, yaw, boxes) = &self.entity;
        let object = EntView::new(
            *position,
            *yaw,
            self.eye,
            boxes.clone(),
            Some(([-0.5, -0.5, 0.0], [0.5, 0.5, 1.5])),
            object_surface,
        );
        let frame = Frame {
            world: &self.world,
            eye: self.eye,
            view: self.view,
            hfov: 1.25,
            illumination: 0.75,
            empty_voxel: EMPTY,
            ents: vec![object],
            frame: ctx.frame,
            intensity: 0.0,
            style: RenderStyle::classic(),
        };
        render(framebuffer, &frame, &RoomSurface);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(
        Walkabout::new(),
        WindowConfig {
            title: "Walkabout".to_owned(),
            width: 640,
            height: 360,
            scale: 1,
        },
    )
}
