//! ECS finite state machines.
use ribbon_app::{App, Stage, States, Time, World, WorldEventsExt};
use ribbon_ecs::Entity;
use std::marker::PhantomData;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateChanged<S: States> {
    pub entity: Entity,
    pub from: S,
    pub to: S,
}
#[derive(Clone, Copy, Debug)]
pub struct StateMachine<S: States> {
    current: S,
    previous: Option<S>,
    time_in_state: f32,
    requested: Option<S>,
}
impl<S: States> StateMachine<S> {
    pub fn new(initial: S) -> Self {
        Self {
            current: initial,
            previous: None,
            time_in_state: 0.0,
            requested: None,
        }
    }
    pub fn current(&self) -> S {
        self.current
    }
    pub fn previous(&self) -> Option<S> {
        self.previous
    }
    pub fn time_in_state(&self) -> f32 {
        self.time_in_state
    }
    pub fn request(&mut self, next: S) {
        self.requested = Some(next);
    }
}
struct Registered<S: States>(PhantomData<S>);
pub fn register_state_machine<S: States>(app: &mut App) {
    if app.world.try_resource::<Registered<S>>().is_some() {
        return;
    }
    app.world.insert_resource(Registered::<S>(PhantomData));
    app.add_event::<StateChanged<S>>();
    app.add_system(Stage::Update, |world: &mut World| {
        let dt = world
            .try_resource::<Time>()
            .map_or(0.0, |t| t.delta.as_secs_f32().max(0.0));
        let changes = world
            .query_mut::<&mut StateMachine<S>>()
            .into_iter()
            .filter_map(|(e, sm)| {
                sm.time_in_state += dt;
                sm.requested
                    .take()
                    .filter(|next| *next != sm.current)
                    .map(|next| {
                        let from = sm.current;
                        sm.previous = Some(from);
                        sm.current = next;
                        sm.time_in_state = 0.0;
                        StateChanged {
                            entity: e,
                            from,
                            to: next,
                        }
                    })
            })
            .collect::<Vec<_>>();
        for event in changes {
            world.send_event(event);
        }
    });
}
