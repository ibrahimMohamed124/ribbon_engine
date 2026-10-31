use crate::{HierarchyError, HierarchyExt, Parent};
use ribbon_app::{LogBuffer, LogLevel};
use ribbon_ecs::{Entity, World};
use ribbon_reflect::{TypeRegistry, Value};
use serde::{Deserialize, Serialize};
use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

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

/// A typed external resource referenced by a serialized prefab component.
///
/// The scene crate deliberately stores a stable type name and path rather than an asset handle;
/// runtime handles are owned by the asset service and are reconstructed by registered codecs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrefabAssetDependency {
    pub local_entity_id: u32,
    pub component: String,
    pub asset_type: String,
    pub path: String,
    pub required: bool,
}

impl SceneDoc {
    pub fn to_ron(&self) -> Result<String, SceneError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|error| SceneError::Ron(error.to_string()))
    }

    pub fn from_ron(source: &str) -> Result<Self, SceneError> {
        ron::de::from_str(source).map_err(|error| SceneError::Ron(error.to_string()))
    }

    /// Discover external resources declared by registered component codecs. Unknown component
    /// formats are intentionally ignored here and remain subject to normal instantiate validation.
    pub fn asset_dependencies(
        &self,
        codecs: &PrefabCodecRegistry,
    ) -> Result<Vec<PrefabAssetDependency>, SceneError> {
        codecs.discover_asset_dependencies(&self.prefab)
    }
}

/// Type-keyed services available to prefab codecs without coupling `ribbon_scene` to assets or rendering.
#[derive(Default)]
pub struct PrefabLoadContext {
    extensions: HashMap<TypeId, Box<dyn Any>>,
}

impl PrefabLoadContext {
    pub fn insert<T: Any>(&mut self, value: T) -> Option<T> {
        self.extensions
            .insert(TypeId::of::<T>(), Box::new(value))
            .and_then(|old| old.downcast::<T>().ok().map(|value| *value))
    }
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.extensions
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref())
    }
    pub fn get_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.extensions
            .get_mut(&TypeId::of::<T>())
            .and_then(|value| value.downcast_mut())
    }

    /// Remove and return a typed service from the context. This is useful when a caller temporarily
    /// moves an engine resource into the context for an atomic prefab restore operation.
    pub fn remove<T: Any>(&mut self) -> Option<T> {
        self.extensions
            .remove(&TypeId::of::<T>())
            .and_then(|value| value.downcast::<T>().ok().map(|value| *value))
    }
}

type EncodePrefabComponent =
    dyn Fn(&World, Entity, &PrefabLoadContext) -> Result<Value, SceneError> + Send + Sync;
type DecodePrefabComponent = dyn Fn(&Value, &mut PrefabLoadContext) -> Result<Box<dyn ribbon_reflect::Reflect>, SceneError>
    + Send
    + Sync;
type DiscoverPrefabDependencies =
    dyn Fn(&Value) -> Result<Vec<(String, String, bool)>, SceneError> + Send + Sync;

struct PrefabCodec {
    encode: Arc<EncodePrefabComponent>,
    decode: Arc<DecodePrefabComponent>,
    discover_dependencies: Arc<DiscoverPrefabDependencies>,
}

/// Component-specific asset-reference codecs. Registration is owned by crates that already know
/// both the component type and the asset service; the scene crate remains renderer/asset agnostic.
#[derive(Default)]
pub struct PrefabCodecRegistry {
    codecs: HashMap<String, PrefabCodec>,
}

impl PrefabCodecRegistry {
    pub fn register(
        &mut self,
        component_name: impl Into<String>,
        encode: impl Fn(&World, Entity, &PrefabLoadContext) -> Result<Value, SceneError>
            + Send
            + Sync
            + 'static,
        decode: impl Fn(
                &Value,
                &mut PrefabLoadContext,
            ) -> Result<Box<dyn ribbon_reflect::Reflect>, SceneError>
            + Send
            + Sync
            + 'static,
    ) {
        self.codecs.insert(
            component_name.into(),
            PrefabCodec {
                encode: Arc::new(encode),
                decode: Arc::new(decode),
                discover_dependencies: Arc::new(|_| Ok(Vec::new())),
            },
        );
    }

