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

/// Cached inputs and outputs for incremental 2D/voxel transform propagation.
///
/// Transform fields are public and can be changed through arbitrary ECS systems, so
/// this cache compares component snapshots rather than relying on mutation APIs or
/// ECS change ticks. This keeps legacy mutation patterns correct while allowing
/// clean branches to skip transform composition and global-component writes.
#[derive(Default)]
struct TransformPropagationCache {
    locals: HashMap<Entity, Transform>,
    parents: HashMap<Entity, Entity>,
    globals: HashMap<Entity, GlobalTransform>,
    scratch_locals: HashMap<Entity, Transform>,
    scratch_parents: HashMap<Entity, Entity>,
}

/// Cached inputs and outputs for incremental quaternion-based 3D transform propagation.
#[derive(Default)]
struct Transform3DPropagationCache {
    locals: HashMap<Entity, Transform3D>,
    parents: HashMap<Entity, Entity>,
    globals: HashMap<Entity, GlobalTransform3D>,
    scratch_locals: HashMap<Entity, Transform3D>,
    scratch_parents: HashMap<Entity, Entity>,
}

/// Recompute global transforms only for changed entities and their affected descendants.
///
/// Local transforms and parent links are snapshotted each call because their fields are
/// currently public. Composition is skipped for clean branches, and `GlobalTransform`
/// is only written when its resolved value differs from the existing component.
pub fn propagate_transforms(world: &mut World) {
    // Double-buffer snapshots so steady-state propagation can reuse map allocations.
    // Move the cache out rather than cloning it; an unwind simply forces a full refresh next call.
    let mut previous = world
        .remove_resource::<TransformPropagationCache>()
        .unwrap_or_default();
    let mut locals = std::mem::take(&mut previous.scratch_locals);
    locals.clear();
    for (entity, value) in world.query::<&Transform>().iter() {
        locals.insert(entity, *value);
    }
    let mut parents = std::mem::take(&mut previous.scratch_parents);
    parents.clear();
    for (entity, parent) in world.query::<&Parent>().iter() {
        parents.insert(entity, parent.0);
    }

    let mut seeds = changed_entities(&previous.locals, &locals);
    seeds.extend(changed_entities(&previous.parents, &parents));
    for &entity in locals.keys() {
        let cached = previous.globals.get(&entity).copied();
        let actual = world.get::<GlobalTransform>(entity).map(|value| *value);
        if cached != actual {
            seeds.insert(entity);
        }
    }
    let dirty = invalidate_descendants(seeds, &previous.parents, &parents);
    let mut computed = HashMap::with_capacity(dirty.len());
    let mut visiting = HashSet::new();
    let dirty_entities = sorted_entities(
        dirty
            .iter()
            .copied()
            .filter(|entity| locals.contains_key(entity)),
    );
    for entity in dirty_entities {
        resolve_dirty(
            entity,
            &locals,
            &parents,
            &dirty,
            &previous.globals,
            &mut computed,
            &mut visiting,
        );
    }

    for &entity in previous.locals.keys() {
        if !locals.contains_key(&entity) {
            let _ = world.remove::<GlobalTransform>(entity);
        }
    }
    let mut globals = previous.globals;
    globals.retain(|entity, _| locals.contains_key(entity));
    for entity in sorted_entities(computed.keys().copied()) {
        let transform = computed[&entity];
        let global = GlobalTransform {
            translation: transform.translation,
            rotation: transform.rotation,
            scale: transform.scale,
        };
        if world.get::<GlobalTransform>(entity).map(|value| *value) != Some(global) {
            let _ = world.insert(entity, global);
        }
        globals.insert(entity, global);
    }
    let mut scratch_locals = previous.locals;
    scratch_locals.clear();
    let mut scratch_parents = previous.parents;
    scratch_parents.clear();
    world.insert_resource(TransformPropagationCache {
        locals,
        parents,
        globals,
        scratch_locals,
        scratch_parents,
    });
}

