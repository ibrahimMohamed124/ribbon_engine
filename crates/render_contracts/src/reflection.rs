//! Reflection metadata for the renderer-neutral component contracts.
//!
//! These implementations live beside the types to satisfy Rust's orphan rules while keeping
//! the contract crate independent from any renderer, asset server, window system, or GPU API.

use crate::{BuiltinMesh, Camera3D, FogSettings, MeshInstance, PointLight, SpotLight, SunLight};
use glam::Vec3;
use ribbon_reflect::{FieldInfo, Reflect, ReflectError, Value, ValueKind};

trait PrefabField: Sized + Copy {
    const KIND: ValueKind;
    fn to_value(&self) -> Value;
    fn from_value(field: &str, value: Value) -> Result<Self, ReflectError>;
}

impl PrefabField for f32 {
    const KIND: ValueKind = ValueKind::Float;
    fn to_value(&self) -> Value {
        Value::Float(f64::from(*self))
    }
    fn from_value(field: &str, value: Value) -> Result<Self, ReflectError> {
        match value {
            Value::Float(value) if value.is_finite() && value.abs() <= f64::from(f32::MAX) => {
                Ok(value as f32)
            }
            _ => Err(ReflectError::TypeMismatch {
                field: field.into(),
                expected: Self::KIND,
            }),
        }
    }
}

impl PrefabField for bool {
    const KIND: ValueKind = ValueKind::Bool;
    fn to_value(&self) -> Value {
        Value::Bool(*self)
    }
    fn from_value(field: &str, value: Value) -> Result<Self, ReflectError> {
        if let Value::Bool(value) = value {
            Ok(value)
        } else {
            Err(ReflectError::TypeMismatch {
                field: field.into(),
                expected: Self::KIND,
            })
        }
    }
}

impl PrefabField for [f32; 3] {
    const KIND: ValueKind = ValueKind::Vec3;
    fn to_value(&self) -> Value {
        Value::Vec3(*self)
    }
    fn from_value(field: &str, value: Value) -> Result<Self, ReflectError> {
        if let Value::Vec3(value) = value {
            Ok(value)
        } else {
            Err(ReflectError::TypeMismatch {
                field: field.into(),
                expected: Self::KIND,
            })
        }
    }
}

impl PrefabField for Vec3 {
    const KIND: ValueKind = ValueKind::Vec3;
    fn to_value(&self) -> Value {
        Value::Vec3(self.to_array())
    }
    fn from_value(field: &str, value: Value) -> Result<Self, ReflectError> {
        if let Value::Vec3(value) = value {
            Ok(Vec3::from_array(value))
        } else {
            Err(ReflectError::TypeMismatch {
                field: field.into(),
                expected: Self::KIND,
            })
        }
    }
}

impl PrefabField for BuiltinMesh {
    const KIND: ValueKind = ValueKind::Str;
    fn to_value(&self) -> Value {
        Value::Str(
            match self {
                BuiltinMesh::Cube => "Cube",
            }
            .into(),
        )
    }
    fn from_value(field: &str, value: Value) -> Result<Self, ReflectError> {
        match value {
            Value::Str(value) if value == "Cube" => Ok(BuiltinMesh::Cube),
            Value::Str(value) => Err(ReflectError::OutOfRange {
                field: field.into(),
                value,
            }),
            _ => Err(ReflectError::TypeMismatch {
                field: field.into(),
                expected: Self::KIND,
            }),
        }
    }
}

macro_rules! impl_reflect_component {
    ($ty:ty, $name:literal, { $($field:ident : $field_ty:ty),+ $(,)? }) => {
        impl Reflect for $ty {
            fn type_name(&self) -> &'static str { $name }
            fn fields(&self) -> &'static [FieldInfo] {
                const FIELDS: &[FieldInfo] = &[$(FieldInfo { name: stringify!($field), kind: <$field_ty as PrefabField>::KIND, doc: stringify!($field), range: None, editable: true }),+];
                FIELDS
            }
            fn get_field(&self, name: &str) -> Option<Value> {
                match name { $(stringify!($field) => Some(PrefabField::to_value(&self.$field)),)+ _ => None }
            }
            fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError> {
                match name { $(stringify!($field) => { self.$field = <$field_ty as PrefabField>::from_value(name, value)?; Ok(()) },)+ _ => Err(ReflectError::UnknownField(name.into())) }
            }
            fn as_any(&self) -> &dyn std::any::Any { self }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
            fn clone_boxed(&self) -> Box<dyn Reflect> { Box::new(*self) }
        }
    };
}

impl_reflect_component!(Camera3D, "Camera3D", { fov_y: f32, near: f32, far: f32 });
impl_reflect_component!(MeshInstance, "MeshInstance", { mesh: BuiltinMesh, color: [f32; 3] });
impl_reflect_component!(SunLight, "SunLight", { direction: Vec3, color: Vec3, ambient: Vec3 });
impl_reflect_component!(SpotLight, "SpotLight", { position: Vec3, direction: Vec3, color: Vec3, intensity: f32, range: f32, inner_cone_angle: f32, outer_cone_angle: f32, casts_shadows: bool });
impl_reflect_component!(PointLight, "PointLight", { position: Vec3, color: Vec3, intensity: f32, range: f32 });
impl_reflect_component!(FogSettings, "FogSettings", { color: Vec3, density: f32, base_height: f32, height_falloff: f32 });
