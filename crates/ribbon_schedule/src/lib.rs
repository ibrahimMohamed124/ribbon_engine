//! Deterministic stages with access-declared parallel systems.
//!
//! Existing systems created with [`SystemConfig::new`] retain exclusive `&mut World`
//! access and always form serial barriers. Systems created with
//! [`SystemConfig::parallel`] receive `&World`, declare every component/resource access,
//! and may run concurrently with compatible systems. Access declarations are a contract:
//! violating them can trigger hecs dynamic-borrow panics or race through user-managed
//! interior mutability. Structural world changes remain exclusive.

use ribbon_ecs::{Component, World};
use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
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

type ExclusiveSystem = Box<dyn FnMut(&mut World) + Send + 'static>;
type ParallelSystem = Box<dyn Fn(&World) + Send + Sync + 'static>;
type ParallelTask<'a> = (usize, String, &'a ParallelSystem);
type SystemTiming = (usize, Duration);
type WorkerTimings = Vec<SystemTiming>;
type Condition = Box<dyn Fn(&World) -> bool + Send + Sync + 'static>;

enum SystemKind {
    Exclusive(ExclusiveSystem),
    Parallel(ParallelSystem),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccessKind {
    Component,
    Resource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccessMode {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AccessKey {
    kind: AccessKind,
    type_id: TypeId,
    type_name: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AccessDeclaration {
    key: AccessKey,
    mode: AccessMode,
}

impl AccessDeclaration {
    fn target_description(self) -> String {
        let kind = match self.key.kind {
            AccessKind::Component => "component",
            AccessKind::Resource => "resource",
        };
        format!("{kind} `{}`", self.key.type_name)
    }
}

/// Builder for one scheduled system.
pub struct SystemConfig {
    kind: SystemKind,
    accesses: Vec<AccessDeclaration>,
    name: Option<String>,
    label: Option<String>,
    before: Vec<String>,
    after: Vec<String>,
    condition: Option<Condition>,
}

impl SystemConfig {
    /// Create a legacy exclusive system. It retains unrestricted `&mut World` access and
    /// therefore executes alone as a serial barrier. Existing callers keep their behavior.
    pub fn new<F>(system: F) -> Self
    where
        F: FnMut(&mut World) + Send + 'static,
    {
        Self {
            kind: SystemKind::Exclusive(Box::new(system)),
            accesses: Vec::new(),
            name: None,
            label: None,
            before: Vec::new(),
            after: Vec::new(),
            condition: None,
        }
    }

    /// Create an access-declared system that can run concurrently with compatible systems.
    ///
    /// The callback receives a shared `World`. It may use `world.query::<&mut T>()` for
    /// component writes; hecs dynamically checks overlapping component borrows. To mutate a
    /// resource, store a lock-backed value (preferably [`ribbon_ecs::SyncResource<T>`]) and declare it as
    /// a resource write. Spawning/despawning entities and changing the resource registry remain
    /// exclusive operations and must be performed by a legacy system/command application.
    ///
    /// Every component and resource the callback can touch must be declared with the methods
    /// below. Reads can run beside reads; any write conflicts with reads/writes of the same type.
    /// This is a contract rather than compile-time access filtering: inaccurate declarations are
    /// a programming error and may cause dynamic-borrow panics or incorrect interior-mutability
    /// behavior. Parallel systems must be `Sync` because one immutable callback may be scheduled
    /// on a worker thread.
    pub fn parallel<F>(system: F) -> Self
    where
        F: Fn(&World) + Send + Sync + 'static,
    {
        Self {
            kind: SystemKind::Parallel(Box::new(system)),
            accesses: Vec::new(),
            name: None,
            label: None,
            before: Vec::new(),
            after: Vec::new(),
            condition: None,
        }
    }

    /// Declare read-only access to component `T`.
    pub fn read_component<T: Component>(mut self) -> Self {
        self.push_access::<T>(AccessKind::Component, AccessMode::Read);
        self
    }

    /// Declare mutable access to component `T`.
    pub fn write_component<T: Component>(mut self) -> Self {
        self.push_access::<T>(AccessKind::Component, AccessMode::Write);
        self
    }

    /// Declare read-only access to resource `T`.
    pub fn read_resource<T: Any + Send + Sync + 'static>(mut self) -> Self {
        self.push_access::<T>(AccessKind::Resource, AccessMode::Read);
        self
    }

    /// Declare mutable access to resource `T`.
    ///
    /// Since parallel callbacks receive `&World`, mutable resource contents must provide
    /// interior mutability, such as [`ribbon_ecs::SyncResource<T>`], `Mutex<T>`, or `RwLock<T>`.
    pub fn write_resource<T: Any + Send + Sync + 'static>(mut self) -> Self {
        self.push_access::<T>(AccessKind::Resource, AccessMode::Write);
        self
    }

    fn push_access<T: 'static>(&mut self, kind: AccessKind, mode: AccessMode) {
        self.accesses.push(AccessDeclaration {
            key: AccessKey {
                kind,
                type_id: TypeId::of::<T>(),
                type_name: std::any::type_name::<T>(),
            },
            mode,
        });
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
    ///
    /// Conditions form serial barriers unless they are absent, because their dynamic accesses
    /// cannot be inferred from the closure. They are evaluated immediately before their system.
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
    kind: SystemKind,
    accesses: Vec<AccessDeclaration>,
    condition: Option<Condition>,
}

/// Typed errors produced when a schedule is built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    /// An ordering constraint names a label not present in the same stage.
    UnknownLabel { system: String, label: String },
    /// System names must be unique throughout one schedule.
    DuplicateName { name: String },
    /// Labels must be unique within one stage.
    DuplicateLabel { stage: Stage, label: String },
    /// The ordering constraints contain a directed cycle.
    Cycle { stage: Stage, systems: Vec<String> },
    /// A parallel system must declare at least one component/resource access.
    MissingAccess { system: String },
    /// Access metadata only applies to access-declared parallel systems.
    AccessDeclarationOnExclusiveSystem { system: String },
    /// A system declared the same component/resource access more than once, even with a
    /// different read/write mode.
    DuplicateAccess { system: String, access: String },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLabel { system, label } => {
                write!(f, "system `{system}` references unknown label `{label}`")
            }
            Self::DuplicateName { name } => write!(f, "duplicate system name `{name}`"),
            Self::DuplicateLabel { stage, label } => {
                write!(f, "duplicate label `{label}` in {stage:?}")
            }
            Self::Cycle { stage, systems } => {
                write!(f, "ordering cycle in {stage:?}: {}", systems.join(", "))
            }
            Self::MissingAccess { system } => write!(
                f,
                "parallel system `{system}` must declare component/resource access"
            ),
            Self::AccessDeclarationOnExclusiveSystem { system } => write!(
                f,
                "exclusive system `{system}` must not declare parallel access metadata"
            ),
            Self::DuplicateAccess { system, access } => {
                write!(f, "system `{system}` declares `{access}` more than once")
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

/// Deterministic ordered collection of systems.
///
/// The serial fallback is always available. When parallel execution is enabled (the default),
/// only built batches of compatible access-declared systems use scoped worker threads; legacy
/// systems, conditional systems, and conflicting systems remain serial barriers. A bounded
/// number of threads is created per parallel batch; a persistent thread pool can be added later
/// if profiling shows thread startup costs are material.
pub struct Schedule {
    entries: Vec<Entry>,
    order: BTreeMap<Stage, Vec<usize>>,
    batches: BTreeMap<Stage, Vec<Vec<usize>>>,
    stats_enabled: bool,
    parallel_enabled: bool,
    max_parallelism: usize,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            order: BTreeMap::new(),
            batches: BTreeMap::new(),
            stats_enabled: false,
            parallel_enabled: true,
            max_parallelism: std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1)
                .max(1),
        }
    }
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

