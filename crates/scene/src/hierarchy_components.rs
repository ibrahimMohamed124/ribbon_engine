//! Hierarchy relationship components and their reflection/serialization contracts.

use ribbon_ecs::Entity;
use ribbon_reflect::{FieldInfo, Reflect, ReflectError, Value, ValueKind};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Parent entity in a transform hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parent(pub Entity);

impl Default for Parent {
    fn default() -> Self {
        Self(Entity::DANGLING)
    }
}

impl Serialize for Parent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0.to_bits().get())
    }
}

impl<'de> Deserialize<'de> for Parent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Entity::from_bits(u64::deserialize(deserializer)?)
            .map(Self)
            .ok_or_else(|| serde::de::Error::custom("invalid entity bits"))
    }
}

/// Direct children in stable insertion order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Children(pub Vec<Entity>);

impl Serialize for Children {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0
            .iter()
            .map(|entity| entity.to_bits().get())
            .collect::<Vec<_>>()
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Children {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<u64>::deserialize(deserializer)?
            .into_iter()
            .map(|bits| {
                Entity::from_bits(bits)
                    .ok_or_else(|| serde::de::Error::custom("invalid entity bits"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }
}

const PARENT_FIELDS: &[FieldInfo] = &[FieldInfo {
    name: "entity",
    kind: ValueKind::Str,
    doc: "Parent entity handle",
    range: None,
    editable: false,
}];
const CHILDREN_FIELDS: &[FieldInfo] = &[FieldInfo {
    name: "entities",
    kind: ValueKind::List,
    doc: "Child entity handles",
    range: None,
    editable: false,
}];

impl Reflect for Parent {
    fn type_name(&self) -> &'static str {
        "Parent"
    }
    fn fields(&self) -> &'static [FieldInfo] {
        PARENT_FIELDS
    }
    fn get_field(&self, name: &str) -> Option<Value> {
        (name == "entity").then(|| Value::Str(format!("{:016x}", self.0.to_bits().get())))
    }
    fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError> {
        match (name, value) {
            ("entity", Value::Str(value)) => {
                let bits =
                    u64::from_str_radix(&value, 16).map_err(|_| ReflectError::TypeMismatch {
                        field: name.into(),
                        expected: ValueKind::Str,
                    })?;
                self.0 = Entity::from_bits(bits).ok_or_else(|| ReflectError::OutOfRange {
                    field: name.into(),
                    value,
                })?;
                Ok(())
            }
            ("entity", _) => Err(ReflectError::TypeMismatch {
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

impl Reflect for Children {
    fn type_name(&self) -> &'static str {
        "Children"
    }
    fn fields(&self) -> &'static [FieldInfo] {
        CHILDREN_FIELDS
    }
    fn get_field(&self, name: &str) -> Option<Value> {
        (name == "entities").then(|| {
            Value::List(
                self.0
                    .iter()
                    .map(|entity| Value::Str(format!("{:016x}", entity.to_bits().get())))
                    .collect(),
            )
        })
    }
    fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError> {
        if name != "entities" {
            return Err(ReflectError::UnknownField(name.into()));
        }
        let Value::List(values) = value else {
            return Err(ReflectError::TypeMismatch {
                field: name.into(),
                expected: ValueKind::List,
            });
        };
        self.0 = values
            .into_iter()
            .map(|value| {
                let Value::Str(value) = value else {
                    return Err(ReflectError::TypeMismatch {
                        field: name.into(),
                        expected: ValueKind::Str,
                    });
                };
                let bits =
                    u64::from_str_radix(&value, 16).map_err(|_| ReflectError::TypeMismatch {
                        field: name.into(),
                        expected: ValueKind::Str,
                    })?;
                Entity::from_bits(bits).ok_or_else(|| ReflectError::OutOfRange {
                    field: name.into(),
                    value,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_boxed(&self) -> Box<dyn Reflect> {
        Box::new(self.clone())
    }
}