    /// Register a component codec that also declares its external resource references.
    /// Each tuple is `(stable_asset_type_name, path, required)`; the scene layer never loads it.
    pub fn register_with_dependencies(
        &mut self,
        component_name: impl Into<String>,
        encode: impl Fn(&World, Entity, &PrefabLoadContext) -> Result<Value, SceneError>
            + Send
            + Sync
            + 'static,
        decode: impl Fn(
                &Value,
                &mut PrefabLoadContext,
            ) -> Result<Box<dyn ribbon_reflect::Reflect>, SceneError>
            + Send
            + Sync
            + 'static,
        discover_dependencies: impl Fn(&Value) -> Result<Vec<(String, String, bool)>, SceneError>
            + Send
            + Sync
            + 'static,
    ) {
        self.codecs.insert(
            component_name.into(),
            PrefabCodec {
                encode: Arc::new(encode),
                decode: Arc::new(decode),
                discover_dependencies: Arc::new(discover_dependencies),
            },
        );
    }

    /// Return stable resource references in prefab/entity order. References from different
    /// entities are preserved so the streaming owner can retain per-consumer ownership; the
    /// asset service is responsible for deduplicating the underlying typed load.
    pub fn discover_asset_dependencies(
        &self,
        prefab: &Prefab,
    ) -> Result<Vec<PrefabAssetDependency>, SceneError> {
        if prefab.format_version != 1 {
            return Err(SceneError::UnsupportedVersion(prefab.format_version));
        }
        let mut local_ids = HashSet::new();
        for entity in &prefab.entities {
            if !local_ids.insert(entity.local_id) {
                return Err(SceneError::DuplicateLocalId(entity.local_id));
            }
        }
        for entity in &prefab.entities {
            if let Some(parent) = entity.parent {
                if !local_ids.contains(&parent) {
                    return Err(SceneError::MissingLocalId(parent));
                }
            }
        }
        let mut seen = HashSet::new();
        let mut dependencies = Vec::new();
        for entity in &prefab.entities {
            for (component, value) in &entity.components {
                let Some(codec) = self.codecs.get(component) else {
                    continue;
                };
                for (asset_type, path, required) in (codec.discover_dependencies)(value)? {
                    if asset_type.trim().is_empty() || path.trim().is_empty() {
                        return Err(SceneError::PrefabCodec(format!(
                            "component `{component}` declared an empty asset type or path"
                        )));
                    }
                    let dependency = PrefabAssetDependency {
                        local_entity_id: entity.local_id,
                        component: component.clone(),
                        asset_type,
                        path,
                        required,
                    };
                    let key = (
                        dependency.local_entity_id,
                        dependency.component.clone(),
                        dependency.asset_type.clone(),
                        dependency.path.clone(),
                        dependency.required,
                    );
                    if seen.insert(key) {
                        dependencies.push(dependency);
                    }
                }
            }
        }
        Ok(dependencies)
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
    MissingAsset(String),
    PrefabCodec(String),
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
            Self::MissingAsset(path) => write!(f, "prefab asset `{path}` could not be restored"),
            Self::PrefabCodec(message) => write!(f, "prefab codec failed: {message}"),
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
    instantiate_with_context(
        world,
        registry,
        prefab,
        parent,
        &PrefabCodecRegistry::default(),
        &mut PrefabLoadContext::default(),
    )
}

/// Instantiate a prefab using caller-registered component codecs and typed restore services.
pub fn instantiate_with_context(
    world: &mut World,
    registry: &TypeRegistry,
    prefab: &Prefab,
    parent: Option<Entity>,
    codecs: &PrefabCodecRegistry,
    context: &mut PrefabLoadContext,
) -> Result<Vec<Entity>, SceneError> {
    instantiate_impl(world, registry, prefab, parent, codecs, context, None)
}

/// Instantiate a prefab while warning and skipping unregistered components.
pub fn instantiate_lenient(
    world: &mut World,
    registry: &TypeRegistry,
    prefab: &Prefab,
    parent: Option<Entity>,
    log: &mut LogBuffer,
) -> Result<Vec<Entity>, SceneError> {
    instantiate_impl(
        world,
        registry,
        prefab,
        parent,
        &PrefabCodecRegistry::default(),
        &mut PrefabLoadContext::default(),
        Some(log),
    )
}

fn instantiate_impl(
    world: &mut World,
    registry: &TypeRegistry,
    prefab: &Prefab,
    parent: Option<Entity>,
    codecs: &PrefabCodecRegistry,
    context: &mut PrefabLoadContext,
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
                if registry.type_id(name).is_none() {
                    if let Some(log) = log.as_deref_mut() {
                        log.push(
                            LogLevel::Warn,
                            format!("skipped unknown prefab component `{name}`"),
                        );
                        continue;
                    }
                    return Err(SceneError::UnknownComponent(name.clone()));
                }
                let component = if let Some(codec) = codecs.codecs.get(name) {
                    (codec.decode)(value, context)?
                } else {
                    let mut component = registry
                        .default_value(name)
                        .ok_or_else(|| SceneError::UnknownComponent(name.clone()))?;
                    let Value::Struct(fields) = value else {
                        return Err(SceneError::InvalidComponentValue(name.clone()));
                    };
                    for (field, value) in fields {
                        component.set_field(field, value.clone())?;
                    }
                    component
                };
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
    capture_with_context(
        world,
        registry,
        filter,
        &PrefabCodecRegistry::default(),
        &PrefabLoadContext::default(),
    )
    .expect("capturing without custom prefab codecs cannot fail")
}

/// Capture registered components using custom serializers for asset-backed fields.
pub fn capture_with_context(
    world: &World,
    registry: &TypeRegistry,
    filter: impl Fn(Entity) -> bool,
    codecs: &PrefabCodecRegistry,
    context: &PrefabLoadContext,
) -> Result<SceneDoc, SceneError> {
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
        .map(
            |(index, &entity)| -> Result<crate::PrefabEntity, SceneError> {
                let parent = world
                    .get::<Parent>(entity)
                    .and_then(|parent| ids.get(&parent.0).copied());
                let mut components = Vec::new();
                for &name in &names {
                    if let Some(value) = registry.get(world, entity, name) {
                        if let Some(codec) = codecs.codecs.get(name) {
                            components
                                .push((name.to_owned(), (codec.encode)(world, entity, context)?));
                        } else {
                            let fields = value
                                .fields()
                                .iter()
                                .filter_map(|field| {
                                    value
                                        .get_field(field.name)
                                        .map(|v| (field.name.to_owned(), v))
                                })
                                .collect();
                            components.push((name.to_owned(), Value::Struct(fields)));
                        }
                    }
                }
                Ok(crate::PrefabEntity {
                    local_id: index as u32,
                    parent,
                    components,
                })
            },
        )
        .collect::<Result<Vec<_>, SceneError>>()?;
    Ok(SceneDoc {
        prefab: Prefab {
            format_version: 1,
            name: "Scene".into(),
            entities: records,
        },
        metadata: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        capture, capture_with_context, instantiate, instantiate_lenient, instantiate_with_context,
        Prefab, PrefabCodecRegistry, PrefabEntity, PrefabLoadContext, SceneDoc, SceneError,
    };
    use crate::{Name, Transform};
    use ribbon_app::LogBuffer;
    use ribbon_ecs::World;
    use ribbon_math::Vect;
    use ribbon_reflect::{TypeRegistry, Value};
    use std::collections::HashMap;

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
    #[test]
    fn prefab_codecs_discover_typed_external_resources_without_loading_them() {
        let prefab = Prefab {
            format_version: 1,
            name: "dependency discovery".into(),
            entities: vec![
                super::PrefabEntity {
                    local_id: 4,
                    parent: None,
                    components: vec![(
                        "Name".into(),
                        Value::Struct(vec![(
                            "path".into(),
                            Value::Str("models/shared.glb".into()),
                        )]),
                    )],
                },
                super::PrefabEntity {
                    local_id: 9,
                    parent: None,
                    components: vec![(
                        "Name".into(),
                        Value::Struct(vec![(
                            "path".into(),
                            Value::Str("models/shared.glb".into()),
                        )]),
                    )],
                },
            ],
        };
        let mut codecs = PrefabCodecRegistry::default();
        codecs.register_with_dependencies(
            "Name",
            |_world, _entity, _context| Ok(Value::Struct(Vec::new())),
            |_value, _context| Ok(Box::new(Name(String::new()))),
            |value| {
                let Value::Struct(fields) = value else {
                    return Err(SceneError::InvalidComponentValue("Name".into()));
                };
                let Some((_, Value::Str(path))) = fields.iter().find(|(field, _)| field == "path")
                else {
                    return Err(SceneError::InvalidComponentValue("Name.path".into()));
                };
                Ok(vec![("test::Model".into(), path.clone(), true)])
            },
        );
        let dependencies = codecs.discover_asset_dependencies(&prefab).unwrap();
        assert_eq!(
            dependencies.len(),
            2,
            "each component consumer must remain visible"
        );
        assert_eq!(dependencies[0].local_entity_id, 4);
        assert_eq!(dependencies[1].local_entity_id, 9);
        assert!(dependencies.iter().all(|dependency| {
            dependency.asset_type == "test::Model"
                && dependency.path == "models/shared.glb"
                && dependency.required
        }));
        let doc = SceneDoc {
            prefab,
            metadata: HashMap::new(),
        };
        assert_eq!(doc.asset_dependencies(&codecs).unwrap(), dependencies);
    }

    #[test]
    fn context_codecs_round_trip_asset_like_references_and_rollback_missing_assets() {
        let mut world = World::new();
        world.spawn((Name("asset://models/room.glb".into()),));
        let registry = registry();
        let mut codecs = PrefabCodecRegistry::default();
        codecs.register(
            "Name",
            |world, entity, context| {
                let _marker = context.get::<String>();
                let name = world
                    .get::<Name>(entity)
                    .ok_or_else(|| SceneError::PrefabCodec("Name missing".into()))?;
                Ok(Value::Struct(vec![(
                    "asset_path".into(),
                    Value::Str(name.0.clone()),
                )]))
            },
            |value, context| {
                let _marker = context.get::<String>();
                let Value::Struct(fields) = value else {
                    return Err(SceneError::InvalidComponentValue("Name".into()));
                };
                let Some((_, Value::Str(path))) =
                    fields.iter().find(|(field, _)| field == "asset_path")
                else {
                    return Err(SceneError::InvalidComponentValue("Name.asset_path".into()));
                };
                Ok(Box::new(Name(path.clone())))
            },
        );
        let mut context = PrefabLoadContext::default();
        context.insert("asset codec test context".to_owned());
        let doc = capture_with_context(&world, &registry, |_| true, &codecs, &context).unwrap();
        let encoded = doc.to_ron().unwrap();
        assert!(encoded.contains("asset://models/room.glb"));
        let decoded = SceneDoc::from_ron(&encoded).unwrap();
        let mut restored = World::new();
        let entities = instantiate_with_context(
            &mut restored,
            &registry,
            &decoded.prefab,
            None,
            &codecs,
            &mut context,
        )
        .unwrap();
        assert_eq!(
            restored.get::<Name>(entities[0]).unwrap().0,
            "asset://models/room.glb"
        );

        let mut missing_codecs = PrefabCodecRegistry::default();
        missing_codecs.register(
            "Name",
            |world, entity, _| {
                let name = world
                    .get::<Name>(entity)
                    .ok_or_else(|| SceneError::PrefabCodec("Name missing".into()))?;
                Ok(Value::Struct(vec![(
                    "asset_path".into(),
                    Value::Str(name.0.clone()),
                )]))
            },
            |value, _| {
                let Value::Struct(fields) = value else {
                    return Err(SceneError::InvalidComponentValue("Name".into()));
                };
                let path = fields
                    .iter()
                    .find_map(|(field, value)| {
                        if field == "asset_path" {
                            if let Value::Str(path) = value {
                                Some(path.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                Err(SceneError::MissingAsset(path))
            },
        );
        let mut missing_world = World::new();
        assert!(instantiate_with_context(
            &mut missing_world,
            &registry,
            &decoded.prefab,
            None,
            &missing_codecs,
            &mut context
        )
        .is_err());
        assert_eq!(
            missing_world.len(),
            0,
            "failed restoration must roll back spawned entities"
        );
    }

    #[test]
    fn prefab_load_context_remove_returns_owned_service_and_clears_slot() {
        let mut context = PrefabLoadContext::default();
        assert_eq!(
            context.insert::<String>("temporary service".to_owned()),
            None
        );
        assert_eq!(
            context.remove::<String>().as_deref(),
            Some("temporary service")
        );
        assert!(context.get::<String>().is_none());
        assert!(context.remove::<String>().is_none());
    }
}
