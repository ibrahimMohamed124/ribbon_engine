use crate::{Children, GlobalTransform, GlobalTransform3D, Parent, Transform, Transform3D};
use engine_ecs::{Entity, World};
use glam::Quat;
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Hierarchy validation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HierarchyError {
    MissingEntity(Entity),
    SelfParent(Entity),
    Cycle { child: Entity, parent: Entity },
}

impl fmt::Display for HierarchyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEntity(entity) => write!(f, "entity {entity:?} does not exist"),
            Self::SelfParent(entity) => write!(f, "entity {entity:?} cannot parent itself"),
            Self::Cycle { child, parent } => {
                write!(f, "parenting {child:?} to {parent:?} would create a cycle")
            }
        }
    }
}
impl std::error::Error for HierarchyError {}

/// Hierarchy operations available on the engine world.
pub trait HierarchyExt {
    fn set_parent(&mut self, child: Entity, parent: Entity) -> Result<(), HierarchyError>;
    fn despawn_recursive(&mut self, entity: Entity) -> Result<(), HierarchyError>;
}

impl HierarchyExt for World {
    fn set_parent(&mut self, child: Entity, parent: Entity) -> Result<(), HierarchyError> {
        if !self.contains(child) {
            return Err(HierarchyError::MissingEntity(child));
        }
        if !self.contains(parent) {
            return Err(HierarchyError::MissingEntity(parent));
        }
        if child == parent {
            return Err(HierarchyError::SelfParent(child));
        }
        let mut current = Some(parent);
        while let Some(entity) = current {
            if entity == child {
                return Err(HierarchyError::Cycle { child, parent });
            }
            current = self.get::<Parent>(entity).map(|parent| parent.0);
        }

        let old_parent = self.get::<Parent>(child).map(|value| value.0);
        if old_parent == Some(parent) {
            return Ok(());
        }
        if let Some(old_parent) = old_parent {
            if let Some(mut children) = self.get_mut::<Children>(old_parent) {
                children.0.retain(|&candidate| candidate != child);
            }
        }
        if self.get::<Children>(parent).is_none() {
            self.insert(parent, Children::default())
                .map_err(|_| HierarchyError::MissingEntity(parent))?;
        }
        if let Some(mut children) = self.get_mut::<Children>(parent) {
            if !children.0.contains(&child) {
                children.0.push(child);
            }
        }
        self.insert(child, Parent(parent))
            .map_err(|_| HierarchyError::MissingEntity(child))
    }

    fn despawn_recursive(&mut self, entity: Entity) -> Result<(), HierarchyError> {
        if !self.contains(entity) {
            return Err(HierarchyError::MissingEntity(entity));
        }
        let children = self
            .get::<Children>(entity)
            .map(|value| value.0.clone())
            .unwrap_or_default();
        for child in children {
            if self.contains(child) {
                self.despawn_recursive(child)?;
            }
        }
        if let Some(parent) = self.get::<Parent>(entity).map(|value| value.0) {
            if let Some(mut children) = self.get_mut::<Children>(parent) {
                children.0.retain(|&candidate| candidate != entity);
            }
        }
        self.despawn(entity)
            .map_err(|_| HierarchyError::MissingEntity(entity))
    }
}

/// Recompute every global transform from hierarchy roots, in deterministic depth-first order.
pub fn propagate_transforms(world: &mut World) {
    let locals: HashMap<_, _> = world
        .query::<&Transform>()
        .iter()
        .map(|(entity, value)| (entity, *value))
        .collect();
    let parents: HashMap<_, _> = world
        .query::<&Parent>()
        .iter()
        .map(|(entity, parent)| (entity, parent.0))
        .collect();
    let mut computed = HashMap::with_capacity(locals.len());
    let mut visiting = HashSet::new();
    for &entity in locals.keys() {
        resolve(entity, &locals, &parents, &mut computed, &mut visiting);
    }
    for (entity, transform) in computed {
        let global = GlobalTransform {
            translation: transform.translation,
            rotation: transform.rotation,
            scale: transform.scale,
        };
        if world.insert(entity, global).is_err() {
            continue;
        }
    }
}

fn resolve(
    entity: Entity,
    locals: &HashMap<Entity, Transform>,
    parents: &HashMap<Entity, Entity>,
    computed: &mut HashMap<Entity, Transform>,
    visiting: &mut HashSet<Entity>,
) -> Transform {
    if let Some(&transform) = computed.get(&entity) {
        return transform;
    }
    let local = locals.get(&entity).copied().unwrap_or_default();
    if !visiting.insert(entity) {
        return local;
    }
    let global = parents
        .get(&entity)
        .filter(|parent| locals.contains_key(parent))
        .map_or(local, |&parent| {
            compose(resolve(parent, locals, parents, computed, visiting), local)
        });
    visiting.remove(&entity);
    computed.insert(entity, global);
    global
}

/// Recompute every 3D global transform from the hierarchy roots.
///
/// Only ancestors carrying [`Transform3D`] participate in a 3D transform chain;
/// this prevents the legacy 2D/voxel-oriented [`Transform`] convention from being
/// implicitly mixed with the mesh renderer's Y-up quaternion convention.
pub fn propagate_transforms_3d(world: &mut World) {
    let locals: HashMap<_, _> = world
        .query::<&Transform3D>()
        .iter()
        .map(|(entity, value)| (entity, *value))
        .collect();
    let parents: HashMap<_, _> = world
        .query::<&Parent>()
        .iter()
        .map(|(entity, parent)| (entity, parent.0))
        .collect();
    let mut computed = HashMap::with_capacity(locals.len());
    let mut visiting = HashSet::new();
    for &entity in locals.keys() {
        resolve_3d(entity, &locals, &parents, &mut computed, &mut visiting);
    }
    for (entity, transform) in computed {
        let _ = world.insert(entity, transform);
    }
}

