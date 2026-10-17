//! Runtime reflection for ECS components, scene serialization, and editor tooling.
//!
//! Reflection is explicit: only registered component types are dynamically visible.

use engine_ecs::{Component, Entity, World};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

/// Serializable editor-facing value tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    List(Vec<Value>),
    Struct(Vec<(String, Value)>),
}

/// Value category exposed by a reflected field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Bool,
    Int,
    Float,
    Str,
    Vec2,
    Vec3,
    Vec4,
    List,
    Struct,
}

/// Static metadata for an editable field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldInfo {
    pub name: &'static str,
    pub kind: ValueKind,
    pub doc: &'static str,
    pub range: Option<(f64, f64)>,
    pub editable: bool,
}

/// Errors from reflected field access or component registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReflectError {
    UnknownField(String),
    TypeMismatch { field: String, expected: ValueKind },
    OutOfRange { field: String, value: String },
    DuplicateName(String),
    UnregisteredType(String),
    MissingEntity(Entity),
}

impl fmt::Display for ReflectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownField(field) => write!(f, "unknown reflected field `{field}`"),
            Self::TypeMismatch { field, expected } => {
                write!(f, "field `{field}` expects {expected:?}")
            }
            Self::OutOfRange { field, value } => {
                write!(f, "value `{value}` is out of range for `{field}`")
            }
            Self::DuplicateName(name) => {
                write!(f, "reflected type name `{name}` is already registered")
            }
            Self::UnregisteredType(name) => write!(f, "reflected type `{name}` is not registered"),
            Self::MissingEntity(entity) => write!(f, "entity {entity:?} does not exist"),
        }
    }
}

impl std::error::Error for ReflectError {}

/// Runtime field access contract for registered component types.
pub trait Reflect: Any + Send + Sync {
    fn type_name(&self) -> &'static str;
    fn fields(&self) -> &'static [FieldInfo];
    fn get_field(&self, name: &str) -> Option<Value>;
    fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn clone_boxed(&self) -> Box<dyn Reflect>;
}

impl Clone for Box<dyn Reflect> {
    fn clone(&self) -> Self {
        self.clone_boxed()
    }
}

/// Conversion used by `reflect_struct!`; unsupported types fail during compilation.
pub trait ReflectField: Sized + 'static {
    const KIND: ValueKind;
    fn to_value(&self) -> Value;
    fn from_value(field: &str, value: Value) -> Result<Self, ReflectError>;
}

macro_rules! integer_fields {
    ($($t:ty),* $(,)?) => {$ (
        impl ReflectField for $t {
            const KIND: ValueKind = ValueKind::Int;
            fn to_value(&self) -> Value { Value::Int(*self as i64) }
            fn from_value(field: &str, value: Value) -> Result<Self, ReflectError> {
                match value { Value::Int(v) => <$t>::try_from(v).map_err(|_| ReflectError::OutOfRange { field: field.into(), value: v.to_string() }), _ => Err(ReflectError::TypeMismatch { field: field.into(), expected: Self::KIND }) }
            }
        }
    )*};
}
integer_fields!(i8, i16, i32, i64, u8, u16, u32);

macro_rules! scalar_field {
    ($t:ty, $kind:ident, $variant:ident, $to:expr, $from:expr) => {
        impl ReflectField for $t {
            const KIND: ValueKind = ValueKind::$kind;
            fn to_value(&self) -> Value {
                Value::$variant(($to)(*self))
            }
            fn from_value(field: &str, value: Value) -> Result<Self, ReflectError> {
                match value {
                    Value::$variant(v) => Ok(($from)(v)),
                    _ => Err(ReflectError::TypeMismatch {
                        field: field.into(),
                        expected: Self::KIND,
                    }),
                }
            }
        }
    };
}
scalar_field!(bool, Bool, Bool, |v: bool| v, |v: bool| v);
scalar_field!(f32, Float, Float, |v: f32| v as f64, |v: f64| v as f32);
scalar_field!(f64, Float, Float, |v: f64| v, |v: f64| v);

