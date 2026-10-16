use voxelworks_core::math::Vect;

pub type Box3 = ([f32; 3], [f32; 3], u8);
pub type Bounds3 = ([f32; 3], [f32; 3]);

/// Maximum instances accepted by the renderer backends.
pub const MAX_ENTITIES: usize = 16;
/// Maximum box primitives in one entity definition.
pub const MAX_BOXES_PER_ENTITY: usize = 64;
/// Maximum total box primitives in one rendered scene.
pub const MAX_ENTITY_BOXES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityKind(pub u32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EntityRenderMode {
    Boxes,
    Billboard {
        atlas_slot: u32,
        width: f32,
        height: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EntityAnimation {
    None,
    BobSpin {
        amplitude: f32,
        speed: f32,
        spin: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EntityShadow {
    None,
    Ellipse { rx: f32, ry: f32 },
}

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
    pub render_mode: EntityRenderMode,
    pub animation: EntityAnimation,
    pub shadow: EntityShadow,
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
