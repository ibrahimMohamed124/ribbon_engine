//! Game state machine: owns the world, player, enemy and pages, advances them
//! each frame and knows how to draw itself. It never touches the OS: input
//! arrives as an [`InputFrame`], sound leaves as [`AudioEvent`]s, pixels go
//! into a [`Framebuffer`].

use crate::audio::{AudioEvent, SpatialAudioEvent};
use crate::enemy::{
    animation_state, flashlight_pulse_factor, footstep_crossings, proximity_intensity, Enemy,
};
use crate::input::{Controls, InputFrame};
use crate::player::Player;
use crate::surfaces::{enemy_view, page_view, StudsSurface};
use crate::ui::hud::{draw_caught, draw_hud, draw_won};
use crate::world::{cell_center, World, AIR, CELLS, FLOOR_TOP};
use engine_math::{forward_vec, hash4, Angles, Rng, Vect};
use engine_render::Framebuffer;
use plugin_rendering::scene::Scene;
use plugin_rendering::style::RenderStyle;
use plugin_rendering::RENDER_DIST;
use plugin_rendering::cpu::render::raycaster::render_with_materials;
use plugin_rendering::cpu::canvas::Canvas;

pub const PAGES_TOTAL: usize = 6;
const PAGE_PICKUP_RADIUS: f32 = 0.9;
/// Seconds the game-over / win screen must be visible before R restarts.
const RESTART_DELAY: f32 = 1.0;
/// Distance (tiles) at which the enemy starts to affect the picture.
const PAGE_MIN_SEPARATION: f32 = 4.5;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum State {
    Playing,
    Caught,
    Won,
}

/// Run difficulty controls the enemy's grace period and chase pressure.
#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl Difficulty {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Easy => "EASY",
            Self::Normal => "NORMAL",
            Self::Hard => "HARD",
        }
    }

    pub const fn enemy_wake_time(self) -> f32 {
        match self {
            Self::Easy => 18.0,
            Self::Normal => crate::enemy::ENEMY_AWAKE_AFTER,
            Self::Hard => 8.0,
        }
    }

    pub const fn enemy_speed_multiplier(self) -> f32 {
        match self {
            Self::Easy => 0.70,
            Self::Normal => 1.0,
            Self::Hard => 1.35,
        }
    }

    pub const fn seen_speed_factor(self) -> f32 {
        match self {
            Self::Easy => 0.08,
            Self::Normal => crate::enemy::SEEN_SPEED_FACTOR,
            Self::Hard => 0.14,
        }
    }
}

pub struct Game {
    pub difficulty: Difficulty,
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
    pub spatial_events: Vec<SpatialAudioEvent>,
    pub enemy_awake_since: Option<f32>,
}

impl Game {
    pub fn new(seed: u32) -> Game {
        Self::new_with_difficulty(seed, Difficulty::Normal)
    }