impl ReflectField for String {
    const KIND: ValueKind = ValueKind::Str;
    fn to_value(&self) -> Value {
        Value::Str(self.clone())
    }
    fn from_value(field: &str, value: Value) -> Result<Self, ReflectError> {
        match value {
            Value::Str(v) => Ok(v),
            _ => Err(ReflectError::TypeMismatch {
                field: field.into(),
                expected: Self::KIND,
            }),
        }
    }
}

macro_rules! vector_field {
    ($t:ty, $kind:ident, $variant:ident) => {
        impl ReflectField for $t {
            const KIND: ValueKind = ValueKind::$kind;
            fn to_value(&self) -> Value {
                Value::$variant(*self)
            }
            fn from_value(field: &str, value: Value) -> Result<Self, ReflectError> {
                match value {
                    Value::$variant(v) => Ok(v),
                    _ => Err(ReflectError::TypeMismatch {
                        field: field.into(),
                        expected: Self::KIND,
                    }),
                }
            }
        }
    };
}
vector_field!([f32; 2], Vec2, Vec2);
vector_field!([f32; 3], Vec3, Vec3);
vector_field!([f32; 4], Vec4, Vec4);

/// Generate a `Reflect` implementation for supported primitive and vector fields.
#[macro_export]
macro_rules! reflect_struct {
    ($type:ty { $($field:ident : $field_type:ty),* $(,)? }) => {
        impl $crate::Reflect for $type {
            fn type_name(&self) -> &'static str { stringify!($type) }
            fn fields(&self) -> &'static [$crate::FieldInfo] {
                const FIELDS: &[$crate::FieldInfo] = &[
                    $($crate::FieldInfo { name: stringify!($field), kind: <$field_type as $crate::ReflectField>::KIND, doc: "", range: None, editable: true }),*
                ];
                FIELDS
            }
            fn get_field(&self, name: &str) -> Option<$crate::Value> {
                match name { $(stringify!($field) => Some(<$field_type as $crate::ReflectField>::to_value(&self.$field)),)* _ => None }
            }
            fn set_field(&mut self, name: &str, value: $crate::Value) -> Result<(), $crate::ReflectError> {
                match name { $(stringify!($field) => { self.$field = <$field_type as $crate::ReflectField>::from_value(stringify!($field), value)?; Ok(()) },)* _ => Err($crate::ReflectError::UnknownField(name.into())) }
            }
            fn as_any(&self) -> &dyn std::any::Any { self }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
            fn clone_boxed(&self) -> Box<dyn $crate::Reflect> { Box::new(self.clone()) }
        }
    };
}

type DefaultFn = fn() -> Box<dyn Reflect>;
type InsertFn = fn(&mut World, Entity, &dyn Reflect) -> Result<(), ReflectError>;
type GetFn = fn(&World, Entity) -> Option<Box<dyn Reflect>>;
type RemoveFn = fn(&mut World, Entity) -> Result<(), ReflectError>;

struct Registration {
    type_id: TypeId,
    make_default: DefaultFn,
    insert: InsertFn,
    get: GetFn,
    remove: RemoveFn,
}

/// Registration and dynamic component access by canonical type name or Rust type id.
#[derive(Default)]
pub struct TypeRegistry {
    by_name: HashMap<&'static str, Registration>,
    by_type: HashMap<TypeId, &'static str>,
}

