#![cfg(feature = "cpu")]

mod common;
#[path = "common/golden.rs"]
mod golden;

use common::Fixture;
use golden::render_cpu;
use ribbon_render::Framebuffer;

fn render(variant: usize) -> Framebuffer {
    render_cpu(&Fixture::new(), variant)
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
