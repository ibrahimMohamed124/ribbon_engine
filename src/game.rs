//! Game state machine: owns the world, player, enemy and pages, advances them
//! each frame and knows how to draw itself. It never touches the OS: input
//! arrives as an [`InputFrame`], sound leaves as [`AudioEvent`]s, pixels go
//! into a [`Framebuffer`].

use crate::audio::AudioEvent;
use crate::enemy::Enemy;
use crate::input::{Controls, InputFrame};
use crate::math::{hash4, Angles, Rng, Vect};
use crate::player::Player;
use crate::renderer::hud::{draw_caught, draw_hud, draw_won};
use crate::renderer::{render, EntView, Frame, Framebuffer, RENDER_DIST};
use crate::world::{cell_center, World, CELLS, FLOOR_TOP};

pub const PAGES_TOTAL: usize = 6;
const PAGE_PICKUP_RADIUS: f32 = 0.9;
/// Seconds the game-over / win screen must be visible before R restarts.
const RESTART_DELAY: f32 = 1.0;
/// Distance (tiles) at which the enemy starts to affect the picture.
const PROXIMITY_RANGE: f32 = 10.0;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum State {
    Playing,
    Caught,
    Won,
}

pub struct Game {
    pub world: World,
    pub player: Player,
    pub enemy: Enemy,
    pub pages: Vec<Vect>,
    pub got: Vec<bool>,
    pub collected: usize,
    pub time: f32,
    pub state: State,
    pub state_t: f32,
    pub flashlight_on: bool,
    /// Sound requests produced by `update`. The caller drains these every frame.
    pub events: Vec<AudioEvent>,
}

impl Game {
    pub fn new(seed: u32) -> Game {
        let mut rng = Rng::new(seed);
        let world = World::generate(&mut rng);

        let mut cells: Vec<(usize, usize)> = Vec::new();
        while cells.len() < PAGES_TOTAL {
            let c = (rng.below(CELLS), rng.below(CELLS));
            if c.0 + c.1 >= 6 && c != (CELLS - 1, CELLS - 1) && !cells.contains(&c) {
                cells.push(c);
            }
        }
        let pages: Vec<Vect> = cells
            .iter()
            .map(|&(cx, cy)| Vect::new(cell_center(cx), cell_center(cy), FLOOR_TOP + 0.95))
            .collect();

        let start = cell_center(0);
        let far = cell_center(CELLS - 1);
        Game {
            world,
            player: Player::new(
                start,
                start,
                Angles {
                    psi: 0.0,
                    phi: 0.785,
                },
            ),
            enemy: Enemy::new(far, far),
            got: vec![false; pages.len()],
            pages,
            collected: 0,
            time: 0.0,
            state: State::Playing,
            state_t: 0.0,
            flashlight_on: true,
            events: Vec::new(),
        }
    }

    /// Start a fresh run on a new maze, keeping the flashlight setting.
    pub fn restart(&mut self, seed: u32) {
        let flashlight_on = self.flashlight_on;
        *self = Game::new(seed);
        self.flashlight_on = flashlight_on;
    }

    /// The game-over / win screen has been up long enough to allow a restart.
    pub fn can_restart(&self) -> bool {
        self.state != State::Playing && self.state_t > RESTART_DELAY
    }

    pub fn enemy_awake(&self) -> bool {
        Enemy::awake(self.time, self.collected)
    }

    pub fn enemy_distance(&self) -> f32 {
        self.enemy.pos.dist_xy(self.player.pos)
    }

    /// 0..1, how threatening the enemy's closeness is (0 while it sleeps).
    pub fn proximity(&self) -> f32 {
        if self.enemy_awake() {
            ((PROXIMITY_RANGE - self.enemy_distance()) / PROXIMITY_RANGE).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    pub fn update(&mut self, input: &InputFrame, dt: f32) {
        if input.toggle_flashlight {
            self.flashlight_on = !self.flashlight_on;
        }
        match self.state {
            State::Playing => self.update_playing(&input.controls, dt),
            State::Caught | State::Won => self.state_t += dt,
        }
    }

    fn update_playing(&mut self, controls: &Controls, dt: f32) {
        self.time += dt;

        let pe = self.player.update(&self.world, controls, dt);
        if pe.jumped {
            self.events.push(AudioEvent::Jump);
        }
        if pe.landed {
            self.events.push(AudioEvent::Land);
        }

        // pages
        for i in 0..self.pages.len() {
            if self.got[i] {
                continue;
            }
            if self.pages[i].dist_xy(self.player.pos) < PAGE_PICKUP_RADIUS {
                self.got[i] = true;
                self.collected += 1;
                self.events.push(AudioEvent::PageCollected);
            }
        }

        // enemy
        let awake = self.enemy_awake();
        if awake {
            let seen = self.enemy.is_seen_from(
                self.player.pos,
                self.player.view,
                &self.world,
                RENDER_DIST,
            );
            let speed = Enemy::speed(self.collected, seen);
            let pp = self.player.pos;
            self.enemy.update(&self.world, pp, speed, dt);
        }

        if awake && self.enemy.has_caught(self.player.pos) {
            self.state = State::Caught;
            self.state_t = 0.0;
            self.events.push(AudioEvent::Caught);
        } else if self.collected >= PAGES_TOTAL {
            self.state = State::Won;
            self.state_t = 0.0;
            self.events.push(AudioEvent::Won);
        }
    }

    /// Draw the current state into `fb`.
    pub fn draw(&self, fb: &mut Framebuffer, frame_no: u32, hfov: f32) {
        match self.state {
            State::Playing => {
                let prox = self.proximity();
                let shake = prox * prox * 0.02;
                let view = Angles {
                    psi: self.player.view.psi + (self.time * 53.0).cos() * shake,
                    phi: self.player.view.phi + (self.time * 37.0).sin() * shake,
                };

                let n = hash4((self.time * 14.0) as i32, 3, 5, 9);
                let flick = if n > 0.93 - prox * 0.6 { 0.12 } else { 1.0 };
                let flash = if self.flashlight_on { flick } else { 0.0 };

                let eye = self.player.pos;
                let mut ents: Vec<EntView> = Vec::with_capacity(1 + self.pages.len());
                ents.push(EntView::enemy(
                    self.enemy.pos,
                    self.enemy.yaw,
                    self.enemy.phase,
                    eye,
                ));
                for (i, p) in self.pages.iter().enumerate() {
                    if self.got[i] {
                        continue;
                    }
                    let bob = (self.time * 2.0 + i as f32).sin() * 0.08;
                    let pos = Vect::new(p.x, p.y, p.z + bob);
                    ents.push(EntView::page(pos, self.time * 1.2 + i as f32, eye));
                }

                let frame = Frame {
                    world: &self.world,
                    eye,
                    view,
                    hfov,
                    flash,
                    ents,
                    frame: frame_no,
                    prox,
                };
                render(fb, &frame);
                draw_hud(
                    fb,
                    self.collected,
                    PAGES_TOTAL,
                    self.player.stamina,
                    self.player.exhausted,
                );
            }
            State::Caught => draw_caught(fb, self.state_t, frame_no),
            State::Won => draw_won(fb, self.state_t, frame_no),
        }
    }

    pub fn window_title(&self, fps: f32) -> String {
        match self.state {
            State::Playing => format!(
                "STUDS | pages {}/{} | {:.0} fps",
                self.collected, PAGES_TOTAL, fps
            ),
            State::Caught => "STUDS | IT GOT YOU - press R".to_string(),
            State::Won => "STUDS | you made it out... for now - press R".to_string(),
        }
    }
}
