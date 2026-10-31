//! Reflection and asset-path codecs for render3d prefab components.
use crate::components::{
    Camera3D, FogSettings, MeshInstance, ModelInstance, PointLight, SpotLight, SunLight,
};
use ribbon_app::World;
use ribbon_assets::AssetServer;
use ribbon_ecs::Entity;
use ribbon_plugin_gltf::ModelAsset;
use ribbon_reflect::{FieldInfo, Reflect, ReflectError, TypeRegistry, Value, ValueKind};
use ribbon_scene::{PrefabCodecRegistry, PrefabLoadContext, SceneError};

const MODEL_INSTANCE_FIELDS: &[FieldInfo] = &[FieldInfo {
    name: "model",
    kind: ValueKind::Str,
    doc: "Asset path; the runtime handle is restored by the registered asset codec.",
    range: None,
    editable: true,
}];

impl Reflect for ModelInstance {
    fn type_name(&self) -> &'static str {
        "ModelInstance"
    }
    fn fields(&self) -> &'static [FieldInfo] {
        MODEL_INSTANCE_FIELDS
    }
    fn get_field(&self, name: &str) -> Option<Value> {
        (name == "model").then(|| Value::Str(String::new()))
    }
    fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError> {
        match (name, value) {
            ("model", Value::Str(path)) => Err(ReflectError::OutOfRange {
                field: name.into(),
                value: format!("{path} requires an AssetServer prefab codec"),
            }),
            ("model", _) => Err(ReflectError::TypeMismatch {
                field: name.into(),
                expected: ValueKind::Str,
            }),
            _ => Err(ReflectError::UnknownField(name.into())),
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_boxed(&self) -> Box<dyn Reflect> {
        Box::new(*self)
    }
}

/// Register the render components used by mesh scenes and the path-based model asset codec.
/// The caller inserts an `AssetServer` into `PrefabLoadContext` before capture/instantiation.
pub fn register_prefab_components(
    registry: &mut TypeRegistry,
    codecs: &mut PrefabCodecRegistry,
) -> Result<(), ReflectError> {
    registry.register_reflect::<Camera3D>()?;
    registry.register_reflect::<MeshInstance>()?;
    registry.register_reflect::<ModelInstance>()?;
    registry.register_reflect::<SunLight>()?;
    registry.register_reflect::<SpotLight>()?;
    registry.register_reflect::<PointLight>()?;
    registry.register_reflect::<FogSettings>()?;

    codecs.register(
        "ModelInstance",
        |world: &World, entity: Entity, context: &PrefabLoadContext| {
            let instance = world.get::<ModelInstance>(entity).ok_or_else(|| {
                SceneError::PrefabCodec("ModelInstance component is missing".into())
            })?;
            let server = context.get::<AssetServer>().ok_or_else(|| {
                SceneError::PrefabCodec("PrefabLoadContext does not contain AssetServer".into())
            })?;
            let path = server.path(instance.model).ok_or_else(|| {
                SceneError::MissingAsset(format!(
                    "handle:{}:{}",
                    instance.model.id(),
                    instance.model.generation()
                ))
            })?;
            Ok(Value::Struct(vec![(
                "model".into(),
                Value::Str(path.to_owned()),
            )]))
        },
        |value: &Value, context: &mut PrefabLoadContext| {
            let Value::Struct(fields) = value else {
                return Err(SceneError::InvalidComponentValue("ModelInstance".into()));
            };
            let Some((_, Value::Str(path))) = fields.iter().find(|(name, _)| name == "model")
            else {
                return Err(SceneError::InvalidComponentValue(
                    "ModelInstance.model".into(),
                ));
            };
            let server = context.get_mut::<AssetServer>().ok_or_else(|| {
                SceneError::PrefabCodec(
                    "PrefabLoadContext does not contain mutable AssetServer".into(),
                )
            })?;
            let handle = server
                .load::<ModelAsset>(path)
                .map_err(|_| SceneError::MissingAsset(path.clone()))?;
            Ok(Box::new(ModelInstance { model: handle }) as Box<dyn Reflect>)
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::register_prefab_components;
    use ribbon_app::World;
    use ribbon_assets::AssetServer;
    use ribbon_reflect::{TypeRegistry, Value};
    use ribbon_scene::{
        instantiate_with_context, Prefab, PrefabCodecRegistry, PrefabEntity, PrefabLoadContext,
        SceneError,
    };

    struct EmptyModelLoader;
    impl ribbon_assets::AssetLoader for EmptyModelLoader {
        type Asset = ribbon_plugin_gltf::ModelAsset;
        fn extensions(&self) -> &[&str] {
            &["glb"]
        }
        fn load(
            &self,
            _bytes: &[u8],
            _context: &mut ribbon_assets::LoadContext,
        ) -> Result<Self::Asset, ribbon_assets::LoadError> {
            Ok(ribbon_plugin_gltf::ModelAsset::default())
        }
    }

    #[test]
    fn model_instance_round_trips_as_a_path_not_a_runtime_handle() {
        let mut registry = TypeRegistry::default();
        let mut codecs = PrefabCodecRegistry::default();
        register_prefab_components(&mut registry, &mut codecs).unwrap();
        registry
            .register_reflect::<ribbon_scene::Transform3D>()
            .unwrap();
        let source = ribbon_assets::MemorySource::default();
        source.insert("models/room.glb", vec![0_u8]);
        let mut server = AssetServer::default();
        server.register_loader(EmptyModelLoader);
        server.add_source(0, source.clone());
        let handle = server
            .load::<ribbon_plugin_gltf::ModelAsset>("models/room.glb")
            .unwrap();
        let mut context = PrefabLoadContext::default();
        context.insert(server);
        let transform = ribbon_scene::Transform3D {
            translation: glam::Vec3::new(1.0, 2.0, 3.0),
            rotation: glam::Quat::IDENTITY,
            scale: glam::Vec3::new(1.0, 2.0, 1.0),
        };
        let light = super::SpotLight {
            position: glam::Vec3::new(2.0, 3.0, 4.0),
            direction: glam::Vec3::new(0.0, -1.0, 0.0),
            color: glam::Vec3::new(0.8, 0.9, 1.0),
            intensity: 12.0,
            range: 18.0,
            inner_cone_angle: 0.2,
            outer_cone_angle: 0.5,
            casts_shadows: true,
        };
        let mut world = World::new();
        let source_entity = world.spawn((transform, light, super::ModelInstance { model: handle }));
        let doc =
            ribbon_scene::capture_with_context(&world, &registry, |_| true, &codecs, &context)
                .unwrap();
        let ron = doc.to_ron().unwrap();
        assert!(ron.contains("models/room.glb"));
        assert!(!ron.contains(&format!("handle:{}", handle.id())));
        let decoded = ribbon_scene::SceneDoc::from_ron(&ron).unwrap();

        // Restore into a fresh world with a separate asset server to prove that the serialized
        // path, rather than the original runtime handle, is enough to resolve the model.
        let mut restore_server = AssetServer::default();
        restore_server.register_loader(EmptyModelLoader);
        restore_server.add_source(0, source);
        let mut restore_context = PrefabLoadContext::default();
        restore_context.insert(restore_server);
        let mut restored = World::new();
        let entities = instantiate_with_context(
            &mut restored,
            &registry,
            &decoded.prefab,
            None,
            &codecs,
            &mut restore_context,
        )
        .unwrap();
        let restored_entity = entities[0];
        let restored_handle = restored
            .get::<super::ModelInstance>(restored_entity)
            .unwrap()
            .model;
        let restored_server = restore_context.get::<AssetServer>().unwrap();
        assert_eq!(
            restored_server.path(restored_handle),
            Some("models/room.glb")
        );
        assert!(restored_server.get(restored_handle).is_some());
        assert_eq!(
            *restored
                .get::<ribbon_scene::Transform3D>(restored_entity)
                .unwrap(),
            transform
        );
        assert_eq!(
            *restored.get::<super::SpotLight>(restored_entity).unwrap(),
            light
        );
        assert!(world.contains(source_entity));
    }

    #[test]
    fn missing_model_asset_is_reported_and_partial_prefab_is_rolled_back() {
        let mut registry = TypeRegistry::default();
        let mut codecs = PrefabCodecRegistry::default();
        register_prefab_components(&mut registry, &mut codecs).unwrap();
        let prefab = Prefab {
            format_version: 1,
            name: "missing-model".into(),
            entities: vec![PrefabEntity {
                local_id: 0,
                parent: None,
                components: vec![(
                    "ModelInstance".into(),
                    Value::Struct(vec![(
                        "model".into(),
                        Value::Str("missing/room.glb".into()),
                    )]),
                )],
            }],
        };
        let mut context = PrefabLoadContext::default();
        context.insert(AssetServer::default());
        let mut world = World::new();
        let error =
            instantiate_with_context(&mut world, &registry, &prefab, None, &codecs, &mut context)
                .unwrap_err();
        assert!(matches!(error, SceneError::MissingAsset(path) if path == "missing/room.glb"));
        assert_eq!(world.len(), 0);
    }
}
