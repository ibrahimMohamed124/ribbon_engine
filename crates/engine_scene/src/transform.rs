use engine_ecs::Entity;
use engine_math::{Angles, Vect};
use engine_reflect::{FieldInfo, Reflect, ReflectError, Value, ValueKind};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Human-readable entity label.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Name(pub String);

impl Reflect for Name {
    fn type_name(&self) -> &'static str {
        "Name"
    }
    fn fields(&self) -> &'static [FieldInfo] {
        &NAME_FIELDS
    }
    fn get_field(&self, field: &str) -> Option<Value> {
        (field == "value").then(|| Value::Str(self.0.clone()))
    }
    fn set_field(&mut self, field: &str, value: Value) -> Result<(), ReflectError> {
        match (field, value) {
            ("value", Value::Str(value)) => {
                self.0 = value;
                Ok(())
            }
            ("value", _) => Err(ReflectError::TypeMismatch {
                field: field.into(),
                expected: ValueKind::Str,
            }),
            _ => Err(ReflectError::UnknownField(field.into())),
        }
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

const NAME_FIELDS: [FieldInfo; 1] = [FieldInfo {
    name: "value",
    kind: ValueKind::Str,
    doc: "Entity name",
    range: None,
    editable: true,
}];

/// Transform relative to an entity's parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vect,
    pub rotation: Angles,
    pub scale: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vect::default(),
            rotation: Angles::default(),
            scale: 1.0,
        }
    }
}

/// Transform composed from the full parent chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobalTransform {
    pub translation: Vect,
    pub rotation: Angles,
    pub scale: f32,
}

impl Default for GlobalTransform {
    fn default() -> Self {
        Self {
            translation: Vect::default(),
            rotation: Angles::default(),
            scale: 1.0,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct TransformData {
    translation: [f32; 3],
    rotation: [f32; 2],
    scale: f32,
}

macro_rules! impl_transform_serde {
    ($type:ty) => {
        impl Serialize for $type {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                TransformData {
                    translation: [self.translation.x, self.translation.y, self.translation.z],
                    rotation: [self.rotation.psi, self.rotation.phi],
                    scale: self.scale,
                }
                .serialize(serializer)
            }
        }
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = TransformData::deserialize(deserializer)?;
                Ok(Self {
                    translation: Vect::new(
                        value.translation[0],
                        value.translation[1],
                        value.translation[2],
                    ),
                    rotation: Angles {
                        psi: value.rotation[0],
                        phi: value.rotation[1],
                    },
                    scale: value.scale,
                })
            }
        }
    };
}
impl_transform_serde!(Transform);
impl_transform_serde!(GlobalTransform);

const TRANSFORM_FIELDS: &[FieldInfo] = &[
    FieldInfo {
        name: "translation",
        kind: ValueKind::Vec3,
        doc: "Local translation",
        range: None,
        editable: true,
    },
    FieldInfo {
        name: "rotation",
        kind: ValueKind::Vec2,
        doc: "Local pitch and yaw in radians",
        range: None,
        editable: true,
    },
    FieldInfo {
        name: "scale",
        kind: ValueKind::Float,
        doc: "Uniform local scale",
        range: Some((0.0, 1000.0)),
        editable: true,
    },
];

macro_rules! impl_transform_reflect {
    ($type:ty) => {
        impl Reflect for $type {
            fn type_name(&self) -> &'static str {
                stringify!($type)
            }
            fn fields(&self) -> &'static [FieldInfo] {
                TRANSFORM_FIELDS
            }
            fn get_field(&self, name: &str) -> Option<Value> {
                match name {
                    "translation" => Some(Value::Vec3([
                        self.translation.x,
                        self.translation.y,
                        self.translation.z,
                    ])),
                    "rotation" => Some(Value::Vec2([self.rotation.psi, self.rotation.phi])),
                    "scale" => Some(Value::Float(self.scale as f64)),
                    _ => None,
                }
            }
            fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError> {
                match (name, value) {
                    ("translation", Value::Vec3(v)) => {
                        self.translation = Vect::new(v[0], v[1], v[2])
                    }
                    ("rotation", Value::Vec2(v)) => {
                        self.rotation = Angles {
                            psi: v[0],
                            phi: v[1],
                        }
                    }
                    ("scale", Value::Float(v)) => self.scale = v as f32,
                    ("translation", _) => {
                        return Err(ReflectError::TypeMismatch {
                            field: name.into(),
                            expected: ValueKind::Vec3,
                        })
                    }
                    ("rotation", _) => {
                        return Err(ReflectError::TypeMismatch {
                            field: name.into(),
                            expected: ValueKind::Vec2,
                        })
                    }
                    ("scale", _) => {
                        return Err(ReflectError::TypeMismatch {
                            field: name.into(),
                            expected: ValueKind::Float,
                        })
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
    };
}
impl_transform_reflect!(Transform);
impl_transform_reflect!(GlobalTransform);

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
