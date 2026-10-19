//! Deterministic, single-threaded engine stages and dependency-ordered systems.
//!
//! Systems receive the world directly. Parameter injection and parallel execution are not part
//! of version one.

use engine_ecs::World;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::{Duration, Instant};

/// Execution stages in declaration/execution order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    Startup,
    First,
    PreUpdate,
    FixedUpdate,
    Update,
    PostUpdate,
    Render,
    Last,
    Shutdown,
}

impl Stage {
    const ALL: [Self; 9] = [
        Self::Startup,
        Self::First,
        Self::PreUpdate,
        Self::FixedUpdate,
        Self::Update,
        Self::PostUpdate,
        Self::Render,
        Self::Last,
        Self::Shutdown,
    ];
}

type System = Box<dyn FnMut(&mut World) + Send + 'static>;
type Condition = Box<dyn Fn(&World) -> bool + Send + Sync + 'static>;

/// Builder for one scheduled system.
pub struct SystemConfig {
    system: System,
    name: Option<String>,
    label: Option<String>,
    before: Vec<String>,
    after: Vec<String>,
    condition: Option<Condition>,
}

impl SystemConfig {
    /// Create a config from a world system.
    pub fn new<F>(system: F) -> Self
    where
        F: FnMut(&mut World) + Send + 'static,
    {
        Self {
            system: Box::new(system),
            name: None,
            label: None,
            before: Vec::new(),
            after: Vec::new(),
            condition: None,
        }
    }

    /// Give this system a unique diagnostic name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add the label used by ordering constraints.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Require this system to run before the system with `label`.
    pub fn before(mut self, label: impl Into<String>) -> Self {
        self.before.push(label.into());
        self
    }

    /// Require this system to run after the system with `label`.
    pub fn after(mut self, label: impl Into<String>) -> Self {
        self.after.push(label.into());
        self
    }

    /// Run only when the read-only world condition returns true.
    pub fn run_if<F>(mut self, condition: F) -> Self
    where
        F: Fn(&World) -> bool + Send + Sync + 'static,
    {
        self.condition = Some(Box::new(condition));
        self
    }
}

struct Entry {
    stage: Stage,
    name: String,
    label: Option<String>,
    before: Vec<String>,
    after: Vec<String>,
    system: System,
    condition: Option<Condition>,
}

/// Typed errors produced when a schedule is built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    /// An ordering constraint names a label not present in the same stage.
    UnknownLabel { system: String, label: String },
    /// System names must be unique throughout one schedule.
    DuplicateName { name: String },
    /// The ordering constraints contain a directed cycle.
    Cycle { stage: Stage, systems: Vec<String> },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLabel { system, label } => {
                write!(f, "system `{system}` references unknown label `{label}`")
            }
            Self::DuplicateName { name } => write!(f, "duplicate system name `{name}`"),
            Self::Cycle { stage, systems } => {
                write!(f, "ordering cycle in {stage:?}: {}", systems.join(", "))
            }
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Per-system timing from the most recent scheduled run.
#[derive(Default)]
pub struct ScheduleStats {
    /// Last observed duration by system name.
    pub systems: BTreeMap<String, Duration>,
}

/// Ordered collection of deterministic systems.
#[derive(Default)]
pub struct Schedule {
    entries: Vec<Entry>,
    order: BTreeMap<Stage, Vec<usize>>,
    stats_enabled: bool,
}

impl Schedule {
    /// Create an empty schedule.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable per-system timings.
    pub fn set_stats_enabled(&mut self, enabled: bool) {
        self.stats_enabled = enabled;
    }

    /// Add a system, defaulting its name to a deterministic insertion index.
    pub fn add_system<F>(&mut self, stage: Stage, system: F)
    where
        F: FnMut(&mut World) + Send + 'static,
    {
        self.add_system_config(stage, SystemConfig::new(system));
    }

