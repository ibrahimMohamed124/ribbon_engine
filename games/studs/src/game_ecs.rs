//! ECS-owned STUDS domain state and simulation systems.
//!
//! The older [`crate::game::Game`] remains a small compatibility facade for the
//! existing domain API and deterministic rendering fixtures. The desktop runtime
//! does not store that aggregate: maze, run progression, player, enemy, and page
//! pickups are represented independently in the ECS world.

use crate::audio::{AudioEvent, SpatialAudioEvent};
use crate::enemy::{footstep_crossings, Enemy};
use crate::game::{Difficulty, Game, State, PAGES_TOTAL};
use crate::input::InputFrame;
use crate::level::World as Maze;
use crate::player::Player;
use ribbon_app::{Time, World};
use ribbon_math::{forward_vec, Vect};
use studs_cpu_renderer::RENDER_DIST;

const PAGE_PICKUP_RADIUS: f32 = 0.9;

/// Marker identifying entities owned by the current maze/run. Restart despawns
/// these entities together before installing a newly generated run.
#[derive(Clone, Copy, Debug, Default)]
pub struct GameOwned;

/// The generated maze and authoritative voxel data.
pub struct MazeResource(pub Maze);

/// A collectible page is an independent ECS entity.
#[derive(Clone, Copy, Debug)]
pub struct PagePickup {
    pub index: usize,
    pub position: Vect,
    pub collected: bool,
}

/// Run-level state that is not naturally owned by a single world entity.
pub struct RunState {
    pub difficulty: Difficulty,
    pub collected: usize,
    pub time: f32,
    pub state: State,
    pub state_t: f32,
    pub flashlight_on: bool,
    pub events: Vec<AudioEvent>,
    pub spatial_events: Vec<SpatialAudioEvent>,
    pub enemy_awake_since: Option<f32>,
}

impl RunState {
    pub fn enemy_awake(&self) -> bool {
        self.time > self.difficulty.enemy_wake_time() || self.collected > 0
    }

