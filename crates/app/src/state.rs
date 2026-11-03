//! Typed application state and deferred state transitions.

use crate::{App, LogLevel, Stage, World};
use ribbon_events::WorldEventsExt;
use std::collections::HashMap;
use std::sync::Mutex;

/// Marker for values suitable for application states.
pub trait States: Copy + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static {}
impl<T> States for T where T: Copy + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static {}

/// Current state. Read-only for games; change it through `NextState`.
pub struct State<S: States>(S);
impl<S: States> State<S> {
    /// Return the current state value.
    pub fn get(&self) -> S {
        self.0
    }
}

/// Request a transition. Applied at the start of the next `PreUpdate` stage.
pub struct NextState<S: States>(Option<S>);
impl<S: States> NextState<S> {
    /// Queue a state transition.
    pub fn set(&mut self, state: S) {
        self.0 = Some(state);
    }
    /// Return the currently pending state, if any.
    pub fn pending(&self) -> Option<S> {
        self.0
    }
}

/// Event sent exactly once for each applied transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateTransition<S: States> {
    pub from: S,
    pub to: S,
}

/// Create a run condition that is true while `state` is active.
pub fn in_state<S: States>(state: S) -> impl Fn(&World) -> bool + Send + Sync + 'static {
    move |world| {
        world
            .try_resource::<State<S>>()
            .is_some_and(|current| current.get() == state)
    }
}

type Hook = Mutex<Box<dyn FnMut(&mut World) + Send>>;
type HookMap<S> = HashMap<S, Vec<Hook>>;
struct StateHooks<S: States> {
    enter: HookMap<S>,
    exit: HookMap<S>,
}
impl<S: States> Default for StateHooks<S> {
    fn default() -> Self {
        Self {
            enter: HashMap::new(),
            exit: HashMap::new(),
        }
    }
}
struct StateInitialized<S: States>(bool, std::marker::PhantomData<S>);

impl App {
    /// Install a state resource and its transition system. Repeated calls preserve the first value.
    pub fn init_state<S: States>(&mut self, initial: S) -> &mut Self {
        if self.world.try_resource::<State<S>>().is_some() {
            return self;
        }
        self.world.insert_resource(State(initial));
        self.world.insert_resource(NextState::<S>(None));
        self.world.insert_resource(StateHooks::<S>::default());
        self.world
            .insert_resource(StateInitialized::<S>(false, std::marker::PhantomData));
        self.add_event::<StateTransition<S>>();
        self.add_system(Stage::PreUpdate, apply_state_transition::<S>);
        self
    }

    /// Register a callback for entry into `state`.
    pub fn on_enter<S: States>(
        &mut self,
        state: S,
        f: impl FnMut(&mut World) + Send + 'static,
    ) -> &mut Self {
        if self.world.try_resource::<StateHooks<S>>().is_none() {
            self.world.resource_mut::<crate::LogBuffer>().push(
                LogLevel::Error,
                format!("on_enter called before init_state for {state:?}"),
            );
            return self;
        }
        self.world
            .resource_mut::<StateHooks<S>>()
            .enter
            .entry(state)
            .or_default()
            .push(Mutex::new(Box::new(f)));
        self
    }

    /// Register a callback for exit from `state`.
    pub fn on_exit<S: States>(
        &mut self,
        state: S,
        f: impl FnMut(&mut World) + Send + 'static,
    ) -> &mut Self {
        if self.world.try_resource::<StateHooks<S>>().is_none() {
            self.world.resource_mut::<crate::LogBuffer>().push(
                LogLevel::Error,
                format!("on_exit called before init_state for {state:?}"),
            );
            return self;
        }
        self.world
            .resource_mut::<StateHooks<S>>()
            .exit
            .entry(state)
            .or_default()
            .push(Mutex::new(Box::new(f)));
        self
    }
}