    /// Enable parallel execution for compatible, access-declared systems.
    pub fn set_parallel_enabled(&mut self, enabled: bool) {
        self.parallel_enabled = enabled;
    }

    /// Whether access-declared parallel batches may use worker threads.
    pub fn is_parallel_enabled(&self) -> bool {
        self.parallel_enabled
    }

    /// Set the maximum number of worker threads for one parallel batch. Zero is clamped to one.
    /// A value of one is a convenient deterministic serial fallback for debugging/comparison.
    pub fn set_max_parallelism(&mut self, max_parallelism: usize) {
        self.max_parallelism = max_parallelism.max(1);
    }

    /// Maximum number of workers used for an eligible parallel batch.
    pub fn max_parallelism(&self) -> usize {
        self.max_parallelism
    }

    /// Return the built execution plan for a stage, grouped into conflict-free batches.
    /// Returns `None` before [`Schedule::build`] has succeeded.
    pub fn execution_plan(&self, stage: Stage) -> Option<Vec<Vec<String>>> {
        self.batches.get(&stage).map(|batches| {
            batches
                .iter()
                .map(|batch| {
                    batch
                        .iter()
                        .map(|&index| self.entries[index].name.clone())
                        .collect()
                })
                .collect()
        })
    }

    /// Add a legacy exclusive system, defaulting its name to a deterministic insertion index.
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
            kind: config.kind,
            accesses: config.accesses,
            condition: config.condition,
        });
        self.order.clear();
        self.batches.clear();
    }

    /// Validate names, labels, access declarations, and cycles; then build a stable execution
    /// order and conflict-free parallel batches for every stage.
    pub fn build(&mut self) -> Result<(), ScheduleError> {
        let mut names = BTreeSet::new();
        for entry in &self.entries {
            if !names.insert(entry.name.clone()) {
                return Err(ScheduleError::DuplicateName {
                    name: entry.name.clone(),
                });
            }
            self.validate_accesses(entry)?;
        }

        let mut order = BTreeMap::new();
        let mut batches = BTreeMap::new();
        for stage in Stage::ALL {
            let members: Vec<_> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| (entry.stage == stage).then_some(index))
                .collect();
            let mut labels = BTreeMap::new();
            for &index in &members {
                if let Some(label) = self.entries[index].label.as_ref() {
                    if labels.insert(label.as_str(), index).is_some() {
                        return Err(ScheduleError::DuplicateLabel {
                            stage,
                            label: label.clone(),
                        });
                    }
                }
            }

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

            // Resolve authored before/after constraints first. Then serialize conflicts in this
            // stable order so automatic access edges never reverse an explicit ordering choice.
            let declared_order = topological_order(stage, &members, &edges, &self.entries)?;
            for left_position in 0..declared_order.len() {
                for right_position in (left_position + 1)..declared_order.len() {
                    let left = declared_order[left_position];
                    let right = declared_order[right_position];
                    if systems_conflict(&self.entries[left], &self.entries[right]) {
                        edges
                            .get_mut(&left)
                            .expect("stage member edge exists")
                            .insert(right);
                    }
                }
            }

            let (stage_order, stage_batches) =
                topological_batches(stage, &members, &edges, &self.entries)?;
            order.insert(stage, stage_order);
            batches.insert(stage, stage_batches);
        }
        self.order = order;
        self.batches = batches;
        Ok(())
    }

    /// Run a stage in the built schedule. Before `build`, systems run in insertion order.
    pub fn run_stage(&mut self, stage: Stage, world: &mut World) {
        if !self.parallel_enabled || self.max_parallelism <= 1 {
            let order = self.order.get(&stage).cloned().unwrap_or_else(|| {
                self.entries
                    .iter()
                    .enumerate()
                    .filter_map(|(index, entry)| (entry.stage == stage).then_some(index))
                    .collect()
            });
            for index in order {
                self.run_one(index, world);
            }
            return;
        }

        let Some(batches) = self.batches.get(&stage).cloned() else {
            // An unbuilt schedule has no validated access graph, so retain the old safe behavior.
            for index in self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| (entry.stage == stage).then_some(index))
                .collect::<Vec<_>>()
            {
                self.run_one(index, world);
            }
            return;
        };

        for batch in batches {
            if batch.len() > 1
                && batch.iter().all(|&index| {
                    matches!(&self.entries[index].kind, SystemKind::Parallel(_))
                        && self.entries[index].condition.is_none()
                })
            {
                self.run_parallel_batch(&batch, world);
            } else {
                // Conditions are evaluated immediately before each system and all legacy systems
                // are barriers, so the built plan guarantees this batch has at most one member.
                for index in batch {
                    self.run_one(index, world);
                }
            }
        }
    }

    fn validate_accesses(&self, entry: &Entry) -> Result<(), ScheduleError> {
        let is_parallel = matches!(&entry.kind, SystemKind::Parallel(_));
        if is_parallel && entry.accesses.is_empty() {
            return Err(ScheduleError::MissingAccess {
                system: entry.name.clone(),
            });
        }
        if !is_parallel && !entry.accesses.is_empty() {
            return Err(ScheduleError::AccessDeclarationOnExclusiveSystem {
                system: entry.name.clone(),
            });
        }
        for (index, access) in entry.accesses.iter().enumerate() {
            if entry.accesses[..index]
                .iter()
                .any(|prior| prior.key == access.key)
            {
                return Err(ScheduleError::DuplicateAccess {
                    system: entry.name.clone(),
                    access: access.target_description(),
                });
            }
        }
        Ok(())
    }

    fn run_one(&mut self, index: usize, world: &mut World) {
        let should_run = match self.entries[index].condition.as_ref() {
            Some(condition) => condition(world),
            None => true,
        };
        if !should_run {
            return;
        }

        let stats_enabled = self.stats_enabled;
        if stats_enabled && world.try_resource::<ScheduleStats>().is_none() {
            world.insert_resource(ScheduleStats::default());
        }
        let name = self.entries[index].name.clone();
        let start = Instant::now();
        match &mut self.entries[index].kind {
            SystemKind::Exclusive(system) => system(world),
            SystemKind::Parallel(system) => system(world),
        }
        if stats_enabled {
            world
                .resource_mut::<ScheduleStats>()
                .systems
                .insert(name, start.elapsed());
        }
    }

    fn run_parallel_batch(&self, indices: &[usize], world: &mut World) {
        let systems: Vec<ParallelTask<'_>> = indices
            .iter()
            .map(|&index| {
                let entry = &self.entries[index];
                let SystemKind::Parallel(system) = &entry.kind else {
                    unreachable!("only parallel systems can share an execution batch")
                };
                (index, entry.name.clone(), system)
            })
            .collect();

        let stats_enabled = self.stats_enabled;
        if stats_enabled && world.try_resource::<ScheduleStats>().is_none() {
            world.insert_resource(ScheduleStats::default());
        }

        let worker_count = systems.len().min(self.max_parallelism).max(1);
        let next_task = AtomicUsize::new(0);
        let world_ref: &World = &*world;
        let mut worker_timings: Vec<SystemTiming> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..worker_count)
                .map(|_| {
                    let systems = &systems;
                    let next_task = &next_task;
                    scope.spawn(move || {
                        let mut timings: WorkerTimings = Vec::new();
                        loop {
                            let task_index = next_task.fetch_add(1, Ordering::Relaxed);
                            let Some((system_index, _, system)) = systems.get(task_index) else {
                                break;
                            };
                            if stats_enabled {
                                let start = Instant::now();
                                system(world_ref);
                                timings.push((*system_index, start.elapsed()));
                            } else {
                                system(world_ref);
                            }
                        }
                        timings
                    })
                })
                .collect();
            let mut all_timings = Vec::with_capacity(systems.len());
            let mut first_panic = None;
            for result in handles.into_iter().map(|handle| handle.join()) {
                match result {
                    Ok(timings) => all_timings.extend(timings),
                    Err(payload) if first_panic.is_none() => first_panic = Some(payload),
                    Err(_payload) => {}
                }
            }
            if let Some(payload) = first_panic {
                std::panic::resume_unwind(payload);
            }
            all_timings
        });

        if stats_enabled {
            worker_timings.sort_by_key(|(index, _)| *index);
            let stats = &mut world.resource_mut::<ScheduleStats>().systems;
            for (index, elapsed) in worker_timings {
                stats.insert(self.entries[index].name.clone(), elapsed);
            }
        }
    }
}