    pub fn enemy_tip_active(&self) -> bool {
        self.enemy_awake_since
            .is_some_and(|awake_at| self.time - awake_at < 8.0)
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

/// The input snapshot selected for the current fixed step.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunStep {
    pub active: bool,
    pub input: InputFrame,
    pub enemy_awake_before_time_advance: bool,
}

/// Move a generated domain state into ECS-owned resources and entities.
pub fn install_game(world: &mut World, game: Game) {
    let old_entities: Vec<_> = world
        .query::<&GameOwned>()
        .iter()
        .map(|(entity, _)| entity)
        .collect();
    for entity in old_entities {
        let _ = world.despawn(entity);
    }

    let Game {
        difficulty,
        world: maze,
        player,
        enemy,
        pages,
        got,
        collected,
        time,
        state,
        state_t,
        flashlight_on,
        events,
        spatial_events,
        enemy_awake_since,
    } = game;

    world.insert_resource(MazeResource(maze));
    world.insert_resource(RunState {
        difficulty,
        collected,
        time,
        state,
        state_t,
        flashlight_on,
        events,
        spatial_events,
        enemy_awake_since,
    });
    world.insert_resource(RunStep::default());
    world.spawn((GameOwned, player));
    world.spawn((GameOwned, enemy));
    for (index, position) in pages.into_iter().enumerate() {
        world.spawn((
            GameOwned,
            PagePickup {
                index,
                position,
                collected: got.get(index).copied().unwrap_or(false),
            },
        ));
    }
}

/// Player movement runs only while a fixed step is actively advancing gameplay.
pub fn update_player(world: &mut World) {
    let step = *world.resource::<RunStep>();
    if !step.active {
        return;
    }
    let dt = world.resource::<Time>().delta.as_secs_f32();
    world.resource_scope::<MazeResource, _>(|world, maze| {
        let events = world
            .query_mut::<&mut Player>()
            .into_iter()
            .next()
            .map(|(_, player)| player.update(&maze.0, &step.input.controls, dt));
        let Some(events) = events else { return };
        let run = world.resource_mut::<RunState>();
        if events.footstep {
            run.events.push(AudioEvent::Footstep);
        }
        if events.jumped {
            run.events.push(AudioEvent::Jump);
        }
        if events.landed {
            run.events.push(AudioEvent::Land);
        }
    });
}

/// Page collection is its own system; every pickup remains an addressable entity.
pub fn collect_pages(world: &mut World) {
    if !world.resource::<RunStep>().active {
        return;
    }
    let player_pos = world
        .query::<&Player>()
        .iter()
        .next()
        .map(|(_, player)| player.pos);
    let Some(player_pos) = player_pos else { return };
    let mut newly_collected = 0usize;
    for (_, pickup) in world.query_mut::<&mut PagePickup>() {
        if !pickup.collected && pickup.position.dist_xy(player_pos) < PAGE_PICKUP_RADIUS {
            pickup.collected = true;
            newly_collected += 1;
        }
    }
    if newly_collected != 0 {
        let run = world.resource_mut::<RunState>();
        run.collected += newly_collected;
        run.events.extend(std::iter::repeat_n(
            AudioEvent::PageCollected,
            newly_collected,
        ));
    }
}

/// Enemy perception, chase/search behavior, and spatial footsteps are updated
/// independently of player movement and pickup collection.
pub fn update_enemy(world: &mut World) {
    let step = *world.resource::<RunStep>();
    if !step.active {
        return;
    }
    let dt = world.resource::<Time>().delta.as_secs_f32();
    world.resource_scope::<MazeResource, _>(|world, maze| {
        world.resource_scope::<RunState, _>(|world, run| {
            let player = world
                .query::<&Player>()
                .iter()
                .next()
                .map(|(_, player)| (player.pos, player.view));
            let Some((player_pos, player_view)) = player else {
                return;
            };

            let awake = run.enemy_awake();
            if !awake {
                return;
            }
            if !step.enemy_awake_before_time_advance {
                run.events.push(AudioEvent::EnemySting);
                run.enemy_awake_since = Some(run.time);
            }

            let mut spatial_events = Vec::new();
            if let Some((_, enemy)) = world.query_mut::<&mut Enemy>().into_iter().next() {
                let player_visible =
                    !enemy.has_seen_player || enemy.can_see_player(player_pos, &maze.0);
                let seen = enemy.is_seen_from(player_pos, player_view, &maze.0, RENDER_DIST);
                let speed = Enemy::speed_with_tuning(
                    run.collected,
                    seen,
                    run.difficulty.enemy_speed_multiplier(),
                    run.difficulty.seen_speed_factor(),
                );
                let previous_phase = enemy.phase;
                enemy.update_hunt(&maze.0, player_pos, player_visible, speed, dt);
                let footsteps = footstep_crossings(previous_phase, enemy.phase);
                if footsteps > 0 {
                    let forward = forward_vec(player_view);
                    let to_enemy =
                        Vect::new(enemy.pos.x - player_pos.x, enemy.pos.y - player_pos.y, 0.0)
                            .normalize();
                    let right = Vect::new(-forward.y, forward.x, 0.0);
                    spatial_events.extend((0..footsteps).map(|_| SpatialAudioEvent {
                        event: AudioEvent::EnemyFootstep,
                        pan: to_enemy.dot(right),
                    }));
                }
            }
            run.spatial_events.extend(spatial_events);
        });
    });
}

/// Resolve terminal conditions after player, pickups, and enemy have updated.
pub fn resolve_run(world: &mut World) {
    if !world.resource::<RunStep>().active {
        return;
    }
    let player_pos = world
        .query::<&Player>()
        .iter()
        .next()
        .map(|(_, player)| player.pos);
    let enemy_pos = world
        .query::<&Enemy>()
        .iter()
        .next()
        .map(|(_, enemy)| enemy.pos);
    let (Some(player_pos), Some(enemy_pos)) = (player_pos, enemy_pos) else {
        return;
    };
    let run = world.resource_mut::<RunState>();
    if run.state != State::Playing {
        return;
    }
    if run.enemy_awake() && enemy_pos.dist_xy(player_pos) < crate::enemy::CATCH_DIST {
        run.state = State::Caught;
        run.state_t = 0.0;
        run.events.push(AudioEvent::Caught);
    } else if run.collected >= PAGES_TOTAL {
        run.state = State::Won;
        run.state_t = 0.0;
        run.events.push(AudioEvent::Won);
    }
}

/// Helpers for systems and rendering to read the current ECS entities safely.
pub fn page_snapshot(world: &World) -> (Vec<Vect>, Vec<bool>) {
    let mut pickups: Vec<PagePickup> = world
        .query::<&PagePickup>()
        .iter()
        .map(|(_, pickup)| *pickup)
        .collect();
    pickups.sort_by_key(|pickup| pickup.index);
    (
        pickups.iter().map(|pickup| pickup.position).collect(),
        pickups.iter().map(|pickup| pickup.collected).collect(),
    )
}

/// Player, enemy, and `(pickup positions, collected flags)` snapshot used for rendering.
pub type ViewParts = (Player, Enemy, (Vec<Vect>, Vec<bool>));

/// Convert ECS-owned components and run state to the compatibility input type
/// expected by the existing rendering helpers, without storing a `Game` resource.
pub fn read_view_parts(world: &World) -> Option<ViewParts> {
    // Clone the authoritative entities only at the rendering boundary; simulation
    // systems continue operating directly on the ECS-owned components.
    let player = world
        .query::<&Player>()
        .into_iter()
        .next()
        .map(|(_, player)| player.clone())?;
    let enemy = world
        .query::<&Enemy>()
        .into_iter()
        .next()
        .map(|(_, enemy)| enemy.clone())?;
    Some((player, enemy, page_snapshot(world)))
}