    pub fn new_with_difficulty(seed: u32, difficulty: Difficulty) -> Game {
        let mut rng = Rng::new(seed);
        let world = World::generate(&mut rng);

        let mut cell_order: Vec<(usize, usize)> = (0..CELLS)
            .flat_map(|cy| (0..CELLS).map(move |cx| (cx, cy)))
            .collect();
        for i in (1..cell_order.len()).rev() {
            let j = rng.below(i + 1);
            cell_order.swap(i, j);
        }
        let start_cell = cell_order[0];
        let enemy_cell = *cell_order
            .iter()
            .find(|&&(cx, cy)| cx.abs_diff(start_cell.0) + cy.abs_diff(start_cell.1) >= 5)
            .expect("maze has cells far enough apart");
        let start = Vect::new(
            cell_center(start_cell.0),
            cell_center(start_cell.1),
            FLOOR_TOP,
        );
        let enemy_start = Vect::new(
            cell_center(enemy_cell.0),
            cell_center(enemy_cell.1),
            FLOOR_TOP,
        );

        let mut pages = Vec::with_capacity(PAGES_TOTAL);
        for minimum_gap in [PAGE_MIN_SEPARATION, 3.0, 0.0] {
            for &(cx, cy) in &cell_order {
                let page = Vect::new(cell_center(cx), cell_center(cy), FLOOR_TOP + 0.95);
                if page.dist_xy(start) < PAGE_MIN_SEPARATION
                    || page.dist_xy(enemy_start) < PAGE_MIN_SEPARATION
                    || pages.iter().any(|&existing: &Vect| {
                        existing == page || existing.dist_xy(page) < minimum_gap
                    })
                {
                    continue;
                }
                pages.push(page);
                if pages.len() == PAGES_TOTAL {
                    break;
                }
            }
            if pages.len() == PAGES_TOTAL {
                break;
            }
        }
        assert_eq!(pages.len(), PAGES_TOTAL, "maze must fit all page pickups");
        Game {
            difficulty,
            world,
            player: Player::new(
                start.x,
                start.y,
                Angles {
                    psi: 0.0,
                    phi: 0.785,
                },
            ),
            enemy: Enemy::new(enemy_start.x, enemy_start.y),
            got: vec![false; pages.len()],
            pages,
            collected: 0,
            time: 0.0,
            state: State::Playing,
            state_t: 0.0,
            flashlight_on: true,
            events: Vec::new(),
            spatial_events: Vec::new(),
            enemy_awake_since: None,
        }
    }

    /// Start a fresh run on a new maze, keeping the flashlight setting.
    pub fn restart(&mut self, seed: u32) {
        let flashlight_on = self.flashlight_on;
        let difficulty = self.difficulty;
        *self = Game::new_with_difficulty(seed, difficulty);
        self.flashlight_on = flashlight_on;
    }

    /// The game-over / win screen has been up long enough to allow a restart.
    pub fn can_restart(&self) -> bool {
        self.state != State::Playing && self.state_t > RESTART_DELAY
    }

    pub fn enemy_awake(&self) -> bool {
        self.time > self.difficulty.enemy_wake_time() || self.collected > 0
    }

    pub fn enemy_tip_active(&self) -> bool {
        self.enemy_awake_since
            .is_some_and(|awake_at| self.time - awake_at < 8.0)
    }

    pub fn enemy_distance(&self) -> f32 {
        self.enemy.pos.dist_xy(self.player.pos)
    }

    /// 0..1, how threatening the enemy's closeness is (0 while it sleeps).
    pub fn proximity(&self) -> f32 {
        proximity_intensity(self.enemy_distance(), self.enemy_awake())
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
        let enemy_was_awake = self.enemy_awake();
        self.time += dt;

        let pe = self.player.update(&self.world, controls, dt);
        if pe.footstep {
            self.events.push(AudioEvent::Footstep);
        }
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
            if !enemy_was_awake {
                self.events.push(AudioEvent::EnemySting);
                self.enemy_awake_since = Some(self.time);
            }
            let player_visible = !self.enemy.has_seen_player
                || self.enemy.can_see_player(self.player.pos, &self.world);
            let seen = self.enemy.is_seen_from(
                self.player.pos,
                self.player.view,
                &self.world,
                RENDER_DIST,
            );
            let speed = Enemy::speed_with_tuning(
                self.collected,
                seen,
                self.difficulty.enemy_speed_multiplier(),
                self.difficulty.seen_speed_factor(),
            );
            let pp = self.player.pos;
            let previous_phase = self.enemy.phase;
            self.enemy
                .update_hunt(&self.world, pp, player_visible, speed, dt);
            let footsteps = footstep_crossings(previous_phase, self.enemy.phase);
            if footsteps > 0 {
                let forward = forward_vec(self.player.view);
                let to_enemy = Vect::new(
                    self.enemy.pos.x - self.player.pos.x,
                    self.enemy.pos.y - self.player.pos.y,
                    0.0,
                )
                .normalize();
                let right = Vect::new(-forward.y, forward.x, 0.0);
                for _ in 0..footsteps {
                    self.spatial_events.push(SpatialAudioEvent {
                        event: AudioEvent::EnemyFootstep,
                        pan: to_enemy.dot(right),
                    });
                }
            }
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
        self.draw_styled(fb, frame_no, hfov, crate::styles::classic());
    }

