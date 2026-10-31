//! Built-in hierarchy, reflected inspector, and runtime status panels.
//!
//! Inspector edits use `ribbon_commands` so they remain undoable.

use ribbon_app::{App, LogBuffer, ScheduleStats, Time};
use ribbon_assets::AssetServer;
use ribbon_commands::{CommandStack, SetField};
use ribbon_ecs::{Entity, World};
use ribbon_editor::{EditorShell, Panel, Selection, Ui};
use ribbon_plugin_render3d::{GpuResourceWarmup, GpuUploadBudget};
use ribbon_plugin_streaming::StreamingCoordinator;
use ribbon_plugins::Plugin;
use ribbon_reflect::{TypeRegistry, Value};
use ribbon_render::profiling::RenderDiagnostics;
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

/// Live view of world-region state, asset worker pressure, memory residency, and GPU upload
/// pacing. The panel remains inert when StreamingPlugin is not installed.
pub struct StreamingPanel;
impl Panel for StreamingPanel {
    fn title(&self) -> &str {
        "Streaming"
    }

    fn ui(&mut self, ui: &mut Ui, world: &mut World) {
        if let Some(coordinator) = world.try_resource::<StreamingCoordinator>() {
            let snapshot = coordinator.snapshot();
            let diagnostics = snapshot.diagnostics;
            for scene in &snapshot.scenes {
                ui.label(&format!(
                    "Scene {}: {:?} | regions {}/{} ready, {} active",
                    scene.name,
                    scene.state,
                    scene.ready_region_count,
                    scene.region_count,
                    scene.active_region_count
                ));
            }
            ui.label(&format!(
                "Regions: {} | active {} | loading {} | failed {}",
                diagnostics.region_count,
                diagnostics.active_regions,
                diagnostics.loading_regions,
                diagnostics.failed_regions
            ));
            ui.label(&format!(
                "Pending requests: {} | memory pressure: {} | invalid transitions: {}",
                diagnostics.pending_asset_requests,
                diagnostics.memory_pressure_events,
                diagnostics.invalid_state_transitions
            ));
            ui.label(&format!(
                "CPU: {} MiB resident ({} MiB pinned, {} MiB over budget) | GPU est: {} MiB ({} MiB over)",
                diagnostics.cpu_resident_bytes_estimate / (1024 * 1024),
                diagnostics.cpu_pinned_bytes_estimate / (1024 * 1024),
                diagnostics.cpu_resident_over_budget_bytes / (1024 * 1024),
                diagnostics.gpu_resident_bytes_estimate / (1024 * 1024),
                diagnostics.gpu_memory_over_budget_bytes / (1024 * 1024),
            ));
            ui.label(&format!(
                "Asset queue: {}/{} | workers: {} | staged commits: {}",
                diagnostics.queued_asset_jobs,
                diagnostics.asset_queue_capacity,
                diagnostics.active_asset_workers,
                world
                    .try_resource::<AssetServer>()
                    .map(|server| server.worker_stats().pending_completions)
                    .unwrap_or(0)
            ));
            let mut regions = snapshot.regions;
            regions.sort_by(|left, right| left.id.cmp(&right.id));
            for region in regions {
                ui.label(&format!(
                    "{}: {:?} | {:.1}m | model CPU {}/{} GPU {}/{} | SceneDoc {} | typed deps {} | prefab entities {} | entities {}",
                    region.name,
                    region.state,
                    region.distance_to_nearest_observer.unwrap_or(f32::INFINITY),
                    region.cpu_ready_models,
                    region.model_count,
                    region.gpu_ready_models,
                    region.model_count,
                    if region.has_scene_doc { "yes" } else { "no" },
                    region.typed_dependency_count,
                    region.prefab_entity_count,
                    region.active_entity_count
                ));
                if let Some(error) = region.last_error {
                    ui.label(&format!("  error: {error}"));
                }
            }
            for observer in snapshot.observers {
                ui.label(&format!(
                    "Observer {}: position ({:.1}, {:.1}, {:.1}) velocity ({:.1}, {:.1}, {:.1})",
                    observer.id,
                    observer.position.x,
                    observer.position.y,
                    observer.position.z,
                    observer.velocity.x,
                    observer.velocity.y,
                    observer.velocity.z
                ));
            }
        } else {
            ui.label("StreamingPlugin is not installed");
        }

        if let Some(server) = world.try_resource::<AssetServer>() {
            let memory = server.cpu_memory_stats();
            let workers = server.worker_stats();
            ui.label(&format!(
                "CPU assets: {} | resident estimate: {} MiB | pinned: {} MiB",
                memory.loaded_assets,
                memory.resident_bytes_estimate / (1024 * 1024),
                memory.pinned_bytes_estimate / (1024 * 1024)
            ));
            ui.label(&format!(
                "Completion poll: {}/{} | staged: {} | byte cap: {}",
                workers.completions_processed_last_poll,
                workers.max_completions_per_poll,
                workers.pending_completions,
                workers
                    .max_completion_bytes_per_poll
                    .map(|bytes| format!("{} MiB", bytes / (1024 * 1024)))
                    .unwrap_or_else(|| "unbounded".to_owned())
            ));
            ui.label(&format!(
                "Asset worker (last poll): read {:.2} ms | decode {:.2} ms",
                workers.source_read_time_ns_last_poll as f64 / 1_000_000.0,
                workers.decode_time_ns_last_poll as f64 / 1_000_000.0
            ));
        }
        if let Some(diagnostics) = world.try_resource::<RenderDiagnostics>() {
            ui.label(&format!(
                "GPU upload last frame: {} operations | {:.2} MiB estimated",
                diagnostics.resources.model_gpu_uploads_this_frame,
                diagnostics.resources.model_gpu_upload_bytes_this_frame as f64 / (1024.0 * 1024.0)
            ));
        }
        if let Some(warmup) = world.try_resource::<GpuResourceWarmup>() {
            let counts = warmup.counts();
            ui.label(&format!(
                "Generic GPU resources: queued {} | preparing {} | ready {} | failed {}",
                counts.queued, counts.preparing, counts.ready, counts.failed
            ));
        }
        if let Some(render) = world.try_resource::<RenderDiagnostics>() {
            ui.label(&format!(
                "Standalone GPU residency: textures {} ({} MiB est) | samplers {}",
                render.resources.live_streamed_textures,
                render.resources.streamed_texture_bytes_estimate / (1024 * 1024),
                render.resources.live_streamed_samplers
            ));
        }
        if let Some(render) = world.try_resource::<RenderDiagnostics>() {
            ui.label(&format!(
                "Generic GPU uploads: {} this frame ({} MiB est) | cumulative {} | failures {} | releases {}",
                render.resources.generic_gpu_uploads_this_frame,
                render.resources.generic_gpu_upload_bytes_this_frame / (1024 * 1024),
                render.resources.generic_gpu_uploads_cumulative,
                render.resources.generic_gpu_upload_failures_cumulative,
                render.resources.generic_gpu_releases_cumulative
            ));
        }

        if let Some(budget) = world.try_resource::<GpuUploadBudget>() {
            ui.label(&format!(
                "GPU warmup: max {} upload operations/frame | byte budget: {}",
                budget.max_models_per_frame,
                budget
                    .max_bytes_per_frame
                    .map(|bytes| format!("{} MiB", bytes / (1024 * 1024)))
                    .unwrap_or_else(|| "unbounded".to_owned())
            ));
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
                Box::new(StreamingPanel),
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
