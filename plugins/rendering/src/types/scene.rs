use crate::entities::{
    EntityInstance, EntityRegistry, MAX_BOXES_PER_ENTITY, MAX_ENTITIES, MAX_ENTITY_BOXES,
};
use crate::style::RenderStyle;
use engine_math::{Angles, Vect};
use plugin_voxel::VoxelWorld;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: Vect,
    pub view: Angles,
    pub hfov: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct LightingParams {
    pub light_level: f32,
    pub intensity: f32,
    pub frame: u32,
    pub style: RenderStyle,
}

#[derive(Clone)]
pub struct Scene<'a> {
    pub world: &'a VoxelWorld,
    pub empty_voxel: u8,
    pub camera: Camera,
    pub lighting: LightingParams,
    pub entities: Vec<EntityInstance>,
    pub entity_registry: &'a EntityRegistry,
}

impl Scene<'_> {
    /// Validate backend-shared entity and box-buffer limits before rendering.
    pub fn validate_entity_limits(&self) -> Result<(), String> {
        if self.entities.len() > MAX_ENTITIES {
            return Err(format!(
                "scene has {} entities; renderer limit is {MAX_ENTITIES}",
                self.entities.len()
            ));
        }
        let mut total_boxes = 0usize;
        for entity in &self.entities {
            let definition = self
                .entity_registry
                .get(entity.kind)
                .ok_or_else(|| format!("unknown entity kind {}", entity.kind.0))?;
            if definition.boxes.len() > MAX_BOXES_PER_ENTITY {
                return Err(format!(
                    "entity kind {} has {} boxes; per-entity limit is {MAX_BOXES_PER_ENTITY}",
                    entity.kind.0,
                    definition.boxes.len()
                ));
            }
            total_boxes = total_boxes.saturating_add(definition.boxes.len());
        }
        if total_boxes > MAX_ENTITY_BOXES {
            return Err(format!(
                "scene has {total_boxes} entity boxes; renderer limit is {MAX_ENTITY_BOXES}"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{EntityBox, EntityDefinition, EntityKind, EntityRenderMode};

    static DEFS: [EntityDefinition; 1] = [EntityDefinition {
        kind: EntityKind(7),
        bounds: None,
        boxes: &[],
        render_mode: EntityRenderMode::Boxes,
        animation: crate::entities::EntityAnimation::None,
        shadow: crate::entities::EntityShadow::None,
    }];
    static OVERSIZED: [EntityBox; MAX_BOXES_PER_ENTITY + 1] = [EntityBox {
        min: [0.0; 3],
        max: [1.0; 3],
        part: 0,
        phase_sway: 0.0,
    }; MAX_BOXES_PER_ENTITY + 1];
    static OVERSIZED_DEFS: [EntityDefinition; 1] = [EntityDefinition {
        boxes: &OVERSIZED,
        ..DEFS[0]
    }];

    fn make_scene<'a>(
        world: &'a VoxelWorld,
        entities: Vec<EntityInstance>,
        registry: &'a EntityRegistry,
    ) -> Scene<'a> {
        Scene {
            world,
            empty_voxel: 0,
            camera: Camera {
                eye: Vect::default(),
                view: Angles::default(),
                hfov: 1.0,
            },
            lighting: LightingParams {
                light_level: 1.0,
                intensity: 1.0,
                frame: 0,
                style: RenderStyle::default(),
            },
            entities,
            entity_registry: registry,
        }
    }

    fn instance() -> EntityInstance {
        EntityInstance {
            kind: EntityKind(7),
            pos: Vect::default(),
            yaw: 0.0,
            params: [0.0; 4],
        }
    }

    #[test]
    fn shared_limits_reject_too_many_entities_and_boxes() {
        let registry = EntityRegistry::new(&DEFS);
        let world = VoxelWorld::new(1, 1, 1, vec![0]);
        let too_many = make_scene(&world, vec![instance(); MAX_ENTITIES + 1], &registry);
        assert!(too_many
            .validate_entity_limits()
            .unwrap_err()
            .contains("entities"));

        let registry = EntityRegistry::new(&OVERSIZED_DEFS);
        let too_many_boxes = make_scene(&world, vec![instance()], &registry);
        assert!(too_many_boxes
            .validate_entity_limits()
            .unwrap_err()
            .contains("per-entity limit"));
    }
}
