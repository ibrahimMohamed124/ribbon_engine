use crate::entities::{EntityInstance, EntityRegistry};
use crate::style::RenderStyle;
use engine_core::math::{Angles, Vect};
use engine_core::voxel::VoxelWorld;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: Vect,
    pub view: Angles,
    pub hfov: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct LightingParams {
    pub illumination: f32,
    pub intensity: f32,
    pub frame: u32,
    pub style: RenderStyle,
}

pub struct Scene<'a> {
    pub world: &'a VoxelWorld,
    pub empty_voxel: u8,
    pub camera: Camera,
    pub lighting: LightingParams,
    pub entities: Vec<EntityInstance>,
    pub entity_registry: &'a EntityRegistry,
}