fn systems_conflict(left: &Entry, right: &Entry) -> bool {
    // Conditions have dynamic accesses unknown to the scheduler, and exclusive systems receive
    // unrestricted &mut World. Both are conservative barriers against every system.
    if left.condition.is_some()
        || right.condition.is_some()
        || matches!(&left.kind, SystemKind::Exclusive(_))
        || matches!(&right.kind, SystemKind::Exclusive(_))
    {
        return true;
    }

    left.accesses.iter().any(|left_access| {
        right.accesses.iter().any(|right_access| {
            left_access.key == right_access.key
                && (left_access.mode == AccessMode::Write || right_access.mode == AccessMode::Write)
        })
    })
}

fn topological_order(
    stage: Stage,
    members: &[usize],
    edges: &BTreeMap<usize, BTreeSet<usize>>,
    entries: &[Entry],
) -> Result<Vec<usize>, ScheduleError> {
    let mut indegree: BTreeMap<usize, usize> = members.iter().map(|&index| (index, 0)).collect();
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
                .map(|(&index, _)| entries[index].name.clone())
                .collect(),
        });
    }
    Ok(sorted)
}

fn topological_batches(
    stage: Stage,
    members: &[usize],
    edges: &BTreeMap<usize, BTreeSet<usize>>,
    entries: &[Entry],
) -> Result<(Vec<usize>, Vec<Vec<usize>>), ScheduleError> {
    let mut indegree: BTreeMap<usize, usize> = members.iter().map(|&index| (index, 0)).collect();
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
    let mut order = Vec::with_capacity(members.len());
    let mut batches = Vec::new();

    while !ready.is_empty() {
        // The whole ready frontier can run together: all explicit dependencies are satisfied and
        // every conflicting pair has already been serialized with an automatic dependency edge.
        let batch: Vec<_> = ready.iter().copied().collect();
        ready.clear();
        let mut next_ready = BTreeSet::new();
        for &index in &batch {
            order.push(index);
            for &target in edges.get(&index).expect("stage member edge exists") {
                let degree = indegree
                    .get_mut(&target)
                    .expect("edge target is a stage member");
                *degree -= 1;
                if *degree == 0 {
                    next_ready.insert(target);
                }
            }
        }
        batches.push(batch);
        ready = next_ready;
    }

    if order.len() != members.len() {
        return Err(ScheduleError::Cycle {
            stage,
            systems: indegree
                .iter()
                .filter(|(_, degree)| **degree > 0)
                .map(|(&index, _)| entries[index].name.clone())
                .collect(),
        });
    }
    Ok((order, batches))
}

#[cfg(test)]
mod tests {
    use super::{Schedule, ScheduleError, ScheduleStats, Stage, SystemConfig};
    use ribbon_ecs::World;
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
