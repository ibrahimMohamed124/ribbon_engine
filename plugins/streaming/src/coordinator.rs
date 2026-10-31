use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::time::Instant;

use glam::Vec3;
use ribbon_app::{App, Stage, World};
use ribbon_assets::{AssetRef, AssetServer, LoadErrorKind, LoadState};
use ribbon_ecs::Entity;
use ribbon_plugin_gltf::ModelAsset;
use ribbon_plugin_physics::{Collider, PhysicsDebugData};
use ribbon_plugin_render3d::{ModelInstance, ModelWarmup, ModelWarmupStatus, PointLight};
use ribbon_plugins::Plugin;
use ribbon_reflect::TypeRegistry;
use ribbon_render::profiling::{RenderDiagnostics, RenderProfileRecorder};
use ribbon_scene::{
    instantiate_with_context, HierarchyExt, PrefabCodecRegistry, PrefabLoadContext, SceneDoc,
    Transform3D,
};

use crate::{
    RegionBounds, StreamingAssetAdapters, StreamingDependencyState, WorldPartitionAsset,
    WorldPartitionIndex,
};

const RECENT_EVENT_CAPACITY: usize = 128;

type ModelKey = (u64, u32);

/// Stable, authored identity for one streamable world region.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(String);

impl RegionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for RegionId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for RegionId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Stable identity for a logical scene composed from one or more streamable regions.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneId(String);

impl SceneId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SceneId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SceneId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Logical scene made from already-authored regions. Each region keeps its own asset, GPU, and
/// physics lifecycle; this descriptor provides one scene-level readiness/activation contract.
#[derive(Clone, Debug)]
pub struct SceneDefinition {
    pub id: SceneId,
    pub name: String,
    pub regions: Vec<RegionId>,
}

impl SceneDefinition {
    pub fn new<I, R>(id: impl Into<SceneId>, name: impl Into<String>, regions: I) -> Self
    where
        I: IntoIterator<Item = R>,
        R: Into<RegionId>,
    {
        Self {
            id: id.into(),
            name: name.into(),
            regions: regions.into_iter().map(Into::into).collect(),
        }
    }
}

/// Aggregated lifecycle state for a scene made from multiple regions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneState {
    Unloaded,
    Requested,
    Loading,
    Ready,
    Activating,
    Active,
    Dormant,
    Failed(Vec<RegionId>),
}

#[derive(Clone, Debug)]
pub struct SceneSnapshot {
    pub id: SceneId,
    pub name: String,
    pub state: SceneState,
    pub region_count: usize,
    pub ready_region_count: usize,
    pub active_region_count: usize,
    pub prefab_entity_count: usize,
    pub failures: Vec<(RegionId, String)>,
}

struct SceneRecord {
    definition: SceneDefinition,
}

/// One renderable model placement owned by a region after activation.
#[derive(Clone, Debug)]
pub struct RegionModel {
    pub asset_path: String,
    pub transform: Transform3D,
}

impl RegionModel {
    pub fn new(path: impl Into<String>, transform: Transform3D) -> Self {
        Self {
            asset_path: path.into(),
            transform,
        }
    }
}

/// Physics descriptor registered before a region becomes active.
#[derive(Clone, Copy, Debug)]
pub struct RegionCollider {
    pub transform: Transform3D,
    pub collider: Collider,
}

impl RegionCollider {
    pub fn new(transform: Transform3D, collider: Collider) -> Self {
        Self {
            transform,
            collider,
        }
    }
}

/// Renderer-independent region light descriptor. The renderer plugin owns the `PointLight`
/// component; the streaming coordinator only tracks the entity lifetime with its region.
#[derive(Clone, Copy, Debug)]
pub struct RegionPointLight {
    pub light: PointLight,
}

impl RegionPointLight {
    pub fn new(light: PointLight) -> Self {
        Self { light }
    }
}

/// Per-region distance policy. Unload distance must be >= preload distance >= activation distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionPolicy {
    pub preload_distance: f32,
    pub activation_distance: f32,
    pub unload_distance: f32,
    pub base_priority: i32,
    /// When false the region can be explicitly prepared and inspected without entering gameplay.
    pub auto_activate: bool,
    /// Spawn render-only placements after GPU readiness but before collision/gameplay activation.
    /// This supports doorway previews while retaining an old active region during the handoff.
    pub preview_when_ready: bool,
}

impl Default for RegionPolicy {
    fn default() -> Self {
        Self {
            preload_distance: 400.0,
            activation_distance: 180.0,
            unload_distance: 520.0,
            base_priority: 0,
            auto_activate: true,
            preview_when_ready: false,
        }
    }
}

/// Declarative scene-region description. Model assets are requested by typed path and shared with
/// other regions through AssetServer deduplication. Region colliders are not inserted until the
/// region is ready to activate.
#[derive(Clone, Debug)]
pub struct RegionDefinition {
    pub id: RegionId,
    pub name: String,
    pub bounds: RegionBounds,
    pub models: Vec<RegionModel>,
    /// Optional asynchronously loaded `.scenedoc` whose prefab entities are owned by this region.
    /// Its required typed asset dependencies are discovered through registered prefab codecs.
    pub scene_doc_path: Option<String>,
    pub colliders: Vec<RegionCollider>,
    pub point_lights: Vec<RegionPointLight>,
    pub dependencies: Vec<RegionId>,
    pub neighbors: Vec<RegionId>,
    pub policy: RegionPolicy,
}

impl RegionDefinition {
    pub fn new(id: impl Into<RegionId>, name: impl Into<String>, bounds: RegionBounds) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            bounds,
            models: Vec::new(),
            scene_doc_path: None,
            colliders: Vec::new(),
            point_lights: Vec::new(),
            dependencies: Vec::new(),
            neighbors: Vec::new(),
            policy: RegionPolicy::default(),
        }
    }

    pub fn with_model(mut self, model: RegionModel) -> Self {
        self.models.push(model);
        self
    }

    /// Attach a serialized scene document. Decoding and declared dependencies remain asynchronous;
    /// the prefab is instantiated only after its required resources are ready.
    pub fn with_scene_doc(mut self, path: impl Into<String>) -> Self {
        self.scene_doc_path = Some(path.into());
        self
    }

    pub fn with_collider(mut self, collider: RegionCollider) -> Self {
        self.colliders.push(collider);
        self
    }

    pub fn with_point_light(mut self, light: RegionPointLight) -> Self {
        self.point_lights.push(light);
        self
    }

    pub fn with_policy(mut self, policy: RegionPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_dependencies(mut self, dependencies: impl IntoIterator<Item = RegionId>) -> Self {
        self.dependencies = dependencies.into_iter().collect();
        self
    }

    pub fn with_neighbors(mut self, neighbors: impl IntoIterator<Item = RegionId>) -> Self {
        self.neighbors = neighbors.into_iter().collect();
        self
    }
}

/// Read-only lifecycle state visible to gameplay and editor tools.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionState {
    Unloaded,
    Requested,
    LoadingAssets,
    WarmingGpu,
    Ready,
    PreparingPhysics,
    Activating,
    Active,
    Deactivating,
    Dormant,
    Unloading,
    Failed(String),
}

/// Observer used for distance-based streaming. World space follows Ribbon's right-handed, Y-up convention.
/// Velocity is optional and only biases speculative request priority; correctness never depends on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamObserver {
    pub id: u64,
    pub position: Vec3,
    pub velocity: Vec3,
}

impl StreamObserver {
    pub fn new(id: u64, position: Vec3) -> Self {
        Self {
            id,
            position,
            velocity: Vec3::ZERO,
        }
    }

    pub fn with_velocity(mut self, velocity: Vec3) -> Self {
        self.velocity = if velocity.is_finite() && velocity.length_squared().is_finite() {
            velocity
        } else {
            Vec3::ZERO
        };
        self
    }
}

/// CPU/GPU residency policy. The effective unleased CPU-cache target is the minimum of
/// `cpu_cache_bytes` and the remaining `cpu_resident_bytes` allowance after pinned assets. Total
/// CPU pressure may additionally release safe dormant region leases. The GPU target uses renderer-
/// tracked estimates and releases only dormant region warmups. None can invalidate active or
/// shared ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamingMemoryBudget {
    /// Upper bound for unleased AssetServer cache entries. The effective value is reduced when
    /// the total CPU residency target leaves less room after pinned assets are accounted for.
    /// Pinned assets are never invalidated to satisfy this cache target.
    pub cpu_cache_bytes: Option<u64>,
    /// Target for total decoded CPU residency. The coordinator may release safe dormant regions;
    /// active, requested, shared-only, or in-range resources can keep the working set above it.
    pub cpu_resident_bytes: Option<u64>,
    /// Soft target for renderer-reported GPU allocation estimates. Values are not driver VRAM data.
    pub gpu_estimate_bytes: Option<u64>,
}

impl Default for StreamingMemoryBudget {
    fn default() -> Self {
        Self {
            cpu_cache_bytes: Some(512 * 1024 * 1024),
            cpu_resident_bytes: Some(2 * 1024 * 1024 * 1024),
            gpu_estimate_bytes: Some(1024 * 1024 * 1024),
        }
    }
}

