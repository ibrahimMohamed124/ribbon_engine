//! Built-in hierarchy, reflected inspector, and runtime status panels.
//!
//! Inspector edits use `ribbon_commands` so they remain undoable.

use ribbon_app::{App, LogBuffer, ScheduleStats, Time};
use ribbon_assets::{AssetRef, AssetServer};
use ribbon_commands::{CommandStack, SetField};
use ribbon_ecs::{Entity, World};
use ribbon_editor::{EditorShell, Panel, Selection, Ui};
use ribbon_plugin_render3d::{GpuResourceWarmup, GpuUploadBudget};
use ribbon_plugin_streaming::{StreamingCoordinator, WorldPartitionAsset};
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
                "Pending requests: {} | memory pressure: {} (CPU {} / GPU {}) | invalid transitions: {}",
                diagnostics.pending_asset_requests,
                diagnostics.memory_pressure_events,
                diagnostics.cpu_region_eviction_events,
                diagnostics.gpu_region_eviction_events,
                diagnostics.invalid_state_transitions
            ));
            ui.label(&format!(
                "Slowest activation: {:.2} ms | last pressure region: {}",
                diagnostics.max_request_to_active_ns as f64 / 1_000_000.0,
                diagnostics
                    .last_memory_pressure_region
                    .as_ref()
                    .map_or("-", |region| region.as_str())
            ));
            ui.label(&format!(
                "Generic GPU residency: {} MiB ready + {} MiB queued | {} MiB over | evictions {} | rejected {}",
                diagnostics.generic_gpu_resident_bytes_estimate / (1024 * 1024),
                diagnostics.generic_gpu_queued_bytes_estimate / (1024 * 1024),
                diagnostics.generic_gpu_budget_overage_bytes / (1024 * 1024),
                diagnostics.generic_gpu_evictions_cumulative,
                diagnostics.generic_gpu_budget_rejections_cumulative
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
                let timing = region.lifecycle_timing;
                ui.label(&format!(
                    "  phases ms: request {:.2} | CPU {:.2} | GPU {:.2} | ready wait {:.2} | physics {:.2} | commit {:.2} | current {:?} {:.2}",
                    timing.requested_ns as f64 / 1_000_000.0,
                    timing.cpu_loading_ns as f64 / 1_000_000.0,
                    timing.gpu_warmup_ns as f64 / 1_000_000.0,
                    timing.ready_wait_ns as f64 / 1_000_000.0,
                    timing.physics_preparation_ns as f64 / 1_000_000.0,
                    timing.activation_ns as f64 / 1_000_000.0,
                    region.state,
                    timing.current_state_elapsed_ns as f64 / 1_000_000.0
                ));
                ui.label(&format!(
                    "  request->active last/max: {:.2}/{:.2} ms | activations {}",
                    timing.last_request_to_active_ns.unwrap_or(0) as f64 / 1_000_000.0,
                    timing.max_request_to_active_ns as f64 / 1_000_000.0,
                    timing.activation_count
                ));
                if let Some(error) = region.last_error {
                    ui.label(&format!("  error: {error}"));
                }
            }
            ui.label("Recent region lifecycle transitions (newest first):");
            for event in diagnostics.recent_events.iter().rev().take(8) {
                ui.label(&format!(
                    "  {}: {:?} -> {:?} in {:.2} ms | {}",
                    event.region.as_str(),
                    event.from,
                    event.to,
                    event.elapsed_ns as f64 / 1_000_000.0,
                    event.message
                ));
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
                "CPU categories (estimated MiB): geometry {:.1} | textures {:.1} | materials {:.1} | animation {:.1} | scene metadata {:.1} | other {:.1}",
                memory.geometry_bytes_estimate as f64 / (1024.0 * 1024.0),
                memory.texture_bytes_estimate as f64 / (1024.0 * 1024.0),
                memory.material_bytes_estimate as f64 / (1024.0 * 1024.0),
                memory.animation_bytes_estimate as f64 / (1024.0 * 1024.0),
                memory.scene_metadata_bytes_estimate as f64 / (1024.0 * 1024.0),
                memory.other_bytes_estimate as f64 / (1024.0 * 1024.0),
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
            ui.label(&format!(
                "glTF model cache: {:.1}/{:.1} MiB estimated | {:.1} MiB over | pressure level {} | pressure events {}",
                diagnostics.resources.model_gpu_cache_bytes_estimate as f64 / (1024.0 * 1024.0),
                diagnostics.resources.model_gpu_cache_budget_bytes as f64 / (1024.0 * 1024.0),
                diagnostics.resources.model_gpu_cache_over_budget_bytes as f64 / (1024.0 * 1024.0),
                diagnostics.resources.model_gpu_cache_pressure_level,
                diagnostics.resources.model_gpu_cache_pressure_events,
            ));
            ui.label(&format!(
                "Device recovery: {} losses | {} attempts | {} recovered | {} failed | recovery CPU payload {:.2} MiB",
                diagnostics.gpu_device_loss_events,
                diagnostics.gpu_device_recovery_attempts,
                diagnostics.gpu_device_recovery_successes,
                diagnostics.gpu_device_recovery_failures,
                diagnostics.resources.generic_gpu_recovery_payload_bytes_estimate as f64 / (1024.0 * 1024.0),
            ));
            if !diagnostics.gpu_last_device_loss_message.is_empty() {
                ui.label(&format!(
                    "Last device-loss reason: {}",
                    diagnostics.gpu_last_device_loss_message
                ));
            }
        }
        if let Some(warmup) = world.try_resource::<GpuResourceWarmup>() {
            let counts = warmup.counts();
            ui.label(&format!(
                "Generic GPU resources: queued {} | preparing {} | ready {} | failed {} | evicted {} | pressure {} | rejected {}",
                counts.queued, counts.preparing, counts.ready, counts.failed,
                counts.evicted_resources, counts.memory_pressure_events, counts.budget_rejections
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

/// Editor-side loader/registration state for an authored `.worldpart` document.
///
/// `asset_path` is public so projects can select their own source path when installing the
/// editor. The v1 editor UI deliberately exposes a load/register action rather than an in-place
/// text editor; the format itself is RON and can be edited in the project's normal source tools.
pub struct AuthoredPartitionEditorState {
    pub asset_path: String,
    lease: Option<AssetRef<WorldPartitionAsset>>,
    registered_generation: Option<(u64, u32)>,
    message: String,
}

impl Default for AuthoredPartitionEditorState {
    fn default() -> Self {
        Self {
            asset_path: "examples/streaming/starter.worldpart".to_owned(),
            lease: None,
            registered_generation: None,
            message: "Choose Load to request and register this authored partition".to_owned(),
        }
    }
}

/// Minimal authored-partition workflow: request a RON `.worldpart` through AssetServer, validate
/// and register the complete descriptor, then keep its lease pinned while the editor is open.
pub struct AuthoredPartitionPanel;
impl Panel for AuthoredPartitionPanel {
    fn title(&self) -> &str {
        "World Partition"
    }

    fn ui(&mut self, ui: &mut Ui, world: &mut World) {
        if world.try_resource::<AssetServer>().is_none()
            || world.try_resource::<StreamingCoordinator>().is_none()
        {
            ui.label("World Partition requires AssetServer and StreamingPlugin");
            return;
        }
        if world
            .try_resource::<AuthoredPartitionEditorState>()
            .is_none()
        {
            world.insert_resource(AuthoredPartitionEditorState::default());
        }

        let asset_path = world
            .resource::<AuthoredPartitionEditorState>()
            .asset_path
            .clone();
        ui.label(&format!("Authored RON asset: {asset_path}"));
        if ui.button("Load and register partition", 220) {
            let request = world.resource_scope::<AssetServer, _>(|_, server| {
                server.request_ref::<WorldPartitionAsset>(&asset_path)
            });
            match request {
                Ok(lease) => {
                    let state = world.resource_mut::<AuthoredPartitionEditorState>();
                    state.lease = Some(lease);
                    state.registered_generation = None;
                    state.message =
                        "Partition requested; waiting for AssetServer decode".to_owned();
                }
                Err(error) => {
                    world.resource_mut::<AuthoredPartitionEditorState>().message =
                        format!("Partition request failed: {error}");
                }
            }
        }

        let pending = world
            .resource::<AuthoredPartitionEditorState>()
            .lease
            .as_ref()
            .map(|lease| (lease.clone(), asset_path.clone()));
        let Some((lease, path)) = pending else {
            ui.label(&world.resource::<AuthoredPartitionEditorState>().message);
            ui.label("Edit the RON file in source control, then load it here.");
            return;
        };
        let handle = lease.handle();
        let marker = (handle.id(), handle.generation());
        let current = world
            .resource::<AssetServer>()
            .current_handle::<WorldPartitionAsset>(&path);
        if current.is_some_and(|current| current != handle) {
            world.resource_mut::<AuthoredPartitionEditorState>().message =
                "A newer partition generation exists; reload the editor session to replace already-registered region IDs safely".to_owned();
        } else if let Some(asset) = world.resource::<AssetServer>().get(handle).cloned() {
            let registered = world
                .resource::<AuthoredPartitionEditorState>()
                .registered_generation;
            if registered != Some(marker) {
                let result = world.resource_scope::<StreamingCoordinator, _>(|_, coordinator| {
                    coordinator.register_partition(&asset)
                });
                let state = world.resource_mut::<AuthoredPartitionEditorState>();
                match result {
                    Ok(regions) => {
                        state.registered_generation = Some(marker);
                        state.message = format!(
                            "Registered partition `{}` with {} regions (generation {})",
                            asset.id,
                            regions.len(),
                            handle.generation()
                        );
                    }
                    Err(error) => {
                        state.message =
                            format!("Partition validation/registration failed: {error}");
                    }
                }
            }
        } else {
            let status = world
                .resource::<AssetServer>()
                .state::<WorldPartitionAsset>(&path)
                .cloned();
            world.resource_mut::<AuthoredPartitionEditorState>().message = match status {
                Some(ribbon_assets::LoadState::Loading) => {
                    "Partition is loading on an AssetServer worker".to_owned()
                }
                Some(ribbon_assets::LoadState::Failed(error)) => {
                    format!("Partition decode failed: {error}")
                }
                Some(ribbon_assets::LoadState::Unloaded) => {
                    "Partition is unloaded; press Load to retry".to_owned()
                }
                Some(ribbon_assets::LoadState::Loaded) | None => {
                    "Partition handle is stale or no longer resident".to_owned()
                }
            };
        }

        ui.label(&world.resource::<AuthoredPartitionEditorState>().message);
        if let Some(snapshot) = world
            .try_resource::<StreamingCoordinator>()
            .map(StreamingCoordinator::snapshot)
        {
            ui.label(&format!(
                "Registered live partition regions: {} | active {} | loading {} | failed {}",
                snapshot.regions.len(),
                snapshot.diagnostics.active_regions,
                snapshot.diagnostics.loading_regions,
                snapshot.diagnostics.failed_regions
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
                Box::new(AuthoredPartitionPanel),
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
