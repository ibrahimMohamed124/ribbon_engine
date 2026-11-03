//! Dynamic per-agent data used by behavior trees.
use glam::Vec3;
use ribbon_ecs::Entity;
use std::collections::HashMap;
#[derive(Clone, Debug, PartialEq)]
pub enum BbValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Vec3(Vec3),
    Entity(Entity),
    Text(String),
}
#[derive(Clone, Debug, Default)]
pub struct Blackboard(HashMap<String, BbValue>);
impl Blackboard {
    pub fn set(&mut self, key: impl Into<String>, value: BbValue) {
        self.0.insert(key.into(), value);
    }
    pub fn get(&self, key: &str) -> Option<&BbValue> {
        self.0.get(key)
    }
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        if let Some(BbValue::Bool(v)) = self.get(key) {
            Some(*v)
        } else {
            None
        }
    }
    pub fn get_int(&self, key: &str) -> Option<i64> {
        if let Some(BbValue::Int(v)) = self.get(key) {
            Some(*v)
        } else {
            None
        }
    }
    pub fn get_float(&self, key: &str) -> Option<f64> {
        if let Some(BbValue::Float(v)) = self.get(key) {
            Some(*v)
        } else {
            None
        }
    }
    pub fn get_vec3(&self, key: &str) -> Option<Vec3> {
        if let Some(BbValue::Vec3(v)) = self.get(key) {
            Some(*v)
        } else {
            None
        }
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}
