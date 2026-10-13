use raycast_blocks::game::{Game, State};
use raycast_blocks::input::{Controls, InputFrame};
use raycast_blocks::math::Rng;
use raycast_blocks::renderer::{Framebuffer, DEFAULT_HFOV};
use raycast_blocks::settings::Settings;
use raycast_blocks::world::World;

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
    game.draw(&mut framebuffer, 137, DEFAULT_HFOV);
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

    let settings = Settings::default().to_text();
    let settings_hash = fnv1a(settings.bytes());
    let round_trip_hash = fnv1a(Settings::parse(&settings).to_text().bytes());

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
