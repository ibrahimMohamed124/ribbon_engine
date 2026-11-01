use super::*;

pub struct GameReadView<'a> {
    pub difficulty: Difficulty,
    pub world: &'a World,
    pub player: &'a Player,
    pub enemy: &'a Enemy,
    pub pages: &'a [Vect],
    pub got: &'a [bool],
    pub collected: usize,
    pub time: f32,
    pub state: State,
    pub state_t: f32,
    pub flashlight_on: bool,
    pub enemy_awake_since: Option<f32>,
}

/// Small immutable model consumed by menu/HUD rendering.
#[derive(Clone, Copy, Debug)]
pub struct GameUiState {
    pub difficulty: Difficulty,
    pub collected: usize,
    pub page_count: usize,
    pub time: f32,
    pub state: State,
    pub enemy_tip_active: bool,
    pub stamina: f32,
    pub exhausted: bool,
}

impl<'a> GameReadView<'a> {
    pub fn from_game(game: &'a Game) -> Self {
        Self {
            difficulty: game.difficulty,
            world: &game.world,
            player: &game.player,
            enemy: &game.enemy,
            pages: &game.pages,
            got: &game.got,
            collected: game.collected,
            time: game.time,
            state: game.state,
            state_t: game.state_t,
            flashlight_on: game.flashlight_on,
            enemy_awake_since: game.enemy_awake_since,
        }
    }

    pub fn ui_state(&self) -> GameUiState {
        GameUiState {
            difficulty: self.difficulty,
            collected: self.collected,
            page_count: self.pages.len(),
            time: self.time,
            state: self.state,
            enemy_tip_active: self.enemy_tip_active(),
            stamina: self.player.stamina,
            exhausted: self.player.exhausted,
        }
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

    pub fn proximity(&self) -> f32 {
        proximity_intensity(self.enemy_distance(), self.enemy_awake())
    }

    pub fn scene_styled(&self, frame_no: u32, hfov: f32, style: RenderStyle) -> Option<Scene<'a>> {
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
                        self.world,
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
                for (index, position) in self.pages.iter().enumerate() {
                    if self.got.get(index).copied().unwrap_or(false) {
                        continue;
                    }
                    ents.push(page_view(
                        Vect::new(position.x, position.y, position.z),
                        self.time,
                        index as f32,
                    ));
                }
                Some(Scene {
                    world: self.world,
                    empty_voxel: AIR,
                    camera: studs_cpu_renderer::Camera {
                        eye,
                        view,
                        hfov: hfov * (1.0 - jumpscare * 0.12),
                    },
                    lighting: studs_cpu_renderer::LightingParams {
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
            State::Caught => "STUDS | IT GOT YOU - press R".to_owned(),
            State::Won => "STUDS | you made it out... for now - press R".to_owned(),
        }
    }
}