fn apply_state_transition<S: States>(world: &mut World) {
    let initialized = world.resource::<StateInitialized<S>>().0;
    if !initialized {
        world.resource_mut::<StateInitialized<S>>().0 = true;
        let current = world.resource::<State<S>>().get();
        run_hooks::<S>(world, current, true);
    }
    let next = world.resource_mut::<NextState<S>>().0.take();
    let Some(next) = next else {
        return;
    };
    let current = world.resource::<State<S>>().get();
    if next == current {
        return;
    }
    run_hooks::<S>(world, current, false);
    world.resource_mut::<State<S>>().0 = next;
    world.send_event(StateTransition {
        from: current,
        to: next,
    });
    run_hooks::<S>(world, next, true);
}

fn run_hooks<S: States>(world: &mut World, state: S, entering: bool) {
    world.resource_scope::<StateHooks<S>, _>(|world, hooks| {
        let callbacks = if entering {
            hooks.enter.get_mut(&state)
        } else {
            hooks.exit.get_mut(&state)
        };
        if let Some(callbacks) = callbacks {
            for callback in callbacks {
                if let Ok(callback) = callback.get_mut() {
                    callback(world);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, HeadlessRunner, SystemConfig};
    use ribbon_events::{EventCursor, WorldEventsExt};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum Screen {
        Menu,
        Playing,
        Other,
    }

    #[test]
    fn transition_runs_exit_then_event_then_enter() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut app = App::new();
        app.init_state(Screen::Menu);
        let l = log.clone();
        app.on_exit(Screen::Menu, move |_| l.lock().unwrap().push("exit"));
        let l = log.clone();
        let mut event_cursor = EventCursor::<StateTransition<Screen>>::default();
        app.on_enter(Screen::Playing, move |world| {
            if event_cursor
                .read(world.events::<StateTransition<Screen>>())
                .any(|event| event.from == Screen::Menu && event.to == Screen::Playing)
            {
                l.lock().unwrap().push("event");
            }
            l.lock().unwrap().push("enter");
        });
        app.world
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Playing);
        app.update();
        assert_eq!(*log.lock().unwrap(), ["exit", "event", "enter"]);
    }

    #[test]
    fn same_state_is_ignored() {
        let mut app = App::new();
        app.init_state(Screen::Menu);
        app.world
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Menu);
        app.update();
        assert_eq!(app.world.resource::<State<Screen>>().get(), Screen::Menu);
    }
    #[test]
    fn condition_gates_system() {
        let mut app = App::new();
        app.init_state(Screen::Menu);
        let n = Arc::new(Mutex::new(0));
        let c = n.clone();
        app.add_system_config(
            crate::Stage::Update,
            SystemConfig::new(move |_| *c.lock().unwrap() += 1).run_if(in_state(Screen::Menu)),
        );
        app.update();
        app.world
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Playing);
        app.update();
        assert_eq!(*n.lock().unwrap(), 1);
    }
    #[test]
    fn hook_request_applies_next_frame() {
        let mut app = App::new();
        app.init_state(Screen::Menu);
        app.on_enter(Screen::Playing, |w| {
            w.resource_mut::<NextState<Screen>>().set(Screen::Other)
        });
        app.world
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Playing);
        app.update();
        assert_eq!(app.world.resource::<State<Screen>>().get(), Screen::Playing);
        app.update();
        assert_eq!(app.world.resource::<State<Screen>>().get(), Screen::Other);
    }
    #[test]
    fn double_init_keeps_first_state() {
        let mut app = App::new();
        app.init_state(Screen::Menu);
        app.init_state(Screen::Playing);
        assert_eq!(app.world.resource::<State<Screen>>().get(), Screen::Menu);
    }
    #[test]
    fn initial_enter_fires_once() {
        let mut app = App::new();
        app.init_state(Screen::Menu);
        let n = Arc::new(Mutex::new(0));
        let c = n.clone();
        app.on_enter(Screen::Menu, move |_| *c.lock().unwrap() += 1);
        app.set_runner(HeadlessRunner::new(0));
        app.update();
        app.update();
        assert_eq!(*n.lock().unwrap(), 1);
    }
}
