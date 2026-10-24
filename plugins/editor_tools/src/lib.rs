//! Built-in hierarchy, reflected inspector, and runtime status panels.
//!
//! Inspector edits use `ribbon_commands` so they remain undoable.

use ribbon_app::{App, LogBuffer, ScheduleStats, Time};
use ribbon_commands::{CommandStack, SetField};
use ribbon_ecs::{Entity, World};
use ribbon_editor::{EditorShell, Panel, Selection, Ui};
use ribbon_plugins::Plugin;
use ribbon_reflect::{TypeRegistry, Value};
use ribbon_scene::{Name, Parent};
use std::collections::HashSet;

/// Tree view of named scene entities.
pub struct HierarchyPanel;
impl Panel for HierarchyPanel {
    fn title(&self) -> &str {
        "Hierarchy"
    }
    fn ui(&mut self, ui: &mut Ui, world: &mut World) {
        let mut rows: Vec<_> = world
            .query::<()>()
            .iter()
            .map(|(entity, ())| {
                let name = world
                    .get::<Name>(entity)
                    .map(|n| n.0.clone())
                    .unwrap_or_else(|| format!("Entity {entity:?}"));
                let depth = depth_of(world, entity);
                (entity, name, depth)
            })
            .collect();
        rows.sort_by_key(|(entity, _, _)| entity.to_bits());
        for (entity, name, depth) in rows {
            if ui.button(&format!("{}{}", "  ".repeat(depth), name), 170) {
                let selected = world.resource_mut::<Selection>();
                selected.0.clear();
                selected.0.insert(entity);
            }
        }
    }
}
fn depth_of(world: &World, entity: Entity) -> usize {
    let mut depth = 0;
    let mut parent = world.get::<Parent>(entity).map(|p| p.0);
    while let Some(entity) = parent {
        depth += 1;
        if depth > 128 {
            break;
        }
        parent = world.get::<Parent>(entity).map(|p| p.0);
    }
    depth
}

/// Inspector that edits reflected numeric fields through reversible commands.
pub struct InspectorPanel;
impl Panel for InspectorPanel {
    fn title(&self) -> &str {
        "Inspector"
    }
    fn ui(&mut self, ui: &mut Ui, world: &mut World) {
        let selected: Vec<_> = world.resource::<Selection>().0.iter().copied().collect();
        let mut fields = Vec::new();
        world.resource_scope::<TypeRegistry, _>(|world, registry| {
            for entity in selected {
                for component in registry.names() {
                    if let Some(value) = registry.get(world, entity, component) {
                        for field in value.fields() {
                            if field.editable {
                                if let Some(current) = value.get_field(field.name) {
                                    fields.push((
                                        entity,
                                        component.to_owned(),
                                        field.name.to_owned(),
                                        field.kind,
                                        current,
                                        field.range,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        });
        for (entity, component, field, kind, old, range) in fields {
            ui.label(&format!("{component}.{field}"));
            let Some(initial) = old.clone().into_float() else {
                ui.label(&format!("{kind:?}"));
                continue;
            };
            let mut edited = initial;
            let bounds = range.unwrap_or((-1_000_000.0, 1_000_000.0));
            ui.drag_value(&field, &mut edited, bounds, 0.01);
            if edited != initial {
                let command = SetField::new(entity, component, field, old, Value::Float(edited));
                let _ = world.resource_scope::<CommandStack, _>(|world, stack| {
                    stack.execute(world, Box::new(command))
                });
            }
        }
    }
}
trait IntoFloat {
    fn into_float(self) -> Option<f64>;
}
impl IntoFloat for Value {
    fn into_float(self) -> Option<f64> {
        if let Value::Float(value) = self {
            Some(value)
        } else {
            None
        }
    }
}

/// Frame timing, entity count, scheduled system timings, and recent log messages.
pub struct StatsPanel;
impl Panel for StatsPanel {
    fn title(&self) -> &str {
        "Stats"
    }
    fn ui(&mut self, ui: &mut Ui, world: &mut World) {
        let frame_ms = world
            .try_resource::<Time>()
            .map_or(0.0, |time| time.delta.as_secs_f64() * 1000.0);
        ui.label(&format!("Frame: {frame_ms:.2} ms"));
        ui.label(&format!("Entities: {}", world.len()));
        if let Some(stats) = world.try_resource::<ScheduleStats>() {
            let mut systems: Vec<_> = stats.systems.iter().collect();
            systems.sort_by_key(|(_, duration)| std::cmp::Reverse(**duration));
            for (name, duration) in systems.into_iter().take(10) {
                ui.label(&format!(
                    "{name}: {:.2} ms",
                    duration.as_secs_f64() * 1000.0
                ));
            }
        }
        if let Some(log) = world.try_resource::<LogBuffer>() {
            for (_, message) in log.iter().rev().take(5) {
                ui.label(message);
            }
        }
    }
}

/// Installs the standard panels into the editor shell.
pub struct EditorToolsPlugin;
impl Plugin for EditorToolsPlugin {
    fn name(&self) -> &'static str {
        "EditorToolsPlugin"
    }
    fn dependencies(&self) -> &'static [&'static str] {
        &["EditorPlugin"]
    }
    fn build(&self, app: &mut App) {
        #[cfg(debug_assertions)]
        {
            app.world.resource_mut::<EditorShell>().panels.extend([
                Box::new(HierarchyPanel) as Box<dyn Panel>,
                Box::new(InspectorPanel),
                Box::new(StatsPanel),
            ]);
        }
        #[cfg(not(debug_assertions))]
        let _ = app;
    }
}

/// Helper for creating a selection set without exposing editor internals.
pub fn selection_with(entity: Entity) -> Selection {
    Selection(HashSet::from([entity]))
}

#[cfg(test)]
mod tests {
    use super::InspectorPanel;
    use ribbon_commands::CommandStack;
    use ribbon_ecs::World;
    use ribbon_editor::{Panel, Selection, Ui, UiInput};
    use ribbon_reflect::TypeRegistry;
    use ribbon_scene::Transform;
    use std::collections::HashSet;

    #[test]
    fn inspector_edit_can_be_undone() {
        let mut world = World::new();
        let entity = world.spawn((Transform {
            scale: 1.0,
            ..Transform::default()
        },));
        let mut registry = TypeRegistry::default();
        registry.register::<Transform>().unwrap();
        world.insert_resource(registry);
        world.insert_resource(Selection(HashSet::from([entity])));
        world.insert_resource(CommandStack::default());
        let mut panel = InspectorPanel;
        let mut ui = Ui::new(
            200,
            140,
            UiInput {
                pointer: Some((10.0, 111.0)),
                left_down: true,
                delta: (10.0, 0.0),
                ..UiInput::default()
            },
        );
        panel.ui(&mut ui, &mut world);
        assert!((world.get::<Transform>(entity).unwrap().scale - 1.1).abs() < 1e-6);
        world.resource_scope::<CommandStack, _>(|world, stack| stack.undo(world).unwrap());
        assert_eq!(world.get::<Transform>(entity).unwrap().scale, 1.0);
    }
}
