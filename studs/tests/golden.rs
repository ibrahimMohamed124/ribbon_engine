use engine_scene::entities::WARDEN;
use raycast_blocks::game::{Game, State};
use raycast_blocks::input::{Controls, InputFrame};
use raycast_blocks::math::Rng;
use raycast_blocks::renderer::{Framebuffer, RenderStyle, DEFAULT_HFOV};
use raycast_blocks::settings::Settings;
use raycast_blocks::surfaces::studs_material_table;
use raycast_blocks::world::World;
use render_cpu::render::render_with_materials;

fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn blocks_hash(blocks: &[u8]) -> u64 {
    fnv1a(blocks.iter().copied())
}

fn pixels_hash(pixels: &[u32]) -> u64 {
    fnv1a(pixels.iter().flat_map(|pixel| pixel.to_le_bytes()))
}

fn simulation_fingerprint(game: &Game) -> u64 {
    let state = match game.state {
        State::Playing => 0,
        State::Caught => 1,
        State::Won => 2,
    };
    fnv1a(
        [
            game.player.pos.x.to_bits(),
            game.player.pos.y.to_bits(),
            game.player.pos.z.to_bits(),
            game.enemy.pos.x.to_bits(),
            game.enemy.pos.y.to_bits(),
            game.enemy.pos.z.to_bits(),
            game.collected as u32,
            state,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes),
    )
}

#[test]
fn golden_render_world_simulation_and_settings() {
    let game = Game::new(42);
    let mut framebuffer = Framebuffer::new(160, 90);
    // Golden output is intentionally pinned to the legacy renderer path.
    game.draw_styled(&mut framebuffer, 137, DEFAULT_HFOV, RenderStyle::classic());
    let framebuffer_hash = pixels_hash(&framebuffer.pixels);

    let world_hashes: Vec<_> = [1, 42, 0xdead_beef]
        .into_iter()
        .map(|seed| blocks_hash(World::generate(&mut Rng::new(seed)).blocks()))
        .collect();

    let mut simulated = Game::new(0x5eed);
    for step in 0..600 {
        let phase = step % 180;
        let input = InputFrame {
            controls: Controls {
                forward: phase < 70,
                back: (70..95).contains(&phase),
                left: (95..120).contains(&phase),
                right: (120..145).contains(&phase),
                look_left: phase % 60 < 30,
                look_right: phase % 60 >= 30,
                sprint: step % 120 < 40,
                jump: step % 137 == 0,
                ..Controls::default()
            },
            ..InputFrame::default()
        };
        simulated.update(&input, 0.025);
    }
    let simulation_hash = simulation_fingerprint(&simulated);

    // Keep this golden pinned to the settings text before visual styles were
    // added; the new setting has dedicated round-trip coverage.
    let legacy_settings = |text: String| {
        let mut lines: Vec<_> = text
            .lines()
            .filter(|line| {
                !line.starts_with("visual_style =")
                    && !line.starts_with("quality_preset =")
                    && !line.starts_with("reduce_flashing =")
            })
            .collect();
        lines.push("");
        lines.join("\n")
    };
    let settings = legacy_settings(Settings::default().to_text());
    let settings_hash = fnv1a(settings.bytes());
    let round_trip = legacy_settings(Settings::parse(&settings).to_text());
    let round_trip_hash = fnv1a(round_trip.bytes());

    println!("framebuffer_hash={framebuffer_hash:#018x}");
    println!("world_hashes={world_hashes:#x?}");
    println!("simulation_hash={simulation_hash:#018x}");
    println!(
        "simulation: player={:?}, collected={}, state={:?}, enemy={:?}",
        simulated.player.pos, simulated.collected, simulated.state, simulated.enemy.pos
    );
    println!("settings_hash={settings_hash:#018x}");
    println!("round_trip_hash={round_trip_hash:#018x}");

    assert_eq!(framebuffer_hash, 0xbf6334532d629617);
    assert_eq!(
        world_hashes,
        vec![0x9db8ca5fd26f5d9a, 0xf1e525e4e23cef4a, 0xef53d1573b9ed1e2,]
    );
    assert_eq!(simulation_hash, 0x36aa7954b0bb41c6);
    assert_eq!(settings_hash, 0x7f0672c7d565fef9);
    assert_eq!(round_trip_hash, settings_hash);
}

