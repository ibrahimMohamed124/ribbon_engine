use engine_scene::entities::{
    EntityAnimation, EntityBox, EntityDefinition, EntityKind, EntityRegistry, EntityRenderMode,
    EntityShadow,
};

pub const WARDEN: EntityKind = EntityKind(1);
pub const PAGE: EntityKind = EntityKind(2);
pub const PREVIEW_WARDEN: EntityKind = EntityKind(3);
pub const ENEMY_BOUNDS: ([f32; 3], [f32; 3]) = ([-0.5, -0.75, 0.0], [1.0, 0.75, 2.25]);

const WARDEN_BOXES: [EntityBox; 6] = [
    EntityBox {
        min: [-0.15, -0.35, 0.0],
        max: [0.15, -0.05, 0.9],
        part: 0,
        phase_sway: 1.0,
    },
    EntityBox {
        min: [-0.15, 0.05, 0.0],
        max: [0.15, 0.35, 0.9],
        part: 0,
        phase_sway: -1.0,
    },
    EntityBox {
        min: [-0.2, -0.4, 0.9],
        max: [0.2, 0.4, 1.7],
        part: 0,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [-0.1, -0.65, 1.35],
        max: [0.85, -0.4, 1.7],
        part: 0,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [-0.1, 0.4, 1.35],
        max: [0.85, 0.65, 1.7],
        part: 0,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [-0.25, -0.25, 1.7],
        max: [0.25, 0.25, 2.2],
        part: 1,
        phase_sway: 0.0,
    },
];
const PAGE_BOXES: [EntityBox; 1] = [EntityBox {
    min: [-0.2, -0.015, -0.25],
    max: [0.2, 0.015, 0.25],
    part: 0,
    phase_sway: 0.0,
}];
const PREVIEW_BOXES: [EntityBox; 4] = [
    EntityBox {
        min: [-0.36, -0.36, 0.0],
        max: [0.30, 0.36, 1.8],
        part: 0,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [-0.30, -0.30, 1.55],
        max: [0.30, 0.30, 2.1],
        part: 0,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [0.30, -0.20, 1.82],
        max: [0.36, -0.06, 1.92],
        part: 1,
        phase_sway: 0.0,
    },
    EntityBox {
        min: [0.30, 0.06, 1.82],
        max: [0.36, 0.20, 1.92],
        part: 1,
        phase_sway: 0.0,
    },
];

const DEFINITIONS: [EntityDefinition; 3] = [
    EntityDefinition {
        kind: WARDEN,
        bounds: Some(ENEMY_BOUNDS),
        boxes: &WARDEN_BOXES,
        render_mode: EntityRenderMode::Billboard {
            atlas_slot: 0,
            width: 0.92,
            height: 2.2,
        },
        animation: EntityAnimation::None,
        shadow: EntityShadow::Ellipse { rx: 0.62, ry: 0.34 },
    },
    EntityDefinition {
        kind: PAGE,
        bounds: None,
        boxes: &PAGE_BOXES,
        render_mode: EntityRenderMode::Boxes,
        animation: EntityAnimation::BobSpin {
            amplitude: 0.08,
            speed: 2.0,
            spin: 1.2,
        },
        shadow: EntityShadow::None,
    },
    EntityDefinition {
        kind: PREVIEW_WARDEN,
        bounds: None,
        boxes: &PREVIEW_BOXES,
        render_mode: EntityRenderMode::Boxes,
        animation: EntityAnimation::None,
        shadow: EntityShadow::None,
    },
];

pub static ENTITY_REGISTRY: EntityRegistry = EntityRegistry::new(&DEFINITIONS);