impl TypeRegistry {
    pub fn register<T>(&mut self) -> Result<(), ReflectError>
    where
        T: Component + Reflect + Default + Clone + Serialize + DeserializeOwned,
    {
        let name = T::default().type_name();
        if self
            .by_name
            .get(name)
            .is_some_and(|entry| entry.type_id != TypeId::of::<T>())
        {
            return Err(ReflectError::DuplicateName(name.to_owned()));
        }
        let entry = Registration {
            type_id: TypeId::of::<T>(),
            make_default: || Box::new(T::default()),
            insert: |world, entity, value| {
                let value = value.as_any().downcast_ref::<T>().ok_or_else(|| {
                    ReflectError::TypeMismatch {
                        field: std::any::type_name::<T>().to_owned(),
                        expected: ValueKind::Struct,
                    }
                })?;
                world
                    .insert(entity, value.clone())
                    .map_err(|_| ReflectError::MissingEntity(entity))
            },
            get: |world, entity| {
                world
                    .get::<T>(entity)
                    .map(|value| Box::new((*value).clone()) as Box<dyn Reflect>)
            },
            remove: |world, entity| {
                world
                    .remove::<T>(entity)
                    .map(|_| ())
                    .map_err(|_| ReflectError::MissingEntity(entity))
            },
        };
        self.by_type.insert(entry.type_id, name);
        self.by_name.insert(name, entry);
        Ok(())
    }
    pub fn contains_type<T: 'static>(&self) -> bool {
        self.by_type.contains_key(&TypeId::of::<T>())
    }
    pub fn name_of<T: 'static>(&self) -> Option<&'static str> {
        self.by_type.get(&TypeId::of::<T>()).copied()
    }
    pub fn type_id(&self, name: &str) -> Option<TypeId> {
        self.by_name.get(name).map(|entry| entry.type_id)
    }
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_name.keys().copied()
    }
    pub fn default_value(&self, name: &str) -> Option<Box<dyn Reflect>> {
        self.by_name.get(name).map(|entry| (entry.make_default)())
    }
    pub fn insert(
        &self,
        world: &mut World,
        entity: Entity,
        value: &dyn Reflect,
    ) -> Result<(), ReflectError> {
        let entry = self
            .by_name
            .get(value.type_name())
            .ok_or_else(|| ReflectError::UnregisteredType(value.type_name().into()))?;
        (entry.insert)(world, entity, value)
    }
    pub fn get(&self, world: &World, entity: Entity, name: &str) -> Option<Box<dyn Reflect>> {
        self.by_name
            .get(name)
            .and_then(|entry| (entry.get)(world, entity))
    }
    pub fn remove(
        &self,
        world: &mut World,
        entity: Entity,
        name: &str,
    ) -> Result<(), ReflectError> {
        let entry = self
            .by_name
            .get(name)
            .ok_or_else(|| ReflectError::UnregisteredType(name.into()))?;
        (entry.remove)(world, entity)
    }
}

#[cfg(test)]
mod tests {
    use super::{Reflect, ReflectError, TypeRegistry, Value};
    use engine_ecs::World;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Default, Serialize, Deserialize)]
    struct Demo {
        enabled: bool,
        count: i32,
        position: [f32; 3],
    }
    crate::reflect_struct!(Demo {
        enabled: bool,
        count: i32,
        position: [f32; 3]
    });

    #[test]
    fn fields_roundtrip_and_reject_bad_field_values() {
        let mut value = Demo::default();
        value.set_field("count", Value::Int(17)).unwrap();
        assert_eq!(value.get_field("count"), Some(Value::Int(17)));
        assert!(matches!(
            value.set_field("count", Value::Str("bad".into())),
            Err(ReflectError::TypeMismatch { .. })
        ));
        assert!(matches!(
            value.set_field("missing", Value::Bool(true)),
            Err(ReflectError::UnknownField(_))
        ));
    }

    #[test]
    fn registry_inserts_and_gets_component_values() {
        let mut registry = TypeRegistry::default();
        registry.register::<Demo>().unwrap();
        let mut world = World::new();
        let entity = world.spawn(());
        registry
            .insert(
                &mut world,
                entity,
                &Demo {
                    count: 4,
                    ..Demo::default()
                },
            )
            .unwrap();
        assert_eq!(
            registry
                .get(&world, entity, "Demo")
                .unwrap()
                .get_field("count"),
            Some(Value::Int(4))
        );
    }
}
