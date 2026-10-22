//! Deterministic application lifecycle, scheduling, time, and runner contracts.
//!
//! The app preserves serial legacy systems and may dispatch compatible access-declared systems
//! in parallel. Window and rendering integrations live in other crates.

pub mod time;

use engine_events::{EventCursor, Events, WorldEventsExt};
use engine_schedule::Schedule;
use std::time::Duration;

pub use engine_ecs::{SyncResource, World};
pub use engine_schedule::{ScheduleError, ScheduleStats, Stage, SystemConfig};
pub use time::{FixedTime, LogBuffer, LogLevel, Time};

/// Event emitted to ask the active runner to stop.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppExit;

/// A runner owns the application for its duration.
pub trait Runner {
    /// Start driving the provided app.
    fn run(self: Box<Self>, app: App);
}

type FinishHook = Box<dyn FnOnce(&mut App)>;

/// Deterministic runner that advances by exactly one virtual 60 Hz frame.
pub struct HeadlessRunner {
    frames: u64,
}

impl HeadlessRunner {
    /// Create a runner that updates at most `frames` times.
    pub fn new(frames: u64) -> Self {
        Self { frames }
    }
}

impl Runner for HeadlessRunner {
    fn run(self: Box<Self>, mut app: App) {
        app.startup();
        let mut exit_cursor = EventCursor::<AppExit>::default();
        for _ in 0..self.frames {
            app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
            app.update();
            if exit_cursor
                .read(app.world.events::<AppExit>())
                .next()
                .is_some()
            {
                break;
            }
        }
        app.shutdown();
    }
}

/// Application state, world, schedule, and selected runner.
pub struct App {
    pub world: World,
    schedule: Schedule,
    runner: Option<Box<dyn Runner>>,
    startup_ran: bool,
    shutdown_ran: bool,
    finish_hooks: Vec<FinishHook>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Create an app with the built-in time, log, and exit-event resources.
    pub fn new() -> Self {
        let mut world = World::new();
        world.insert_resource(Time::default());
        world.insert_resource(FixedTime::default());
        world.insert_resource(LogBuffer::default());
        world.add_event::<AppExit>();
        let mut app = Self {
            world,
            schedule: Schedule::new(),
            runner: Some(Box::new(HeadlessRunner::new(0))),
            startup_ran: false,
            shutdown_ran: false,
            finish_hooks: Vec::new(),
        };
        app.add_system(Stage::First, |world| {
            world.resource_scope::<Events<AppExit>, _>(|_, events| events.update());
        });
        app
    }