fn resolve_3d(
    entity: Entity,
    locals: &HashMap<Entity, Transform3D>,
    parents: &HashMap<Entity, Entity>,
    computed: &mut HashMap<Entity, GlobalTransform3D>,
    visiting: &mut HashSet<Entity>,
) -> GlobalTransform3D {
    if let Some(&transform) = computed.get(&entity) {
        return transform;
    }
    let local = locals.get(&entity).copied().unwrap_or_default();
    if !visiting.insert(entity) {
        return GlobalTransform3D::from(local);
    }
    let global = parents
        .get(&entity)
        .filter(|parent| locals.contains_key(parent))
        .map_or_else(
            || GlobalTransform3D::from(local),
            |&parent| {
                compose_3d(
                    resolve_3d(parent, locals, parents, computed, visiting),
                    local,
                )
            },
        );
    visiting.remove(&entity);
    computed.insert(entity, global);
    global
}

fn compose_3d(parent: GlobalTransform3D, child: Transform3D) -> GlobalTransform3D {
    let parent_rotation =
        if parent.rotation.is_finite() && parent.rotation.length_squared() > 1.0e-12 {
            parent.rotation.normalize()
        } else {
            Quat::IDENTITY
        };
    let child_rotation = if child.rotation.is_finite() && child.rotation.length_squared() > 1.0e-12
    {
        child.rotation.normalize()
    } else {
        Quat::IDENTITY
    };
    GlobalTransform3D {
        translation: parent.translation + parent_rotation * (parent.scale * child.translation),
        rotation: (parent_rotation * child_rotation).normalize(),
        scale: parent.scale * child.scale,
    }
}

fn compose(parent: Transform, child: Transform) -> Transform {
    let (sp, cp) = parent.rotation.psi.sin_cos();
    let (sy, cy) = parent.rotation.phi.sin_cos();
    let x = child.translation.x * parent.scale;
    let y = child.translation.y * parent.scale;
    let z = child.translation.z * parent.scale;
    let pitched_x = cp * x + sp * z;
    let pitched_z = -sp * x + cp * z;
    Transform {
        translation: engine_math::Vect::new(
            parent.translation.x + cy * pitched_x - sy * y,
            parent.translation.y + sy * pitched_x + cy * y,
            parent.translation.z + pitched_z,
        ),
        rotation: engine_math::Angles {
            psi: parent.rotation.psi + child.rotation.psi,
            phi: parent.rotation.phi + child.rotation.phi,
        },
        scale: parent.scale * child.scale,
    }
}

#[cfg(test)]
mod tests {
    use super::{propagate_transforms, propagate_transforms_3d, HierarchyError, HierarchyExt};
    use crate::{GlobalTransform, GlobalTransform3D, Transform, Transform3D};
    use engine_ecs::World;
    use engine_math::Vect;
    use glam::{Quat, Vec3};

    #[test]
    fn rejects_cycles_and_despawns_descendants() {
        let mut world = World::new();
        let root = world.spawn((Transform::default(),));
        let child = world.spawn((Transform::default(),));
        let grandchild = world.spawn((Transform::default(),));
        world.set_parent(child, root).unwrap();
        world.set_parent(grandchild, child).unwrap();
        assert!(matches!(
            world.set_parent(root, grandchild),
            Err(HierarchyError::Cycle { .. })
        ));
        world.despawn_recursive(child).unwrap();
        assert!(!world.contains(child) && !world.contains(grandchild));
        assert!(world.get::<crate::Children>(root).unwrap().0.is_empty());
    }

    #[test]
    fn propagates_3d_rotation_scale_and_translation_through_hierarchy() {
        let mut world = World::new();
        let root = world.spawn((Transform3D {
            translation: Vec3::new(1.0, 0.0, 0.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: Vec3::splat(2.0),
        },));
        let child = world.spawn((Transform3D {
            translation: Vec3::X,
            ..Transform3D::default()
        },));
        world.set_parent(child, root).unwrap();

        propagate_transforms_3d(&mut world);

        let global = world.get::<GlobalTransform3D>(child).unwrap();
        let origin = global.matrix().transform_point3(Vec3::ZERO);
        assert!((origin.x - 1.0).abs() < 1.0e-5);
        assert!(origin.y.abs() < 1.0e-5);
        assert!((origin.z + 2.0).abs() < 1.0e-5);
        assert_eq!(global.scale, Vec3::splat(2.0));
    }

    #[test]
    fn propagates_translation_through_a_grandchild() {
        let mut world = World::new();
        let root = world.spawn((Transform {
            translation: Vect::new(1.0, 0.0, 0.0),
            ..Transform::default()
        },));
        let child = world.spawn((Transform {
            translation: Vect::new(0.0, 2.0, 0.0),
            ..Transform::default()
        },));
        let grandchild = world.spawn((Transform {
            translation: Vect::new(0.0, 0.0, 3.0),
            ..Transform::default()
        },));
        world.set_parent(child, root).unwrap();
        world.set_parent(grandchild, child).unwrap();
        propagate_transforms(&mut world);
        let global = world.get::<GlobalTransform>(grandchild).unwrap();
        assert_eq!(global.translation, Vect::new(1.0, 2.0, 3.0));
    }
}
