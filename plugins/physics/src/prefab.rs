//! Prefab reflection and codecs for renderer-independent Rapier components.
//!
//! Physics data remains serializable without exposing Rapier handles or relying on a live solver.

use crate::{Collider, ColliderShape, CollisionLayers, RigidBody, RigidBodyKind};
use glam::Vec3;
use ribbon_app::World;
use ribbon_ecs::Entity;
use ribbon_reflect::{FieldInfo, Reflect, ReflectError, TypeRegistry, Value, ValueKind};
use ribbon_scene::{PrefabCodecRegistry, SceneError};

const COLLIDER_FIELDS: &[FieldInfo] = &[
    FieldInfo {
        name: "shape",
        kind: ValueKind::Str,
        doc: "Collider primitive: ball, cuboid, capsule_y, or cylinder_y.",
        range: None,
        editable: true,
    },
    FieldInfo {
        name: "radius",
        kind: ValueKind::Float,
        doc: "Radius for ball, capsule_y, and cylinder_y shapes.",
        range: Some((0.0001, 1.0e6)),
        editable: true,
    },
    FieldInfo {
        name: "half_height",
        kind: ValueKind::Float,
        doc: "Half height for capsule_y and cylinder_y shapes.",
        range: Some((0.0, 1.0e6)),
        editable: true,
    },
    FieldInfo {
        name: "half_extents",
        kind: ValueKind::Vec3,
        doc: "Half extents for cuboid colliders.",
        range: None,
        editable: true,
    },
    FieldInfo {
        name: "friction",
        kind: ValueKind::Float,
        doc: "Rapier friction coefficient.",
        range: Some((0.0, 1.0e6)),
        editable: true,
    },
    FieldInfo {
        name: "restitution",
        kind: ValueKind::Float,
        doc: "Bounciness in the inclusive range 0..=1.",
        range: Some((0.0, 1.0)),
        editable: true,
    },
    FieldInfo {
        name: "density",
        kind: ValueKind::Float,
        doc: "Rapier density in kilograms per cubic meter.",
        range: Some((0.0001, 1.0e6)),
        editable: true,
    },
    FieldInfo {
        name: "sensor",
        kind: ValueKind::Bool,
        doc: "Whether the collider reports overlaps without contact response.",
        range: None,
        editable: true,
    },
];

const COLLISION_LAYERS_FIELDS: &[FieldInfo] = &[
    FieldInfo {
        name: "memberships",
        kind: ValueKind::Int,
        doc: "Collision membership bit mask.",
        range: Some((0.0, u32::MAX as f64)),
        editable: true,
    },
    FieldInfo {
        name: "filters",
        kind: ValueKind::Int,
        doc: "Collision filter bit mask.",
        range: Some((0.0, u32::MAX as f64)),
        editable: true,
    },
];

const RIGID_BODY_FIELDS: &[FieldInfo] = &[
    FieldInfo {
        name: "kind",
        kind: ValueKind::Str,
        doc: "fixed, dynamic, kinematic_position_based, or kinematic_velocity_based.",
        range: None,
        editable: true,
    },
    FieldInfo {
        name: "linear_velocity",
        kind: ValueKind::Vec3,
        doc: "Initial linear velocity in meters per second.",
        range: None,
        editable: true,
    },
    FieldInfo {
        name: "angular_velocity",
        kind: ValueKind::Vec3,
        doc: "Initial angular velocity in radians per second.",
        range: None,
        editable: true,
    },
    FieldInfo {
        name: "linear_damping",
        kind: ValueKind::Float,
        doc: "Linear damping coefficient.",
        range: Some((0.0, 1.0e6)),
        editable: true,
    },
    FieldInfo {
        name: "angular_damping",
        kind: ValueKind::Float,
        doc: "Angular damping coefficient.",
        range: Some((0.0, 1.0e6)),
        editable: true,
    },
    FieldInfo {
        name: "ccd_enabled",
        kind: ValueKind::Str,
        doc: "CCD policy: inherit, enabled, or disabled.",
        range: None,
        editable: true,
    },
];

fn type_mismatch(field: &str, expected: ValueKind) -> ReflectError {
    ReflectError::TypeMismatch {
        field: field.to_owned(),
        expected,
    }
}

fn number(value: Value, field: &str) -> Result<f32, ReflectError> {
    let converted = match value {
        Value::Float(value) if value.is_finite() && value.abs() <= f32::MAX as f64 => value as f32,
        Value::Int(value) => value as f32,
        Value::Float(value) => {
            return Err(ReflectError::OutOfRange {
                field: field.into(),
                value: value.to_string(),
            });
        }
        _ => return Err(type_mismatch(field, ValueKind::Float)),
    };
    if converted.is_finite() {
        Ok(converted)
    } else {
        Err(ReflectError::OutOfRange {
            field: field.into(),
            value: converted.to_string(),
        })
    }
}

