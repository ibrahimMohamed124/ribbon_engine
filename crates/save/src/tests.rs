use super::*;
use ribbon_app::{App, World};
use ribbon_plugins::Plugin;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Progress {
    score: u32,
}
impl SaveSection for Progress {
    const KEY: &'static str = "test.progress";
    const VERSION: u32 = 1;
    fn capture(world: &World) -> Self {
        Self {
            score: *world.resource::<u32>(),
        }
    }
    fn apply(self, world: &mut World) -> Result<(), SaveError> {
        world.insert_resource(self.score);
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Failing {
    value: u32,
}
impl SaveSection for Failing {
    const KEY: &'static str = "test.z_failing";
    const VERSION: u32 = 2;
    fn capture(_: &World) -> Self {
        Self { value: 0 }
    }
    fn apply(self, _: &mut World) -> Result<(), SaveError> {
        Err(SaveError::Apply("deliberate failure".into()))
    }
    fn migrate(from_version: u32, _: &str) -> Result<Self, SaveError> {
        Err(SaveError::UnsupportedSectionVersion {
            key: Self::KEY.into(),
            found: from_version,
            expected: Self::VERSION,
        })
    }
}
fn app(root: &std::path::Path, with_failing: bool) -> App {
    let mut app = App::new();
    SavePlugin {
        config: SaveConfig {
            qualifier: "test".into(),
            organization: "Ribbon".into(),
            application: "SaveTest".into(),
            game_id: "test-game".into(),
            root_override: Some(root.into()),
        },
    }
    .build(&mut app);
    app.world.insert_resource(42u32);
    app.register_save_section::<Progress>();
    if with_failing {
        app.register_save_section::<Failing>();
    }
    app
}

#[test]
fn roundtrip_and_backup_restore_corrupt_current_file() {
    let temp = tempfile::tempdir().expect("temp directory");
    let mut app = app(temp.path(), false);
    let first = save_to_slot(&app.world, "autosave", "first").expect("save");
    assert_eq!(first.game_id, "test-game");
    *app.world.resource_mut::<u32>() = 9;
    save_to_slot(&app.world, "autosave", "second").expect("overwrite");
    let path = temp.path().join("saves/autosave.rsav");
    std::fs::write(&path, "corrupt").expect("corrupt current file");
    *app.world.resource_mut::<u32>() = 0;
    let meta = load_from_slot(&mut app.world, "autosave").expect("backup recovery");
    assert_eq!(meta.label, "first");
    assert_eq!(*app.world.resource::<u32>(), 42);
}

#[test]
fn bad_slots_are_rejected_and_listing_and_delete_work() {
    let temp = tempfile::tempdir().expect("temp directory");
    let app = app(temp.path(), false);
    for slot in ["../x", "a/b", "", "abcdefghijklmnopqrstuvwxyz1234567"] {
        assert!(matches!(
            save_to_slot(&app.world, slot, "x"),
            Err(SaveError::InvalidSlot(_))
        ));
    }
    save_to_slot(&app.world, "slot_a", "A").expect("save");
    save_to_slot(&app.world, "slot_b", "B").expect("save");
    assert_eq!(list_slots(&app.world).expect("list").len(), 2);
    delete_slot(&app.world, "slot_a").expect("delete");
    assert!(!slot_exists(&app.world, "slot_a"));
}

#[test]
fn two_phase_load_does_not_apply_when_a_section_cannot_be_prepared() {
    let temp = tempfile::tempdir().expect("temp directory");
    let source = app(temp.path(), true);
    save_to_slot(&source.world, "slot", "x").expect("save");
    let path = temp.path().join("saves/slot.rsav");
    let mut data = std::fs::read_to_string(&path).expect("read file");
    let body_start = data
        .find('\n')
        .and_then(|n| data[n + 1..].find('\n').map(|m| n + 1 + m + 1))
        .expect("body offset");
    let mut env: crate::envelope::Envelope = ron::from_str(&data[body_start..]).expect("envelope");
    env.sections
        .iter_mut()
        .find(|r| r.key == Failing::KEY)
        .expect("failing section")
        .version = 1;
    let body = ron::ser::to_string_pretty(&env, ron::ser::PrettyConfig::default()).expect("encode");
    data = format!(
        "RSAV1\ncrc32:{:08x}\n{body}",
        crc32fast::hash(body.as_bytes())
    );
    std::fs::write(path, data).expect("write crafted save");
    let mut target = app(temp.path(), true);
    *target.world.resource_mut::<u32>() = 7;
    assert!(matches!(
        load_from_slot(&mut target.world, "slot"),
        Err(SaveError::UnsupportedSectionVersion { .. })
    ));
    assert_eq!(*target.world.resource::<u32>(), 7);
}

#[test]
fn wrong_game_and_roundtrip_ron_are_checked() {
    let encoded = ron::to_string(&Progress { score: 12 }).expect("encode progress");
    assert_eq!(
        ron::from_str::<Progress>(&encoded)
            .expect("decode progress")
            .score,
        12
    );
    let temp = tempfile::tempdir().expect("temp directory");
    let mut app = app(temp.path(), false);
    save_to_slot(&app.world, "slot", "x").expect("save");
    app.world.resource_mut::<SaveConfig>().game_id = "other".into();
    assert!(matches!(
        load_from_slot(&mut app.world, "slot"),
        Err(SaveError::WrongGame { .. })
    ));
}

#[test]
fn save_request_produces_finished_event_during_app_update() {
    use ribbon_app::{EventCursor, WorldEventsExt};
    let temp = tempfile::tempdir().expect("temp directory");
    let mut app = app(temp.path(), false);
    let mut cursor = EventCursor::<SaveFinished>::default();
    app.world.send_event(SaveRequest {
        slot: "event_slot".into(),
        label: "event".into(),
    });
    app.update();
    let finished = cursor
        .read(app.world.events::<SaveFinished>())
        .next()
        .expect("finished event");
    assert!(finished.result.is_ok());
}
