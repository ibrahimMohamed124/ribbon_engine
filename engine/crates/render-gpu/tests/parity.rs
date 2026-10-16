use voxelworks_api::{Framebuffer, Renderer};
use voxelworks_core::{math::Vect, voxel::Hit};
use voxelworks_render_cpu::{render::lighting::SurfaceShader, renderer::CpuRenderer};
use voxelworks_render_gpu::GpuRenderer;
use voxelworks_scene::style::RenderStyle;
use voxelworks_testkit::{styles, Fixture, HEIGHT, WIDTH};

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
        _kind: voxelworks_scene::entities::EntityKind,
        _point: [f32; 3],
        _axis: usize,
        _sign: f32,
        part: u8,
    ) -> voxelworks_render_cpu::render::lighting::EntitySurface {
        voxelworks_render_cpu::render::lighting::EntitySurface {
            gray: if part == 0 { 0.32 } else { 0.48 },
            emissive: false,
        }
    }
}

fn render_cpu(fixture: &Fixture, variant: usize) -> Framebuffer {
    let mut renderer = CpuRenderer::new(WIDTH as u32, HEIGHT as u32, FixtureShader);
    let mut output = Framebuffer::new(WIDTH, HEIGHT);
    renderer.render_frame(
        &fixture.frame(styles()[variant], variant),
        &fixture.materials,
    );
    renderer.present(&mut output);
    output
}

fn error(cpu: &Framebuffer, gpu: &Framebuffer) -> (f64, u8) {
    let mut sum = 0u64;
    let mut max = 0u8;
    let mut count = 0u64;
    for (a, b) in cpu.pixels.iter().zip(&gpu.pixels) {
        for shift in [0, 8, 16] {
            let delta =
                (((a >> shift) & 0xff) as i16 - ((b >> shift) & 0xff) as i16).unsigned_abs() as u8;
            sum += u64::from(delta);
            max = max.max(delta);
            count += 1;
        }
    }
    (sum as f64 / count as f64, max)
}

#[test]
fn cpu_gpu_parity_on_shared_neutral_fixture() {
    let fixture = Fixture::new();
    let mut gpu = match GpuRenderer::new(WIDTH as u32, HEIGHT as u32) {
        Ok(gpu) => gpu,
        Err(message) => {
            println!("SKIP: CPU/GPU parity fixture needs a GPU adapter: {message}");
            return;
        }
    };

    for variant in 0..3 {
        let cpu = render_cpu(&fixture, variant);
        let plan = fixture.frame(styles()[variant], variant);
        gpu.render_frame(&plan, &fixture.materials);
        assert_eq!(gpu.take_error(), None);
        let mut output = Framebuffer::new(WIDTH, HEIGHT);
        gpu.present(&mut output);
        let (mean, max) = error(&cpu, &output);
        println!("neutral fixture variant {variant}: mean abs channel error={mean:.4}, max={max}");
        assert!(
            mean <= 1.0,
            "mean CPU/GPU channel error {mean:.4} exceeds 1"
        );
        assert!(max <= 3, "max CPU/GPU channel error {max} exceeds 3");
    }
}
