//! Slot file operations, CRC validation, backup recovery, and transactional application.
use crate::{
    envelope::{Envelope, SectionRecord},
    section::{ApplyFn, SaveRegistry},
    SaveConfig, SaveError, SaveMeta,
};
use ribbon_app::{LogLevel, Time, World};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
const FORMAT: u32 = 1;
fn valid_slot(slot: &str) -> bool {
    !slot.is_empty()
        && slot.len() <= 32
        && slot
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn root(cfg: &SaveConfig) -> Result<PathBuf, SaveError> {
    let base = if let Some(path) = &cfg.root_override {
        path.clone()
    } else {
        directories::ProjectDirs::from(&cfg.qualifier, &cfg.organization, &cfg.application)
            .ok_or(SaveError::NotConfigured)?
            .data_dir()
            .to_path_buf()
    };
    Ok(base.join("saves"))
}
fn slot_path(world: &World, slot: &str) -> Result<PathBuf, SaveError> {
    if !valid_slot(slot) {
        return Err(SaveError::InvalidSlot(slot.into()));
    }
    Ok(root(world.resource::<SaveConfig>())?.join(format!("{slot}.rsav")))
}
fn parse(path: &Path) -> Result<Envelope, SaveError> {
    let text = fs::read_to_string(path).map_err(|e| SaveError::Io(e.to_string()))?;
    let mut lines = text.splitn(3, '\n');
    if lines.next() != Some("RSAV1") {
        return Err(SaveError::Corrupt("invalid magic".into()));
    }
    let header = lines
        .next()
        .ok_or_else(|| SaveError::Corrupt("missing checksum".into()))?;
    let body = lines
        .next()
        .ok_or_else(|| SaveError::Corrupt("missing body".into()))?;
    let expected = header
        .strip_prefix("crc32:")
        .ok_or_else(|| SaveError::Corrupt("invalid checksum header".into()))?;
    if expected != format!("{:08x}", crc32fast::hash(body.as_bytes())) {
        return Err(SaveError::Crc);
    }
    ron::from_str(body).map_err(|e| SaveError::Ron(e.to_string()))
}
fn parse_with_backup(path: &Path, world: &mut World) -> Result<Envelope, SaveError> {
    let backup = path.with_extension("rsav.bak");
    if !path.exists() {
        if !backup.exists() {
            return Err(SaveError::NoSuchSlot(
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_owned(),
            ));
        }
        world.resource_mut::<ribbon_app::LogBuffer>().push(
            LogLevel::Warn,
            format!(
                "save `{}` is missing; recovering from backup",
                path.display()
            ),
        );
        return parse(&backup);
    }
    match parse(path) {
        Ok(envelope) => Ok(envelope),
        Err(error @ (SaveError::Corrupt(_) | SaveError::Crc | SaveError::Ron(_))) => {
            if backup.exists() {
                world.resource_mut::<ribbon_app::LogBuffer>().push(
                    LogLevel::Warn,
                    format!(
                        "save `{}` failed validation ({error}); retrying backup",
                        path.display()
                    ),
                );
                parse(&backup)
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}
pub fn save_to_slot(world: &World, slot: &str, label: &str) -> Result<SaveMeta, SaveError> {
    let path = slot_path(world, slot)?;
    let cfg = world.resource::<SaveConfig>();
    let sections = world
        .resource::<SaveRegistry>()
        .0
        .iter()
        .map(|s| {
            Ok(SectionRecord {
                key: s.key().into(),
                version: s.version(),
                data: s.capture_ron(world)?,
            })
        })
        .collect::<Result<Vec<_>, SaveError>>()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let play_seconds = world.try_resource::<crate::PlayTime>().map_or(0.0, |p| p.0);
    let meta = SaveMeta {
        slot: slot.into(),
        game_id: cfg.game_id.clone(),
        engine_save_format: FORMAT,
        created_unix_secs: now,
        play_seconds,
        label: label.into(),
    };
    let body = ron::ser::to_string_pretty(
        &Envelope {
            format: FORMAT,
            meta: meta.clone(),
            sections,
        },
        ron::ser::PrettyConfig::default(),
    )
    .map_err(|e| SaveError::Ron(e.to_string()))?;
    let text = format!(
        "RSAV1\ncrc32:{:08x}\n{body}",
        crc32fast::hash(body.as_bytes())
    );
    let parent = path
        .parent()
        .ok_or_else(|| SaveError::Io("save path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension("rsav.tmp");
    let backup = path.with_extension("rsav.bak");
    let written = (|| -> Result<(), SaveError> {
        let mut file = File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if path.exists() {
            if backup.exists() {
                fs::remove_file(&backup)?;
            }
            fs::rename(&path, &backup)?;
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written?;
    Ok(meta)
}
pub fn load_from_slot(world: &mut World, slot: &str) -> Result<SaveMeta, SaveError> {
    let path = slot_path(world, slot)?;
    let envelope = parse_with_backup(&path, world)?;
    if envelope.format != FORMAT {
        return Err(SaveError::UnsupportedEnvelope(envelope.format));
    }
    let expected = world.resource::<SaveConfig>().game_id.clone();
    if envelope.meta.game_id != expected {
        return Err(SaveError::WrongGame {
            expected,
            found: envelope.meta.game_id,
        });
    }
    let registry = world
        .remove_resource::<SaveRegistry>()
        .ok_or(SaveError::NotConfigured)?;
    let result = (|| {
        let mut actions: Vec<ApplyFn> = Vec::with_capacity(registry.0.len());
        for ops in &registry.0 {
            let rec = envelope
                .sections
                .iter()
                .find(|r| r.key == ops.key())
                .ok_or_else(|| SaveError::MissingSection(ops.key().into()))?;
            actions.push(ops.prepare_apply(&rec.data, rec.version)?);
        }
        for rec in &envelope.sections {
            if !registry.0.iter().any(|s| s.key() == rec.key) {
                world.resource_mut::<ribbon_app::LogBuffer>().push(
                    LogLevel::Warn,
                    format!("unknown save section `{}` ignored", rec.key),
                );
            }
        }
        for apply in actions {
            apply(world)?;
        }
        if world.try_resource::<crate::PlayTime>().is_some() {
            world.resource_mut::<crate::PlayTime>().0 = envelope.meta.play_seconds;
        }
        Ok(envelope.meta)
    })();
    world.insert_resource(registry);
    result
}
pub fn list_slots(world: &World) -> Result<Vec<SaveMeta>, SaveError> {
    let dir = root(world.resource::<SaveConfig>())?;
    let mut metas = Vec::new();
    if !dir.exists() {
        return Ok(metas);
    }
    let mut primary_slots = std::collections::HashSet::new();
    let entries = fs::read_dir(&dir)?.collect::<Result<Vec<_>, _>>()?;
    // Prefer a valid primary file. If it exists but is corrupt, expose a valid
    // backup instead of hiding the recoverable slot from the menu.
    for entry in &entries {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "rsav")
        {
            let slot = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned();
            let backup = path.with_extension("rsav.bak");
            match parse(&path) {
                Ok(envelope) if envelope.format == FORMAT => {
                    primary_slots.insert(slot);
                    metas.push(envelope.meta);
                }
                Ok(_) => {
                    primary_slots.insert(slot);
                }
                Err(primary_error) if backup.exists() => match parse(&backup) {
                    Ok(envelope) if envelope.format == FORMAT => {
                        primary_slots.insert(slot);
                        metas.push(envelope.meta);
                    }
                    Ok(_) => {
                        primary_slots.insert(slot);
                    }
                    Err(backup_error) => {
                        primary_slots.insert(slot);
                        eprintln!(
                            "[ribbon_save] skipping unreadable save {} and backup {}: {primary_error}; {backup_error}",
                            path.display(),
                            backup.display()
                        );
                    }
                },
                Err(error) => {
                    primary_slots.insert(slot);
                    eprintln!(
                        "[ribbon_save] skipping unreadable save {}: {error}",
                        path.display()
                    );
                }
            }
        }
    }
    for entry in &entries {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(slot) = name.strip_suffix(".rsav.bak") else {
            continue;
        };
        if primary_slots.contains(slot) {
            continue;
        }
        match parse(&path) {
            Ok(envelope) if envelope.format == FORMAT => metas.push(envelope.meta),
            Ok(_) => {}
            Err(error) => eprintln!(
                "[ribbon_save] skipping unreadable backup {}: {error}",
                path.display()
            ),
        }
    }
    metas.sort_by(|a, b| {
        b.created_unix_secs
            .cmp(&a.created_unix_secs)
            .then_with(|| a.slot.cmp(&b.slot))
    });
    Ok(metas)
}
pub fn delete_slot(world: &World, slot: &str) -> Result<(), SaveError> {
    let path = slot_path(world, slot)?;
    let backup = path.with_extension("rsav.bak");
    if !path.exists() && !backup.exists() {
        return Err(SaveError::NoSuchSlot(slot.into()));
    }
    if path.exists() {
        fs::remove_file(&path)?;
    }
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
}
pub fn slot_exists(world: &World, slot: &str) -> bool {
    slot_path(world, slot)
        .is_ok_and(|path| path.exists() || path.with_extension("rsav.bak").exists())
}
pub(crate) fn advance_play_time(world: &mut World) {
    let dt = world
        .try_resource::<Time>()
        .map_or(0.0, |t| t.delta.as_secs_f64());
    world.resource_mut::<crate::PlayTime>().0 += dt.max(0.0);
}