fn vector(value: Value, field: &str) -> Result<Vec3, ReflectError> {
    match value {
        Value::Vec3(value) if value.iter().all(|value| value.is_finite()) => {
            Ok(Vec3::from_array(value))
        }
        Value::Vec3(value) => Err(ReflectError::OutOfRange {
            field: field.into(),
            value: format!("{value:?}"),
        }),
        _ => Err(type_mismatch(field, ValueKind::Vec3)),
    }
}

impl Reflect for Collider {
    fn type_name(&self) -> &'static str {
        "Collider"
    }

    fn fields(&self) -> &'static [FieldInfo] {
        COLLIDER_FIELDS
    }

    fn get_field(&self, name: &str) -> Option<Value> {
        Some(match name {
            "shape" => Value::Str(
                match self.shape {
                    ColliderShape::Ball { .. } => "ball",
                    ColliderShape::Cuboid { .. } => "cuboid",
                    ColliderShape::CapsuleY { .. } => "capsule_y",
                    ColliderShape::CylinderY { .. } => "cylinder_y",
                }
                .to_owned(),
            ),
            "radius" => Value::Float(match self.shape {
                ColliderShape::Ball { radius }
                | ColliderShape::CapsuleY { radius, .. }
                | ColliderShape::CylinderY { radius, .. } => radius as f64,
                ColliderShape::Cuboid { .. } => 0.0,
            }),
            "half_height" => Value::Float(match self.shape {
                ColliderShape::CapsuleY { half_height, .. }
                | ColliderShape::CylinderY { half_height, .. } => half_height as f64,
                _ => 0.0,
            }),
            "half_extents" => Value::Vec3(match self.shape {
                ColliderShape::Cuboid { half_extents } => half_extents.to_array(),
                _ => [0.0; 3],
            }),
            "friction" => Value::Float(self.friction as f64),
            "restitution" => Value::Float(self.restitution as f64),
            "density" => Value::Float(self.density as f64),
            "sensor" => Value::Bool(self.sensor),
            _ => return None,
        })
    }

    fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError> {
        match name {
            "shape" => {
                let Value::Str(shape) = value else {
                    return Err(type_mismatch(name, ValueKind::Str));
                };
                self.shape = match shape.as_str() {
                    "ball" => ColliderShape::Ball { radius: 0.5 },
                    "cuboid" => ColliderShape::Cuboid {
                        half_extents: Vec3::splat(0.5),
                    },
                    "capsule_y" => ColliderShape::CapsuleY {
                        half_height: 0.5,
                        radius: 0.25,
                    },
                    "cylinder_y" => ColliderShape::CylinderY {
                        half_height: 0.5,
                        radius: 0.25,
                    },
                    _ => {
                        return Err(ReflectError::OutOfRange {
                            field: name.into(),
                            value: shape,
                        });
                    }
                };
            }
            "radius" => {
                let radius = number(value, name)?;
                match &mut self.shape {
                    ColliderShape::Ball { radius: current }
                    | ColliderShape::CapsuleY {
                        radius: current, ..
                    }
                    | ColliderShape::CylinderY {
                        radius: current, ..
                    } => *current = radius,
                    ColliderShape::Cuboid { .. } => {}
                }
            }
            "half_height" => {
                let half_height = number(value, name)?;
                match &mut self.shape {
                    ColliderShape::CapsuleY {
                        half_height: current,
                        ..
                    }
                    | ColliderShape::CylinderY {
                        half_height: current,
                        ..
                    } => *current = half_height,
                    _ => {}
                }
            }
            "half_extents" => {
                let half_extents = vector(value, name)?;
                if let ColliderShape::Cuboid {
                    half_extents: current,
                } = &mut self.shape
                {
                    *current = half_extents;
                }
            }
            "friction" => self.friction = number(value, name)?,
            "restitution" => self.restitution = number(value, name)?,
            "density" => self.density = number(value, name)?,
            "sensor" => match value {
                Value::Bool(value) => self.sensor = value,
                _ => return Err(type_mismatch(name, ValueKind::Bool)),
            },
            _ => return Err(ReflectError::UnknownField(name.into())),
        }
        Ok(())
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

impl Reflect for CollisionLayers {
    fn type_name(&self) -> &'static str {
        "CollisionLayers"
    }

    fn fields(&self) -> &'static [FieldInfo] {
        COLLISION_LAYERS_FIELDS
    }

    fn get_field(&self, name: &str) -> Option<Value> {
        match name {
            "memberships" => Some(Value::Int(i64::from(self.memberships))),
            "filters" => Some(Value::Int(i64::from(self.filters))),
            _ => None,
        }
    }

    fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError> {
        let Value::Int(value) = value else {
            return Err(type_mismatch(name, ValueKind::Int));
        };
        let value = u32::try_from(value).map_err(|_| ReflectError::OutOfRange {
            field: name.into(),
            value: value.to_string(),
        })?;
        match name {
            "memberships" => self.memberships = value,
            "filters" => self.filters = value,
            _ => return Err(ReflectError::UnknownField(name.into())),
        }
        Ok(())
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

impl Reflect for RigidBody {
    fn type_name(&self) -> &'static str {
        "RigidBody"
    }

    fn fields(&self) -> &'static [FieldInfo] {
        RIGID_BODY_FIELDS
    }

    fn get_field(&self, name: &str) -> Option<Value> {
        Some(match name {
            "kind" => Value::Str(
                match self.kind {
                    RigidBodyKind::Fixed => "fixed",
                    RigidBodyKind::Dynamic => "dynamic",
                    RigidBodyKind::KinematicPositionBased => "kinematic_position_based",
                    RigidBodyKind::KinematicVelocityBased => "kinematic_velocity_based",
                }
                .to_owned(),
            ),
            "linear_velocity" => Value::Vec3(self.linear_velocity.to_array()),
            "angular_velocity" => Value::Vec3(self.angular_velocity.to_array()),
            "linear_damping" => Value::Float(self.linear_damping as f64),
            "angular_damping" => Value::Float(self.angular_damping as f64),
            "ccd_enabled" => Value::Str(
                match self.ccd_enabled {
                    None => "inherit",
                    Some(true) => "enabled",
                    Some(false) => "disabled",
                }
                .to_owned(),
            ),
            _ => return None,
        })
    }

    fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError> {
        match name {
            "kind" => {
                let Value::Str(value) = value else {
                    return Err(type_mismatch(name, ValueKind::Str));
                };
                self.kind = match value.as_str() {
                    "fixed" => RigidBodyKind::Fixed,
                    "dynamic" => RigidBodyKind::Dynamic,
                    "kinematic_position_based" => RigidBodyKind::KinematicPositionBased,
                    "kinematic_velocity_based" => RigidBodyKind::KinematicVelocityBased,
                    _ => {
                        return Err(ReflectError::OutOfRange {
                            field: name.into(),
                            value: value.clone(),
                        });
                    }
                };
            }
            "linear_velocity" => self.linear_velocity = vector(value, name)?,
            "angular_velocity" => self.angular_velocity = vector(value, name)?,
            "linear_damping" => self.linear_damping = number(value, name)?,
            "angular_damping" => self.angular_damping = number(value, name)?,
            "ccd_enabled" => {
                let Value::Str(value) = value else {
                    return Err(type_mismatch(name, ValueKind::Str));
                };
                self.ccd_enabled = match value.as_str() {
                    "inherit" => None,
                    "enabled" => Some(true),
                    "disabled" => Some(false),
                    _ => {
                        return Err(ReflectError::OutOfRange {
                            field: name.into(),
                            value: value.clone(),
                        });
                    }
                };
            }
            _ => return Err(ReflectError::UnknownField(name.into())),
        }
        Ok(())
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

fn encode_component(
    world: &World,
    entity: Entity,
    component_name: &str,
) -> Result<Value, SceneError> {
    let component = match component_name {
        "Collider" => world
            .get::<Collider>(entity)
            .map(|value| Box::new(*value) as Box<dyn Reflect>),
        "CollisionLayers" => world
            .get::<CollisionLayers>(entity)
            .map(|value| Box::new(*value) as Box<dyn Reflect>),
        "RigidBody" => world
            .get::<RigidBody>(entity)
            .map(|value| Box::new(*value) as Box<dyn Reflect>),
        _ => None,
    }
    .ok_or_else(|| SceneError::PrefabCodec(format!("{component_name} component is missing")))?;
    let fields = component
        .fields()
        .iter()
        .filter_map(|field| {
            component
                .get_field(field.name)
                .map(|value| (field.name.to_owned(), value))
        })
        .collect();
    Ok(Value::Struct(fields))
}

fn decode_component<T: Reflect + Default>(
    value: &Value,
    name: &str,
) -> Result<Box<dyn Reflect>, SceneError> {
    let Value::Struct(fields) = value else {
        return Err(SceneError::InvalidComponentValue(name.into()));
    };
    let mut component = T::default();
    for (field, value) in fields {
        component.set_field(field, value.clone())?;
    }
    Ok(Box::new(component))
}

/// Register reflection types and prefab codecs for physics components. The codec stores desired
/// shape/body configuration only; live Rapier handles are always reconstructed by PhysicsPlugin.
pub fn register_prefab_components(
    registry: &mut TypeRegistry,
    codecs: &mut PrefabCodecRegistry,
) -> Result<(), ReflectError> {
    registry.register_reflect::<Collider>()?;
    registry.register_reflect::<CollisionLayers>()?;
    registry.register_reflect::<RigidBody>()?;

    codecs.register(
        "Collider",
        |world, entity, _| encode_component(world, entity, "Collider"),
        |value, _| {
            let component = decode_component::<Collider>(value, "Collider")?;
            component
                .as_any()
                .downcast_ref::<Collider>()
                .expect("Collider decoder returns Collider")
                .validate()
                .map_err(|error| {
                    SceneError::PrefabCodec(format!("invalid collider in prefab: {error}"))
                })?;
            Ok(component)
        },
    );
    codecs.register(
        "CollisionLayers",
        |world, entity, _| encode_component(world, entity, "CollisionLayers"),
        |value, _| decode_component::<CollisionLayers>(value, "CollisionLayers"),
    );
    codecs.register(
        "RigidBody",
        |world, entity, _| encode_component(world, entity, "RigidBody"),
        |value, _| {
            let component = decode_component::<RigidBody>(value, "RigidBody")?;
            component
                .as_any()
                .downcast_ref::<RigidBody>()
                .expect("RigidBody decoder returns RigidBody")
                .validate()
                .map_err(|error| {
                    SceneError::PrefabCodec(format!("invalid rigid body in prefab: {error}"))
                })?;
            Ok(component)
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::register_prefab_components;
    use crate::{Collider, CollisionLayers, RigidBody};
    use glam::Vec3;
    use ribbon_app::World;
    use ribbon_reflect::TypeRegistry;
    use ribbon_scene::{
        capture_with_context, instantiate_with_context, PrefabCodecRegistry, PrefabLoadContext,
    };

    #[test]
    fn streamed_physics_components_round_trip_without_rapier_handles() {
        let mut registry = TypeRegistry::default();
        let mut codecs = PrefabCodecRegistry::default();
        register_prefab_components(&mut registry, &mut codecs).unwrap();

        let mut source = World::new();
        let entity = source.spawn((
            Collider::capsule_y(0.7, 0.3),
            CollisionLayers::new(0x3, 0x7),
            RigidBody {
                linear_velocity: Vec3::new(1.0, 2.0, 3.0),
                ccd_enabled: Some(true),
                ..RigidBody::default()
            },
        ));
        let document = capture_with_context(
            &source,
            &registry,
            |_| true,
            &codecs,
            &PrefabLoadContext::default(),
        )
        .unwrap();
        let ron = document.to_ron().unwrap();
        let decoded = ribbon_scene::SceneDoc::from_ron(&ron).unwrap();

        let mut target = World::new();
        let restored = instantiate_with_context(
            &mut target,
            &registry,
            &decoded.prefab,
            None,
            &codecs,
            &mut PrefabLoadContext::default(),
        )
        .unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(
            *target.get::<Collider>(restored[0]).unwrap(),
            Collider::capsule_y(0.7, 0.3)
        );
        assert_eq!(
            *target.get::<CollisionLayers>(restored[0]).unwrap(),
            CollisionLayers::new(0x3, 0x7)
        );
        assert_eq!(
            target
                .get::<RigidBody>(restored[0])
                .unwrap()
                .linear_velocity,
            Vec3::new(1.0, 2.0, 3.0)
        );
        assert_eq!(
            target.get::<RigidBody>(restored[0]).unwrap().ccd_enabled,
            Some(true)
        );
        assert!(source.contains(entity));
    }

    #[test]
    fn invalid_collider_data_is_rejected_during_prefab_restore() {
        let mut registry = TypeRegistry::default();
        let mut codecs = PrefabCodecRegistry::default();
        register_prefab_components(&mut registry, &mut codecs).unwrap();

        let mut source = World::new();
        let entity = source.spawn((Collider::ball(-1.0),));
        let document = capture_with_context(
            &source,
            &registry,
            |_| true,
            &codecs,
            &PrefabLoadContext::default(),
        )
        .unwrap();
        let mut target = World::new();
        let result = instantiate_with_context(
            &mut target,
            &registry,
            &document.prefab,
            None,
            &codecs,
            &mut PrefabLoadContext::default(),
        );
        assert!(result.is_err());
        assert!(target.query::<()>().iter().next().is_none());
        assert!(source.contains(entity));
    }
}
