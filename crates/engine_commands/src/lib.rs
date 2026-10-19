//! Undoable world editing commands and a bounded history stack.
//!
//! Commands resolve registered components through `TypeRegistry`; persistence belongs to `engine_scene`.

use engine_ecs::{Entity, World};
use engine_reflect::{ReflectError, TypeRegistry, Value};
use engine_scene::{capture, instantiate, Children, HierarchyExt, Parent, Prefab, SceneDoc};
use std::any::Any;
use std::fmt;
use std::time::{Duration, Instant};

/// A user-visible operation that can be reversed and replayed.
pub trait Command: Any + Send + Sync {
    fn name(&self) -> String;
    fn apply(&mut self, world: &mut World) -> Result<(), CommandError>;
    fn undo(&mut self, world: &mut World) -> Result<(), CommandError>;
    fn merge(&mut self, _other: &dyn Command) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any;
}

/// Invalid command, missing component, or hierarchy operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandError(pub String);
impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CommandError {}
impl From<ReflectError> for CommandError {
    fn from(error: ReflectError) -> Self {
        Self(error.to_string())
    }
}

/// Edit one field on a reflected component. Consecutive edits of the same field merge for 500 ms.
pub struct SetField {
    pub entity: Entity,
    pub component: String,
    pub field: String,
    pub old: Value,
    pub new: Value,
    changed_at: Instant,
}
impl SetField {
    pub fn new(
        entity: Entity,
        component: impl Into<String>,
        field: impl Into<String>,
        old: Value,
        new: Value,
    ) -> Self {
        Self {
            entity,
            component: component.into(),
            field: field.into(),
            old,
            new,
            changed_at: Instant::now(),
        }
    }
    fn set(&self, world: &mut World, new_value: Value) -> Result<(), CommandError> {
        let component = self.component.clone();
        let field = self.field.clone();
        world.resource_scope::<TypeRegistry, _>(|world, registry| {
            let mut reflected = registry
                .get(world, self.entity, &component)
                .ok_or_else(|| {
                    CommandError(format!(
                        "entity {:?} has no component `{component}`",
                        self.entity
                    ))
                })?;
            reflected.set_field(&field, new_value)?;
            registry.insert(world, self.entity, reflected.as_ref())?;
            Ok(())
        })
    }
}
impl Command for SetField {
    fn name(&self) -> String {
        format!("Set {}.{}", self.component, self.field)
    }
    fn apply(&mut self, world: &mut World) -> Result<(), CommandError> {
        self.set(world, self.new.clone())
    }
    fn undo(&mut self, world: &mut World) -> Result<(), CommandError> {
        self.set(world, self.old.clone())
    }
    fn merge(&mut self, other: &dyn Command) -> bool {
        let Some(other) = other.as_any().downcast_ref::<Self>() else {
            return false;
        };
        if self.entity != other.entity
            || self.component != other.component
            || self.field != other.field
            || other.changed_at.saturating_duration_since(self.changed_at)
                > Duration::from_millis(500)
        {
            return false;
        }
        self.new = other.new.clone();
        self.changed_at = other.changed_at;
        true
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Add a default instance of a registered component.
pub struct AddComponent {
    pub entity: Entity,
    pub component: String,
}
impl Command for AddComponent {
    fn name(&self) -> String {
        format!("Add {}", self.component)
    }
    fn apply(&mut self, world: &mut World) -> Result<(), CommandError> {
        let name = self.component.clone();
        world.resource_scope::<TypeRegistry, _>(|world, registry| {
            let value = registry
                .default_value(&name)
                .ok_or_else(|| CommandError(format!("unknown component `{name}`")))?;
            registry.insert(world, self.entity, value.as_ref())?;
            Ok(())
        })
    }
    fn undo(&mut self, world: &mut World) -> Result<(), CommandError> {
        let name = self.component.clone();
        world.resource_scope::<TypeRegistry, _>(|world, registry| {
            registry
                .remove(world, self.entity, &name)
                .map_err(Into::into)
        })
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Remove a registered component while retaining its reflected value for undo.
pub struct RemoveComponent {
    pub entity: Entity,
    pub component: String,
    snapshot: Option<Value>,
}
impl RemoveComponent {
    pub fn new(entity: Entity, component: impl Into<String>) -> Self {
        Self {
            entity,
            component: component.into(),
            snapshot: None,
        }
    }
}
impl Command for RemoveComponent {
    fn name(&self) -> String {
        format!("Remove {}", self.component)
    }
    fn apply(&mut self, world: &mut World) -> Result<(), CommandError> {
        let name = self.component.clone();
        world.resource_scope::<TypeRegistry, _>(|world, registry| {
            let value = registry.get(world, self.entity, &name).ok_or_else(|| {
                CommandError(format!(
                    "entity {:?} has no component `{name}`",
                    self.entity
                ))
            })?;
            self.snapshot = Some(Value::Struct(
                value
                    .fields()
                    .iter()
                    .filter_map(|f| value.get_field(f.name).map(|v| (f.name.to_owned(), v)))
                    .collect(),
            ));
            registry
                .remove(world, self.entity, &name)
                .map_err(Into::into)
        })
    }
    fn undo(&mut self, world: &mut World) -> Result<(), CommandError> {
        let name = self.component.clone();
        let snapshot = self
            .snapshot
            .clone()
            .ok_or_else(|| CommandError("component snapshot is missing".into()))?;
        world.resource_scope::<TypeRegistry, _>(|world, registry| {
            let mut value = registry
                .default_value(&name)
                .ok_or_else(|| CommandError(format!("unknown component `{name}`")))?;
            let Value::Struct(fields) = snapshot else {
                return Err(CommandError("invalid component snapshot".into()));
            };
            for (field, field_value) in fields {
                value.set_field(&field, field_value)?;
            }
            registry
                .insert(world, self.entity, value.as_ref())
                .map_err(Into::into)
        })
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Instantiate a prefab as one undoable spawn operation.
pub struct SpawnEntity {
    pub prefab: Prefab,
    pub parent: Option<Entity>,
    spawned: Vec<Entity>,
}
impl SpawnEntity {
    pub fn new(prefab: Prefab, parent: Option<Entity>) -> Self {
        Self {
            prefab,
            parent,
            spawned: Vec::new(),
        }
    }
}
impl Command for SpawnEntity {
    fn name(&self) -> String {
        format!("Spawn {}", self.prefab.name)
    }
    fn apply(&mut self, world: &mut World) -> Result<(), CommandError> {
        let prefab = self.prefab.clone();
        let parent = self.parent;
        self.spawned = world.resource_scope::<TypeRegistry, _>(|world, registry| {
            instantiate(world, registry, &prefab, parent).map_err(|e| CommandError(e.to_string()))
        })?;
        Ok(())
    }
    fn undo(&mut self, world: &mut World) -> Result<(), CommandError> {
        let spawned = self.spawned.clone();
        for entity in spawned {
            if world.contains(entity) {
                world
                    .despawn_recursive(entity)
                    .map_err(|e| CommandError(e.to_string()))?;
            }
        }
        Ok(())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Despawn an entity tree while retaining a scene snapshot for undo.
pub struct DespawnEntity {
    pub entity: Entity,
    snapshot: Option<SceneDoc>,
    parent: Option<Entity>,
}
impl DespawnEntity {
    pub fn new(entity: Entity) -> Self {
        Self {
            entity,
            snapshot: None,
            parent: None,
        }
    }
}
impl Command for DespawnEntity {
    fn name(&self) -> String {
        "Despawn entity".into()
    }
    fn apply(&mut self, world: &mut World) -> Result<(), CommandError> {
        if !world.contains(self.entity) {
            return Err(CommandError(format!(
                "entity {:?} no longer exists",
                self.entity
            )));
        }
        self.parent = world.get::<Parent>(self.entity).map(|parent| parent.0);
        let root = self.entity;
        world.resource_scope::<TypeRegistry, _>(|world, registry| {
            if self.snapshot.is_none() {
                let descendants = descendant_set(world, root);
                self.snapshot = Some(capture(world, registry, |entity| {
                    descendants.contains(&entity)
                }));
            }
        });
        world
            .despawn_recursive(self.entity)
            .map_err(|e| CommandError(e.to_string()))
    }
    fn undo(&mut self, world: &mut World) -> Result<(), CommandError> {
        let snapshot = self
            .snapshot
            .clone()
            .ok_or_else(|| CommandError("entity snapshot is missing".into()))?;
        let parent = self.parent.filter(|parent| world.contains(*parent));
        let entities = world.resource_scope::<TypeRegistry, _>(|world, registry| {
            instantiate(world, registry, &snapshot.prefab, parent)
                .map_err(|e| CommandError(e.to_string()))
        })?;
        let root_index = snapshot
            .prefab
            .entities
            .iter()
            .position(|record| record.parent.is_none())
            .unwrap_or(0);
        self.entity = entities
            .get(root_index)
            .copied()
            .ok_or_else(|| CommandError("restored snapshot has no root".into()))?;
        Ok(())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Move an entity between parents, or detach it from its current parent.
pub struct Reparent {
    pub entity: Entity,
    pub parent: Option<Entity>,
    previous: Option<Entity>,
}
impl Reparent {
    pub fn new(entity: Entity, parent: Option<Entity>) -> Self {
        Self {
            entity,
            parent,
            previous: None,
        }
    }
}
impl Reparent {
    fn apply_parent(
        world: &mut World,
        entity: Entity,
        parent: Option<Entity>,
    ) -> Result<(), CommandError> {
        match parent {
            Some(parent) => world
                .set_parent(entity, parent)
                .map_err(|e| CommandError(e.to_string())),
            None => {
                if let Some(old) = world.get::<Parent>(entity).map(|p| p.0) {
                    if let Some(mut children) = world.get_mut::<Children>(old) {
                        children.0.retain(|child| *child != entity);
                    }
                    let _ = world.remove::<Parent>(entity);
                }
                Ok(())
            }
        }
    }
}
impl Command for Reparent {
    fn name(&self) -> String {
        "Reparent entity".into()
    }
    fn apply(&mut self, world: &mut World) -> Result<(), CommandError> {
        self.previous = world.get::<Parent>(self.entity).map(|p| p.0);
        Self::apply_parent(world, self.entity, self.parent)
    }
    fn undo(&mut self, world: &mut World) -> Result<(), CommandError> {
        Self::apply_parent(world, self.entity, self.previous)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn descendant_set(world: &World, root: Entity) -> std::collections::HashSet<Entity> {
    let mut set = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if !set.insert(entity) {
            continue;
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.0.iter().copied());
        }
    }
    set
}

struct CompoundCommand {
    label: String,
    commands: Vec<Box<dyn Command>>,
}
impl Command for CompoundCommand {
    fn name(&self) -> String {
        self.label.clone()
    }
    fn apply(&mut self, world: &mut World) -> Result<(), CommandError> {
        for (applied, command) in self.commands.iter_mut().enumerate() {
            if let Err(error) = command.apply(world) {
                for previous in self.commands[..applied].iter_mut().rev() {
                    let _ = previous.undo(world);
                }
                return Err(error);
            }
        }
        Ok(())
    }
    fn undo(&mut self, world: &mut World) -> Result<(), CommandError> {
        for command in self.commands.iter_mut().rev() {
            command.undo(world)?;
        }
        Ok(())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Bounded undo/redo stack. A grouped edit is treated as one history entry.
pub struct CommandStack {
    history: Vec<Box<dyn Command>>,
    cursor: usize,
    limit: usize,
    clean_cursor: Option<usize>,
    group: Option<(String, Vec<Box<dyn Command>>)>,
}
impl Default for CommandStack {
    fn default() -> Self {
        Self::new(256)
    }
}
impl CommandStack {
    pub fn new(limit: usize) -> Self {
        Self {
            history: Vec::new(),
            cursor: 0,
            limit,
            clean_cursor: Some(0),
            group: None,
        }
    }
    pub fn execute(
        &mut self,
        world: &mut World,
        mut command: Box<dyn Command>,
    ) -> Result<(), CommandError> {
        command.apply(world)?;
        if let Some((_, commands)) = &mut self.group {
            commands.push(command);
            return Ok(());
        }
        self.history.truncate(self.cursor);
        if let Some(last) = self.history.last_mut() {
            if last.merge(command.as_ref()) {
                self.clean_cursor = None;
                return Ok(());
            }
        }
        self.history.push(command);
        self.cursor += 1;
        self.trim();
        Ok(())
    }
    pub fn undo(&mut self, world: &mut World) -> Result<(), CommandError> {
        if self.cursor == 0 {
            return Ok(());
        }
        self.history[self.cursor - 1].undo(world)?;
        self.cursor -= 1;
        Ok(())
    }
    pub fn redo(&mut self, world: &mut World) -> Result<(), CommandError> {
        if self.cursor >= self.history.len() {
            return Ok(());
        }
        self.history[self.cursor].apply(world)?;
        self.cursor += 1;
        Ok(())
    }
    pub fn begin_group(&mut self, name: impl Into<String>) {
        self.group = Some((name.into(), Vec::new()));
    }
    pub fn end_group(&mut self) {
        if let Some((label, commands)) = self.group.take() {
            if !commands.is_empty() {
                self.history.truncate(self.cursor);
                self.history
                    .push(Box::new(CompoundCommand { label, commands }));
                self.cursor += 1;
                self.trim();
            }
        }
    }
    pub fn clear(&mut self) {
        self.history.clear();
        self.cursor = 0;
        self.clean_cursor = Some(0);
        self.group = None;
    }
    pub fn mark_clean(&mut self) {
        self.clean_cursor = Some(self.cursor);
    }
    pub fn is_dirty(&self) -> bool {
        self.clean_cursor != Some(self.cursor)
    }
    pub fn len(&self) -> usize {
        self.history.len()
    }
    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }
    fn trim(&mut self) {
        if self.limit == 0 {
            self.history.clear();
            self.cursor = 0;
            self.clean_cursor = None;
            return;
        }
        if self.history.len() > self.limit {
            let remove = self.history.len() - self.limit;
            self.history.drain(..remove);
            self.cursor = self.cursor.saturating_sub(remove);
            self.clean_cursor = self
                .clean_cursor
                .map(|clean| clean.checked_sub(remove).unwrap_or(usize::MAX));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandStack, SetField};
    use engine_ecs::World;
    use engine_reflect::{TypeRegistry, Value};
    use engine_scene::Name;

    #[test]
    fn set_field_undo_redo_and_merge_keep_one_history_item() {
        let mut world = World::new();
        let entity = world.spawn((Name("before".into()),));
        let mut registry = TypeRegistry::default();
        registry.register::<Name>().unwrap();
        world.insert_resource(registry);
        let old = Value::Str("before".into());
        let mut stack = CommandStack::default();
        stack
            .execute(
                &mut world,
                Box::new(SetField::new(
                    entity,
                    "Name",
                    "value",
                    old.clone(),
                    Value::Str("middle".into()),
                )),
            )
            .unwrap();
        stack
            .execute(
                &mut world,
                Box::new(SetField::new(
                    entity,
                    "Name",
                    "value",
                    Value::Str("middle".into()),
                    Value::Str("after".into()),
                )),
            )
            .unwrap();
        assert_eq!(stack.len(), 1);
        stack.undo(&mut world).unwrap();
        assert_eq!(world.get::<Name>(entity).unwrap().0, "before");
        stack.redo(&mut world).unwrap();
        assert_eq!(world.get::<Name>(entity).unwrap().0, "after");
    }

    #[test]
    fn grouped_edits_roundtrip_scene_and_new_edit_clears_redo() {
        use engine_scene::capture;
        let mut world = World::new();
        let entity = world.spawn((Name("before".into()),));
        let mut registry = TypeRegistry::default();
        registry.register::<Name>().unwrap();
        world.insert_resource(registry);
        let before = world.resource_scope::<TypeRegistry, _>(|world, registry| {
            capture(world, registry, |_| true)
        });
        let mut stack = CommandStack::default();
        stack.begin_group("rename");
        for (from, to) in [("before", "middle"), ("middle", "after")] {
            stack
                .execute(
                    &mut world,
                    Box::new(SetField::new(
                        entity,
                        "Name",
                        "value",
                        Value::Str(from.into()),
                        Value::Str(to.into()),
                    )),
                )
                .unwrap();
        }
        stack.end_group();
        assert_eq!(stack.len(), 1);
        stack.undo(&mut world).unwrap();
        let after_undo = world.resource_scope::<TypeRegistry, _>(|world, registry| {
            capture(world, registry, |_| true)
        });
        assert_eq!(before, after_undo);
        stack.redo(&mut world).unwrap();
        stack.undo(&mut world).unwrap();
        stack
            .execute(
                &mut world,
                Box::new(SetField::new(
                    entity,
                    "Name",
                    "value",
                    Value::Str("before".into()),
                    Value::Str("new edit".into()),
                )),
            )
            .unwrap();
        stack.redo(&mut world).unwrap();
        assert_eq!(world.get::<Name>(entity).unwrap().0, "new edit");
    }

    #[test]
    fn history_limit_discards_oldest_undo_entry() {
        let mut world = World::new();
        let a = world.spawn((Name("a".into()),));
        let b = world.spawn((Name("b".into()),));
        let mut registry = TypeRegistry::default();
        registry.register::<Name>().unwrap();
        world.insert_resource(registry);
        let mut stack = CommandStack::new(1);
        stack
            .execute(
                &mut world,
                Box::new(SetField::new(
                    a,
                    "Name",
                    "value",
                    Value::Str("a".into()),
                    Value::Str("a1".into()),
                )),
            )
            .unwrap();
        stack
            .execute(
                &mut world,
                Box::new(SetField::new(
                    b,
                    "Name",
                    "value",
                    Value::Str("b".into()),
                    Value::Str("b1".into()),
                )),
            )
            .unwrap();
        assert_eq!(stack.len(), 1);
        stack.undo(&mut world).unwrap();
        assert_eq!(world.get::<Name>(a).unwrap().0, "a1");
        assert_eq!(world.get::<Name>(b).unwrap().0, "b");
    }

    #[test]
    fn prefab_spawn_despawn_and_reparent_commands_keep_hierarchy_consistent() {
        use super::{DespawnEntity, Reparent, SpawnEntity};
        use engine_scene::{HierarchyExt, Parent, Prefab, PrefabEntity};
        let mut world = World::new();
        let root = world.spawn((Name("root".into()),));
        let child = world.spawn((Name("child".into()),));
        let alternate = world.spawn((Name("alternate".into()),));
        world.set_parent(child, root).unwrap();
        let mut registry = TypeRegistry::default();
        registry.register::<Name>().unwrap();
        world.insert_resource(registry);

        let mut stack = CommandStack::default();
        stack
            .execute(&mut world, Box::new(Reparent::new(child, Some(alternate))))
            .unwrap();
        assert_eq!(world.get::<Parent>(child).unwrap().0, alternate);
        stack.undo(&mut world).unwrap();
        assert_eq!(world.get::<Parent>(child).unwrap().0, root);

        stack
            .execute(&mut world, Box::new(DespawnEntity::new(root)))
            .unwrap();
        assert!(!world.contains(root) && !world.contains(child));
        stack.undo(&mut world).unwrap();
        assert_eq!(world.len(), 3);
        let restored_root = world
            .query::<&Name>()
            .iter()
            .find(|(_, name)| name.0 == "root")
            .unwrap()
            .0;
        let restored_child = world
            .query::<&Name>()
            .iter()
            .find(|(_, name)| name.0 == "child")
            .unwrap()
            .0;
        assert_eq!(
            world.get::<Parent>(restored_child).unwrap().0,
            restored_root
        );

        let prefab = Prefab {
            format_version: 1,
            name: "spawned".into(),
            entities: vec![PrefabEntity {
                local_id: 0,
                parent: None,
                components: vec![(
                    "Name".into(),
                    Value::Struct(vec![("value".into(), Value::Str("spawned".into()))]),
                )],
            }],
        };
        stack
            .execute(&mut world, Box::new(SpawnEntity::new(prefab, None)))
            .unwrap();
        assert!(world
            .query::<&Name>()
            .iter()
            .any(|(_, name)| name.0 == "spawned"));
        stack.undo(&mut world).unwrap();
        assert!(!world
            .query::<&Name>()
            .iter()
            .any(|(_, name)| name.0 == "spawned"));
    }
}
