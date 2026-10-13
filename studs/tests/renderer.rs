use raycast_blocks::game::{Game, PAGES_TOTAL};
use raycast_blocks::renderer::entities::ray_box;
use raycast_blocks::renderer::hud::{draw_caught, draw_hud, draw_won, fill_rect};
use raycast_blocks::renderer::{Framebuffer, DEFAULT_HFOV};

#[test]
fn ray_box_front_face() {
    let (t, axis, sign) = ray_box(
        [-1.0, 0.5, 0.5],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
    )
    .unwrap();
    assert!((t - 1.0).abs() < 1e-5);
    assert_eq!(axis, 0);
    assert_eq!(sign, -1.0);
}

#[test]
fn ray_box_miss_and_inside() {
    // passes above the box
    assert!(ray_box([-1.0, 0.5, 2.0], [1.0, 0.0, 0.0], [0.0; 3], [1.0; 3]).is_none());
    // origin inside the box
    assert!(ray_box([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], [0.0; 3], [1.0; 3]).is_none());
}

#[test]
fn framebuffer_has_the_requested_size() {
    let fb = Framebuffer::new(64, 36);
    assert_eq!(fb.pixels.len(), 64 * 36);
    assert!(fb.pixels.iter().all(|&p| p == 0));
}

#[test]
fn fill_rect_clips_to_the_framebuffer() {
    let mut fb = Framebuffer::new(10, 10);
    fill_rect(&mut fb, -5, -5, 8, 8, 0x00ff_ffff); // only the 3x3 corner is visible
    assert_eq!(fb.pixels[0], 0x00ff_ffff);
    assert_eq!(fb.pixels[2 * 10 + 2], 0x00ff_ffff);
    assert_eq!(fb.pixels[3 * 10 + 3], 0);
    fill_rect(&mut fb, 8, 8, 10, 10, 0x00ff_ffff); // runs off the far edge: must not panic
    assert_eq!(fb.pixels[9 * 10 + 9], 0x00ff_ffff);
}

#[test]
fn hud_shows_collected_pages() {
    let mut fb = Framebuffer::new(320, 180);
    draw_hud(&mut fb, 1, PAGES_TOTAL, 1.0, false);
    assert_eq!(fb.pixels[6 * 320 + 6], 0x00dc_dcdc); // first page icon is lit
    assert_eq!(fb.pixels[6 * 320 + 16], 0x0030_3030); // second is an empty frame

    let mut fb = Framebuffer::new(320, 180);
    draw_hud(&mut fb, 0, PAGES_TOTAL, 1.0, false);
    assert_eq!(fb.pixels[6 * 320 + 6], 0x0030_3030);
}

#[test]
fn overlays_render_without_panicking_on_odd_sizes() {
    let mut fb = Framebuffer::new(97, 53);
    draw_caught(&mut fb, 0.05, 1); // white flash
    assert!(fb.pixels.iter().all(|&p| p == 0x00ff_ffff));
    draw_caught(&mut fb, 2.0, 7);
    draw_won(&mut fb, 1.0, 3);
}

#[test]
fn rendering_is_deterministic_and_not_black() {
    let game = Game::new(2024);
    let mut a = Framebuffer::new(64, 36);
    let mut b = Framebuffer::new(64, 36);
    game.draw(&mut a, 5, DEFAULT_HFOV);
    game.draw(&mut b, 5, DEFAULT_HFOV);
    assert_eq!(a.pixels, b.pixels);
    assert!(a.pixels.iter().any(|&p| p != 0));
}

#[test]
fn flashlight_makes_the_view_brighter() {
    let mut game = Game::new(2024);
    let sum = |fb: &Framebuffer| fb.pixels.iter().map(|&p| (p & 0xff) as u64).sum::<u64>();

    game.flashlight_on = true;
    let mut on = Framebuffer::new(64, 36);
    game.draw(&mut on, 1, DEFAULT_HFOV);

    game.flashlight_on = false;
    let mut off = Framebuffer::new(64, 36);
    game.draw(&mut off, 1, DEFAULT_HFOV);

    assert!(sum(&on) > sum(&off));
}