#[test]
fn moody_render_is_deterministic_and_matches_its_golden() {
    let game = Game::new(42);
    let mut first = Framebuffer::new(160, 90);
    let mut second = Framebuffer::new(160, 90);
    game.draw_styled(&mut first, 137, DEFAULT_HFOV, RenderStyle::moody());
    game.draw_styled(&mut second, 137, DEFAULT_HFOV, RenderStyle::moody());

    let first_hash = pixels_hash(&first.pixels);
    let second_hash = pixels_hash(&second.pixels);
    println!("moody_framebuffer_hash={first_hash:#018x}");
    assert_eq!(
        first_hash, second_hash,
        "rayon rendering must be deterministic"
    );
    // Re-bless intentionally with `cargo test -p studs --test golden -- --nocapture`.
    assert_eq!(first_hash, 0xf4892832fa68b8c2);
}

#[test]
fn realistic_render_is_deterministic_and_matches_its_golden() {
    let game = Game::new(42);
    let mut first = Framebuffer::new(160, 90);
    let mut second = Framebuffer::new(160, 90);
    game.draw_styled(&mut first, 137, DEFAULT_HFOV, RenderStyle::realistic());
    game.draw_styled(&mut second, 137, DEFAULT_HFOV, RenderStyle::realistic());
    let first_hash = pixels_hash(&first.pixels);
    let second_hash = pixels_hash(&second.pixels);
    println!("realistic_framebuffer_hash={first_hash:#018x}");
    assert_eq!(
        first_hash, second_hash,
        "realistic rendering must be deterministic"
    );
    assert_eq!(first_hash, 0x5e666badd78f6dc5);
}

#[test]
fn realistic_monster_sprite_is_visible_and_matches_its_golden() {
    let mut game = Game::new(42);
    let eye = game.player.pos;
    let clear = (0..64)
        .map(|i| i as f32 * std::f32::consts::TAU / 64.0)
        .find(|&angle| {
            let dir = raycast_blocks::math::Vect::new(angle.cos(), angle.sin(), 0.0);
            game.world.cast(eye, dir, 3.0).is_none()
        })
        .expect("spawn cell should have an open neighboring corridor");
    game.player.view.phi = clear;
    let dir = raycast_blocks::math::Vect::new(clear.cos(), clear.sin(), 0.0);
    game.enemy.pos = eye + dir.scale(2.4);
    game.enemy.yaw = clear + std::f32::consts::PI;
    let materials = studs_material_table();
    assert!(
        materials.sprite_atlas.is_some(),
        "baked monster atlas must load"
    );
    let mut scene = game
        .scene_styled(137, DEFAULT_HFOV, RenderStyle::realistic())
        .unwrap();
    let center_ray = raycast_blocks::math::forward_vec(scene.camera.view);
    let warden = scene
        .entities
        .iter()
        .find(|entity| entity.kind == WARDEN)
        .unwrap();
    let sprite_hit = render_cpu::render::entities::SpriteView::new(*warden, scene.camera.eye)
        .hit(center_ray, materials.sprite_atlas.unwrap());
    let sprite_hit = sprite_hit.expect("center ray should intersect the monster sprite");
    let wall_hit = game.world.cast(scene.camera.eye, center_ray, 10.0).unwrap();
    assert!(
        sprite_hit.t < wall_hit.dist,
        "monster must be in front of the corridor wall"
    );
    let mut with_monster = Framebuffer::new(160, 90);
    render_with_materials(
        &mut with_monster,
        &scene,
        &raycast_blocks::surfaces::StudsSurface,
        materials,
    );
    let mut background = Framebuffer::new(160, 90);
    scene.entities.retain(|entity| entity.kind != WARDEN);
    render_with_materials(
        &mut background,
        &scene,
        &raycast_blocks::surfaces::StudsSurface,
        materials,
    );
    let hash = pixels_hash(&with_monster.pixels);
    let changed = with_monster
        .pixels
        .iter()
        .zip(&background.pixels)
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 0, "monster must contribute visible pixels");
    println!("realistic_monster_framebuffer_hash={hash:#018x}");
    assert_eq!(hash, 0x99f2b567fd1244eb);
}
