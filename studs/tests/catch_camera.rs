use engine_core::math::Vect;
use studs::config::DEFAULT_HFOV;
use studs::{
    enemy::EnemyAnimation,
    game::{Game, State},
};

#[test]
fn catch_scene_looks_up_zooms_and_uses_attack_pose() {
    let mut game = Game::new(42);
    game.time = 20.0;
    game.enemy.pos = Vect::new(game.player.pos.x, game.player.pos.y, game.player.pos.z);
    let (before_psi, before_hfov) = {
        let before = game
            .scene_styled(1, DEFAULT_HFOV, studs::styles::realistic())
            .unwrap();
        (before.camera.view.psi, before.camera.hfov)
    };
    game.state = State::Caught;
    game.state_t = 0.45;
    let caught = game
        .scene_styled(2, DEFAULT_HFOV, studs::styles::realistic())
        .unwrap();
    assert!(caught.camera.view.psi > before_psi + 0.5);
    assert!(caught.camera.hfov < before_hfov);
    assert_eq!(
        caught.entities[0].params[1],
        EnemyAnimation::Attack as usize as f32
    );
}