/// Recompute 3D globals incrementally using the same `Parent` graph as 2D transforms.
///
/// Only ancestors carrying [`Transform3D`] participate in a 3D transform chain; the
/// legacy [`Transform`] convention is not implicitly mixed with quaternion transforms.
pub fn propagate_transforms_3d(world: &mut World) {
    // Reuse the alternate snapshot maps so steady-state propagation avoids repeated allocations.
    let mut previous = world
        .remove_resource::<Transform3DPropagationCache>()
        .unwrap_or_default();
    let mut locals = std::mem::take(&mut previous.scratch_locals);
    locals.clear();
    for (entity, value) in world.query::<&Transform3D>().iter() {
        locals.insert(entity, *value);
    }
    let mut parents = std::mem::take(&mut previous.scratch_parents);
    parents.clear();
    for (entity, parent) in world.query::<&Parent>().iter() {
        parents.insert(entity, parent.0);
    }

    let mut seeds = changed_entities(&previous.locals, &locals);
    seeds.extend(changed_entities(&previous.parents, &parents));
    for &entity in locals.keys() {
        let cached = previous.globals.get(&entity).copied();
        let actual = world.get::<GlobalTransform3D>(entity).map(|value| *value);
        if cached != actual {
            seeds.insert(entity);
        }
    }
    let dirty = invalidate_descendants(seeds, &previous.parents, &parents);
    let mut computed = HashMap::with_capacity(dirty.len());
    let mut visiting = HashSet::new();
    let dirty_entities = sorted_entities(
        dirty
            .iter()
            .copied()
            .filter(|entity| locals.contains_key(entity)),
    );
    for entity in dirty_entities {
        resolve_dirty_3d(
            entity,
            &locals,
            &parents,
            &dirty,
            &previous.globals,
            &mut computed,
            &mut visiting,
        );
    }

    for &entity in previous.locals.keys() {
        if !locals.contains_key(&entity) {
            let _ = world.remove::<GlobalTransform3D>(entity);
        }
    }
    let mut globals = previous.globals;
    globals.retain(|entity, _| locals.contains_key(entity));
    for entity in sorted_entities(computed.keys().copied()) {
        let global = computed[&entity];
        if world.get::<GlobalTransform3D>(entity).map(|value| *value) != Some(global) {
            let _ = world.insert(entity, global);
        }
        globals.insert(entity, global);
    }
    let mut scratch_locals = previous.locals;
    scratch_locals.clear();
    let mut scratch_parents = previous.parents;
    scratch_parents.clear();
    world.insert_resource(Transform3DPropagationCache {
        locals,
        parents,
        globals,
        scratch_locals,
        scratch_parents,
    });
}

/// Return entities whose component value was added, removed, or changed.
fn changed_entities<T: Copy + PartialEq>(
    previous: &HashMap<Entity, T>,
    current: &HashMap<Entity, T>,
) -> HashSet<Entity> {
    let mut changed = HashSet::new();
    for (&entity, value) in current {
        if previous.get(&entity) != Some(value) {
            changed.insert(entity);
        }
    }
    for &entity in previous.keys() {
        if !current.contains_key(&entity) {
            changed.insert(entity);
        }
    }
    changed
}

/// Mark descendants reachable through either the old or current parent graph dirty.
/// Including old edges is important when an entity is reparented or removed.
fn invalidate_descendants(
    seeds: HashSet<Entity>,
    previous_parents: &HashMap<Entity, Entity>,
    current_parents: &HashMap<Entity, Entity>,
) -> HashSet<Entity> {
    let mut children_by_parent: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for parents in [previous_parents, current_parents] {
        for (&child, &parent) in parents.iter() {
            children_by_parent.entry(parent).or_default().push(child);
        }
    }

    let mut dirty = seeds;
    let mut stack: Vec<_> = dirty.iter().copied().collect();
    while let Some(parent) = stack.pop() {
        if let Some(children) = children_by_parent.get(&parent) {
            for &child in children {
                if dirty.insert(child) {
                    stack.push(child);
                }
            }
        }
    }
    dirty
}

/// Stable entity order for deterministic traversal and component writes.
fn sorted_entities(entities: impl IntoIterator<Item = Entity>) -> Vec<Entity> {
    let mut entities: Vec<_> = entities.into_iter().collect();
    entities.sort_unstable_by_key(|entity| entity.to_bits().get());
    entities
}

fn resolve_dirty(
    entity: Entity,
    locals: &HashMap<Entity, Transform>,
    parents: &HashMap<Entity, Entity>,
    dirty: &HashSet<Entity>,
    cached_globals: &HashMap<Entity, GlobalTransform>,
    computed: &mut HashMap<Entity, Transform>,
    visiting: &mut HashSet<Entity>,
) -> Transform {
    if !dirty.contains(&entity) {
        if let Some(&global) = cached_globals.get(&entity) {
            return Transform {
                translation: global.translation,
                rotation: global.rotation,
                scale: global.scale,
            };
        }
    }
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
            compose(
                resolve_dirty(
                    parent,
                    locals,
                    parents,
                    dirty,
                    cached_globals,
                    computed,
                    visiting,
                ),
                local,
            )
        });
    visiting.remove(&entity);
    computed.insert(entity, global);
    global
}

fn resolve_dirty_3d(
    entity: Entity,
    locals: &HashMap<Entity, Transform3D>,
    parents: &HashMap<Entity, Entity>,
    dirty: &HashSet<Entity>,
    cached_globals: &HashMap<Entity, GlobalTransform3D>,
    computed: &mut HashMap<Entity, GlobalTransform3D>,
    visiting: &mut HashSet<Entity>,
) -> GlobalTransform3D {
    if !dirty.contains(&entity) {
        if let Some(&global) = cached_globals.get(&entity) {
            return global;
        }
    }
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
                    resolve_dirty_3d(
                        parent,
                        locals,
                        parents,
                        dirty,
                        cached_globals,
                        computed,
                        visiting,
                    ),
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
