use voxelworks_api::{Framebuffer, Renderer};
use voxelworks_core::{math::Vect, voxel::Hit};
use voxelworks_render_cpu::{render::lighting::SurfaceShader, renderer::CpuRenderer};
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

fn render(variant: usize) -> Framebuffer {
    let fixture = Fixture::new();
    let mut renderer = CpuRenderer::new(WIDTH as u32, HEIGHT as u32, FixtureShader);
    let mut output = Framebuffer::new(WIDTH, HEIGHT);
    renderer.render_frame(
        &fixture.frame(styles()[variant % 3], variant),
        &fixture.materials,
    );
    renderer.present(&mut output);
    output
}

fn hash(frame: &Framebuffer) -> u64 {
    frame
        .pixels
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, pixel| {
            (hash ^ u64::from(*pixel)).wrapping_mul(0x100_0000_01b3)
        })
}

#[test]
fn neutral_fixture_cpu_goldens_cover_styles_entities_layers_brightness_and_effects() {
    let frames = [render(0), render(1), render(2)];
    let measured: Vec<_> = frames.iter().map(hash).collect();
    println!("neutral CPU fixture hashes: {measured:#x?}");
    assert_eq!(
        measured,
        [
            0xc0aa_9194_1785_e603,
            0x6666_e233_3fc5_196c,
            0x3260_fd9f_7e08_7f2e,
        ]
    );
}
