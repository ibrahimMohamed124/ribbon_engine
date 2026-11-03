use super::*;
use crate::section::SaveRegistry;
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Migrating {
    score: u32,
    title: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct MigratingV1 {
    score: u32,
}
impl SaveSection for Migrating {
    const KEY: &'static str = "test.migrating";
    const VERSION: u32 = 2;
    fn capture(world: &World) -> Self {
        Self {
            score: *world.resource::<u32>(),
            title: "current".into(),
        }
    }
    fn apply(self, world: &mut World) -> Result<(), SaveError> {
        world.insert_resource(self.score);
        world.insert_resource(self.title);
        Ok(())
    }
    fn migrate(from_version: u32, raw_ron: &str) -> Result<Self, SaveError> {
        if from_version != 1 {
            return Err(SaveError::UnsupportedSectionVersion {
                key: Self::KEY.into(),
                found: from_version,
                expected: Self::VERSION,
            });
        }
        let old: MigratingV1 = ron::from_str(raw_ron).map_err(|e| SaveError::Ron(e.to_string()))?;
        Ok(Self {
            score: old.score,
            title: "migrated-v1".into(),
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

#[test]
fn missing_primary_save_recovers_from_backup_after_interrupted_replace() {
    let temp = tempfile::tempdir().expect("temp directory");
    let mut app = app(temp.path(), false);
    save_to_slot(&app.world, "autosave", "known-good").expect("first save");
    *app.world.resource_mut::<u32>() = 99;
    save_to_slot(&app.world, "autosave", "newer").expect("backup-producing overwrite");

    let primary = temp.path().join("saves/autosave.rsav");
    let backup = temp.path().join("saves/autosave.rsav.bak");
    assert!(backup.exists());
    std::fs::remove_file(&primary).expect("simulate interruption after old file moved");
    *app.world.resource_mut::<u32>() = 0;

    assert!(slot_exists(&app.world, "autosave"));
    let meta = load_from_slot(&mut app.world, "autosave").expect("backup-only recovery");
    assert_eq!(meta.label, "known-good");
    assert_eq!(*app.world.resource::<u32>(), 42);
}

#[test]
fn registered_section_successfully_migrates_an_older_version() {
    let temp = tempfile::tempdir().expect("temp directory");
    let mut source = App::new();
    SavePlugin {
        config: SaveConfig {
            qualifier: "test".into(),
            organization: "Ribbon".into(),
            application: "SaveTest".into(),
            game_id: "test-game".into(),
            root_override: Some(temp.path().into()),
        },
    }
    .build(&mut source);
    source.world.insert_resource(17_u32);
    source.register_save_section::<Migrating>();
    save_to_slot(&source.world, "migrate", "v2").expect("save v2");

    let path = temp.path().join("saves/migrate.rsav");
    let text = std::fs::read_to_string(&path).expect("read save");
    let body_start = text
        .find("\ncrc32:")
        .and_then(|i| text[i + 1..].find('\n').map(|n| i + 1 + n + 1))
        .expect("body start");
    let mut envelope: crate::envelope::Envelope =
        ron::from_str(&text[body_start..]).expect("decode envelope");
    let record = envelope
        .sections
        .iter_mut()
        .find(|r| r.key == Migrating::KEY)
        .expect("record");
    record.version = 1;
    record.data = ron::to_string(&MigratingV1 { score: 73 }).expect("encode v1");
    let body = ron::ser::to_string_pretty(&envelope, ron::ser::PrettyConfig::default())
        .expect("encode envelope");
    std::fs::write(
        &path,
        format!(
            "RSAV1\ncrc32:{:08x}\n{body}",
            crc32fast::hash(body.as_bytes())
        ),
    )
    .expect("write v1 save");

    let mut target = App::new();
    SavePlugin {
        config: SaveConfig {
            qualifier: "test".into(),
            organization: "Ribbon".into(),
            application: "SaveTest".into(),
            game_id: "test-game".into(),
            root_override: Some(temp.path().into()),
        },
    }
    .build(&mut target);
    target.world.insert_resource(0_u32);
    target.register_save_section::<Migrating>();
    load_from_slot(&mut target.world, "migrate").expect("successful v1 migration");
    assert_eq!(*target.world.resource::<u32>(), 73);
    assert_eq!(target.world.resource::<String>(), "migrated-v1");
}

#[test]
fn missing_registered_section_is_reported_without_applying_other_sections() {
    let temp = tempfile::tempdir().expect("temp directory");
    let source = app(temp.path(), false);
    save_to_slot(&source.world, "missing", "x").expect("save");
    let path = temp.path().join("saves/missing.rsav");
    let text = std::fs::read_to_string(&path).expect("read save");
    let body_start = text
        .find("\ncrc32:")
        .and_then(|i| text[i + 1..].find('\n').map(|n| i + 1 + n + 1))
        .expect("body start");
    let mut envelope: crate::envelope::Envelope =
        ron::from_str(&text[body_start..]).expect("decode envelope");
    envelope.sections.clear();
    let body = ron::ser::to_string_pretty(&envelope, ron::ser::PrettyConfig::default())
        .expect("encode envelope");
    std::fs::write(
        &path,
        format!(
            "RSAV1\ncrc32:{:08x}\n{body}",
            crc32fast::hash(body.as_bytes())
        ),
    )
    .expect("write missing-section save");

    let mut target = app(temp.path(), false);
    target.register_save_section::<Failing>();
    *target.world.resource_mut::<u32>() = 9;
    assert!(
        matches!(load_from_slot(&mut target.world, "missing"), Err(SaveError::MissingSection(key)) if key == Progress::KEY)
    );
    assert_eq!(*target.world.resource::<u32>(), 9);
}

#[test]
fn duplicate_section_registration_is_ignored() {
    let temp = tempfile::tempdir().expect("temp directory");
    let mut app = app(temp.path(), false);
    app.register_save_section::<Progress>();
    assert_eq!(app.world.resource::<SaveRegistry>().0.len(), 1);
}

#[test]
fn load_request_produces_load_finished_event_during_app_update() {
    use ribbon_app::{EventCursor, WorldEventsExt};
    let temp = tempfile::tempdir().expect("temp directory");
    let mut app = app(temp.path(), false);
    save_to_slot(&app.world, "load_event", "event").expect("save");
    let mut cursor = EventCursor::<LoadFinished>::default();
    app.world.send_event(LoadRequest {
        slot: "load_event".into(),
    });
    app.update();
    let finished = cursor
        .read(app.world.events::<LoadFinished>())
        .next()
        .expect("load finished event");
    assert!(finished.result.is_ok());
}

#[test]
fn backup_only_slot_is_listed_and_can_be_deleted() {
    let temp = tempfile::tempdir().expect("temp directory");
    let app = app(temp.path(), false);
    save_to_slot(&app.world, "backup_only", "recoverable").expect("first save");
    save_to_slot(&app.world, "backup_only", "newer").expect("overwrite");
    let primary = temp.path().join("saves/backup_only.rsav");
    std::fs::remove_file(&primary).expect("simulate interrupted primary replace");

    let slots = list_slots(&app.world).expect("list backup-only slot");
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].slot, "backup_only");
    assert_eq!(slots[0].label, "recoverable");
    assert!(slot_exists(&app.world, "backup_only"));
    delete_slot(&app.world, "backup_only").expect("delete backup-only slot");
    assert!(!slot_exists(&app.world, "backup_only"));
}

#[test]
fn list_slots_uses_backup_when_primary_exists_but_is_corrupt() {
    let temp = tempfile::tempdir().expect("temp directory");
    let mut app = app(temp.path(), false);
    save_to_slot(&app.world, "recoverable", "older-valid").expect("first save");
    *app.world.resource_mut::<u32>() = 100;
    save_to_slot(&app.world, "recoverable", "newer-valid").expect("second save");

    let primary = temp.path().join("saves/recoverable.rsav");
    let backup = temp.path().join("saves/recoverable.rsav.bak");
    assert!(backup.exists());
    std::fs::write(&primary, "corrupt primary").expect("corrupt primary");

    let slots = list_slots(&app.world).expect("list recoverable slots");
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].slot, "recoverable");
    assert_eq!(slots[0].label, "older-valid");
}

#[test]
fn list_slots_prefers_primary_once_when_a_backup_also_exists() {
    let temp = tempfile::tempdir().expect("temp directory");
    let mut app = app(temp.path(), false);
    save_to_slot(&app.world, "single", "older").expect("first save");
    *app.world.resource_mut::<u32>() = 84;
    save_to_slot(&app.world, "single", "newer").expect("second save");

    let slots = list_slots(&app.world).expect("list slot with primary and backup");
    assert_eq!(
        slots.len(),
        1,
        "primary and backup must not duplicate a slot"
    );
    assert_eq!(slots[0].slot, "single");
    assert_eq!(slots[0].label, "newer");
}