    /// Insert or replace a world resource.
    pub fn insert_resource<T: Send + Sync + 'static>(&mut self, value: T) -> &mut Self {
        self.world.insert_resource(value);
        self
    }

    /// Add a system to a stage.
    pub fn add_system<F>(&mut self, stage: Stage, system: F) -> &mut Self
    where
        F: FnMut(&mut World) + Send + 'static,
    {
        self.schedule.add_system(stage, system);
        self
    }

    /// Add a configured system to a stage.
    pub fn add_system_config(&mut self, stage: Stage, config: SystemConfig) -> &mut Self {
        self.schedule.add_system_config(stage, config);
        self
    }

    /// Enable or disable access-declared parallel system execution. The default is enabled;
    /// legacy systems and dependency/access barriers remain serial either way.
    pub fn set_parallel_enabled(&mut self, enabled: bool) -> &mut Self {
        self.schedule.set_parallel_enabled(enabled);
        self
    }

    /// Set the maximum number of workers used for one compatible parallel batch. Zero is
    /// clamped to one, which is useful for serial comparison.
    pub fn set_max_parallelism(&mut self, max_parallelism: usize) -> &mut Self {
        self.schedule.set_max_parallelism(max_parallelism);
        self
    }

    /// Inspect the built conflict-free execution batches for a stage. Returns `None` until the
    /// schedule has been built successfully.
    pub fn execution_plan(&self, stage: Stage) -> Option<Vec<Vec<String>>> {
        self.schedule.execution_plan(stage)
    }

    /// Register typed event storage and age it at the beginning of each frame.
    pub fn add_event<T: Send + Sync + 'static>(&mut self) -> &mut Self {
        self.world.add_event::<T>();
        self.add_system(Stage::First, |world| {
            world.resource_scope::<Events<T>, _>(|_, events| events.update());
        });
        self
    }

    /// Select the runner used by [`run`](Self::run).
    pub fn set_runner(&mut self, runner: impl Runner + 'static) -> &mut Self {
        self.runner = Some(Box::new(runner));
        self
    }

    /// Validate the schedule and run plugin finish hooks in registration order.
    pub fn build(mut self) -> Result<Self, ScheduleError> {
        self.schedule.build()?;
        for hook in std::mem::take(&mut self.finish_hooks) {
            hook(&mut self);
        }
        self.schedule.build()?;
        Ok(self)
    }

    /// Start the app once, running its one-time startup stage.
    pub fn startup(&mut self) {
        if !self.startup_ran {
            self.schedule.run_stage(Stage::Startup, &mut self.world);
            self.startup_ran = true;
        }
    }

    /// Run the one-time shutdown stage.
    pub fn shutdown(&mut self) {
        if !self.shutdown_ran {
            self.schedule.run_stage(Stage::Shutdown, &mut self.world);
            self.shutdown_ran = true;
        }
    }

    /// Advance one frame using the current `Time::delta`.
    pub fn update(&mut self) {
        self.startup();
        let delta = self.world.resource::<Time>().delta;
        {
            let time = self.world.resource_mut::<Time>();
            time.elapsed = time.elapsed.saturating_add(delta);
            time.frame = time.frame.saturating_add(1);
        }
        for stage in [Stage::First, Stage::PreUpdate] {
            self.schedule.run_stage(stage, &mut self.world);
        }

        let fixed_steps = self.world.resource_mut::<FixedTime>().advance(delta);
        if self.world.resource::<FixedTime>().dropped_excess {
            self.world.resource_mut::<FixedTime>().dropped_excess = false;
            self.world.resource_mut::<LogBuffer>().push(
                LogLevel::Warn,
                "FixedUpdate exceeded 8 steps; excess time dropped",
            );
        }
        let frame_delta = self.world.resource::<Time>().delta;
        let fixed_delta = self.world.resource::<FixedTime>().step;
        for _ in 0..fixed_steps {
            self.world.resource_mut::<Time>().delta = fixed_delta;
            self.schedule.run_stage(Stage::FixedUpdate, &mut self.world);
        }
        self.world.resource_mut::<Time>().delta = frame_delta;
        for stage in [Stage::Update, Stage::PostUpdate, Stage::Render, Stage::Last] {
            self.schedule.run_stage(stage, &mut self.world);
        }
    }

    /// Run the app with its configured runner.
    pub fn run(mut self) {
        for hook in std::mem::take(&mut self.finish_hooks) {
            hook(&mut self);
        }
        if let Err(error) = self.schedule.build() {
            self.world
                .resource_mut::<LogBuffer>()
                .push(LogLevel::Error, format!("cannot run app: {error}"));
            return;
        }
        if let Some(runner) = self.runner.take() {
            runner.run(self);
        }
    }

    /// Add a deferred plugin finish hook. Used by the plugin extension API.
    pub fn add_finish_hook(&mut self, hook: Box<dyn FnOnce(&mut App)>) {
        self.finish_hooks.push(hook);
    }
}

#[cfg(test)]
mod tests {
    use super::{App, HeadlessRunner};
    use engine_schedule::Stage;
    use std::sync::{Arc, Mutex};

    #[test]
    fn headless_runner_executes_ten_frames_in_stage_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut app = App::new();
        app.set_runner(HeadlessRunner::new(10));
        for (stage, name) in [
            (Stage::Startup, "startup"),
            (Stage::First, "first"),
            (Stage::PreUpdate, "pre"),
            (Stage::FixedUpdate, "fixed"),
            (Stage::Update, "update"),
            (Stage::PostUpdate, "post"),
            (Stage::Render, "render"),
            (Stage::Last, "last"),
        ] {
            let log = Arc::clone(&log);
            app.add_system(stage, move |_| log.lock().unwrap().push(name));
        }
        app.run();
        let log = log.lock().unwrap();
        assert_eq!(log.first(), Some(&"startup"));
        assert_eq!(log.len(), 1 + 10 * 7);
        assert_eq!(
            &log[1..8],
            ["first", "pre", "fixed", "update", "post", "render", "last"]
        );
    }
}
