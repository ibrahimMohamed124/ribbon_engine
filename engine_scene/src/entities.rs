use engine_core::math::Vect;

pub type Box3 = ([f32; 3], [f32; 3], u8);
pub type Bounds3 = ([f32; 3], [f32; 3]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityKind(pub u32);

pub const WARDEN: EntityKind = EntityKind(1);
pub const PAGE: EntityKind = EntityKind(2);
pub const PREVIEW_WARDEN: EntityKind = EntityKind(3);

#[derive(Clone, Copy, Debug)]
pub struct EntityBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub part: u8,
    /// Displacement in x as a fraction of `sin(params[0]) * 0.28`.
    pub phase_sway: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct EntityDefinition {
    pub kind: EntityKind,
    pub bounds: Option<Bounds3>,
    pub boxes: &'static [EntityBox],
}

#[derive(Clone, Copy, Debug)]
pub struct EntityRegistry {
    pub definitions: &'static [EntityDefinition],
}

impl EntityRegistry {
    pub const fn new(definitions: &'static [EntityDefinition]) -> Self {
        Self { definitions }
    }

    pub fn get(&self, kind: EntityKind) -> Option<&EntityDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.kind == kind)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EntityInstance {
    pub kind: EntityKind,
    pub pos: Vect,
    pub yaw: f32,
    pub params: [f32; 4],
}

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

const PREVIEW_WARDEN_BOXES: [EntityBox; 4] = [
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

pub const ENEMY_BOUNDS: Bounds3 = ([-0.5, -0.75, 0.0], [1.0, 0.75, 2.25]);

const BUILTIN_DEFINITIONS: [EntityDefinition; 3] = [
    EntityDefinition {
        kind: WARDEN,
        bounds: Some(ENEMY_BOUNDS),
        boxes: &WARDEN_BOXES,
    },
    EntityDefinition {
        kind: PAGE,
        bounds: None,
        boxes: &PAGE_BOXES,
    },
    EntityDefinition {
        kind: PREVIEW_WARDEN,
        bounds: None,
        boxes: &PREVIEW_WARDEN_BOXES,
    },
];

pub static BUILTIN_ENTITY_REGISTRY: EntityRegistry = EntityRegistry::new(&BUILTIN_DEFINITIONS);