    /// Add a configured system.
    pub fn add_system_config(&mut self, stage: Stage, config: SystemConfig) {
        let index = self.entries.len();
        self.entries.push(Entry {
            stage,
            name: config.name.unwrap_or_else(|| format!("system_{index}")),
            label: config.label,
            before: config.before,
            after: config.after,
            system: config.system,
            condition: config.condition,
        });
        self.order.clear();
    }

    /// Validate names/labels/cycles and compute stable topological order.
    pub fn build(&mut self) -> Result<(), ScheduleError> {
        let mut names = BTreeSet::new();
        for entry in &self.entries {
            if !names.insert(entry.name.clone()) {
                return Err(ScheduleError::DuplicateName {
                    name: entry.name.clone(),
                });
            }
        }

        let mut order = BTreeMap::new();
        for stage in Stage::ALL {
            let members: Vec<_> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| (entry.stage == stage).then_some(index))
                .collect();
            let labels: BTreeMap<_, _> = members
                .iter()
                .filter_map(|&index| {
                    self.entries[index]
                        .label
                        .as_ref()
                        .map(|label| (label.as_str(), index))
                })
                .collect();
            let mut edges: BTreeMap<usize, BTreeSet<usize>> = members
                .iter()
                .map(|&index| (index, BTreeSet::new()))
                .collect();
            for &index in &members {
                let entry = &self.entries[index];
                for label in &entry.before {
                    let Some(&target) = labels.get(label.as_str()) else {
                        return Err(ScheduleError::UnknownLabel {
                            system: entry.name.clone(),
                            label: label.clone(),
                        });
                    };
                    edges
                        .get_mut(&index)
                        .expect("stage member edge exists")
                        .insert(target);
                }
                for label in &entry.after {
                    let Some(&source) = labels.get(label.as_str()) else {
                        return Err(ScheduleError::UnknownLabel {
                            system: entry.name.clone(),
                            label: label.clone(),
                        });
                    };
                    edges
                        .get_mut(&source)
                        .expect("stage member edge exists")
                        .insert(index);
                }
            }

            let mut indegree: BTreeMap<usize, usize> =
                members.iter().map(|&index| (index, 0)).collect();
            for targets in edges.values() {
                for &target in targets {
                    *indegree
                        .get_mut(&target)
                        .expect("edge target is a stage member") += 1;
                }
            }
            let mut ready: BTreeSet<_> = indegree
                .iter()
                .filter_map(|(&index, &degree)| (degree == 0).then_some(index))
                .collect();
            let mut sorted = Vec::with_capacity(members.len());
            while let Some(index) = ready.pop_first() {
                sorted.push(index);
                for &target in edges.get(&index).expect("stage member edge exists") {
                    let degree = indegree
                        .get_mut(&target)
                        .expect("edge target is a stage member");
                    *degree -= 1;
                    if *degree == 0 {
                        ready.insert(target);
                    }
                }
            }
            if sorted.len() != members.len() {
                return Err(ScheduleError::Cycle {
                    stage,
                    systems: indegree
                        .iter()
                        .filter(|(_, degree)| **degree > 0)
                        .map(|(&index, _)| self.entries[index].name.clone())
                        .collect(),
                });
            }
            order.insert(stage, sorted);
        }
        self.order = order;
        Ok(())
    }

    /// Run a stage in the built order, or insertion order before `build` is called.
    pub fn run_stage(&mut self, stage: Stage, world: &mut World) {
        let order = self.order.get(&stage).cloned().unwrap_or_else(|| {
            self.entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| (entry.stage == stage).then_some(index))
                .collect()
        });
        for index in order {
            let entry = &mut self.entries[index];
            if entry
                .condition
                .as_ref()
                .is_some_and(|condition| !condition(world))
            {
                continue;
            }
            if self.stats_enabled {
                if world.try_resource::<ScheduleStats>().is_none() {
                    world.insert_resource(ScheduleStats::default());
                }
                let start = Instant::now();
                (entry.system)(world);
                world
                    .resource_mut::<ScheduleStats>()
                    .systems
                    .insert(entry.name.clone(), start.elapsed());
            } else {
                (entry.system)(world);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Schedule, ScheduleError, ScheduleStats, Stage, SystemConfig};
    use engine_ecs::World;
    use std::sync::{Arc, Mutex};

    fn recorder(
        log: Arc<Mutex<Vec<&'static str>>>,
        value: &'static str,
    ) -> impl FnMut(&mut World) + Send {
        move |_| log.lock().unwrap().push(value)
    }

    #[test]
    fn ordering_constraints_override_insertion_and_ties_stay_stable() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut schedule = Schedule::new();
        schedule.add_system_config(
            Stage::Update,
            SystemConfig::new(recorder(Arc::clone(&log), "second"))
                .label("second")
                .after("first"),
        );
        schedule.add_system_config(
            Stage::Update,
            SystemConfig::new(recorder(Arc::clone(&log), "first")).label("first"),
        );
        schedule.add_system(Stage::Update, recorder(Arc::clone(&log), "tie-a"));
        schedule.add_system(Stage::Update, recorder(Arc::clone(&log), "tie-b"));
        schedule.build().unwrap();
        schedule.run_stage(Stage::Update, &mut World::new());
        assert_eq!(*log.lock().unwrap(), ["first", "second", "tie-a", "tie-b"]);
    }

    #[test]
    fn build_reports_ordering_cycles_with_involved_system_names() {
        let mut schedule = Schedule::new();
        schedule.add_system_config(
            Stage::Update,
            SystemConfig::new(|_| {})
                .name("alpha")
                .label("a")
                .before("b"),
        );
        schedule.add_system_config(
            Stage::Update,
            SystemConfig::new(|_| {})
                .name("beta")
                .label("b")
                .before("a"),
        );
        let error = schedule.build().unwrap_err();
        assert!(
            matches!(error, ScheduleError::Cycle { systems, .. } if systems == ["alpha", "beta"])
        );
    }

    #[test]
    fn run_condition_skips_systems_and_stats_record_the_last_run() {
        let mut world = World::new();
        world.insert_resource(0u32);
        let mut schedule = Schedule::new();
        schedule.set_stats_enabled(true);
        schedule.add_system_config(
            Stage::Update,
            SystemConfig::new(|world| *world.resource_mut::<u32>() += 1)
                .name("enabled")
                .run_if(|world| *world.resource::<u32>() == 0),
        );
        schedule.add_system_config(
            Stage::Update,
            SystemConfig::new(|world| *world.resource_mut::<u32>() += 100)
                .name("skipped")
                .run_if(|_| false),
        );
        schedule.build().unwrap();
        schedule.run_stage(Stage::Update, &mut world);
        assert_eq!(*world.resource::<u32>(), 1);
        assert!(world
            .resource::<ScheduleStats>()
            .systems
            .contains_key("enabled"));
        assert!(!world
            .resource::<ScheduleStats>()
            .systems
            .contains_key("skipped"));
    }

    #[test]
    fn build_rejects_unknown_labels_and_duplicate_names() {
        let mut unknown = Schedule::new();
        unknown.add_system_config(
            Stage::Update,
            SystemConfig::new(|_| {}).name("lonely").before("missing"),
        );
        assert!(
            matches!(unknown.build(), Err(ScheduleError::UnknownLabel { system, label }) if system == "lonely" && label == "missing")
        );

        let mut duplicate = Schedule::new();
        duplicate.add_system_config(Stage::Update, SystemConfig::new(|_| {}).name("same"));
        duplicate.add_system_config(Stage::Last, SystemConfig::new(|_| {}).name("same"));
        assert_eq!(
            duplicate.build(),
            Err(ScheduleError::DuplicateName {
                name: "same".to_owned()
            })
        );
    }
}