    pub fn draw_styled(&self, fb: &mut Framebuffer, frame_no: u32, hfov: f32, style: RenderStyle) {
        if let Some(scene) = self.scene_styled(frame_no, hfov, style) {
            render_with_materials(fb, &scene, &StudsSurface, StudsSurface.materials());
        }
        self.draw_overlay(fb, frame_no);
    }

    pub fn scene_styled(&self, frame_no: u32, hfov: f32, style: RenderStyle) -> Option<Scene<'_>> {
        match self.state {
            State::Playing | State::Caught => {
                let prox = self.proximity();
                let shake = prox * prox * 0.02;
                let jumpscare = if self.state == State::Caught {
                    (self.state_t / 0.45).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let view = Angles {
                    psi: self.player.view.psi + (self.time * 53.0).cos() * shake + jumpscare * 0.55,
                    phi: self.player.view.phi + (self.time * 37.0).sin() * shake,
                };

                let n = hash4((self.time * 14.0) as i32, 3, 5, 9);
                let pulse = n > 0.93 - prox * 0.6;
                let flick = flashlight_pulse_factor(prox, pulse, style.limit_flicker);
                let flash = if self.flashlight_on { flick } else { 0.0 };

                let eye = self.player.pos;
                let mut ents = Vec::with_capacity(1 + self.pages.len());
                let enemy_awake = self.enemy_awake();
                let distance = self.player.pos.dist_xy(self.enemy.pos);
                let enemy_seen = enemy_awake
                    && self.enemy.is_seen_from(
                        self.player.pos,
                        self.player.view,
                        &self.world,
                        RENDER_DIST,
                    );
                let animation = animation_state(
                    enemy_awake,
                    enemy_seen,
                    distance,
                    Enemy::speed_with_tuning(
                        self.collected,
                        enemy_seen,
                        self.difficulty.enemy_speed_multiplier(),
                        self.difficulty.seen_speed_factor(),
                    ),
                    self.enemy.moving,
                );
                let animation_phase = if self.enemy.moving {
                    self.enemy.phase
                } else {
                    self.time * 0.22
                };
                ents.push(enemy_view(
                    self.enemy.pos,
                    self.enemy.yaw,
                    animation_phase,
                    animation as usize,
                    eye,
                ));
                for (i, p) in self.pages.iter().enumerate() {
                    if self.got[i] {
                        continue;
                    }
                    let pos = Vect::new(p.x, p.y, p.z);
                    ents.push(page_view(pos, self.time, i as f32));
                }

                Some(Scene {
                    world: &self.world,
                    empty_voxel: AIR,
                    camera: plugin_rendering::Camera {
                        eye,
                        view,
                        hfov: hfov * (1.0 - jumpscare * 0.12),
                    },
                    lighting: plugin_rendering::LightingParams {
                        light_level: flash,
                        intensity: if style.billboards_enabled {
                            (prox * if enemy_seen { 1.15 } else { 1.0 }).clamp(0.0, 1.0)
                        } else {
                            0.0
                        },
                        frame: frame_no,
                        style,
                    },
                    entities: ents,
                    entity_registry: &crate::entities::ENTITY_REGISTRY,
                })
            }
            State::Won => None,
        }
    }

    pub fn draw_overlay(&self, fb: &mut impl Canvas, frame_no: u32) {
        match self.state {
            State::Playing => draw_hud(
                fb,
                self.collected,
                PAGES_TOTAL,
                self.player.stamina,
                self.player.exhausted,
            ),
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
