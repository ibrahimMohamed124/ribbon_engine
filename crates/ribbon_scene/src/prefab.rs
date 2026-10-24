use crate::{HierarchyError, HierarchyExt, Parent};
use ribbon_app::{LogBuffer, LogLevel};
use ribbon_ecs::{Entity, World};
use ribbon_reflect::{TypeRegistry, Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Versioned, entity-id-independent scene/prefab data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prefab {
    pub format_version: u32,
    pub name: String,
    pub entities: Vec<PrefabEntity>,
}

/// One prefab entity and its parent local id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrefabEntity {
    pub local_id: u32,
    pub parent: Option<u32>,
    pub components: Vec<(String, Value)>,
}

/// Prefab plus arbitrary project metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneDoc {
    pub prefab: Prefab,
    pub metadata: HashMap<String, String>,
}

impl SceneDoc {
    pub fn to_ron(&self) -> Result<String, SceneError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|error| SceneError::Ron(error.to_string()))
    }

    pub fn from_ron(source: &str) -> Result<Self, SceneError> {
        ron::de::from_str(source).map_err(|error| SceneError::Ron(error.to_string()))
    }
}

/// Prefab validation, reflection, hierarchy, or serialization failure.
#[derive(Debug)]
pub enum SceneError {
    UnsupportedVersion(u32),
    DuplicateLocalId(u32),
    MissingLocalId(u32),
    UnknownComponent(String),
    InvalidComponentValue(String),
    Reflect(ribbon_reflect::ReflectError),
    Hierarchy(HierarchyError),
    Ron(String),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported prefab format version {version}")
            }
            Self::DuplicateLocalId(id) => write!(f, "prefab repeats local entity id {id}"),
            Self::MissingLocalId(id) => write!(f, "prefab parent local id {id} does not exist"),
            Self::UnknownComponent(name) => write!(f, "unknown prefab component `{name}`"),
            Self::InvalidComponentValue(name) => {
                write!(f, "invalid serialized value for component `{name}`")
            }
            Self::Reflect(error) => error.fmt(f),
            Self::Hierarchy(error) => error.fmt(f),
            Self::Ron(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for SceneError {}
impl From<ribbon_reflect::ReflectError> for SceneError {
    fn from(value: ribbon_reflect::ReflectError) -> Self {
        Self::Reflect(value)
    }
}
impl From<HierarchyError> for SceneError {
    fn from(value: HierarchyError) -> Self {
        Self::Hierarchy(value)
    }
}

/// Instantiate a prefab, rejecting any unregistered component names.
pub fn instantiate(
    world: &mut World,
    registry: &TypeRegistry,
    prefab: &Prefab,
    parent: Option<Entity>,
) -> Result<Vec<Entity>, SceneError> {
    instantiate_impl(world, registry, prefab, parent, None)
}

/// Instantiate a prefab while warning and skipping unregistered components.
pub fn instantiate_lenient(
    world: &mut World,
    registry: &TypeRegistry,
    prefab: &Prefab,
    parent: Option<Entity>,
    log: &mut LogBuffer,
) -> Result<Vec<Entity>, SceneError> {
    instantiate_impl(world, registry, prefab, parent, Some(log))
}

fn instantiate_impl(
    world: &mut World,
    registry: &TypeRegistry,
    prefab: &Prefab,
    parent: Option<Entity>,
    mut log: Option<&mut LogBuffer>,
) -> Result<Vec<Entity>, SceneError> {
    if prefab.format_version != 1 {
        return Err(SceneError::UnsupportedVersion(prefab.format_version));
    }
    if let Some(parent) = parent {
        if !world.contains(parent) {
            return Err(SceneError::Hierarchy(HierarchyError::MissingEntity(parent)));
        }
    }
    let mut ids = HashSet::new();
    for entity in &prefab.entities {
        if !ids.insert(entity.local_id) {
            return Err(SceneError::DuplicateLocalId(entity.local_id));
        }
    }
    for entity in &prefab.entities {
        if let Some(parent) = entity.parent {
            if !ids.contains(&parent) {
                return Err(SceneError::MissingLocalId(parent));
            }
        }
    }

    let spawned: Vec<_> = prefab.entities.iter().map(|_| world.spawn(())).collect();
    let map: HashMap<_, _> = prefab
        .entities
        .iter()
        .zip(&spawned)
        .map(|(record, &entity)| (record.local_id, entity))
        .collect();
    let result = (|| {
        for (record, &entity) in prefab.entities.iter().zip(&spawned) {
            for (name, value) in &record.components {
                let Some(mut component) = registry.default_value(name) else {
                    if let Some(log) = log.as_deref_mut() {
                        log.push(
                            LogLevel::Warn,
                            format!("skipped unknown prefab component `{name}`"),
                        );
                        continue;
                    }
                    return Err(SceneError::UnknownComponent(name.clone()));
                };
                let Value::Struct(fields) = value else {
                    return Err(SceneError::InvalidComponentValue(name.clone()));
                };
                for (field, value) in fields {
                    component.set_field(field, value.clone())?;
                }
                registry.insert(world, entity, component.as_ref())?;
            }
        }
        for (record, &entity) in prefab.entities.iter().zip(&spawned) {
            match record.parent {
                Some(local_parent) => world.set_parent(entity, map[&local_parent])?,
                None => {
                    if let Some(parent) = parent {
                        world.set_parent(entity, parent)?;
                    }
                }
            }
        }
        Ok(spawned.clone())
    })();
    if result.is_err() {
        for entity in spawned {
            let _ = world.despawn_recursive(entity);
        }
    }
    result
}

/// Capture registered components and transform-parent relations for selected entities.
pub fn capture(
    world: &World,
    registry: &TypeRegistry,
    filter: impl Fn(Entity) -> bool,
) -> SceneDoc {
    let mut entities: Vec<_> = world
        .query::<()>()
        .iter()
        .map(|(entity, ())| entity)
        .filter(|&entity| filter(entity))
        .collect();
    entities.sort_by_key(|entity| entity.to_bits());
    let ids: HashMap<_, _> = entities
        .iter()
        .enumerate()
        .map(|(index, &entity)| (entity, index as u32))
        .collect();
    let mut names: Vec<_> = registry.names().collect();
    // Hierarchy is encoded with prefab-local ids below; copying live Entity
    // handles from Parent/Children would defeat remapping on instantiation.
    names.retain(|name| !matches!(*name, "Parent" | "Children"));
    names.sort_unstable();
    let records = entities
        .iter()
        .enumerate()
        .map(|(index, &entity)| {
            let parent = world
                .get::<Parent>(entity)
                .and_then(|parent| ids.get(&parent.0).copied());
            let components = names
                .iter()
                .filter_map(|&name| {
                    registry.get(world, entity, name).map(|value| {
                        let fields = value
                            .fields()
                            .iter()
                            .filter_map(|field| {
                                value
                                    .get_field(field.name)
                                    .map(|v| (field.name.to_owned(), v))
                            })
                            .collect();
                        (name.to_owned(), Value::Struct(fields))
                    })
                })
                .collect();
            crate::PrefabEntity {
                local_id: index as u32,
                parent,
                components,
            }
        })
        .collect();
    SceneDoc {
        prefab: Prefab {
            format_version: 1,
            name: "Scene".into(),
            entities: records,
        },
        metadata: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{capture, instantiate, instantiate_lenient, Prefab, PrefabEntity, SceneDoc};
    use crate::{Name, Transform};
    use ribbon_app::LogBuffer;
    use ribbon_ecs::World;
    use ribbon_math::Vect;
    use ribbon_reflect::{TypeRegistry, Value};

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::default();
        registry.register::<Name>().unwrap();
        registry.register::<Transform>().unwrap();
        registry.register::<crate::GlobalTransform>().unwrap();
        registry
    }

    #[test]
    fn ron_roundtrip_and_value_serde_preserve_scene_data() {
        let mut world = World::new();
        let entity = world.spawn((
            Name("root".into()),
            Transform {
                translation: Vect::new(1.0, 2.0, 3.0),
                ..Transform::default()
            },
        ));
        let registry = registry();
        let doc = capture(&world, &registry, |_| true);
        let encoded = doc.to_ron().unwrap();
        assert_eq!(SceneDoc::from_ron(&encoded).unwrap(), doc);
        let value = Value::Struct(vec![(
            "numbers".into(),
            Value::List(vec![Value::Int(1), Value::Vec2([2.0, 3.0])]),
        )]);
        let encoded = ron::to_string(&value).unwrap();
        assert_eq!(ron::from_str::<Value>(&encoded).unwrap(), value);
        assert!(world.contains(entity));
    }

    #[test]
    fn instantiate_remaps_local_parent_ids_and_handles_unknown_types_by_mode() {
        let prefab = Prefab {
            format_version: 1,
            name: "two entities".into(),
            entities: vec![
                PrefabEntity {
                    local_id: 10,
                    parent: None,
                    components: vec![(
                        "Name".into(),
                        Value::Struct(vec![("value".into(), Value::Str("parent".into()))]),
                    )],
                },
                PrefabEntity {
                    local_id: 40,
                    parent: Some(10),
                    components: vec![("Missing".into(), Value::Struct(vec![]))],
                },
            ],
        };
        let mut world = World::new();
        let registry = registry();
        assert!(instantiate(&mut world, &registry, &prefab, None).is_err());
        assert_eq!(world.len(), 0);
        let mut log = LogBuffer::default();
        let entities = instantiate_lenient(&mut world, &registry, &prefab, None, &mut log).unwrap();
        assert_eq!(
            world.get::<crate::Parent>(entities[1]).unwrap().0,
            entities[0]
        );
        assert_eq!(world.get::<Name>(entities[0]).unwrap().0, "parent");
        assert_eq!(log.iter().count(), 1);
    }
}
