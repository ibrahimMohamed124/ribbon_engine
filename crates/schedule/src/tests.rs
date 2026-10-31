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
    assert!(matches!(error, ScheduleError::Cycle { systems, .. } if systems == ["alpha", "beta"]));
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