/// Work and memory limits applied by the streaming coordinator.
#[derive(Clone, Debug)]
pub struct StreamingConfig {
    pub memory: StreamingMemoryBudget,
    pub spatial_cell_size: f32,
    pub max_new_region_requests_per_update: usize,
    /// Maximum number of unique typed asset paths submitted to AssetServer per Update.
    pub max_asset_requests_per_update: usize,
    pub max_region_activations_per_update: usize,
    /// Maximum number of streamed collider/model/light entities spawned during one Update.
    pub max_entity_spawns_per_update: usize,
    pub max_active_regions: usize,
    pub profile_cpu: bool,
    pub prefetch_neighbors: bool,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            memory: StreamingMemoryBudget::default(),
            spatial_cell_size: 256.0,
            max_new_region_requests_per_update: 4,
            max_asset_requests_per_update: 64,
            max_region_activations_per_update: 2,
            max_entity_spawns_per_update: 256,
            max_active_regions: 64,
            profile_cpu: false,
            prefetch_neighbors: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RegionSnapshot {
    pub id: RegionId,
    pub name: String,
    pub bounds: RegionBounds,
    pub policy: RegionPolicy,
    pub state: RegionState,
    pub distance_to_nearest_observer: Option<f32>,
    pub scheduling_priority: i32,
    pub model_count: usize,
    pub has_scene_doc: bool,
    pub prefab_entity_count: usize,
    /// Number of additional typed CPU dependencies retained from the SceneDoc codecs.
    pub typed_dependency_count: usize,
    pub point_light_count: usize,
    pub collider_count: usize,
    /// Number of ECS entities currently owned by the region (including render-only previews).
    pub active_entity_count: usize,
    /// Number of model/light preview or active render entities already spawned.
    pub visual_entity_count: usize,
    pub spawned_model_instances: usize,
    pub resident_cpu_bytes_estimate: u64,
    pub cpu_ready_models: usize,
    pub gpu_ready_models: usize,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamEvent {
    pub region: RegionId,
    pub from: RegionState,
    pub to: RegionState,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct StreamDiagnostics {
    pub region_count: usize,
    pub loading_regions: usize,
    pub ready_regions: usize,
    pub active_regions: usize,
    pub dormant_regions: usize,
    pub failed_regions: usize,
    pub queued_asset_jobs: usize,
    pub asset_queue_capacity: usize,
    /// Region asset paths that have not yet been submitted because of the per-update budget.
    pub pending_asset_requests: usize,
    pub active_asset_workers: usize,
    pub asset_worker_count: usize,
    pub cpu_resident_bytes_estimate: u64,
    pub cpu_pinned_bytes_estimate: u64,
    pub cpu_unpinned_bytes_estimate: u64,
    pub gpu_resident_bytes_estimate: u64,
    /// Current effective AssetServer unpinned-cache budget; zero is reported both for a disabled
    /// budget and for an explicitly configured zero-byte budget.
    pub cpu_unpinned_cache_budget_bytes: u64,
    /// Unleased decoded cache bytes above the effective AssetServer budget.
    pub cpu_memory_over_budget_bytes: u64,
    /// Total decoded CPU bytes above `cpu_resident_bytes` (streaming residency policy).
    pub cpu_resident_over_budget_bytes: u64,
    /// Configured total CPU residency target; zero is reported both when disabled and for `Some(0)`.
    pub cpu_resident_budget_bytes: u64,
    /// Renderer-tracked GPU estimate above `gpu_estimate_bytes`.
    pub gpu_memory_over_budget_bytes: u64,
    /// Cumulative CPU- or GPU-pressure region releases, not physical bytes freed.
    pub memory_pressure_events: u64,
    pub cpu_budget_evictions_cumulative: u64,
    pub invalid_state_transitions: u64,
    pub region_transitions: u64,
    pub last_update_cpu_ns: u64,
    pub last_update_profiled: bool,
    pub recent_events: VecDeque<StreamEvent>,
}

#[derive(Clone, Debug, Default)]
pub struct StreamingSnapshot {
    pub scenes: Vec<SceneSnapshot>,
    pub regions: Vec<RegionSnapshot>,
    pub observers: Vec<StreamObserver>,
    pub diagnostics: StreamDiagnostics,
}

enum VisualSpawnItem {
    Model(RegionModel, ribbon_assets::Handle<ModelAsset>),
    PointLight(PointLight),
}

struct RegionRecord {
    definition: RegionDefinition,
    state: RegionState,
    leases: BTreeMap<String, AssetRef<ModelAsset>>,
    scene_doc_lease: Option<AssetRef<SceneDoc>>,
    scene_doc_dependencies_resolved: bool,
    scene_doc_instantiated: bool,
    prefab_entities: Vec<Entity>,
    /// Type-erased strong leases for required non-ModelAsset dependencies declared by SceneDoc codecs.
    typed_dependency_leases: BTreeMap<(String, String), Box<dyn Any + Send + Sync>>,
    pending_typed_dependency_paths: Vec<(String, String)>,
    next_typed_dependency_request: usize,
    pending_asset_paths: Vec<String>,
    next_asset_request: usize,
    visual_entities: Vec<Entity>,
    collider_entities: Vec<Entity>,
    pending_collider_cleanup: Vec<Entity>,
    pending_collider_spawn: usize,
    pending_visual_spawn: usize,
    manual_requested: bool,
    forced_activation: bool,
    scene_request_users: BTreeSet<SceneId>,
    scene_activation_users: BTreeSet<SceneId>,
    requested_dependencies: bool,
    dependency_users: usize,
    last_distance: Option<f32>,
    scheduling_priority: i32,
    last_used_sequence: u64,
    resident_cpu_bytes_estimate: u64,
    cpu_ready_models: usize,
    gpu_ready_models: usize,
}

impl RegionRecord {
    fn has_request_intent(&self) -> bool {
        self.manual_requested || !self.scene_request_users.is_empty() || self.dependency_users > 0
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct RegionInterest {
    preload: bool,
    activate: bool,
    beyond_unload: bool,
    nearest_distance: Option<f32>,
    priority: i32,
}

/// Engine-level coordinator. It owns region metadata and CPU leases, not asset values, GPU objects,
/// scene transform propagation, or Rapier state.
pub struct StreamingCoordinator {
    config: StreamingConfig,
    index: WorldPartitionIndex,
    regions: BTreeMap<RegionId, RegionRecord>,
    scenes: BTreeMap<SceneId, SceneRecord>,
    observers: BTreeMap<u64, StreamObserver>,
    warmup_users: HashMap<ModelKey, (ribbon_assets::Handle<ModelAsset>, usize)>,
    next_use_sequence: u64,
    diagnostics: StreamDiagnostics,
}

impl Default for StreamingCoordinator {
    fn default() -> Self {
        Self::new(StreamingConfig::default())
    }
}

impl StreamingCoordinator {
    pub fn new(mut config: StreamingConfig) -> Self {
        let index = WorldPartitionIndex::new(config.spatial_cell_size);
        // Keep the public configuration consistent with the sanitized spatial index value.
        config.spatial_cell_size = index.cell_size();
        Self {
            index,
            config,
            regions: BTreeMap::new(),
            scenes: BTreeMap::new(),
            observers: BTreeMap::new(),
            warmup_users: HashMap::new(),
            next_use_sequence: 1,
            diagnostics: StreamDiagnostics::default(),
        }
    }

    pub fn config(&self) -> &StreamingConfig {
        &self.config
    }

    /// Update bounded per-frame work budgets without rebuilding the spatial index or resetting state.
    pub fn set_frame_budgets(
        &mut self,
        new_requests: usize,
        region_activations: usize,
        entity_spawns: usize,
    ) {
        self.config.max_new_region_requests_per_update = new_requests;
        self.config.max_region_activations_per_update = region_activations;
        self.config.max_entity_spawns_per_update = entity_spawns;
    }

    /// Change the maximum number of unique model paths submitted to AssetServer per Update.
    /// A value of zero pauses new submissions while already submitted jobs continue normally.
    pub fn set_asset_request_budget(&mut self, requests: usize) {
        self.config.max_asset_requests_per_update = requests;
    }

    pub fn register_region(&mut self, mut definition: RegionDefinition) -> Result<(), String> {
        if definition.id.as_str().trim().is_empty() {
            return Err("region ID must not be empty".to_owned());
        }
        if self.regions.contains_key(&definition.id) {
            return Err(format!(
                "region `{}` is already registered",
                definition.id.as_str()
            ));
        }
        validate_policy(definition.policy)?;
        if definition.name.trim().is_empty() {
            return Err("region name must not be empty".to_owned());
        }
        if definition
            .scene_doc_path
            .as_ref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err("region scene document path must not be empty".to_owned());
        }
        if !definition.bounds.min.is_finite()
            || !definition.bounds.max.is_finite()
            || definition.bounds.min.x > definition.bounds.max.x
            || definition.bounds.min.y > definition.bounds.max.y
            || definition.bounds.min.z > definition.bounds.max.z
        {
            return Err("region bounds must be finite and ordered minimum-to-maximum".to_owned());
        }
        for model in &mut definition.models {
            if model.asset_path.trim().is_empty() || !valid_transform(model.transform) {
                return Err(format!(
                    "region `{}` contains an invalid model path or transform",
                    definition.id.as_str()
                ));
            }
            // Store a canonical unit quaternion so scene propagation and physics/render extraction
            // receive the same rotation representation, not merely a transform that can be fixed
            // later by one consumer.
            model.transform.rotation = model.transform.rotation.normalize();
        }
        for collider in &mut definition.colliders {
            if !valid_transform(collider.transform) {
                return Err(format!(
                    "region `{}` contains an invalid collider transform",
                    definition.id.as_str()
                ));
            }
            collider.transform.rotation = collider.transform.rotation.normalize();
            collider
                .collider
                .validate_for_scale(collider.transform.scale)
                .map_err(|message| {
                    format!(
                        "region `{}` has invalid collider: {message}",
                        definition.id.as_str()
                    )
                })?;
        }
        for point_light in &definition.point_lights {
            let light = point_light.light;
            if !light.position.is_finite()
                || !light.color.is_finite()
                || light.color.min_element() < 0.0
                || !light.intensity.is_finite()
                || light.intensity < 0.0
                || !light.range.is_finite()
                || light.range <= 0.0
            {
                return Err(format!(
                    "region `{}` contains an invalid point light",
                    definition.id.as_str()
                ));
            }
        }
        let mut unique_dependencies = HashSet::new();
        for dependency in &definition.dependencies {
            if dependency == &definition.id {
                return Err("a region cannot depend on itself".to_owned());
            }
            if !self.regions.contains_key(dependency) {
                return Err(format!(
                    "dependency region `{}` must be registered first",
                    dependency.as_str()
                ));
            }
            if !unique_dependencies.insert(dependency.clone()) {
                return Err(format!("duplicate dependency `{}`", dependency.as_str()));
            }
        }
        self.index
            .insert(definition.id.clone(), definition.bounds)
            .map_err(str::to_owned)?;
        let id = definition.id.clone();
        self.regions.insert(
            id,
            RegionRecord {
                definition,
                state: RegionState::Unloaded,
                leases: BTreeMap::new(),
                scene_doc_lease: None,
                scene_doc_dependencies_resolved: false,
                scene_doc_instantiated: false,
                prefab_entities: Vec::new(),
                typed_dependency_leases: BTreeMap::new(),
                pending_typed_dependency_paths: Vec::new(),
                next_typed_dependency_request: 0,
                pending_asset_paths: Vec::new(),
                next_asset_request: 0,
                visual_entities: Vec::new(),
                collider_entities: Vec::new(),
                pending_collider_cleanup: Vec::new(),
                pending_collider_spawn: 0,
                pending_visual_spawn: 0,
                manual_requested: false,
                forced_activation: false,
                scene_request_users: BTreeSet::new(),
                scene_activation_users: BTreeSet::new(),
                requested_dependencies: false,
                dependency_users: 0,
                last_distance: None,
                scheduling_priority: 0,
                last_used_sequence: 0,
                resident_cpu_bytes_estimate: 0,
                cpu_ready_models: 0,
                gpu_ready_models: 0,
            },
        );
        Ok(())
    }

    /// Register all regions from an authored partition. Region dependencies are topologically
    /// ordered; unknown references and cycles are rejected before mutating the coordinator.
    pub fn register_partition(
        &mut self,
        partition: &WorldPartitionAsset,
    ) -> Result<Vec<RegionId>, String> {
        let definitions = partition.region_definitions()?;
        let partition_ids: HashSet<_> = definitions
            .iter()
            .map(|definition| definition.id.clone())
            .collect();
        for definition in &definitions {
            if self.regions.contains_key(&definition.id) {
                return Err(format!(
                    "world partition `{}` contains region `{}` which is already registered",
                    partition.id,
                    definition.id.as_str()
                ));
            }
            validate_policy(definition.policy)?;
            let mut unique_dependencies = HashSet::new();
            for dependency in &definition.dependencies {
                if dependency == &definition.id {
                    return Err(format!(
                        "region `{}` cannot depend on itself",
                        definition.id.as_str()
                    ));
                }
                if !unique_dependencies.insert(dependency) {
                    return Err(format!(
                        "region `{}` contains duplicate dependency `{}`",
                        definition.id.as_str(),
                        dependency.as_str()
                    ));
                }
                if !partition_ids.contains(dependency) && !self.regions.contains_key(dependency) {
                    return Err(format!(
                        "region `{}` depends on unknown region `{}`",
                        definition.id.as_str(),
                        dependency.as_str()
                    ));
                }
            }
            let mut unique_neighbors = HashSet::new();
            for neighbor in &definition.neighbors {
                if !unique_neighbors.insert(neighbor) {
                    return Err(format!(
                        "region `{}` contains duplicate neighbor `{}`",
                        definition.id.as_str(),
                        neighbor.as_str()
                    ));
                }
                if !partition_ids.contains(neighbor) && !self.regions.contains_key(neighbor) {
                    return Err(format!(
                        "region `{}` names unknown neighbor `{}`",
                        definition.id.as_str(),
                        neighbor.as_str()
                    ));
                }
            }
        }

        let mut pending: BTreeMap<RegionId, RegionDefinition> = definitions
            .into_iter()
            .map(|definition| (definition.id.clone(), definition))
            .collect();
        let mut registered: HashSet<_> = self.regions.keys().cloned().collect();
        let mut order = Vec::with_capacity(pending.len());
        while !pending.is_empty() {
            let next = pending
                .iter()
                .find(|(_, definition)| {
                    definition
                        .dependencies
                        .iter()
                        .all(|dependency| registered.contains(dependency))
                })
                .map(|(id, _)| id.clone());
            let Some(id) = next else {
                let blocked = pending
                    .values()
                    .map(|definition| definition.id.as_str().to_owned())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "world partition `{}` contains a region dependency cycle among: {blocked}",
                    partition.id
                ));
            };
            let definition = pending.remove(&id).expect("topological candidate exists");
            // All validation and reference resolution was performed above, so this registration
            // cannot leave a partially accepted partition for malformed authored input.
            self.register_region(definition)?;
            registered.insert(id.clone());
            order.push(id);
        }
        Ok(order)
    }

    /// Register a logical scene composed from regions already registered with this coordinator.
    /// The operation is atomic: invalid/duplicate regions do not leave a partial scene entry.
    pub fn register_scene(&mut self, mut definition: SceneDefinition) -> Result<(), String> {
        if definition.id.as_str().trim().is_empty() {
            return Err("scene ID must not be empty".to_owned());
        }
        if self.scenes.contains_key(&definition.id) {
            return Err(format!(
                "scene `{}` is already registered",
                definition.id.as_str()
            ));
        }
        if definition.name.trim().is_empty() {
            return Err("scene name must not be empty".to_owned());
        }
        if definition.regions.is_empty() {
            return Err("a streamable scene must contain at least one region".to_owned());
        }
        let mut unique = BTreeSet::new();
        for region in &definition.regions {
            if !self.regions.contains_key(region) {
                return Err(format!(
                    "scene `{}` references unknown region `{}`",
                    definition.id.as_str(),
                    region.as_str()
                ));
            }
            if !unique.insert(region.clone()) {
                return Err(format!(
                    "scene `{}` contains duplicate region `{}`",
                    definition.id.as_str(),
                    region.as_str()
                ));
            }
        }
        // Canonical ordering makes snapshots deterministic regardless of authoring order.
        definition.regions.sort();
        self.scenes
            .insert(definition.id.clone(), SceneRecord { definition });
        Ok(())
    }

    /// Request all regions belonging to a scene. Region-level priority, dependency, and distance
    /// rules remain authoritative; the scene only aggregates their lifecycle.
    pub fn request_scene(&mut self, id: &SceneId) -> Result<(), String> {
        let regions = self
            .scenes
            .get(id)
            .map(|scene| scene.definition.regions.clone())
            .ok_or_else(|| format!("unknown scene `{}`", id.as_str()))?;
        for region in regions {
            let newly_requested = {
                let record = self
                    .regions
                    .get_mut(&region)
                    .ok_or_else(|| format!("unknown region `{}`", region.as_str()))?;
                record.scene_request_users.insert(id.clone());
                record.state == RegionState::Unloaded
            };
            if newly_requested {
                self.set_state(
                    &region,
                    RegionState::Requested,
                    "logical scene request accepted",
                );
            }
        }
        Ok(())
    }

    /// Request and force activation of every region in the logical scene. Readiness still waits
    /// for each region's CPU assets, GPU warmup, and physics preparation.
    pub fn activate_scene(&mut self, id: &SceneId) -> Result<(), String> {
        self.request_scene(id)?;
        let regions = self
            .scenes
            .get(id)
            .map(|scene| scene.definition.regions.clone())
            .ok_or_else(|| format!("unknown scene `{}`", id.as_str()))?;
        for region in regions {
            let record = self
                .regions
                .get_mut(&region)
                .ok_or_else(|| format!("unknown region `{}`", region.as_str()))?;
            record.scene_activation_users.insert(id.clone());
        }
        Ok(())
    }

    /// Release the explicit scene request. Observers, neighboring regions, and other dependents
    /// may still keep individual regions resident.
    pub fn release_scene_request(&mut self, id: &SceneId) -> Result<(), String> {
        let regions = self
            .scenes
            .get(id)
            .map(|scene| scene.definition.regions.clone())
            .ok_or_else(|| format!("unknown scene `{}`", id.as_str()))?;
        for region in regions {
            let record = self
                .regions
                .get_mut(&region)
                .ok_or_else(|| format!("unknown region `{}`", region.as_str()))?;
            record.scene_request_users.remove(id);
            record.scene_activation_users.remove(id);
        }
        Ok(())
    }

    /// Retry every failed region in a scene after its own physics cleanup has completed.
    /// Returns the number of regions whose retry was accepted.
    pub fn retry_scene(&mut self, id: &SceneId) -> Result<usize, String> {
        let regions = self
            .scenes
            .get(id)
            .map(|scene| scene.definition.regions.clone())
            .ok_or_else(|| format!("unknown scene `{}`", id.as_str()))?;
        let failed: Vec<_> = regions
            .into_iter()
            .filter(|region| {
                matches!(
                    self.regions.get(region).map(|record| &record.state),
                    Some(RegionState::Failed(_))
                )
            })
            .collect();
        for region in &failed {
            self.retry_region_internal(region, false)?;
        }
        Ok(failed.len())
    }

    /// Aggregate state for all regions in a scene. Ready means every region is Ready; Active
    /// means every region has completed physics activation. Partial activation/teardown reports
    /// Activating so callers never treat a mixed scene as fully ready or fully active.
    pub fn scene_state(&self, id: &SceneId) -> Option<SceneState> {
        let scene = self.scenes.get(id)?;
        let states: Vec<_> = scene
            .definition
            .regions
            .iter()
            .filter_map(|region| {
                self.regions
                    .get(region)
                    .map(|record| (region.clone(), record.state.clone()))
            })
            .collect();
        if states.len() != scene.definition.regions.len() {
            return Some(SceneState::Failed(Vec::new()));
        }
        let failed: Vec<_> = states
            .iter()
            .filter_map(|(region, state)| {
                if matches!(state, RegionState::Failed(_)) {
                    Some(region.clone())
                } else {
                    None
                }
            })
            .collect();
        if !failed.is_empty() {
            return Some(SceneState::Failed(failed));
        }
        if states
            .iter()
            .all(|(_, state)| *state == RegionState::Active)
        {
            return Some(SceneState::Active);
        }
        if states.iter().any(|(_, state)| {
            *state == RegionState::Active
                || matches!(
                    state,
                    RegionState::PreparingPhysics
                        | RegionState::Activating
                        | RegionState::Deactivating
                        | RegionState::Unloading
                )
        }) {
            return Some(SceneState::Activating);
        }
        if states.iter().all(|(_, state)| *state == RegionState::Ready) {
            return Some(SceneState::Ready);
        }
        if states
            .iter()
            .all(|(_, state)| *state == RegionState::Unloaded)
        {
            return Some(SceneState::Unloaded);
        }
        if states
            .iter()
            .all(|(_, state)| matches!(state, RegionState::Dormant | RegionState::Unloaded))
        {
            return Some(SceneState::Dormant);
        }
        if states
            .iter()
            .any(|(_, state)| matches!(state, RegionState::LoadingAssets | RegionState::WarmingGpu))
        {
            return Some(SceneState::Loading);
        }
        if states
            .iter()
            .any(|(_, state)| *state == RegionState::Requested)
        {
            return Some(SceneState::Requested);
        }
        Some(SceneState::Loading)
    }

    pub fn scene_snapshots(&self) -> Vec<SceneSnapshot> {
        self.scenes
            .values()
            .map(|scene| {
                let mut ready_region_count = 0;
                let mut active_region_count = 0;
                let mut prefab_entity_count: usize = 0;
                let mut failures = Vec::new();
                for id in &scene.definition.regions {
                    if let Some(record) = self.regions.get(id) {
                        prefab_entity_count =
                            prefab_entity_count.saturating_add(record.prefab_entities.len());
                        if record.state == RegionState::Ready || record.state == RegionState::Active
                        {
                            ready_region_count += 1;
                        }
                        if record.state == RegionState::Active {
                            active_region_count += 1;
                        }
                        if let RegionState::Failed(message) = &record.state {
                            failures.push((id.clone(), message.clone()));
                        }
                    }
                }
                SceneSnapshot {
                    id: scene.definition.id.clone(),
                    name: scene.definition.name.clone(),
                    state: self
                        .scene_state(&scene.definition.id)
                        .unwrap_or(SceneState::Unloaded),
                    region_count: scene.definition.regions.len(),
                    ready_region_count,
                    active_region_count,
                    prefab_entity_count,
                    failures,
                }
            })
            .collect()
    }

    pub fn add_observer(&mut self, observer: StreamObserver) -> Result<(), String> {
        if !observer.position.is_finite() {
            return Err("stream observer position must be finite".to_owned());
        }
        let observer = observer.with_velocity(observer.velocity);
        self.observers.insert(observer.id, observer);
        Ok(())
    }

    pub fn update_observer(&mut self, observer: StreamObserver) -> Result<(), String> {
        self.add_observer(observer)
    }

    pub fn observer(&self, id: u64) -> Option<StreamObserver> {
        self.observers.get(&id).copied()
    }

    pub fn remove_observer(&mut self, id: u64) -> bool {
        self.observers.remove(&id).is_some()
    }

    /// Request a region even when no observer is currently near it. The request is deduplicated.
    pub fn request_region(&mut self, id: &RegionId) -> Result<(), String> {
        let record = self
            .regions
            .get_mut(id)
            .ok_or_else(|| format!("unknown region `{}`", id.as_str()))?;
        record.manual_requested = true;
        if record.state == RegionState::Unloaded {
            self.set_state(
                id,
                RegionState::Requested,
                "explicit region request accepted",
            );
        }
        Ok(())
    }

    /// Force activation after CPU, GPU and physics preparation have completed.
    pub fn activate_region(&mut self, id: &RegionId) -> Result<(), String> {
        self.request_region(id)?;
        if let Some(record) = self.regions.get_mut(id) {
            record.forced_activation = true;
        }
        Ok(())
    }

    /// Stop the explicit request. Distance observers or dependent regions may still retain it.
    pub fn release_region_request(&mut self, id: &RegionId) -> Result<(), String> {
        let record = self
            .regions
            .get_mut(id)
            .ok_or_else(|| format!("unknown region `{}`", id.as_str()))?;
        record.manual_requested = false;
        record.forced_activation = false;
        Ok(())
    }

    /// Retry a failed region after any old physics handles have been reconciled away.
    pub fn retry_region(&mut self, id: &RegionId) -> Result<(), String> {
        self.retry_region_internal(id, true)
    }

    fn retry_region_internal(
        &mut self,
        id: &RegionId,
        explicit_request: bool,
    ) -> Result<(), String> {
        let record = self
            .regions
            .get_mut(id)
            .ok_or_else(|| format!("unknown region `{}`", id.as_str()))?;
        if !matches!(&record.state, RegionState::Failed(_)) {
            return Err("only a failed region can be retried".to_owned());
        }
        if !record.pending_collider_cleanup.is_empty() {
            return Err(
                "region cleanup is still waiting for the physics reconciliation stage".to_owned(),
            );
        }
        if explicit_request {
            record.manual_requested = true;
        }
        record.resident_cpu_bytes_estimate = 0;
        record.cpu_ready_models = 0;
        record.gpu_ready_models = 0;
        record.pending_collider_spawn = 0;
        record.pending_visual_spawn = 0;
        self.set_state(
            id,
            RegionState::Unloaded,
            "failed region retry accepted after cleanup",
        );
        Ok(())
    }

    pub fn region_state(&self, id: &RegionId) -> Option<&RegionState> {
        self.regions.get(id).map(|record| &record.state)
    }

    pub fn model_handle(
        &self,
        id: &RegionId,
        path: &str,
    ) -> Option<ribbon_assets::Handle<ModelAsset>> {
        self.regions.get(id)?.leases.get(path).map(AssetRef::handle)
    }

    pub fn snapshot(&self) -> StreamingSnapshot {
        StreamingSnapshot {
            scenes: self.scene_snapshots(),
            regions: self
                .regions
                .values()
                .map(|record| RegionSnapshot {
                    id: record.definition.id.clone(),
                    name: record.definition.name.clone(),
                    bounds: record.definition.bounds,
                    policy: record.definition.policy,
                    state: record.state.clone(),
                    distance_to_nearest_observer: record.last_distance,
                    scheduling_priority: record.scheduling_priority,
                    model_count: record.definition.models.len(),
                    has_scene_doc: record.definition.scene_doc_path.is_some(),
                    prefab_entity_count: record.prefab_entities.len(),
                    typed_dependency_count: record.typed_dependency_leases.len(),
                    point_light_count: record.definition.point_lights.len(),
                    collider_count: record.definition.colliders.len(),
                    active_entity_count: record.visual_entities.len()
                        + record.collider_entities.len()
                        + record.prefab_entities.len(),
                    visual_entity_count: record.visual_entities.len(),
                    spawned_model_instances: record
                        .pending_visual_spawn
                        .min(record.definition.models.len()),
                    resident_cpu_bytes_estimate: record.resident_cpu_bytes_estimate,
                    cpu_ready_models: record.cpu_ready_models,
                    gpu_ready_models: record.gpu_ready_models,
                    last_error: match &record.state {
                        RegionState::Failed(message) => Some(message.clone()),
                        _ => None,
                    },
                })
                .collect(),
            observers: self.observers.values().copied().collect(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    pub fn diagnostics(&self) -> &StreamDiagnostics {
        &self.diagnostics
    }

    fn tick(&mut self, world: &mut World) {
        let started = (self.config.profile_cpu
            || world.try_resource::<RenderProfileRecorder>().is_some())
        .then(Instant::now);

        world.resource_scope::<AssetServer, _>(|_, server| {
            // AssetServer only evicts unleased values. Combine the explicit cache target with the
            // remaining portion of the total CPU target after pinned leases are accounted for.
            // This preserves active ownership while letting the cache shrink as total residency
            // approaches its soft budget.
            let stats = server.cpu_memory_stats();
            let resident_unpinned_allowance = self
                .config
                .memory
                .cpu_resident_bytes
                .map(|budget| budget.saturating_sub(stats.pinned_bytes_estimate));
            let effective_unpinned_budget = match (
                self.config.memory.cpu_cache_bytes,
                resident_unpinned_allowance,
            ) {
                (Some(cache), Some(resident)) => Some(cache.min(resident)),
                (Some(cache), None) => Some(cache),
                (None, Some(resident)) => Some(resident),
                (None, None) => None,
            };
            server.set_cpu_memory_budget_bytes(effective_unpinned_budget);
        });

        let active_neighbors: HashSet<RegionId> = if self.config.prefetch_neighbors {
            self.regions
                .values()
                .filter(|record| record.state == RegionState::Active)
                .flat_map(|record| record.definition.neighbors.iter().cloned())
                .collect()
        } else {
            HashSet::new()
        };

        let mut candidates = self.nearby_candidates();
        candidates.extend(active_neighbors.iter().cloned());
        let mut starts: Vec<(RegionId, i32)> = candidates
            .iter()
            .filter_map(|id| {
                let record = self.regions.get(id)?;
                if record.state != RegionState::Unloaded && record.state != RegionState::Requested {
                    return None;
                }
                let interest = self.interest_for(&record.definition);
                (interest.preload || record.has_request_intent() || active_neighbors.contains(id))
                    .then_some((id.clone(), interest.priority))
            })
            .collect();
        starts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        let start_budget = self.config.max_new_region_requests_per_update;
        let start_order: Vec<_> = starts.into_iter().take(start_budget).collect();

        // Preserve the priority ordering when admitting region requests. Inserting IDs into a set
        // and iterating that set would silently turn this back into lexical/FIFO scheduling.
        for (id, priority) in start_order {
            if let Some(record) = self.regions.get_mut(&id) {
                record.scheduling_priority = priority;
            }
            self.begin_asset_request(world, &id);
        }

        // Existing in-flight/resident regions are always advanced, even when they leave the spatial
        // query window, so unload hysteresis and failure cleanup cannot become stranded. Include
        // dependency-pinned regions even when they are outside all observer spatial queries.
        let mut work_ids: BTreeSet<RegionId> = candidates.into_iter().collect();
        work_ids.extend(self.regions.iter().filter_map(|(id, record)| {
            (record.state != RegionState::Unloaded || record.has_request_intent())
                .then_some(id.clone())
        }));
        let mut ordered_work: Vec<_> = work_ids
            .into_iter()
            .map(|id| {
                let priority = self
                    .regions
                    .get(&id)
                    .map(|record| self.interest_for(&record.definition).priority)
                    .unwrap_or(i32::MIN);
                (id, priority)
            })
            .collect();
        ordered_work.sort_by(|(left, left_priority), (right, right_priority)| {
            right_priority
                .cmp(left_priority)
                .then_with(|| left.cmp(right))
        });
        let mut asset_request_budget = self.config.max_asset_requests_per_update;
        let mut activation_budget = self.config.max_region_activations_per_update;
        let mut entity_spawn_budget = self.config.max_entity_spawns_per_update;
        for (id, _) in ordered_work {
            self.advance_region(
                world,
                &id,
                active_neighbors.contains(&id),
                &mut asset_request_budget,
                &mut activation_budget,
                &mut entity_spawn_budget,
            );
        }

        self.apply_memory_pressure(world, &active_neighbors);
        self.refresh_diagnostics(world, started);
    }

    fn nearby_candidates(&self) -> HashSet<RegionId> {
        let query_radius = self
            .regions
            .values()
            .map(|record| record.definition.policy.unload_distance)
            .fold(0.0_f32, f32::max);
        let mut candidates = HashSet::new();
        for observer in self.observers.values() {
            let delta = Vec3::splat(query_radius);
            if let Ok(query_bounds) =
                RegionBounds::new(observer.position - delta, observer.position + delta)
            {
                candidates.extend(self.index.query(query_bounds));
            }
        }
        candidates.extend(self.regions.iter().filter_map(|(id, record)| {
            (record.state != RegionState::Unloaded || record.has_request_intent())
                .then_some(id.clone())
        }));
        candidates
    }

    fn interest_for(&self, definition: &RegionDefinition) -> RegionInterest {
        // With no observers, distance no longer pins a region. Explicit requests and dependent
        // regions still retain it through `has_request_intent` in the lifecycle coordinator.
        let has_request_intent = self
            .regions
            .get(&definition.id)
            .is_some_and(RegionRecord::has_request_intent);
        let mut interest = RegionInterest {
            beyond_unload: !has_request_intent,
            ..RegionInterest::default()
        };
        let mut best_priority = i32::MIN;
        for observer in self.observers.values() {
            let distance = definition.bounds.distance_to_point(observer.position);
            interest.nearest_distance = Some(
                interest
                    .nearest_distance
                    .map_or(distance, |old| old.min(distance)),
            );
            interest.preload |= distance <= definition.policy.preload_distance;
            interest.activate |= distance <= definition.policy.activation_distance;
            interest.beyond_unload &= distance > definition.policy.unload_distance;
            let range = definition.policy.unload_distance.max(1.0);
            let proximity = (((range - distance).max(0.0) / range) * 100.0) as i32;
            let toward_region =
                (definition.bounds.min + definition.bounds.max) * 0.5 - observer.position;
            let directional_bonus = if observer.velocity.length_squared() > 1.0e-4
                && toward_region.length_squared() > 1.0e-4
            {
                (observer
                    .velocity
                    .normalize()
                    .dot(toward_region.normalize())
                    .max(0.0)
                    * 50.0) as i32
            } else {
                0
            };
            let priority = definition
                .policy
                .base_priority
                .saturating_add(proximity)
                .saturating_add(directional_bonus);
            best_priority = best_priority.max(priority);
        }
        interest.priority = if best_priority == i32::MIN {
            definition.policy.base_priority
        } else {
            best_priority
        };
        interest
    }

    fn begin_asset_request(&mut self, world: &mut World, id: &RegionId) {
        let Some(record) = self.regions.get(id) else {
            return;
        };
        if record.state != RegionState::Unloaded && record.state != RegionState::Requested {
            return;
        }
        let definition = record.definition.clone();
        let from_state = record.state.clone();
        if let Err(message) = self.acquire_dependencies(id) {
            self.fail_region(world, id, message);
            return;
        }

        // One lease/request per distinct typed path, even if hundreds of placements use the model.
        let pending_asset_paths: Vec<_> = definition
            .models
            .iter()
            .map(|model| model.asset_path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let use_sequence = self.next_use_sequence;
        self.next_use_sequence = self.next_use_sequence.saturating_add(1);
        if let Some(record) = self.regions.get_mut(id) {
            record.leases.clear();
            record.scene_doc_lease = None;
            record.scene_doc_dependencies_resolved = false;
            record.scene_doc_instantiated = false;
            record.pending_typed_dependency_paths.clear();
            record.next_typed_dependency_request = 0;
            record.prefab_entities.clear();
            record.pending_asset_paths = pending_asset_paths;
            record.next_asset_request = 0;
            record.last_used_sequence = use_sequence;
            record.resident_cpu_bytes_estimate = 0;
            record.cpu_ready_models = 0;
            record.gpu_ready_models = 0;
            record.pending_collider_spawn = 0;
            record.pending_visual_spawn = 0;
        }
        if from_state == RegionState::Unloaded {
            self.set_state(
                id,
                RegionState::Requested,
                "region admitted by streaming scheduler",
            );
        }
        self.set_state(
            id,
            RegionState::LoadingAssets,
            "bounded CPU asset requests scheduled",
        );
    }

    fn queue_asset_request_batch(&mut self, world: &mut World, id: &RegionId, budget: &mut usize) {
        if *budget == 0 {
            return;
        }
        let Some((scene_doc_path, has_scene_doc_lease, priority)) =
            self.regions.get(id).map(|record| {
                (
                    record.definition.scene_doc_path.clone(),
                    record.scene_doc_lease.is_some(),
                    record.scheduling_priority,
                )
            })
        else {
            return;
        };

        // Serialized scene descriptions use the exact same bounded AssetServer worker queue as
        // all other CPU assets. A full queue is backpressure, not a terminal region failure.
        if let Some(path) = scene_doc_path.filter(|_| !has_scene_doc_lease) {
            let result = world.resource_scope::<AssetServer, _>(|_, server| {
                server.retry_ref_with_priority::<SceneDoc>(&path, priority)
            });
            match result {
                Ok(lease) => {
                    if let Some(record) = self.regions.get_mut(id) {
                        record.scene_doc_lease = Some(lease);
                    }
                    *budget = budget.saturating_sub(1);
                }
                Err(error) if error.kind == LoadErrorKind::QueueFull => {
                    *budget = 0;
                    return;
                }
                Err(error) => {
                    self.fail_region(
                        world,
                        id,
                        format!("scene document request `{path}` failed: {error}"),
                    );
                    return;
                }
            }
        }

        if *budget == 0 {
            return;
        }
        let Some((paths, priority)) = self.regions.get(id).and_then(|record| {
            let end = record
                .next_asset_request
                .saturating_add(*budget)
                .min(record.pending_asset_paths.len());
            (end > record.next_asset_request).then(|| {
                (
                    record.pending_asset_paths[record.next_asset_request..end].to_vec(),
                    record.scheduling_priority,
                )
            })
        }) else {
            return;
        };

        for path in paths {
            let result = world.resource_scope::<AssetServer, _>(|_, server| {
                server.retry_ref_with_priority::<ModelAsset>(&path, priority)
            });
            match result {
                Ok(lease) => {
                    if let Some(record) = self.regions.get_mut(id) {
                        record.leases.insert(path, lease);
                        record.next_asset_request = record.next_asset_request.saturating_add(1);
                    }
                    *budget = budget.saturating_sub(1);
                }
                Err(error) if error.kind == LoadErrorKind::QueueFull => {
                    // Queue saturation is backpressure, not a broken asset. The failed path is
                    // retried on the next Update after AssetServer has committed completions.
                    *budget = 0;
                    break;
                }
                Err(error) => {
                    self.fail_region(world, id, format!("asset request `{path}` failed: {error}"));
                    return;
                }
            }
        }
        if *budget == 0 {
            return;
        }
        loop {
            let next = self.regions.get(id).and_then(|record| {
                record
                    .pending_typed_dependency_paths
                    .get(record.next_typed_dependency_request)
                    .cloned()
                    .map(|(asset_type, path)| (asset_type, path, record.scheduling_priority))
            });
            let Some((asset_type, path, priority)) = next else {
                break;
            };
            let result = world.resource_scope::<AssetServer, _>(|world, server| {
                world.resource::<StreamingAssetAdapters>().request(
                    server,
                    &asset_type,
                    &path,
                    priority,
                )
            });
            match result {
                Ok(lease) => {
                    if let Some(record) = self.regions.get_mut(id) {
                        record
                            .typed_dependency_leases
                            .insert((asset_type, path), lease);
                        record.next_typed_dependency_request =
                            record.next_typed_dependency_request.saturating_add(1);
                    }
                    *budget = budget.saturating_sub(1);
                    if *budget == 0 {
                        break;
                    }
                }
                Err(error) if error.kind == LoadErrorKind::QueueFull => {
                    *budget = 0;
                    break;
                }
                Err(error) => {
                    self.fail_region(
                        world,
                        id,
                        format!("typed asset dependency request `{path}` failed: {error}"),
                    );
                    return;
                }
            }
        }
    }

    fn acquire_dependencies(&mut self, id: &RegionId) -> Result<(), String> {
        let dependencies = {
            let Some(record) = self.regions.get_mut(id) else {
                return Err("region disappeared".to_owned());
            };
            if record.requested_dependencies {
                return Ok(());
            }
            record.requested_dependencies = true;
            record.definition.dependencies.clone()
        };
        for dependency in dependencies {
            let needs_request = {
                let record = self.regions.get_mut(&dependency).ok_or_else(|| {
                    format!(
                        "dependency `{}` is no longer registered",
                        dependency.as_str()
                    )
                })?;
                record.dependency_users = record.dependency_users.saturating_add(1);
                record.state == RegionState::Unloaded
            };
            if needs_request {
                self.set_state(
                    &dependency,
                    RegionState::Requested,
                    "region requested by dependent region",
                );
            }
        }
        Ok(())
    }

    fn release_dependencies(&mut self, id: &RegionId) {
        let dependencies = if let Some(record) = self.regions.get_mut(id) {
            if !record.requested_dependencies {
                return;
            }
            record.requested_dependencies = false;
            record.definition.dependencies.clone()
        } else {
            return;
        };
        for dependency in dependencies {
            if let Some(record) = self.regions.get_mut(&dependency) {
                record.dependency_users = record.dependency_users.saturating_sub(1);
            }
        }
    }

    fn advance_region(
        &mut self,
        world: &mut World,
        id: &RegionId,
        is_neighbor_prefetch: bool,
        asset_request_budget: &mut usize,
        activation_budget: &mut usize,
        entity_spawn_budget: &mut usize,
    ) {
        let Some((definition, state, manual, forced_activation, distance, priority, old_priority)) =
            self.regions.get(id).map(|record| {
                let interest = self.interest_for(&record.definition);
                (
                    record.definition.clone(),
                    record.state.clone(),
                    record.has_request_intent(),
                    record.forced_activation || !record.scene_activation_users.is_empty(),
                    interest.nearest_distance,
                    interest.priority,
                    record.scheduling_priority,
                )
            })
        else {
            return;
        };

        if let Some(record) = self.regions.get_mut(id) {
            record.last_distance = distance;
            record.scheduling_priority = priority;
        }
        if priority > old_priority {
            if state == RegionState::LoadingAssets {
                self.raise_pending_asset_priority(world, id, priority);
            } else if state == RegionState::WarmingGpu {
                self.raise_pending_gpu_priority(world, id, priority);
            }
        }
        let interest = self.interest_for(&definition);
        let beyond_unload = interest.beyond_unload && !manual && !is_neighbor_prefetch;
        let should_activate =
            forced_activation || (definition.policy.auto_activate && interest.activate);

        match state {
            RegionState::Unloaded => {
                if manual || interest.preload || is_neighbor_prefetch {
                    self.set_state(
                        id,
                        RegionState::Requested,
                        "observer or dependency entered preload range",
                    );
                }
            }
            RegionState::Requested => {
                // Starts are budgeted and ordered in tick(); wait for the next update if omitted.
                if beyond_unload && !manual && !is_neighbor_prefetch {
                    self.set_state(
                        id,
                        RegionState::Unloaded,
                        "request left the streaming range before loading began",
                    );
                }
            }
            RegionState::LoadingAssets => {
                if beyond_unload {
                    self.begin_unload(world, id);
                } else {
                    self.queue_asset_request_batch(world, id, asset_request_budget);
                    match self.dependencies_readiness(id) {
                        Ok(true) => self.check_cpu_readiness(world, id),
                        Ok(false) => {}
                        Err(message) => self.fail_region(world, id, message),
                    }
                }
            }
            RegionState::WarmingGpu => {
                if beyond_unload {
                    self.begin_unload(world, id);
                } else {
                    self.check_gpu_readiness(world, id);
                }
            }
            RegionState::Ready => {
                if beyond_unload {
                    self.begin_unload(world, id);
                } else {
                    if definition.policy.preview_when_ready {
                        self.spawn_visual_batch(world, id, entity_spawn_budget);
                    }
                    if should_activate
                        && *activation_budget > 0
                        && self.active_region_count() < self.config.max_active_regions
                    {
                        *activation_budget = activation_budget.saturating_sub(1);
                        self.begin_physics_preparation(world, id, entity_spawn_budget);
                    }
                }
            }
            RegionState::PreparingPhysics => {
                if beyond_unload {
                    self.begin_unload(world, id);
                } else {
                    self.spawn_visual_batch(world, id, entity_spawn_budget);
                    if self.visual_entities_ready(id) {
                        self.spawn_collider_batch(world, id, entity_spawn_budget);
                        let all_registered = self.physics_entities_registered(world, id);
                        if all_registered {
                            self.set_state(
                                id,
                                RegionState::Activating,
                                "required Rapier colliders are registered",
                            );
                        }
                    }
                }
            }
            RegionState::Activating => {
                if beyond_unload {
                    self.begin_unload(world, id);
                } else {
                    self.spawn_visual_batch(world, id, entity_spawn_budget);
                    if self.regions.get(id).is_some_and(|record| {
                        record.state == RegionState::Activating
                            && record.pending_visual_spawn
                                >= record.definition.models.len()
                                    + record.definition.point_lights.len()
                    }) {
                        self.set_state(id, RegionState::Active, "region content activated");
                    }
                }
            }
            RegionState::Active => {
                if beyond_unload {
                    self.begin_unload(world, id);
                } else if !interest.activate && !forced_activation {
                    self.begin_deactivation(world, id);
                }
            }
            RegionState::Deactivating => {
                if self.physics_cleanup_complete(world, id) {
                    self.set_state(
                        id,
                        RegionState::Dormant,
                        "region deactivated; CPU/GPU residency retained",
                    );
                }
            }
            RegionState::Dormant => {
                if beyond_unload {
                    self.begin_unload(world, id);
                } else if should_activate
                    && *activation_budget > 0
                    && self.active_region_count() < self.config.max_active_regions
                {
                    *activation_budget = activation_budget.saturating_sub(1);
                    self.begin_physics_preparation(world, id, entity_spawn_budget);
                }
            }
            RegionState::Unloading => {
                if self.physics_cleanup_complete(world, id) {
                    if !beyond_unload || manual || is_neighbor_prefetch {
                        // A request can re-enter range while Rapier is still removing the old
                        // colliders. Retain CPU/GPU leases and return to Dormant instead of
                        // completing an obsolete unload; a later activation can reuse warm data.
                        self.set_state(
                            id,
                            RegionState::Dormant,
                            "unload cancelled after the region became wanted again",
                        );
                    } else {
                        self.release_region_assets(world, id);
                        self.release_dependencies(id);
                        self.set_state(
                            id,
                            RegionState::Unloaded,
                            "region resources released after physics cleanup",
                        );
                    }
                }
            }
            RegionState::Failed(_) => {
                if self.physics_cleanup_complete(world, id) {
                    if let Some(record) = self.regions.get_mut(id) {
                        record.pending_collider_cleanup.clear();
                    }
                }
            }
        }
    }

    fn raise_pending_asset_priority(&self, world: &mut World, id: &RegionId, priority: i32) {
        let paths: Vec<_> = self
            .regions
            .get(id)
            .map(|record| record.leases.keys().cloned().collect())
            .unwrap_or_default();
        world.resource_scope::<AssetServer, _>(|_, server| {
            for path in paths {
                // A returned weak handle is intentionally discarded: the region's AssetRef remains
                // the ownership token. This call only upgrades the queued worker priority.
                let _ = server.request_with_priority::<ModelAsset>(&path, priority);
            }
        });
    }

    fn raise_pending_gpu_priority(&self, world: &mut World, id: &RegionId, priority: i32) {
        let requests: Vec<_> = self
            .regions
            .get(id)
            .map(|record| {
                record
                    .leases
                    .iter()
                    .map(|(path, lease)| (path.clone(), lease.handle()))
                    .collect()
            })
            .unwrap_or_default();
        let warmup = world.resource_mut::<ModelWarmup>();
        for (path, handle) in requests {
            warmup.request_with_priority(handle, path, priority);
        }
    }

    fn dependencies_readiness(&self, id: &RegionId) -> Result<bool, String> {
        let Some(record) = self.regions.get(id) else {
            return Err("region disappeared while resolving dependencies".to_owned());
        };
        for dependency_id in &record.definition.dependencies {
            let Some(dependency) = self.regions.get(dependency_id) else {
                return Err(format!(
                    "required dependency region `{}` was removed",
                    dependency_id.as_str()
                ));
            };
            match &dependency.state {
                RegionState::Ready
                | RegionState::PreparingPhysics
                | RegionState::Activating
                | RegionState::Active
                | RegionState::Deactivating
                | RegionState::Dormant => {}
                RegionState::Failed(reason) => {
                    return Err(format!(
                        "required dependency region `{}` failed: {reason}",
                        dependency_id.as_str()
                    ));
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    fn check_cpu_readiness(&mut self, world: &mut World, id: &RegionId) {
        let Some((
            document_path,
            document_lease,
            dependencies_resolved,
            all_requests_submitted,
            priority,
        )) = self.regions.get(id).map(|record| {
            (
                record.definition.scene_doc_path.clone(),
                record.scene_doc_lease.as_ref().map(AssetRef::handle),
                record.scene_doc_dependencies_resolved,
                record.next_asset_request >= record.pending_asset_paths.len()
                    && record.next_typed_dependency_request
                        >= record.pending_typed_dependency_paths.len()
                    && (record.definition.scene_doc_path.is_none()
                        || record.scene_doc_lease.is_some()),
                record.scheduling_priority,
            )
        })
        else {
            return;
        };
        if !all_requests_submitted {
            return;
        }

        // Resolve typed resources declared by SceneDoc component codecs before considering CPU
        // readiness. The document and every required ModelAsset remain strongly leased until the
        // region is explicitly unloaded; AssetServer still deduplicates shared typed paths.
        if let (Some(path), Some(handle)) = (document_path.as_ref(), document_lease) {
            let (document, document_state, current_handle) = {
                let server = world.resource::<AssetServer>();
                (
                    server.get(handle).cloned(),
                    server.state::<SceneDoc>(path).cloned(),
                    server.current_handle::<SceneDoc>(path),
                )
            };
            let Some(document) = document else {
                if current_handle.is_some() && current_handle != Some(handle) {
                    // SceneDoc hot reload is generation-aware just like model hot reload. Keep
                    // the previous document lease until the replacement generation is pinned.
                    match document_state {
                        Some(LoadState::Failed(error)) => {
                            self.fail_region(
                                world,
                                id,
                                format!("reloaded scene document `{path}` failed: {error}"),
                            );
                            return;
                        }
                        Some(LoadState::Loading | LoadState::Loaded | LoadState::Unloaded) => {
                            let reacquired = world.resource_scope::<AssetServer, _>(|_, server| {
                                server.request_ref_with_priority::<SceneDoc>(path, priority)
                            });
                            match reacquired {
                                Ok(lease) => {
                                    let old_lease = self.regions.get_mut(id).and_then(|record| {
                                        let old = record.scene_doc_lease.replace(lease);
                                        record.scene_doc_dependencies_resolved = false;
                                        old
                                    });
                                    if let Some(old_lease) = old_lease {
                                        let old_handle = old_lease.handle();
                                        drop(old_lease);
                                        world.resource_scope::<AssetServer, _>(|_, server| {
                                            let _ = server.cancel_if_unleased(old_handle);
                                        });
                                    }
                                }
                                Err(error) if error.kind == LoadErrorKind::QueueFull => return,
                                Err(error) => self.fail_region(
                                    world,
                                    id,
                                    format!("could not reacquire reloaded scene document `{path}`: {error}"),
                                ),
                            }
                            return;
                        }
                        None => {}
                    }
                }
                match document_state {
                    Some(LoadState::Loading) => return,
                    Some(LoadState::Failed(error)) => {
                        self.fail_region(
                            world,
                            id,
                            format!("required scene document `{path}` failed: {error}"),
                        );
                        return;
                    }
                    Some(LoadState::Unloaded) => {
                        let reacquired = world.resource_scope::<AssetServer, _>(|_, server| {
                            server.request_ref_with_priority::<SceneDoc>(path, priority)
                        });
                        match reacquired {
                            Ok(lease) => {
                                let old_lease = self
                                    .regions
                                    .get_mut(id)
                                    .and_then(|record| record.scene_doc_lease.replace(lease));
                                if let Some(old_lease) = old_lease {
                                    let old_handle = old_lease.handle();
                                    drop(old_lease);
                                    world.resource_scope::<AssetServer, _>(|_, server| {
                                        let _ = server.cancel_if_unleased(old_handle);
                                    });
                                }
                            }
                            Err(error) if error.kind == LoadErrorKind::QueueFull => return,
                            Err(error) => self.fail_region(
                                world,
                                id,
                                format!("could not reacquire scene document `{path}`: {error}"),
                            ),
                        }
                        return;
                    }
                    Some(LoadState::Loaded) => {
                        self.fail_region(world, id, format!("scene document `{path}` is marked Loaded but has no resident CPU value"));
                        return;
                    }
                    None => {
                        self.fail_region(
                            world,
                            id,
                            format!("scene document `{path}` disappeared from AssetServer"),
                        );
                        return;
                    }
                }
            };

            if !dependencies_resolved {
                let dependency_result = world
                    .try_resource::<PrefabCodecRegistry>()
                    .map(|codecs| document.asset_dependencies(codecs));
                let dependencies = match dependency_result {
                    Some(Ok(dependencies)) => dependencies,
                    Some(Err(error)) => {
                        self.fail_region(
                            world,
                            id,
                            format!("scene document `{path}` dependency discovery failed: {error}"),
                        );
                        return;
                    }
                    None => {
                        self.fail_region(world, id, "SceneDoc preparation requires PrefabCodecRegistry; ensure Render3dPlugin is installed before StreamingPlugin".to_owned());
                        return;
                    }
                };
                let model_type = std::any::type_name::<ModelAsset>();
                let mut discovered_models = Vec::new();
                let mut discovered_typed = Vec::new();
                for dependency in dependencies {
                    if !dependency.required {
                        // Optional resources are deliberately not readiness blockers. Their codec
                        // must implement a fallback when the optional handle is not resident.
                        continue;
                    }
                    if dependency.asset_type == model_type {
                        discovered_models.push(dependency.path);
                    } else {
                        let adapter_registered = world
                            .try_resource::<StreamingAssetAdapters>()
                            .is_some_and(|adapters| adapters.contains(&dependency.asset_type));
                        if !adapter_registered {
                            self.fail_region(
                                world,
                                id,
                                format!(
                                    "scene document `{path}` declares required asset type `{}` for component `{}`, but no StreamingAssetAdapters entry is registered",
                                    dependency.asset_type, dependency.component
                                ),
                            );
                            return;
                        }
                        discovered_typed.push((dependency.asset_type, dependency.path));
                    }
                }
                discovered_models.sort();
                discovered_models.dedup();
                discovered_typed.sort();
                discovered_typed.dedup();
                let mut obsolete_model_paths = Vec::new();
                let mut obsolete_typed_leases = Vec::new();
                if let Some(record) = self.regions.get_mut(id) {
                    // SceneDoc generations may declare a different dependency graph. Keep the
                    // region's authored model placements, but discard dependencies that belonged
                    // only to the previous document generation before evaluating readiness.
                    let mut desired_model_paths: BTreeSet<String> = record
                        .definition
                        .models
                        .iter()
                        .map(|model| model.asset_path.clone())
                        .collect();
                    desired_model_paths.extend(discovered_models);
                    obsolete_model_paths = record
                        .leases
                        .keys()
                        .filter(|path| !desired_model_paths.contains(*path))
                        .cloned()
                        .collect();
                    // Keep obsolete lease removal outside this borrow so its GPU warmup user
                    // count and AssetServer pin are released through the normal ownership path.

                    // Put already-pinned assets first and not-yet-requested assets last. The
                    // request cursor can then resume safely without re-requesting an existing
                    // lease or skipping a new dependency after a document hot reload.
                    let mut leased_model_paths: Vec<String> = desired_model_paths
                        .iter()
                        .filter(|path| record.leases.contains_key(*path))
                        .cloned()
                        .collect();
                    let leased_model_count = leased_model_paths.len();
                    leased_model_paths.extend(
                        desired_model_paths
                            .iter()
                            .filter(|path| !record.leases.contains_key(*path))
                            .cloned(),
                    );
                    record.pending_asset_paths = leased_model_paths;
                    record.next_asset_request = leased_model_count;

                    let desired_typed: BTreeSet<(String, String)> =
                        discovered_typed.into_iter().collect();
                    let obsolete_keys: Vec<_> = record
                        .typed_dependency_leases
                        .keys()
                        .filter(|key| !desired_typed.contains(*key))
                        .cloned()
                        .collect();
                    for key in obsolete_keys {
                        if let Some(lease) = record.typed_dependency_leases.remove(&key) {
                            obsolete_typed_leases.push((key.0, lease));
                        }
                    }
                    let mut typed_paths: Vec<_> = desired_typed
                        .iter()
                        .filter(|key| record.typed_dependency_leases.contains_key(*key))
                        .cloned()
                        .collect();
                    let leased_typed_count = typed_paths.len();
                    typed_paths.extend(
                        desired_typed
                            .iter()
                            .filter(|key| !record.typed_dependency_leases.contains_key(*key))
                            .cloned(),
                    );
                    record.pending_typed_dependency_paths = typed_paths;
                    record.next_typed_dependency_request = leased_typed_count;
                    record.scene_doc_dependencies_resolved = true;
                }

                // Release obsolete document-owned leases only after dropping the RegionRecord
                // borrow. Shared AssetServer consumers and ModelWarmup users remain pinned.
                for path in obsolete_model_paths {
                    self.release_region_model_lease(world, id, &path);
                }
                if !obsolete_typed_leases.is_empty() {
                    world.resource_scope::<AssetServer, _>(|world, server| {
                        if let Some(adapters) = world.try_resource::<StreamingAssetAdapters>() {
                            for (asset_type, lease) in obsolete_typed_leases {
                                adapters.release(server, &asset_type, lease);
                            }
                        }
                    });
                }
                if let Some(record) = self.regions.get(id) {
                    if record.next_asset_request < record.pending_asset_paths.len()
                        || record.next_typed_dependency_request
                            < record.pending_typed_dependency_paths.len()
                    {
                        return;
                    }
                }
            }
        }

        // Dependency discovery can prune resources from a superseded SceneDoc generation.
        // Refresh the model lease snapshot before readiness and GPU warmup accounting so an
        // obsolete dependency cannot be warmed for one extra frame.
        let leases: Vec<(String, ribbon_assets::Handle<ModelAsset>)> = self
            .regions
            .get(id)
            .map(|record| {
                record
                    .leases
                    .iter()
                    .map(|(path, lease)| (path.clone(), lease.handle()))
                    .collect()
            })
            .unwrap_or_default();

        let mut waiting = false;
        let mut failure = None;
        let mut stale_generations = Vec::new();
        let mut resident_bytes = 0_u64;
        {
            let server = world.resource::<AssetServer>();
            for (path, handle) in &leases {
                if let Some(asset) = server.get(*handle) {
                    resident_bytes = resident_bytes.saturating_add(asset.estimated_size_bytes());
                    continue;
                }

                // A hot reload or another consumer's retry can replace the generation associated
                // with this region's old lease. Never wait forever on that stale handle: reacquire
                // the current path generation below, sharing any already queued/loaded operation.
                let current_handle = server.current_handle::<ModelAsset>(path);
                let path_state = server.state::<ModelAsset>(path);
                if current_handle != Some(*handle) {
                    match path_state {
                        Some(LoadState::Failed(error)) => {
                            failure = Some(format!("required model `{path}` failed: {error}"));
                            break;
                        }
                        Some(LoadState::Loaded | LoadState::Loading | LoadState::Unloaded) => {
                            // The path now points at another generation. Reacquire it before
                            // asking the renderer to warm it; keep the old lease until replacement.
                            stale_generations.push(path.clone());
                        }
                        None => {
                            failure = Some(format!(
                                "required model `{path}` disappeared from AssetServer while preparing region `{}`",
                                id.as_str()
                            ));
                            break;
                        }
                    }
                } else {
                    match path_state {
                        Some(LoadState::Loading) => waiting = true,
                        Some(LoadState::Unloaded) => stale_generations.push(path.clone()),
                        Some(LoadState::Failed(error)) => {
                            failure = Some(format!("required model `{path}` failed: {error}"));
                            break;
                        }
                        Some(LoadState::Loaded) => {
                            failure = Some(format!(
                                "required model `{path}` is marked Loaded but has no resident CPU value"
                            ));
                            break;
                        }
                        None => {
                            failure = Some(format!(
                                "required model `{path}` disappeared from AssetServer while preparing region `{}`",
                                id.as_str()
                            ));
                            break;
                        }
                    }
                }
            }
        }

        if let Some(message) = failure {
            self.fail_region(world, id, message);
            return;
        }

        let typed_dependency_states: Vec<_> = {
            let Some(record) = self.regions.get(id) else {
                return;
            };
            let server = world.resource::<AssetServer>();
            let adapters = world.resource::<StreamingAssetAdapters>();
            record
                .typed_dependency_leases
                .iter()
                .map(|((asset_type, path), lease)| {
                    (
                        asset_type.clone(),
                        path.clone(),
                        adapters.readiness(server, asset_type, path, lease.as_ref()),
                    )
                })
                .collect()
        };
        let mut stale_typed_dependencies = Vec::new();
        for (asset_type, path, state) in typed_dependency_states {
            match state {
                StreamingDependencyState::Ready { estimated_bytes } => {
                    resident_bytes = resident_bytes.saturating_add(estimated_bytes);
                }
                StreamingDependencyState::Loading => waiting = true,
                StreamingDependencyState::Stale => {
                    stale_typed_dependencies.push((asset_type, path));
                }
                StreamingDependencyState::Failed(message) => {
                    self.fail_region(
                        world,
                        id,
                        format!("required typed dependency failed: {message}"),
                    );
                    return;
                }
            }
        }
        for (asset_type, path) in stale_typed_dependencies {
            let reacquired = world.resource_scope::<AssetServer, _>(|world, server| {
                world.resource::<StreamingAssetAdapters>().request(
                    server,
                    &asset_type,
                    &path,
                    priority,
                )
            });
            match reacquired {
                Ok(lease) => {
                    let key = (asset_type.clone(), path.clone());
                    let old_lease = self
                        .regions
                        .get_mut(id)
                        .and_then(|record| record.typed_dependency_leases.insert(key, lease));
                    if let Some(old_lease) = old_lease {
                        world.resource_scope::<AssetServer, _>(|world, server| {
                            if let Some(adapters) = world.try_resource::<StreamingAssetAdapters>() {
                                adapters.release(server, &asset_type, old_lease);
                            }
                        });
                    }
                    waiting = true;
                }
                Err(error) if error.kind == LoadErrorKind::QueueFull => waiting = true,
                Err(error) => {
                    self.fail_region(
                        world,
                        id,
                        format!("could not reacquire typed dependency `{path}`: {error}"),
                    );
                    return;
                }
            }
        }

        for path in stale_generations {
            let reacquired = world.resource_scope::<AssetServer, _>(|_, server| {
                server.request_ref_with_priority::<ModelAsset>(&path, priority)
            });
            match reacquired {
                Ok(lease) => {
                    if let Some(record) = self.regions.get_mut(id) {
                        // Replacing the old lease drops its stale generation reference only after
                        // the new generation is safely pinned.
                        record.leases.insert(path, lease);
                    }
                    waiting = true;
                }
                Err(error) if error.kind == LoadErrorKind::QueueFull => {
                    // A stale generation remains pinned until the current one can be reacquired.
                    // Retry on the next Update; queue saturation is backpressure, not asset loss.
                    waiting = true;
                }
                Err(error) => {
                    self.fail_region(
                        world,
                        id,
                        format!("could not reacquire model `{path}`: {error}"),
                    );
                    return;
                }
            }
        }

        if waiting {
            return;
        }

        for (path, handle) in leases {
            // The list may have contained only current generations if we reached this point.
            // Use the current lease handle in case this path was replaced by hot reload between
            // Updates (the ECS schedule is single-threaded, but AssetServer may commit reloads).
            let current_handle = self
                .regions
                .get(id)
                .and_then(|record| record.leases.get(&path))
                .map(AssetRef::handle)
                .unwrap_or(handle);
            let key = model_key(current_handle);
            let users = self.warmup_users.entry(key).or_insert((current_handle, 0));
            // ModelWarmup deduplicates by ID/generation and upgrades the shared request priority.
            // Every region contributes its interest; no extra GPU entry or upload is created.
            world.resource_mut::<ModelWarmup>().request_with_priority(
                current_handle,
                path.clone(),
                priority,
            );
            users.1 = users.1.saturating_add(1);
        }
        if let Some(record) = self.regions.get_mut(id) {
            record.resident_cpu_bytes_estimate = resident_bytes;
            record.cpu_ready_models = record.leases.len();
            record.gpu_ready_models = 0;
        }
        self.set_state(
            id,
            RegionState::WarmingGpu,
            "CPU assets resident; renderer warmup requested",
        );
    }

    fn check_gpu_readiness(&mut self, world: &mut World, id: &RegionId) {
        let handles: Vec<_> = self
            .regions
            .get(id)
            .map(|record| record.leases.values().map(AssetRef::handle).collect())
            .unwrap_or_default();
        let statuses: Vec<_> = {
            let warmup = world.resource::<ModelWarmup>();
            handles
                .into_iter()
                .map(|handle| warmup.status(handle).cloned())
                .collect()
        };
        let mut waiting = false;
        let mut failure = None;
        let mut ready_count = 0;
        for status in statuses {
            match status {
                Some(ModelWarmupStatus::Ready) => ready_count += 1,
                Some(ModelWarmupStatus::Failed(message)) => failure = Some(message),
                Some(ModelWarmupStatus::WaitingForCpu | ModelWarmupStatus::WaitingForGpu)
                | None => waiting = true,
            }
        }
        if let Some(record) = self.regions.get_mut(id) {
            record.gpu_ready_models = ready_count;
        }
        if let Some(message) = failure {
            self.fail_region(world, id, format!("GPU warmup failed: {message}"));
        } else if !waiting {
            self.set_state(
                id,
                RegionState::Ready,
                "all required model assets are CPU- and GPU-ready",
            );
        }
    }

    fn begin_physics_preparation(
        &mut self,
        world: &mut World,
        id: &RegionId,
        entity_spawn_budget: &mut usize,
    ) {
        if let Err(message) = self.instantiate_region_scene_doc(world, id) {
            self.fail_region(world, id, message);
            return;
        }
        if let Some(record) = self.regions.get_mut(id) {
            record.pending_collider_spawn = 0;
            record.collider_entities.clear();
        }
        self.set_state(
            id,
            RegionState::PreparingPhysics,
            "preparing render placements before collision activation",
        );
        self.spawn_visual_batch(world, id, entity_spawn_budget);
        if self.visual_entities_ready(id) {
            self.spawn_collider_batch(world, id, entity_spawn_budget);
            if self.physics_entities_registered(world, id) {
                self.set_state(
                    id,
                    RegionState::Activating,
                    "required Rapier colliders are registered",
                );
            }
        }
    }

    fn instantiate_region_scene_doc(
        &mut self,
        world: &mut World,
        id: &RegionId,
    ) -> Result<(), String> {
        let Some((already_instantiated, lease)) = self.regions.get(id).map(|record| {
            (
                record.scene_doc_instantiated,
                record.scene_doc_lease.as_ref().map(AssetRef::handle),
            )
        }) else {
            return Err("region disappeared while instantiating its scene document".to_owned());
        };
        if already_instantiated || lease.is_none() {
            return Ok(());
        }
        let handle = lease.expect("checked above");
        let document = world
            .try_resource::<AssetServer>()
            .and_then(|server| server.get(handle).cloned())
            .ok_or_else(|| {
                "prepared SceneDoc disappeared before prefab instantiation".to_owned()
            })?;

        // Prefab codecs need mutable AssetServer access to restore path-backed components. Move
        // the resource into the short-lived load context, instantiate atomically, then restore all
        // resources before returning to the normal app schedule. The codec contract forbids IO here.
        let Some(server) = world.remove_resource::<AssetServer>() else {
            return Err(
                "AssetServer resource is unavailable during prefab instantiation".to_owned(),
            );
        };
        let Some(registry) = world.remove_resource::<TypeRegistry>() else {
            world.insert_resource(server);
            return Err(
                "TypeRegistry resource is unavailable during prefab instantiation".to_owned(),
            );
        };
        let Some(codecs) = world.remove_resource::<PrefabCodecRegistry>() else {
            world.insert_resource(registry);
            world.insert_resource(server);
            return Err(
                "PrefabCodecRegistry resource is unavailable during prefab instantiation"
                    .to_owned(),
            );
        };
        let mut context = PrefabLoadContext::default();
        context.insert(server);
        let result = instantiate_with_context(
            world,
            &registry,
            &document.prefab,
            None,
            &codecs,
            &mut context,
        );
        let server = context.remove::<AssetServer>();
        world.insert_resource(codecs);
        world.insert_resource(registry);
        if let Some(server) = server {
            world.insert_resource(server);
        } else {
            return Err("PrefabLoadContext lost the AssetServer during instantiation".to_owned());
        }
        let entities = result
            .map_err(|error| format!("scene document prefab instantiation failed: {error}"))?;
        if let Some(record) = self.regions.get_mut(id) {
            record.prefab_entities = entities;
            record.scene_doc_instantiated = true;
        }
        Ok(())
    }

    fn visual_entities_ready(&self, id: &RegionId) -> bool {
        self.regions.get(id).is_some_and(|record| {
            record.pending_visual_spawn
                >= record.definition.models.len() + record.definition.point_lights.len()
        })
    }

    fn spawn_collider_batch(&mut self, world: &mut World, id: &RegionId, budget: &mut usize) {
        if *budget == 0 {
            return;
        }
        let Some((batch, end)) = self.regions.get(id).and_then(|record| {
            let start = record.pending_collider_spawn;
            let end = start
                .saturating_add(*budget)
                .min(record.definition.colliders.len());
            (end > start).then(|| (record.definition.colliders[start..end].to_vec(), end))
        }) else {
            return;
        };
        let batch_len = batch.len();
        let mut spawned = Vec::with_capacity(batch_len);
        for item in batch {
            spawned.push(world.spawn((item.transform, item.collider)));
        }
        if let Some(record) = self.regions.get_mut(id) {
            record.collider_entities.extend(spawned);
            record.pending_collider_spawn = end;
        }
        *budget = budget.saturating_sub(batch_len);
    }

    fn physics_entities_registered(&self, world: &World, id: &RegionId) -> bool {
        let Some(record) = self.regions.get(id) else {
            return false;
        };
        // An empty current entity list is not sufficient when a spawn budget deferred the first
        // batch. Require every authored collider entity to have been created before accepting the
        // physics readiness contract.
        if record.pending_collider_spawn < record.definition.colliders.len() {
            return false;
        }
        let mut required_entities = record.collider_entities.clone();
        // SceneDoc-prefab colliders participate in the same Rapier readiness contract as colliders
        // authored directly on RegionDefinition. Otherwise a region could become playable while
        // its prefab-owned collision shapes are still waiting for physics reconciliation.
        required_entities.extend(
            record
                .prefab_entities
                .iter()
                .copied()
                .filter(|entity| world.get::<Collider>(*entity).is_some()),
        );
        if required_entities.is_empty() {
            return true;
        }
        let Some(debug) = world.try_resource::<PhysicsDebugData>() else {
            return false;
        };
        let registered: HashSet<_> = debug.colliders.iter().map(|entry| entry.entity).collect();
        required_entities
            .iter()
            .all(|entity| registered.contains(entity))
    }

    fn spawn_visual_batch(&mut self, world: &mut World, id: &RegionId, budget: &mut usize) {
        if *budget == 0 {
            return;
        }
        let batch = self.regions.get(id).and_then(|record| {
            let models = &record.definition.models;
            let lights = &record.definition.point_lights;
            let start = record.pending_visual_spawn;
            let total = models.len().saturating_add(lights.len());
            let end = start.saturating_add(*budget).min(total);
            if end <= start {
                return None;
            }
            let mut items = Vec::with_capacity(end - start);
            for index in start..end {
                if index < models.len() {
                    let model = models[index].clone();
                    let Some(handle) = record.leases.get(&model.asset_path).map(AssetRef::handle)
                    else {
                        return Some(Err(model.asset_path));
                    };
                    items.push(VisualSpawnItem::Model(model, handle));
                } else {
                    items.push(VisualSpawnItem::PointLight(
                        lights[index - models.len()].light,
                    ));
                }
            }
            Some(Ok((items, end)))
        });
        let Some(batch) = batch else { return };
        let (items, end) = match batch {
            Ok(batch) => batch,
            Err(path) => {
                self.fail_region(
                    world,
                    id,
                    format!("model handle `{path}` disappeared before activation"),
                );
                return;
            }
        };

        let batch_len = items.len();
        let mut spawned = Vec::with_capacity(batch_len);
        for item in items {
            match item {
                VisualSpawnItem::Model(model, handle) => {
                    spawned.push(world.spawn((model.transform, ModelInstance { model: handle })));
                }
                VisualSpawnItem::PointLight(light) => {
                    spawned.push(world.spawn((light,)));
                }
            }
        }
        if let Some(record) = self.regions.get_mut(id) {
            record.visual_entities.extend(spawned);
            record.pending_visual_spawn = end;
        }
        *budget = budget.saturating_sub(batch_len);
    }

    fn begin_deactivation(&mut self, world: &mut World, id: &RegionId) {
        self.despawn_region_entities(world, id);
        let pending_empty = self
            .regions
            .get(id)
            .is_none_or(|record| record.pending_collider_cleanup.is_empty());
        if pending_empty {
            self.set_state(
                id,
                RegionState::Dormant,
                "region deactivated; residency retained",
            );
        } else {
            self.set_state(
                id,
                RegionState::Deactivating,
                "waiting for physics to remove inactive region colliders",
            );
        }
    }

    fn begin_unload(&mut self, world: &mut World, id: &RegionId) {
        self.despawn_region_entities(world, id);
        let pending_empty = self
            .regions
            .get(id)
            .is_none_or(|record| record.pending_collider_cleanup.is_empty());
        if pending_empty {
            self.release_region_assets(world, id);
            self.release_dependencies(id);
            self.set_state(id, RegionState::Unloaded, "region unloaded");
        } else {
            self.set_state(
                id,
                RegionState::Unloading,
                "waiting for physics cleanup before releasing region leases",
            );
        }
    }

    fn despawn_region_entities(&mut self, world: &mut World, id: &RegionId) {
        let Some(record) = self.regions.get_mut(id) else {
            return;
        };
        for entity in record.visual_entities.drain(..) {
            let _ = world.despawn(entity);
        }
        let prefab_entities = std::mem::take(&mut record.prefab_entities);
        // Record prefab-owned colliders before despawning their hierarchy so Rapier cleanup is
        // observed before leases are released and the same entities cannot outlive the region.
        for &entity in &prefab_entities {
            if world.get::<Collider>(entity).is_some() {
                record.pending_collider_cleanup.push(entity);
            }
        }
        for entity in prefab_entities {
            let _ = world.despawn_recursive(entity);
        }
        record.scene_doc_instantiated = false;
        for entity in record.collider_entities.drain(..) {
            let _ = world.despawn(entity);
            record.pending_collider_cleanup.push(entity);
        }
        // Deactivation removes the ECS placements but retains their asset/GPU residency. Reset
        // the spawn cursor so a later reactivation recreates the region's visuals instead of
        // treating the old placement count as proof that entities still exist.
        record.pending_visual_spawn = 0;
    }

    fn physics_cleanup_complete(&mut self, world: &World, id: &RegionId) -> bool {
        let Some(record) = self.regions.get_mut(id) else {
            return true;
        };
        let Some(debug) = world.try_resource::<PhysicsDebugData>() else {
            // Never release leases or free an activation slot while the physics owner cannot
            // confirm that despawned collider entities have left its simulation world.
            return record.pending_collider_cleanup.is_empty();
        };
        let registered: HashSet<_> = debug.colliders.iter().map(|entry| entry.entity).collect();
        record
            .pending_collider_cleanup
            .retain(|entity| registered.contains(entity));
        record.pending_collider_cleanup.is_empty()
    }

    fn release_region_model_lease(&mut self, world: &mut World, id: &RegionId, path: &str) {
        let lease = self
            .regions
            .get_mut(id)
            .and_then(|record| record.leases.remove(path));
        let Some(lease) = lease else { return };
        let handle = lease.handle();
        let key = model_key(handle);
        let release_warmup = if let Some((_, users)) = self.warmup_users.get_mut(&key) {
            *users = users.saturating_sub(1);
            *users == 0
        } else {
            false
        };
        if release_warmup {
            self.warmup_users.remove(&key);
            if world.try_resource::<ModelWarmup>().is_some() {
                world.resource_mut::<ModelWarmup>().release(handle);
            }
        }
        drop(lease);
        if world.try_resource::<AssetServer>().is_some() {
            world.resource_scope::<AssetServer, _>(|_, server| {
                let _ = server.cancel_if_unleased(handle);
            });
        }
    }

    fn release_region_assets(&mut self, world: &mut World, id: &RegionId) {
        let handles: Vec<_> = self
            .regions
            .get(id)
            .map(|record| record.leases.values().map(AssetRef::handle).collect())
            .unwrap_or_default();
        let scene_doc_handle = self
            .regions
            .get(id)
            .and_then(|record| record.scene_doc_lease.as_ref().map(AssetRef::handle));
        let typed_leases = self
            .regions
            .get_mut(id)
            .map(|record| std::mem::take(&mut record.typed_dependency_leases))
            .unwrap_or_default();
        for handle in handles.iter().copied() {
            let key = model_key(handle);
            let release = if let Some((_, users)) = self.warmup_users.get_mut(&key) {
                *users = users.saturating_sub(1);
                *users == 0
            } else {
                false
            };
            if release {
                self.warmup_users.remove(&key);
                world.resource_mut::<ModelWarmup>().release(handle);
            }
        }
        if let Some(record) = self.regions.get_mut(id) {
            // Drop this region's strong references before considering cancellation. The asset
            // server will preserve a shared in-flight request while any other region/consumer
            // still owns a lease for the same generation.
            record.leases.clear();
            record.scene_doc_lease = None;
            record.scene_doc_dependencies_resolved = false;
            record.scene_doc_instantiated = false;
            record.prefab_entities.clear();
            record.pending_typed_dependency_paths.clear();
            record.next_typed_dependency_request = 0;
            record.pending_asset_paths.clear();
            record.next_asset_request = 0;
            record.resident_cpu_bytes_estimate = 0;
            record.cpu_ready_models = 0;
            record.gpu_ready_models = 0;
            record.pending_visual_spawn = 0;
            record.pending_collider_spawn = 0;
        }
        if world.try_resource::<AssetServer>().is_some() {
            // Remove the adapter registry before scoping AssetServer out of World. Looking the
            // registry up from inside resource_scope would borrow World while its mutable scope
            // borrow is still active (E0502). Restore it immediately after all typed leases have
            // been released; codecs and callers continue to observe the same registry resource.
            let adapters = world.remove_resource::<StreamingAssetAdapters>();
            world.resource_scope::<AssetServer, _>(|_, server| {
                for handle in handles {
                    let _ = server.cancel_if_unleased(handle);
                }
                if let Some(handle) = scene_doc_handle {
                    let _ = server.cancel_if_unleased(handle);
                }
                if let Some(adapters) = adapters.as_ref() {
                    for ((asset_type, _path), lease) in typed_leases {
                        adapters.release(server, &asset_type, lease);
                    }
                }
                // If no adapter registry is installed, dropping the erased leases still releases
                // their CPU pins; only queued-load cancellation is skipped.
            });
            if let Some(adapters) = adapters {
                world.insert_resource(adapters);
            }
        }
    }

    fn fail_region(&mut self, world: &mut World, id: &RegionId, message: String) {
        self.despawn_region_entities(world, id);
        // Warmup cache retention is released when visuals are gone; Rapier colliders are independent
        // of model assets and are reconciled before this region can be retried.
        self.release_region_assets(world, id);
        self.release_dependencies(id);
        self.set_state(
            id,
            RegionState::Failed(message),
            "region preparation or activation failed",
        );
    }

    fn apply_memory_pressure(&mut self, world: &mut World, active_neighbors: &HashSet<RegionId>) {
        let cpu_stats = world
            .try_resource::<AssetServer>()
            .map(AssetServer::cpu_memory_stats);
        let gpu_used = world
            .try_resource::<RenderDiagnostics>()
            .map(|diagnostics| diagnostics.resources.total_bytes_estimate())
            .unwrap_or(0);
        let cpu_budget = self.config.memory.cpu_resident_bytes;
        let gpu_budget = self.config.memory.gpu_estimate_bytes;
        let cpu_over = cpu_budget.is_some_and(|budget| {
            cpu_stats.is_some_and(|stats| stats.resident_bytes_estimate > budget)
        });
        let gpu_over = gpu_budget.is_some_and(|budget| gpu_used > budget);
        if !cpu_over && !gpu_over {
            return;
        }

        let candidate = self
            .regions
            .iter()
            .filter(|(id, record)| {
                record.state == RegionState::Dormant
                    && !record.has_request_intent()
                    && !active_neighbors.contains(*id)
                    // A dormant region inside any observer's preload range is still wanted.
                    // Memory pressure must not fight partition policy and cause reload thrashing.
                    && !self.interest_for(&record.definition).preload
            })
            .filter(|(_, record)| {
                let cpu_releasable = cpu_over
                    && world.try_resource::<AssetServer>().is_some_and(|server| {
                        record
                            .leases
                            .values()
                            .any(|lease| server.strong_ref_count(lease.handle()) == 1)
                    });
                let gpu_releasable = gpu_over
                    && record.leases.values().any(|lease| {
                        self.warmup_users
                            .get(&model_key(lease.handle()))
                            .is_some_and(|(_, users)| *users == 1)
                    });
                cpu_releasable || gpu_releasable
            })
            // Lower-priority, least-recently-used dormant content leaves first. Release only one
            // region per Update so cleanup and cache reconciliation remain bounded.
            .min_by_key(|(_, record)| (record.scheduling_priority, record.last_used_sequence))
            .map(|(id, _)| id.clone());
        if let Some(id) = candidate {
            self.diagnostics.memory_pressure_events =
                self.diagnostics.memory_pressure_events.saturating_add(1);
            self.begin_unload(world, &id);
        }
    }

    fn active_region_count(&self) -> usize {
        self.regions
            .values()
            .filter(|record| {
                matches!(
                    &record.state,
                    RegionState::Active
                        | RegionState::Deactivating
                        | RegionState::PreparingPhysics
                        | RegionState::Activating
                ) || (!record.pending_collider_cleanup.is_empty()
                    && matches!(
                        &record.state,
                        RegionState::Unloading | RegionState::Failed(_)
                    ))
            })
            .count()
    }

    fn set_state(&mut self, id: &RegionId, state: RegionState, message: &str) {
        let from = {
            let Some(record) = self.regions.get(id) else {
                return;
            };
            if record.state == state {
                return;
            }
            record.state.clone()
        };
        if !transition_allowed(&from, &state) {
            self.diagnostics.invalid_state_transitions =
                self.diagnostics.invalid_state_transitions.saturating_add(1);
            if self.diagnostics.recent_events.len() == RECENT_EVENT_CAPACITY {
                self.diagnostics.recent_events.pop_front();
            }
            self.diagnostics.recent_events.push_back(StreamEvent {
                region: id.clone(),
                from: from.clone(),
                to: from,
                message: format!(
                    "rejected invalid region-state transition to {state:?}: {message}"
                ),
            });
            return;
        }
        let use_sequence = if state == RegionState::Active {
            let sequence = self.next_use_sequence;
            self.next_use_sequence = self.next_use_sequence.saturating_add(1);
            Some(sequence)
        } else {
            None
        };
        if let Some(record) = self.regions.get_mut(id) {
            record.state = state.clone();
            if let Some(sequence) = use_sequence {
                record.last_used_sequence = sequence;
            }
        }
        self.record_transition(id, from, state, message);
    }

    fn record_transition(
        &mut self,
        id: &RegionId,
        from: RegionState,
        to: RegionState,
        message: &str,
    ) {
        self.diagnostics.region_transitions = self.diagnostics.region_transitions.saturating_add(1);
        if self.diagnostics.recent_events.len() == RECENT_EVENT_CAPACITY {
            self.diagnostics.recent_events.pop_front();
        }
        self.diagnostics.recent_events.push_back(StreamEvent {
            region: id.clone(),
            from,
            to,
            message: message.to_owned(),
        });
    }

    fn refresh_diagnostics(&mut self, world: &World, started: Option<Instant>) {
        let cpu = world
            .try_resource::<AssetServer>()
            .map(AssetServer::cpu_memory_stats);
        let workers = world
            .try_resource::<AssetServer>()
            .map(AssetServer::worker_stats);
        let gpu = world
            .try_resource::<RenderDiagnostics>()
            .map(|diagnostics| diagnostics.resources.total_bytes_estimate())
            .unwrap_or(0);
        let mut diagnostics = StreamDiagnostics {
            region_count: self.regions.len(),
            gpu_resident_bytes_estimate: gpu,
            ..StreamDiagnostics::default()
        };
        if let Some(stats) = cpu {
            diagnostics.cpu_budget_evictions_cumulative = stats.budget_evictions_cumulative;
            diagnostics.cpu_unpinned_cache_budget_bytes = stats.budget_bytes.unwrap_or(0);
            diagnostics.cpu_memory_over_budget_bytes = stats.over_budget_bytes;
            diagnostics.cpu_resident_bytes_estimate = stats.resident_bytes_estimate;
            diagnostics.cpu_pinned_bytes_estimate = stats.pinned_bytes_estimate;
            diagnostics.cpu_unpinned_bytes_estimate = stats.unpinned_bytes_estimate;
            diagnostics.cpu_memory_over_budget_bytes = stats.over_budget_bytes;
        }
        if let Some(stats) = workers {
            diagnostics.queued_asset_jobs = stats.queued_jobs;
            diagnostics.asset_queue_capacity = stats.queue_capacity;
            diagnostics.active_asset_workers = stats.active_workers;
            diagnostics.asset_worker_count = stats.worker_count;
        }
        diagnostics.memory_pressure_events = self.diagnostics.memory_pressure_events;
        diagnostics.invalid_state_transitions = self.diagnostics.invalid_state_transitions;
        diagnostics.region_transitions = self.diagnostics.region_transitions;
        diagnostics.recent_events = self.diagnostics.recent_events.clone();
        for record in self.regions.values() {
            diagnostics.pending_asset_requests = diagnostics
                .pending_asset_requests
                .saturating_add(
                    record
                        .pending_asset_paths
                        .len()
                        .saturating_sub(record.next_asset_request),
                )
                .saturating_add(
                    record
                        .pending_typed_dependency_paths
                        .len()
                        .saturating_sub(record.next_typed_dependency_request),
                )
                .saturating_add(usize::from(
                    record.definition.scene_doc_path.is_some() && record.scene_doc_lease.is_none(),
                ));
            match &record.state {
                RegionState::LoadingAssets
                | RegionState::WarmingGpu
                | RegionState::PreparingPhysics
                | RegionState::Activating => diagnostics.loading_regions += 1,
                RegionState::Ready => diagnostics.ready_regions += 1,
                RegionState::Active => diagnostics.active_regions += 1,
                RegionState::Dormant | RegionState::Deactivating | RegionState::Unloading => {
                    diagnostics.dormant_regions += 1
                }
                RegionState::Failed(_) => diagnostics.failed_regions += 1,
                RegionState::Unloaded | RegionState::Requested => {}
            }
        }
        if let Some(budget) = self.config.memory.cpu_resident_bytes {
            diagnostics.cpu_resident_budget_bytes = budget;
            diagnostics.cpu_resident_over_budget_bytes = diagnostics
                .cpu_resident_bytes_estimate
                .saturating_sub(budget);
        }
        if let Some(budget) = self.config.memory.gpu_estimate_bytes {
            diagnostics.gpu_memory_over_budget_bytes = diagnostics
                .gpu_resident_bytes_estimate
                .saturating_sub(budget);
        }
        if let Some(started) = started {
            diagnostics.last_update_cpu_ns =
                started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            diagnostics.last_update_profiled = true;
        }
        self.diagnostics = diagnostics;
    }
}

fn transition_allowed(from: &RegionState, to: &RegionState) -> bool {
    use RegionState::*;
    if matches!(to, Failed(_)) {
        return !matches!(from, Failed(_));
    }
    match from {
        Unloaded => matches!(to, Requested),
        Requested => matches!(to, LoadingAssets | Unloaded),
        LoadingAssets | WarmingGpu | Ready | PreparingPhysics | Activating | Active | Dormant => {
            matches!(to, Unloading | Unloaded)
                || matches!(
                    (from, to),
                    (LoadingAssets, WarmingGpu)
                        | (WarmingGpu, Ready)
                        | (Ready, PreparingPhysics)
                        | (PreparingPhysics, Activating)
                        | (Activating, Active)
                        | (Active, Deactivating)
                        | (Active, Dormant)
                        | (Dormant, PreparingPhysics)
                )
        }
        Deactivating => matches!(to, Dormant | Unloading | Unloaded),
        Unloading => matches!(to, Unloaded | Dormant),
        Failed(_) => matches!(to, Unloaded),
    }
}

fn model_key(handle: ribbon_assets::Handle<ModelAsset>) -> ModelKey {
    (handle.id(), handle.generation())
}

fn validate_policy(policy: RegionPolicy) -> Result<(), String> {
    let valid = |value: f32| value.is_finite() && value >= 0.0;
    if !valid(policy.activation_distance)
        || !valid(policy.preload_distance)
        || !valid(policy.unload_distance)
    {
        return Err("region streaming distances must be finite and non-negative".to_owned());
    }
    if policy.preload_distance < policy.activation_distance {
        return Err("preload distance must be >= activation distance".to_owned());
    }
    if policy.unload_distance < policy.preload_distance {
        return Err("unload distance must be >= preload distance".to_owned());
    }
    Ok(())
}

fn valid_transform(transform: Transform3D) -> bool {
    transform.translation.is_finite()
        && transform.rotation.is_finite()
        && transform.rotation.length_squared() > 1.0e-12
        && transform.scale.is_finite()
        && transform.scale.min_element() > 0.0
}

/// Installs region coordination after the existing asset, scene, renderer, and physics plugins.
#[derive(Clone, Debug, Default)]
pub struct StreamingPlugin {
    pub config: StreamingConfig,
}

impl StreamingPlugin {
    pub fn new(config: StreamingConfig) -> Self {
        Self { config }
    }
}

impl Plugin for StreamingPlugin {
    fn name(&self) -> &'static str {
        "StreamingPlugin"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &[
            "AssetPlugin",
            "ScenePlugin",
            "PhysicsPlugin",
            "Render3dPlugin",
        ]
    }

    fn build(&self, app: &mut App) {
        // SceneDoc is decoded by the same bounded AssetServer worker pool as other CPU assets.
        // The .scenedoc extension avoids taking ownership of generic .ron application files.
        app.world
            .resource_mut::<AssetServer>()
            .register_loader(crate::SceneDocLoader);
        app.world
            .resource_mut::<AssetServer>()
            .register_loader(crate::WorldPartitionLoader);
        app.insert_resource(StreamingCoordinator::new(self.config.clone()));
        app.insert_resource(StreamDiagnostics::default());
        if app.world.try_resource::<StreamingAssetAdapters>().is_none() {
            app.insert_resource(StreamingAssetAdapters::default());
        }
        app.add_system(Stage::Update, update_streaming);
        // RenderProfilePlugin is normally installed after the streaming plugin. Publishing at Last
        // places these counters in the same RenderDiagnostics snapshot consumed by its CSV recorder.
        app.add_system(Stage::Last, publish_streaming_profile_fields);
    }
}

fn update_streaming(world: &mut World) {
    world.resource_scope::<StreamingCoordinator, _>(|world, coordinator| coordinator.tick(world));
    let snapshot = world
        .resource::<StreamingCoordinator>()
        .diagnostics()
        .clone();
    world.insert_resource(snapshot);
}

fn publish_streaming_profile_fields(world: &mut World) {
    let Some(stream) = world.try_resource::<StreamDiagnostics>().cloned() else {
        return;
    };
    let Some(mut resources) = world
        .try_resource::<RenderDiagnostics>()
        .map(|item| item.resources.clone())
    else {
        return;
    };
    resources.streaming_region_count = stream.region_count;
    resources.streaming_loading_regions = stream.loading_regions;
    resources.streaming_ready_regions = stream.ready_regions;
    resources.streaming_active_regions = stream.active_regions;
    resources.streaming_dormant_regions = stream.dormant_regions;
    resources.streaming_failed_regions = stream.failed_regions;
    resources.streaming_asset_jobs_queued = stream.queued_asset_jobs;
    resources.streaming_asset_queue_capacity = stream.asset_queue_capacity;
    resources.streaming_asset_requests_pending = stream.pending_asset_requests;
    resources.streaming_asset_workers_active = stream.active_asset_workers;
    resources.streaming_cpu_update_ns = stream.last_update_cpu_ns;
    resources.streaming_cpu_memory_bytes_estimate = stream.cpu_resident_bytes_estimate;
    resources.streaming_cpu_cache_budget_bytes = stream.cpu_unpinned_cache_budget_bytes;
    resources.streaming_cpu_cache_over_budget_bytes = stream.cpu_memory_over_budget_bytes;
    resources.streaming_cpu_resident_budget_bytes = stream.cpu_resident_budget_bytes;
    resources.streaming_cpu_resident_over_budget_bytes = stream.cpu_resident_over_budget_bytes;
    resources.streaming_gpu_memory_bytes_estimate = stream.gpu_resident_bytes_estimate;
    resources.streaming_memory_pressure_events = stream.memory_pressure_events;
    resources.streaming_cpu_budget_evictions_cumulative = stream.cpu_budget_evictions_cumulative;
    resources.streaming_invalid_state_transitions = stream.invalid_state_transitions;
    world
        .resource_mut::<RenderDiagnostics>()
        .record_resources(resources);
}

#[cfg(test)]
mod tests {
    use super::{
        RegionDefinition, RegionId, RegionPolicy, StreamObserver, StreamingConfig,
        StreamingCoordinator,
    };
    use crate::{RegionBounds, StreamingAssetAdapters};
    use glam::Vec3;
    use ribbon_app::World;
    use std::collections::HashSet;

    // Mirror the resource installed by StreamingPlugin::build. Unit tests construct the
    // coordinator directly, so they must provide its adapter registry explicitly.
    fn test_world() -> World {
        let mut world = World::new();
        world.insert_resource(StreamingAssetAdapters::default());
        world
    }

    fn region() -> RegionDefinition {
        region_with_id("test")
    }

    fn region_with_id(id: &str) -> RegionDefinition {
        RegionDefinition::new(
            RegionId::new(id),
            format!("{id} region"),
            RegionBounds::new(Vec3::splat(-10.0), Vec3::splat(10.0)).unwrap(),
        )
    }

    #[test]
    fn authored_partition_registration_sorts_dependencies_and_rejects_cycles_atomically() {
        use crate::{WorldPartitionAsset, WorldRegionAsset};

        let region = |id: &str, dependencies: Vec<String>| WorldRegionAsset {
            id: id.to_owned(),
            name: format!("{id} region"),
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [10.0, 10.0, 10.0],
            scene_doc_path: None,
            models: Vec::new(),
            dependencies,
            neighbors: Vec::new(),
            policy: Default::default(),
        };
        let partition = WorldPartitionAsset {
            format_version: 1,
            id: "authored-world".into(),
            name: "Authored World".into(),
            // Deliberately put the dependent region first to verify topological registration.
            regions: vec![
                region("village", vec!["room".into()]),
                region("room", Vec::new()),
            ],
        };
        let mut coordinator = StreamingCoordinator::default();
        let registered = coordinator.register_partition(&partition).unwrap();
        assert_eq!(
            registered,
            vec![RegionId::new("room"), RegionId::new("village")]
        );
        assert_eq!(coordinator.snapshot().regions.len(), 2);

        let cyclic = WorldPartitionAsset {
            format_version: 1,
            id: "cyclic-world".into(),
            name: "Cyclic World".into(),
            regions: vec![
                region("alpha", vec!["beta".into()]),
                region("beta", vec!["alpha".into()]),
            ],
        };
        let before = coordinator.snapshot().regions.len();
        assert!(coordinator
            .register_partition(&cyclic)
            .unwrap_err()
            .contains("cycle"));
        assert_eq!(coordinator.snapshot().regions.len(), before);
    }

    #[test]
    fn scene_doc_region_loads_and_owns_prefab_entities_through_activation_and_unload() {
        use crate::SceneDocLoader;
        use ribbon_assets::{AssetServer, MemorySource};
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;
        use ribbon_reflect::TypeRegistry;
        use ribbon_scene::{Prefab, PrefabCodecRegistry, PrefabEntity, SceneDoc};
        use std::collections::HashMap;

        let document = SceneDoc {
            prefab: Prefab {
                format_version: 1,
                name: "streamed prefab region".into(),
                entities: vec![PrefabEntity {
                    local_id: 7,
                    parent: None,
                    components: Vec::new(),
                }],
            },
            metadata: HashMap::new(),
        };
        let source = MemorySource::default();
        source.insert(
            "regions/test.scenedoc",
            document.to_ron().unwrap().into_bytes(),
        );
        let mut assets = AssetServer::default();
        assets.register_loader(SceneDocLoader);
        assets.add_source(0, source);
        // Preload in test setup; the production coordinator uses the same lease API via its
        // bounded worker queue and never performs synchronous IO itself.
        assets.load::<SceneDoc>("regions/test.scenedoc").unwrap();

        let mut world = test_world();
        world.insert_resource(assets);
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(TypeRegistry::default());
        world.insert_resource(PrefabCodecRegistry::default());
        world.insert_resource(PhysicsDebugData::default());

        let mut coordinator = StreamingCoordinator::default();
        let id = RegionId::new("scene-doc-region");
        coordinator
            .register_region(
                region_with_id("scene-doc-region").with_scene_doc("regions/test.scenedoc"),
            )
            .unwrap();
        coordinator.request_region(&id).unwrap();
        coordinator.tick(&mut world);
        coordinator.tick(&mut world);
        assert_eq!(
            coordinator.region_state(&id),
            Some(&super::RegionState::Ready)
        );

        coordinator.activate_region(&id).unwrap();
        coordinator.tick(&mut world);
        coordinator.tick(&mut world);
        assert_eq!(
            coordinator.region_state(&id),
            Some(&super::RegionState::Active)
        );
        let snapshot = coordinator.snapshot();
        assert_eq!(snapshot.regions[0].prefab_entity_count, 1);
        let entity = coordinator.regions[&id].prefab_entities[0];
        assert!(world.contains(entity));

        coordinator.release_region_request(&id).unwrap();
        coordinator.tick(&mut world);
        assert_eq!(
            coordinator.region_state(&id),
            Some(&super::RegionState::Unloaded)
        );
        assert!(!world.contains(entity));
    }

    #[test]
    fn prefab_owned_colliders_are_part_of_region_physics_readiness_and_cleanup() {
        use glam::{Quat, Vec3};
        use ribbon_plugin_physics::{
            Collider, ColliderShape, PhysicsDebugCollider, PhysicsDebugData,
        };

        let mut world = test_world();
        world.insert_resource(PhysicsDebugData::default());
        let mut coordinator = StreamingCoordinator::default();
        let id = RegionId::new("prefab-physics-region");
        coordinator
            .register_region(region_with_id("prefab-physics-region"))
            .unwrap();
        let entity = world.spawn((Collider::cuboid(Vec3::ONE),));
        coordinator
            .regions
            .get_mut(&id)
            .unwrap()
            .prefab_entities
            .push(entity);

        assert!(!coordinator.physics_entities_registered(&world, &id));
        world
            .resource_mut::<PhysicsDebugData>()
            .colliders
            .push(PhysicsDebugCollider {
                entity,
                shape: ColliderShape::Cuboid {
                    half_extents: Vec3::ONE,
                },
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
                sensor: false,
            });
        assert!(coordinator.physics_entities_registered(&world, &id));

        coordinator.despawn_region_entities(&mut world, &id);
        assert!(!world.contains(entity));
        assert_eq!(
            coordinator.regions[&id].pending_collider_cleanup,
            vec![entity]
        );
        assert!(!coordinator.physics_cleanup_complete(&world, &id));
        world.resource_mut::<PhysicsDebugData>().colliders.clear();
        assert!(coordinator.physics_cleanup_complete(&world, &id));
    }

    #[test]
    fn logical_scene_aggregates_region_readiness_and_activation() {
        use super::{RegionState, SceneDefinition, SceneId, SceneState};

        let mut coordinator = StreamingCoordinator::default();
        let first = RegionId::new("scene-first");
        let second = RegionId::new("scene-second");
        coordinator
            .register_region(region_with_id(first.as_str()))
            .unwrap();
        coordinator
            .register_region(region_with_id(second.as_str()))
            .unwrap();
        let scene = SceneId::new("intro");
        coordinator
            .register_scene(SceneDefinition::new(
                scene.clone(),
                "Intro Scene",
                [first.clone(), second.clone()],
            ))
            .unwrap();
        coordinator.request_scene(&scene).unwrap();
        assert_eq!(coordinator.scene_state(&scene), Some(SceneState::Requested));

        coordinator.regions.get_mut(&first).unwrap().state = RegionState::Ready;
        coordinator.regions.get_mut(&second).unwrap().state = RegionState::Ready;
        assert_eq!(coordinator.scene_state(&scene), Some(SceneState::Ready));

        coordinator.regions.get_mut(&first).unwrap().state = RegionState::Active;
        assert_eq!(
            coordinator.scene_state(&scene),
            Some(SceneState::Activating)
        );
        coordinator.regions.get_mut(&first).unwrap().state = RegionState::Ready;
        coordinator.regions.get_mut(&second).unwrap().state = RegionState::Activating;
        assert_eq!(
            coordinator.scene_state(&scene),
            Some(SceneState::Activating)
        );
        // A scene becomes Active only when every member region is Active. Restore the
        // first region as well before asserting the aggregate terminal state.
        coordinator.regions.get_mut(&first).unwrap().state = RegionState::Active;
        coordinator.regions.get_mut(&second).unwrap().state = RegionState::Active;
        assert_eq!(coordinator.scene_state(&scene), Some(SceneState::Active));

        coordinator.regions.get_mut(&second).unwrap().state =
            RegionState::Failed("test failure".to_owned());
        assert!(matches!(
            coordinator.scene_state(&scene),
            Some(SceneState::Failed(regions)) if regions == vec![second]
        ));
    }

    #[test]
    fn scene_requests_are_reference_counted_for_shared_regions() {
        use super::{SceneDefinition, SceneId};

        let mut coordinator = StreamingCoordinator::default();
        let shared = RegionId::new("shared-scene-region");
        coordinator
            .register_region(region_with_id(shared.as_str()))
            .unwrap();
        let first = SceneId::new("scene-a");
        let second = SceneId::new("scene-b");
        coordinator
            .register_scene(SceneDefinition::new(
                first.clone(),
                "Scene A",
                [shared.clone()],
            ))
            .unwrap();
        coordinator
            .register_scene(SceneDefinition::new(
                second.clone(),
                "Scene B",
                [shared.clone()],
            ))
            .unwrap();
        coordinator.request_scene(&first).unwrap();
        coordinator.request_scene(&second).unwrap();
        coordinator.release_scene_request(&first).unwrap();
        assert!(coordinator
            .regions
            .get(&shared)
            .unwrap()
            .has_request_intent());
        coordinator.release_scene_request(&second).unwrap();
        assert!(!coordinator
            .regions
            .get(&shared)
            .unwrap()
            .has_request_intent());

        coordinator.request_region(&shared).unwrap();
        coordinator.request_scene(&first).unwrap();
        coordinator.release_scene_request(&first).unwrap();
        assert!(coordinator
            .regions
            .get(&shared)
            .unwrap()
            .has_request_intent());
    }

    #[test]
    fn registration_rejects_invalid_hysteresis_and_duplicate_ids() {
        let mut coordinator = StreamingCoordinator::new(StreamingConfig::default());
        let invalid = region().with_policy(RegionPolicy {
            activation_distance: 100.0,
            preload_distance: 50.0,
            unload_distance: 200.0,
            ..RegionPolicy::default()
        });
        assert!(coordinator.register_region(invalid).is_err());
        coordinator.register_region(region()).unwrap();
        assert!(coordinator.register_region(region()).is_err());
    }

    #[test]
    fn multiple_observers_use_any_for_preload_and_activation() {
        let mut coordinator = StreamingCoordinator::default();
        coordinator.register_region(region()).unwrap();
        coordinator
            .add_observer(StreamObserver::new(1, Vec3::splat(1000.0)))
            .unwrap();
        coordinator
            .add_observer(StreamObserver::new(2, Vec3::ZERO))
            .unwrap();
        let interest = coordinator.interest_for(&region());
        assert!(interest.preload);
        assert!(interest.activate);
        assert!(!interest.beyond_unload);
    }

    #[test]
    fn no_observer_does_not_evict_an_explicitly_requested_region() {
        let mut coordinator = StreamingCoordinator::default();
        coordinator.register_region(region()).unwrap();
        let id = RegionId::new("test");
        coordinator.request_region(&id).unwrap();
        assert!(coordinator.regions.get(&id).unwrap().manual_requested);
        assert!(!coordinator.interest_for(&region()).beyond_unload);
    }

    #[test]
    fn dependencies_must_exist_and_duplicate_dependencies_are_rejected() {
        let mut coordinator = StreamingCoordinator::default();
        let missing = region().with_dependencies([RegionId::new("missing")]);
        assert!(coordinator.register_region(missing).is_err());
        let dependency = RegionDefinition::new(
            RegionId::new("base"),
            "base",
            RegionBounds::new(Vec3::ZERO, Vec3::ONE).unwrap(),
        );
        coordinator.register_region(dependency).unwrap();
        let child = RegionDefinition::new(
            RegionId::new("child"),
            "child",
            RegionBounds::new(Vec3::splat(2.0), Vec3::splat(3.0)).unwrap(),
        )
        .with_dependencies([RegionId::new("base"), RegionId::new("base")]);
        assert!(coordinator.register_region(child).is_err());
    }

    #[test]
    fn failed_dependency_propagates_instead_of_leaving_parent_loading_forever() {
        use super::{RegionDefinition, RegionState, StreamingCoordinator};
        use ribbon_assets::AssetServer;
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;

        let mut world = test_world();
        world.insert_resource(AssetServer::default());
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());

        let dependency_id = RegionId::new("failed-dependency");
        let parent_id = RegionId::new("waiting-parent");
        let mut coordinator = StreamingCoordinator::default();
        coordinator
            .register_region(RegionDefinition::new(
                dependency_id.clone(),
                "failed dependency",
                RegionBounds::new(Vec3::ZERO, Vec3::ONE).unwrap(),
            ))
            .unwrap();
        coordinator
            .register_region(
                RegionDefinition::new(
                    parent_id.clone(),
                    "parent",
                    RegionBounds::new(Vec3::splat(2.0), Vec3::splat(3.0)).unwrap(),
                )
                .with_dependencies([dependency_id.clone()]),
            )
            .unwrap();
        coordinator.regions.get_mut(&dependency_id).unwrap().state =
            RegionState::Failed("decoder rejected required content".to_owned());
        coordinator.request_region(&parent_id).unwrap();

        coordinator.tick(&mut world);

        assert!(matches!(
            coordinator.region_state(&parent_id),
            Some(RegionState::Failed(reason))
                if reason.contains("failed-dependency") && reason.contains("decoder rejected")
        ));
        assert_eq!(
            coordinator
                .regions
                .get(&dependency_id)
                .unwrap()
                .dependency_users,
            0
        );
    }

    #[test]
    fn empty_region_prepares_activates_and_unloads_without_duplicate_entities() {
        use super::{RegionState, StreamingCoordinator};
        use ribbon_assets::AssetServer;
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;

        let mut world = test_world();
        world.insert_resource(AssetServer::default());
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());

        let mut coordinator = StreamingCoordinator::default();
        let id = RegionId::new("test");
        coordinator.register_region(region()).unwrap();
        coordinator.request_region(&id).unwrap();
        coordinator.request_region(&id).unwrap();

        coordinator.tick(&mut world);
        assert_eq!(
            coordinator.region_state(&id),
            Some(&RegionState::WarmingGpu)
        );
        coordinator.tick(&mut world);
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Ready));

        coordinator.activate_region(&id).unwrap();
        coordinator.tick(&mut world);
        assert_eq!(
            coordinator.region_state(&id),
            Some(&RegionState::Activating)
        );
        coordinator.tick(&mut world);
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Active));
        assert_eq!(
            world.len(),
            0,
            "an empty region must not create ECS entities"
        );

        coordinator.release_region_request(&id).unwrap();
        coordinator.tick(&mut world);
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Unloaded));
    }

    #[test]
    fn dormant_region_recreates_visual_entities_when_reactivated() {
        use super::{RegionPointLight, RegionState, StreamingCoordinator};
        use ribbon_assets::AssetServer;
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::{ModelWarmup, PointLight};

        let mut world = test_world();
        world.insert_resource(AssetServer::default());
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());
        let id = RegionId::new("reusable-region");
        let definition = region_with_id("reusable-region")
            .with_point_light(RegionPointLight::new(PointLight::default()));
        let mut coordinator = StreamingCoordinator::default();
        coordinator.register_region(definition).unwrap();
        coordinator.activate_region(&id).unwrap();
        coordinator.tick(&mut world);
        coordinator.tick(&mut world);
        coordinator.tick(&mut world);
        coordinator.tick(&mut world);
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Active));
        assert_eq!(world.len(), 1);

        coordinator.release_region_request(&id).unwrap();
        coordinator
            .add_observer(StreamObserver::new(4, Vec3::new(200.0, 0.0, 0.0)))
            .unwrap();
        coordinator.tick(&mut world);
        // This region contains no colliders, so there is no physics cleanup to wait for.
        // Deactivation can therefore complete directly to Dormant in the same update.
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Dormant));
        coordinator.tick(&mut world);
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Dormant));
        assert_eq!(world.len(), 0);
        assert_eq!(
            coordinator.regions.get(&id).unwrap().pending_visual_spawn,
            0
        );

        coordinator
            .update_observer(StreamObserver::new(4, Vec3::ZERO))
            .unwrap();
        coordinator.tick(&mut world);
        assert_eq!(
            coordinator.region_state(&id),
            Some(&RegionState::Activating)
        );
        coordinator.tick(&mut world);
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Active));
        assert_eq!(
            world.len(),
            1,
            "reactivation must recreate the visual entity"
        );
    }

    #[test]
    fn zero_entity_spawn_budget_cannot_mark_unspawned_colliders_ready() {
        use super::{RegionCollider, RegionPointLight, RegionState, StreamingCoordinator};
        use glam::Vec3;
        use ribbon_assets::AssetServer;
        use ribbon_plugin_physics::{Collider, PhysicsDebugData};
        use ribbon_plugin_render3d::ModelWarmup;
        use ribbon_scene::Transform3D;

        let mut world = test_world();
        world.insert_resource(AssetServer::default());
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());
        let id = RegionId::new("one-collider");
        let definition = RegionDefinition::new(
            id.clone(),
            "one collider",
            RegionBounds::new(Vec3::ZERO, Vec3::ONE).unwrap(),
        )
        .with_collider(RegionCollider::new(
            Transform3D::default(),
            Collider::cuboid(Vec3::ONE),
        ))
        .with_point_light(RegionPointLight::new(
            ribbon_plugin_render3d::PointLight::default(),
        ));
        let mut coordinator = StreamingCoordinator::new(StreamingConfig {
            max_entity_spawns_per_update: 0,
            ..StreamingConfig::default()
        });
        coordinator.register_region(definition).unwrap();
        coordinator.activate_region(&id).unwrap();
        coordinator.tick(&mut world);
        coordinator.tick(&mut world);
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Ready));
        coordinator.tick(&mut world);
        assert_eq!(
            coordinator.region_state(&id),
            Some(&RegionState::PreparingPhysics)
        );
        assert_eq!(world.len(), 0);
        assert_eq!(
            coordinator.regions.get(&id).unwrap().pending_visual_spawn,
            0
        );
        assert_eq!(
            coordinator.regions.get(&id).unwrap().pending_collider_spawn,
            0
        );
        assert!(!coordinator.physics_entities_registered(&world, &id));
    }

    #[test]
    fn observer_velocity_prioritizes_regions_in_the_direction_of_travel() {
        let mut coordinator = StreamingCoordinator::new(StreamingConfig {
            spatial_cell_size: 10.0,
            ..StreamingConfig::default()
        });
        let policy = RegionPolicy {
            activation_distance: 0.0,
            preload_distance: 1000.0,
            unload_distance: 1200.0,
            base_priority: 5,
            auto_activate: false,
            preview_when_ready: false,
        };
        let forward = RegionDefinition::new(
            RegionId::new("forward"),
            "forward",
            RegionBounds::new(Vec3::new(90.0, -1.0, -1.0), Vec3::new(100.0, 1.0, 1.0)).unwrap(),
        )
        .with_policy(policy);
        let backward = RegionDefinition::new(
            RegionId::new("backward"),
            "backward",
            RegionBounds::new(Vec3::new(-100.0, -1.0, -1.0), Vec3::new(-90.0, 1.0, 1.0)).unwrap(),
        )
        .with_policy(policy);
        coordinator.register_region(forward.clone()).unwrap();
        coordinator.register_region(backward.clone()).unwrap();
        coordinator
            .add_observer(StreamObserver::new(7, Vec3::ZERO).with_velocity(Vec3::X))
            .unwrap();
        assert!(
            coordinator.interest_for(&forward).priority
                > coordinator.interest_for(&backward).priority
        );
    }

    #[test]
    fn admission_budget_starts_the_highest_priority_region_first() {
        use super::{RegionState, StreamingConfig, StreamingCoordinator};
        use ribbon_assets::AssetServer;
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;

        let mut world = test_world();
        world.insert_resource(AssetServer::default());
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());

        let bounds = RegionBounds::new(Vec3::splat(-10.0), Vec3::splat(10.0)).unwrap();
        let high = RegionDefinition::new(RegionId::new("critical"), "critical", bounds)
            .with_policy(RegionPolicy {
                base_priority: 100,
                ..RegionPolicy::default()
            });
        let low = RegionDefinition::new(RegionId::new("background"), "background", bounds)
            .with_policy(RegionPolicy {
                base_priority: -10,
                ..RegionPolicy::default()
            });
        let high_id = high.id.clone();
        let low_id = low.id.clone();
        let mut coordinator = StreamingCoordinator::new(StreamingConfig {
            max_new_region_requests_per_update: 1,
            ..StreamingConfig::default()
        });
        coordinator.register_region(high).unwrap();
        coordinator.register_region(low).unwrap();
        coordinator
            .add_observer(StreamObserver::new(1, Vec3::ZERO))
            .unwrap();

        coordinator.tick(&mut world);
        assert_eq!(
            coordinator.region_state(&high_id),
            Some(&RegionState::WarmingGpu)
        );
        assert_eq!(
            coordinator.region_state(&low_id),
            Some(&RegionState::Requested)
        );
    }

    #[test]
    fn current_loading_generation_is_not_released_and_reacquired_each_update() {
        use super::{RegionModel, RegionState, StreamingCoordinator};
        use ribbon_assets::{AssetServer, FsSource};
        use ribbon_plugin_gltf::{GltfLoader, ModelAsset};
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;
        use ribbon_scene::Transform3D;
        use std::path::Path;

        let mut assets = AssetServer::default();
        assets.register_loader(GltfLoader);
        assets.add_source(
            0,
            FsSource::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models")),
        );
        let mut world = test_world();
        world.insert_resource(assets);
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());

        let id = RegionId::new("current-loading-generation");
        let mut coordinator = StreamingCoordinator::default();
        coordinator
            .register_region(
                RegionDefinition::new(
                    id.clone(),
                    "current loading generation",
                    RegionBounds::new(Vec3::splat(-10.0), Vec3::splat(10.0)).unwrap(),
                )
                .with_model(RegionModel::new("checker_cube.glb", Transform3D::default())),
            )
            .unwrap();
        coordinator.request_region(&id).unwrap();

        coordinator.tick(&mut world);
        let handle = coordinator.model_handle(&id, "checker_cube.glb").unwrap();
        assert_eq!(
            world
                .resource::<AssetServer>()
                .current_handle::<ModelAsset>("checker_cube.glb"),
            Some(handle)
        );
        assert_eq!(world.resource::<AssetServer>().strong_ref_count(handle), 1);
        assert_eq!(
            coordinator.region_state(&id),
            Some(&RegionState::LoadingAssets)
        );

        coordinator.tick(&mut world);
        assert_eq!(
            world.resource::<AssetServer>().strong_ref_count(handle),
            1,
            "a current in-flight generation should be waited on, not re-leased every frame"
        );
        assert_eq!(
            coordinator.model_handle(&id, "checker_cube.glb"),
            Some(handle)
        );
        assert_eq!(
            coordinator.region_state(&id),
            Some(&RegionState::LoadingAssets)
        );
    }

    #[test]
    fn per_update_asset_request_budget_limits_submissions_for_large_regions() {
        use super::{RegionModel, RegionState, StreamingConfig, StreamingCoordinator};
        use ribbon_assets::{AssetServer, FsSource};
        use ribbon_plugin_gltf::GltfLoader;
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;
        use ribbon_scene::Transform3D;
        use std::path::Path;

        let mut world = test_world();
        let mut assets = AssetServer::default();
        assets.register_loader(GltfLoader);
        assets.add_source(
            0,
            FsSource::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models")),
        );
        world.insert_resource(assets);
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());

        let id = RegionId::new("budgeted-assets");
        let definition = RegionDefinition::new(
            id.clone(),
            "budgeted assets",
            RegionBounds::new(Vec3::splat(-10.0), Vec3::splat(10.0)).unwrap(),
        )
        .with_model(RegionModel::new(
            "bare_triangle.glb",
            Transform3D::default(),
        ))
        .with_model(RegionModel::new("checker_cube.glb", Transform3D::default()));
        let mut coordinator = StreamingCoordinator::new(StreamingConfig {
            max_asset_requests_per_update: 1,
            ..StreamingConfig::default()
        });
        coordinator.register_region(definition).unwrap();
        coordinator.request_region(&id).unwrap();

        coordinator.tick(&mut world);
        let record = coordinator.regions.get(&id).unwrap();
        assert_eq!(record.next_asset_request, 1);
        assert_eq!(record.leases.len(), 1);
        assert_eq!(coordinator.diagnostics().pending_asset_requests, 1);
        assert_eq!(
            coordinator.region_state(&id),
            Some(&RegionState::LoadingAssets)
        );

        coordinator.tick(&mut world);
        let record = coordinator.regions.get(&id).unwrap();
        assert_eq!(record.next_asset_request, 2);
        assert_eq!(record.leases.len(), 2);
        assert_eq!(coordinator.diagnostics().pending_asset_requests, 0);
    }

    #[test]
    fn recent_stream_events_remain_available_across_updates() {
        use super::{RegionState, StreamingCoordinator};
        use ribbon_assets::AssetServer;
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;

        let mut world = test_world();
        world.insert_resource(AssetServer::default());
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());
        let id = RegionId::new("event-history");
        let mut coordinator = StreamingCoordinator::default();
        coordinator
            .register_region(region_with_id(id.as_str()))
            .unwrap();
        coordinator.request_region(&id).unwrap();
        coordinator.tick(&mut world);
        let first_history = coordinator.diagnostics().recent_events.clone();
        assert!(!first_history.is_empty());
        coordinator.tick(&mut world);
        let second_history = &coordinator.diagnostics().recent_events;
        assert!(second_history.len() >= first_history.len());
        assert!(second_history
            .iter()
            .any(|event| { event.region == id && event.to == RegionState::WarmingGpu }));
    }

    #[test]
    fn invalid_state_transitions_are_rejected_and_counted() {
        use super::{RegionState, StreamingCoordinator};
        let mut coordinator = StreamingCoordinator::default();
        let id = RegionId::new("state-machine");
        coordinator
            .register_region(region_with_id(id.as_str()))
            .unwrap();
        coordinator.set_state(&id, RegionState::Active, "test invalid transition");
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Unloaded));
        assert_eq!(coordinator.diagnostics().invalid_state_transitions, 1);
        assert!(coordinator
            .diagnostics()
            .recent_events
            .back()
            .unwrap()
            .message
            .contains("rejected invalid"));
    }

    #[test]
    fn deactivating_region_still_reserves_active_region_capacity_until_physics_cleanup() {
        use super::{RegionState, StreamingCoordinator};
        let mut coordinator = StreamingCoordinator::default();
        let id = RegionId::new("deactivating");
        coordinator
            .register_region(region_with_id(id.as_str()))
            .unwrap();
        coordinator.regions.get_mut(&id).unwrap().state = RegionState::Deactivating;
        assert_eq!(coordinator.active_region_count(), 1);
    }

    #[test]
    fn unloading_region_with_pending_physics_cleanup_keeps_its_activation_slot() {
        use super::{RegionState, StreamingCoordinator};
        let mut world = test_world();
        let entity = world.spawn(());
        let mut coordinator = StreamingCoordinator::default();
        let id = RegionId::new("unloading-slot");
        coordinator
            .register_region(region_with_id(id.as_str()))
            .unwrap();
        let record = coordinator.regions.get_mut(&id).unwrap();
        record.state = RegionState::Unloading;
        record.pending_collider_cleanup.push(entity);
        assert_eq!(coordinator.active_region_count(), 1);
        assert!(!coordinator.physics_cleanup_complete(&world, &id));
    }

    #[test]
    fn unload_waiting_for_physics_is_cancelled_when_an_observer_reenters_range() {
        use super::{RegionState, StreamingCoordinator};
        use ribbon_plugin_physics::PhysicsDebugData;

        let mut world = test_world();
        let collider_entity = world.spawn(());
        world.insert_resource(PhysicsDebugData::default());
        let id = RegionId::new("reentered-unload");
        let mut coordinator = StreamingCoordinator::default();
        coordinator
            .register_region(region_with_id(id.as_str()))
            .unwrap();
        {
            let record = coordinator.regions.get_mut(&id).unwrap();
            record.state = RegionState::Unloading;
            record.pending_collider_cleanup.push(collider_entity);
            record.manual_requested = false;
        }
        coordinator
            .add_observer(StreamObserver::new(7, Vec3::ZERO))
            .unwrap();
        let mut asset_budget = 0;
        let mut activation_budget = 0;
        let mut entity_budget = 0;
        coordinator.advance_region(
            &mut world,
            &id,
            false,
            &mut asset_budget,
            &mut activation_budget,
            &mut entity_budget,
        );
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Dormant));
        assert!(coordinator
            .regions
            .get(&id)
            .unwrap()
            .pending_collider_cleanup
            .is_empty());
    }

    #[test]
    fn active_transition_refreshes_region_lru_sequence() {
        use super::{RegionState, StreamingCoordinator};
        let mut coordinator = StreamingCoordinator::default();
        let id = RegionId::new("lru-region");
        coordinator
            .register_region(region_with_id(id.as_str()))
            .unwrap();
        let initial = coordinator.regions.get(&id).unwrap().last_used_sequence;
        coordinator.set_state(&id, RegionState::Requested, "test request");
        coordinator.set_state(&id, RegionState::LoadingAssets, "test load");
        coordinator.set_state(&id, RegionState::WarmingGpu, "test warmup");
        coordinator.set_state(&id, RegionState::Ready, "test ready");
        coordinator.set_state(&id, RegionState::PreparingPhysics, "test physics");
        coordinator.set_state(&id, RegionState::Activating, "test activating");
        coordinator.set_state(&id, RegionState::Active, "test active");
        assert!(coordinator.regions.get(&id).unwrap().last_used_sequence > initial);
    }

    #[test]
    fn missing_physics_snapshot_does_not_confirm_pending_collider_cleanup() {
        use super::{RegionState, StreamingCoordinator};
        let mut world = test_world();
        let entity = world.spawn(());
        let mut coordinator = StreamingCoordinator::default();
        let id = RegionId::new("await-physics-snapshot");
        coordinator
            .register_region(region_with_id(id.as_str()))
            .unwrap();
        let record = coordinator.regions.get_mut(&id).unwrap();
        record.state = RegionState::Deactivating;
        record.pending_collider_cleanup.push(entity);
        assert!(!coordinator.physics_cleanup_complete(&world, &id));
        assert_eq!(
            coordinator
                .regions
                .get(&id)
                .unwrap()
                .pending_collider_cleanup,
            vec![entity]
        );
        assert_eq!(coordinator.active_region_count(), 1);
    }

    #[test]
    fn invalid_spatial_cell_size_is_normalized_in_public_config() {
        let coordinator = StreamingCoordinator::new(StreamingConfig {
            spatial_cell_size: f32::NAN,
            ..StreamingConfig::default()
        });
        assert_eq!(
            coordinator.config().spatial_cell_size,
            coordinator.index.cell_size()
        );
        assert!(coordinator.config().spatial_cell_size.is_finite());
        assert!(coordinator.config().spatial_cell_size > 0.0);
    }

    #[test]
    fn region_registration_normalizes_non_unit_rotations_and_rejects_negative_light_color() {
        use super::{RegionModel, RegionPointLight, StreamingCoordinator};
        use ribbon_plugin_render3d::PointLight;
        use ribbon_scene::Transform3D;

        let mut coordinator = StreamingCoordinator::default();
        let unit_rotation = glam::Quat::from_rotation_y(0.75);
        let rotation = glam::Quat::from_xyzw(
            unit_rotation.x * 3.0,
            unit_rotation.y * 3.0,
            unit_rotation.z * 3.0,
            unit_rotation.w * 3.0,
        );
        let definition = region().with_model(RegionModel::new(
            "checker_cube.glb",
            Transform3D {
                rotation,
                ..Transform3D::default()
            },
        ));
        let id = definition.id.clone();
        coordinator.register_region(definition).unwrap();
        let stored = coordinator.regions.get(&id).unwrap().definition.models[0]
            .transform
            .rotation;
        assert!((stored.length_squared() - 1.0).abs() < 1.0e-5);

        let invalid_light = region().with_point_light(RegionPointLight::new(PointLight {
            color: Vec3::new(1.0, -0.1, 1.0),
            ..PointLight::default()
        }));
        assert!(coordinator.register_region(invalid_light).is_err());
    }

    #[test]
    fn cpu_residency_pressure_releases_a_unique_dormant_region_lease() {
        use super::{model_key, RegionModel, RegionState, StreamingConfig, StreamingCoordinator};
        use ribbon_assets::{AssetServer, FsSource};
        use ribbon_plugin_gltf::{GltfLoader, ModelAsset};
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;
        use ribbon_render::profiling::RenderDiagnostics;
        use ribbon_scene::Transform3D;
        use std::path::Path;

        let mut assets = AssetServer::default();
        assets.register_loader(GltfLoader);
        assets.add_source(
            0,
            FsSource::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models")),
        );
        let lease = assets
            .load_ref::<ModelAsset>("checker_cube.glb")
            .expect("test GLB loads synchronously and can be pinned");
        let handle = lease.handle();
        let bytes = assets
            .get(handle)
            .expect("loaded asset is resident")
            .estimated_size_bytes();
        assert!(bytes > 0);

        let mut world = test_world();
        world.insert_resource(assets);
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());
        world.insert_resource(RenderDiagnostics::default());
        world
            .resource_mut::<ModelWarmup>()
            .request(handle, "checker_cube.glb");

        let id = RegionId::new("cpu-pressure-region");
        let definition = region_with_id(id.as_str())
            .with_model(RegionModel::new("checker_cube.glb", Transform3D::default()));
        let mut coordinator = StreamingCoordinator::new(StreamingConfig {
            memory: super::StreamingMemoryBudget {
                cpu_cache_bytes: None,
                cpu_resident_bytes: Some(1),
                gpu_estimate_bytes: None,
            },
            ..StreamingConfig::default()
        });
        coordinator.register_region(definition).unwrap();
        {
            let record = coordinator.regions.get_mut(&id).unwrap();
            record.state = RegionState::Dormant;
            record.resident_cpu_bytes_estimate = bytes;
            record.leases.insert("checker_cube.glb".to_owned(), lease);
        }
        coordinator
            .warmup_users
            .insert(model_key(handle), (handle, 1));
        assert_eq!(world.resource::<AssetServer>().strong_ref_count(handle), 1);
        assert_eq!(world.resource::<ModelWarmup>().len(), 1);

        coordinator.apply_memory_pressure(&mut world, &HashSet::new());

        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Unloaded));
        assert_eq!(coordinator.diagnostics().memory_pressure_events, 1);
        assert_eq!(world.resource::<AssetServer>().strong_ref_count(handle), 0);
        assert!(world.resource::<ModelWarmup>().is_empty());
        let stats = world.resource::<AssetServer>().cpu_memory_stats();
        assert_eq!(stats.pinned_bytes_estimate, 0);
        assert_eq!(stats.unpinned_bytes_estimate, bytes);

        // The coordinator only releases ownership. AssetServer performs its own unpinned cache
        // eviction when its separate cache budget is enforced.
        world
            .resource_mut::<AssetServer>()
            .set_cpu_memory_budget_bytes(Some(0));
        assert_eq!(
            world
                .resource_mut::<AssetServer>()
                .enforce_cpu_memory_budget(),
            1
        );
        assert!(world.resource::<AssetServer>().get(handle).is_none());
    }

    #[test]
    fn cpu_total_budget_reduces_unpinned_cache_allowance_without_invalidating_pinned_assets() {
        use ribbon_assets::{AssetServer, FsSource};
        use ribbon_plugin_gltf::{GltfLoader, ModelAsset};
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;
        use std::path::Path;

        let mut assets = AssetServer::default();
        assets.register_loader(GltfLoader);
        assets.add_source(
            0,
            FsSource::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models")),
        );
        let lease = assets.load_ref::<ModelAsset>("checker_cube.glb").unwrap();
        let handle = lease.handle();
        let pinned_bytes = assets.cpu_memory_stats().pinned_bytes_estimate;
        assert!(pinned_bytes > 0);

        let mut world = test_world();
        world.insert_resource(assets);
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());
        let mut coordinator = StreamingCoordinator::new(StreamingConfig {
            memory: super::StreamingMemoryBudget {
                cpu_cache_bytes: None,
                cpu_resident_bytes: Some(pinned_bytes.saturating_sub(1)),
                gpu_estimate_bytes: None,
            },
            ..StreamingConfig::default()
        });

        coordinator.tick(&mut world);

        let stats = world.resource::<AssetServer>().cpu_memory_stats();
        assert_eq!(stats.budget_bytes, Some(0));
        assert_eq!(stats.pinned_bytes_estimate, pinned_bytes);
        assert!(world.resource::<AssetServer>().get(handle).is_some());
        assert_eq!(coordinator.diagnostics().cpu_unpinned_cache_budget_bytes, 0);
        assert_eq!(coordinator.diagnostics().cpu_memory_over_budget_bytes, 0);
        assert_eq!(coordinator.diagnostics().cpu_resident_over_budget_bytes, 1);
    }

    #[test]
    fn shared_cpu_asset_leases_prevent_region_pressure_from_invalidating_other_regions() {
        use super::{RegionModel, RegionState, StreamingConfig, StreamingCoordinator};
        use ribbon_assets::{AssetServer, FsSource};
        use ribbon_plugin_gltf::{GltfLoader, ModelAsset};
        use ribbon_plugin_physics::PhysicsDebugData;
        use ribbon_plugin_render3d::ModelWarmup;
        use ribbon_render::profiling::RenderDiagnostics;
        use ribbon_scene::Transform3D;
        use std::path::Path;

        let mut assets = AssetServer::default();
        assets.register_loader(GltfLoader);
        assets.add_source(
            0,
            FsSource::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models")),
        );
        let first = assets.load_ref::<ModelAsset>("checker_cube.glb").unwrap();
        let handle = first.handle();
        let second = assets
            .strong_ref(handle)
            .expect("second consumer shares loaded asset");

        let mut world = test_world();
        world.insert_resource(assets);
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(PhysicsDebugData::default());
        world.insert_resource(RenderDiagnostics::default());
        let mut coordinator = StreamingCoordinator::new(StreamingConfig {
            memory: super::StreamingMemoryBudget {
                cpu_cache_bytes: None,
                cpu_resident_bytes: Some(1),
                gpu_estimate_bytes: None,
            },
            ..StreamingConfig::default()
        });
        let first_id = RegionId::new("shared-first");
        let second_id = RegionId::new("shared-second");
        coordinator
            .register_region(
                RegionDefinition::new(
                    first_id.clone(),
                    "shared first",
                    RegionBounds::new(Vec3::splat(-10.0), Vec3::splat(10.0)).unwrap(),
                )
                .with_model(RegionModel::new("checker_cube.glb", Transform3D::default())),
            )
            .unwrap();
        coordinator
            .register_region(
                RegionDefinition::new(
                    second_id.clone(),
                    "shared second",
                    RegionBounds::new(Vec3::splat(20.0), Vec3::splat(30.0)).unwrap(),
                )
                .with_model(RegionModel::new("checker_cube.glb", Transform3D::default())),
            )
            .unwrap();
        coordinator.regions.get_mut(&first_id).unwrap().state = RegionState::Dormant;
        coordinator
            .regions
            .get_mut(&first_id)
            .unwrap()
            .leases
            .insert("checker_cube.glb".to_owned(), first);
        coordinator.regions.get_mut(&second_id).unwrap().state = RegionState::Dormant;
        coordinator
            .regions
            .get_mut(&second_id)
            .unwrap()
            .leases
            .insert("checker_cube.glb".to_owned(), second);

        coordinator.apply_memory_pressure(&mut world, &HashSet::new());

        assert_eq!(
            coordinator.region_state(&first_id),
            Some(&RegionState::Dormant)
        );
        assert_eq!(
            coordinator.region_state(&second_id),
            Some(&RegionState::Dormant)
        );
        assert_eq!(world.resource::<AssetServer>().strong_ref_count(handle), 2);
    }

    #[test]
    fn memory_pressure_does_not_evict_dormant_regions_inside_preload_or_neighbor_range() {
        use super::{RegionState, StreamingCoordinator};
        use ribbon_render::profiling::RenderDiagnostics;

        let mut world = test_world();
        let mut diagnostics = RenderDiagnostics::default();
        diagnostics.resources.mesh_bytes_estimate = 1024;
        world.insert_resource(diagnostics);

        let id = RegionId::new("wanted-dormant");
        let mut definition = region_with_id(id.as_str());
        definition.policy = RegionPolicy {
            preload_distance: 100.0,
            activation_distance: 20.0,
            unload_distance: 150.0,
            ..RegionPolicy::default()
        };
        let mut coordinator = StreamingCoordinator::new(StreamingConfig {
            memory: super::StreamingMemoryBudget {
                cpu_cache_bytes: None,
                cpu_resident_bytes: None,
                gpu_estimate_bytes: Some(1),
            },
            ..StreamingConfig::default()
        });
        coordinator.register_region(definition).unwrap();
        coordinator.regions.get_mut(&id).unwrap().state = RegionState::Dormant;
        coordinator
            .add_observer(StreamObserver::new(1, Vec3::ZERO))
            .unwrap();
        coordinator.apply_memory_pressure(&mut world, &HashSet::new());
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Dormant));

        coordinator.remove_observer(1);
        let neighbor = HashSet::from([id.clone()]);
        coordinator.apply_memory_pressure(&mut world, &neighbor);
        assert_eq!(coordinator.region_state(&id), Some(&RegionState::Dormant));
    }

    #[test]
    fn collider_shape_scale_and_zero_rotation_are_validated_before_loading() {
        use super::{RegionCollider, RegionModel, StreamingCoordinator};
        use ribbon_plugin_physics::Collider;
        use ribbon_scene::Transform3D;
        let mut coordinator = StreamingCoordinator::default();
        let invalid_ball = RegionDefinition::new(
            RegionId::new("invalid-ball"),
            "invalid ball",
            RegionBounds::new(Vec3::ZERO, Vec3::ONE).unwrap(),
        )
        .with_collider(RegionCollider::new(
            Transform3D {
                scale: Vec3::new(1.0, 2.0, 1.0),
                ..Transform3D::default()
            },
            Collider::ball(1.0),
        ));
        assert!(coordinator.register_region(invalid_ball).is_err());

        let invalid_rotation = RegionDefinition::new(
            RegionId::new("invalid-rotation"),
            "invalid rotation",
            RegionBounds::new(Vec3::ZERO, Vec3::ONE).unwrap(),
        )
        .with_model(RegionModel::new(
            "checker_cube.glb",
            Transform3D {
                rotation: glam::Quat::from_xyzw(0.0, 0.0, 0.0, 0.0),
                ..Transform3D::default()
            },
        ));
        assert!(coordinator.register_region(invalid_rotation).is_err());
    }
}
