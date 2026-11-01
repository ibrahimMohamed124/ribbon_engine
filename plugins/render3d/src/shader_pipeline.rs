//! Generation-aware, renderer-owned shader and render-pipeline asset registry.
//!
//! CPU-side WGSL and pipeline recipes are retained independently from device-owned objects.
//! All `wgpu` creation, validation, and destruction is performed by `pump`/`reset_after_device_loss`
//! on the render-owning thread. Applications must increment the key generation when shader source
//! or pipeline configuration changes; old generations remain alive while any consumer holds a lease.

use crate::gpu_streaming::GpuResourceKey;
use ribbon_assets::{
    AssetLoader, AssetMemoryBreakdown, AssetRef, AssetServer, LoadContext, LoadError, LoadErrorKind,
};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

type ValidationFuture = Pin<Box<dyn Future<Output = Option<wgpu::Error>> + Send + 'static>>;

#[derive(Debug, Default)]
struct LeaseCounter(AtomicUsize);

impl LeaseCounter {
    fn acquire(self: &Arc<Self>, key: GpuResourceKey) -> ShaderModuleLease {
        self.0.fetch_add(1, Ordering::AcqRel);
        ShaderModuleLease {
            key,
            counter: Arc::clone(self),
        }
    }

    fn acquire_pipeline(self: &Arc<Self>, key: GpuResourceKey) -> RenderPipelineLease {
        self.0.fetch_add(1, Ordering::AcqRel);
        RenderPipelineLease {
            key,
            counter: Arc::clone(self),
        }
    }

    fn count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

/// Strong lease for an immutable shader-module generation.
#[derive(Debug)]
pub struct ShaderModuleLease {
    key: GpuResourceKey,
    counter: Arc<LeaseCounter>,
}

impl ShaderModuleLease {
    pub fn key(&self) -> GpuResourceKey {
        self.key
    }
}

impl Clone for ShaderModuleLease {
    fn clone(&self) -> Self {
        self.counter.acquire(self.key)
    }
}

impl Drop for ShaderModuleLease {
    fn drop(&mut self) {
        self.counter.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Strong lease for an immutable render-pipeline generation.
#[derive(Debug)]
pub struct RenderPipelineLease {
    key: GpuResourceKey,
    counter: Arc<LeaseCounter>,
}

impl RenderPipelineLease {
    pub fn key(&self) -> GpuResourceKey {
        self.key
    }
}

impl Clone for RenderPipelineLease {
    fn clone(&self) -> Self {
        self.counter.acquire_pipeline(self.key)
    }
}

impl Drop for RenderPipelineLease {
    fn drop(&mut self) {
        self.counter.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// State of one shader module generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShaderModuleState {
    Queued,
    Preparing,
    Ready,
    Failed(String),
}

/// State of one pipeline generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderPipelineState {
    WaitingForShader,
    Preparing,
    Ready,
    Failed(String),
}

/// Stable CPU-side description for a shader module. WGSL remains available for device recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderModuleAsset {
    pub label: String,
    pub wgsl: String,
}

/// Stable namespace used to derive GPU shader identities from AssetServer IDs.
pub const SHADER_ASSET_GPU_NAMESPACE: u64 = 0x5348_4452_4153_5345;

/// Asynchronous AssetServer loader for standalone WGSL source files.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShaderSourceLoader;

impl AssetLoader for ShaderSourceLoader {
    type Asset = ShaderModuleAsset;

    fn extensions(&self) -> &[&str] {
        &["wgsl"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        let wgsl = std::str::from_utf8(bytes).map_err(|error| {
            LoadError::new(
                LoadErrorKind::Decode,
                &context.path,
                format!("WGSL source is not valid UTF-8: {error}"),
            )
        })?;
        if wgsl.trim().is_empty() {
            return Err(LoadError::new(
                LoadErrorKind::Decode,
                &context.path,
                "WGSL source is empty",
            ));
        }
        Ok(ShaderModuleAsset {
            label: context.path.clone(),
            wgsl: wgsl.to_owned(),
        })
    }

    fn estimated_size_bytes(&self, asset: &Self::Asset) -> u64 {
        (std::mem::size_of::<ShaderModuleAsset>() as u64)
            .saturating_add(asset.label.capacity() as u64)
            .saturating_add(asset.wgsl.capacity() as u64)
    }

    fn memory_breakdown(&self, asset: &Self::Asset) -> AssetMemoryBreakdown {
        AssetMemoryBreakdown::other(self.estimated_size_bytes(asset))
    }
}

/// Pipeline builder invoked only on the render-owning thread after its shader is Ready.
///
/// The callback must recreate every device-owned object it needs from `device` and `shader` each
/// time it is invoked. For portable bind-group contracts, prefer `layout: None` and obtain layouts
/// from the resulting pipeline with `get_bind_group_layout`; explicit layouts may be created inside
/// the callback and are owned by the resulting wgpu pipeline after creation.
pub type RenderPipelineBuilder = Arc<
    dyn Fn(&wgpu::Device, &wgpu::ShaderModule) -> Result<wgpu::RenderPipeline, String>
        + Send
        + Sync
        + 'static,
>;

struct ShaderEntry {
    asset: ShaderModuleAsset,
    /// Pins the originating AssetServer generation when this shader came from a `.wgsl` asset.
    source_lease: Option<AssetRef<ShaderModuleAsset>>,
    state: ShaderModuleState,
    module: Option<wgpu::ShaderModule>,
    validation: Option<Mutex<ValidationFuture>>,
    leases: Arc<LeaseCounter>,
}

struct PipelineEntry {
    label: String,
    /// Caller-computed stable hash of the full pipeline descriptor and layout contract.
    recipe_fingerprint: u64,
    shader: ShaderModuleLease,
    builder: RenderPipelineBuilder,
    state: RenderPipelineState,
    pipeline: Option<wgpu::RenderPipeline>,
    validation: Option<Mutex<ValidationFuture>>,
    leases: Arc<LeaseCounter>,
}

/// Counts and bounded-work information for the shader/pipeline registry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderPipelineRegistryStats {
    pub shader_modules: usize,
    pub shader_modules_ready: usize,
    pub shader_modules_failed: usize,
    pub render_pipelines: usize,
    pub render_pipelines_ready: usize,
    pub render_pipelines_failed: usize,
    pub pending_validations: usize,
    pub retained_wgsl_bytes: u64,
    pub source_asset_leases: usize,
    pub leased_shader_modules: usize,
    pub leased_render_pipelines: usize,
    pub creations_started: u64,
    pub validation_failures: u64,
    pub released_generations: u64,
}

/// Renderer-thread registry for shader modules and render pipelines.
///
/// This is a first-class asset registry, not a wrapper around the renderer's private static fields:
/// callers provide stable generation keys, retain leases, inspect readiness/failure, and the
/// registry replays retained recipes after device loss. It intentionally does not claim an exact
/// GPU byte size for shader/pipeline driver allocations; wgpu exposes no portable accounting API.
#[derive(Default)]
pub struct ShaderPipelineRegistry {
    shaders: BTreeMap<GpuResourceKey, ShaderEntry>,
    pipelines: BTreeMap<GpuResourceKey, PipelineEntry>,
    creations_started: u64,
    validation_failures: u64,
    released_generations: u64,
}

impl ShaderPipelineRegistry {
    /// Queue or share an immutable WGSL source generation.
    pub fn request_shader(
        &mut self,
        key: GpuResourceKey,
        asset: ShaderModuleAsset,
    ) -> Result<ShaderModuleLease, String> {
        if asset.label.trim().is_empty() || asset.wgsl.trim().is_empty() {
            return Err("shader label and WGSL source must not be empty".to_owned());
        }
        if let Some(entry) = self.shaders.get(&key) {
            if entry.asset != asset {
                return Err(format!(
                    "shader key {key:?} already identifies different content; increment generation"
                ));
            }
            return Ok(entry.leases.acquire(key));
        }
        let leases = Arc::new(LeaseCounter::default());
        let lease = leases.acquire(key);
        self.shaders.insert(
            key,
            ShaderEntry {
                asset,
                source_lease: None,
                state: ShaderModuleState::Queued,
                module: None,
                validation: None,
                leases,
            },
        );
        Ok(lease)
    }

    /// Pin a loaded WGSL asset from AssetServer and queue its device-owned shader generation.
    /// The GPU key is derived from the asset's stable ID and generation, so hot reload creates a
    /// separate shader generation while any older pipeline/consumer lease remains valid.
    pub fn request_shader_asset(
        &mut self,
        assets: &AssetServer,
        source: &AssetRef<ShaderModuleAsset>,
    ) -> Result<ShaderModuleLease, String> {
        let handle = source.handle();
        let asset = assets.get(handle).ok_or_else(|| {
            format!(
                "WGSL source generation {:?} is not CPU-ready in AssetServer",
                handle
            )
        })?;
        let key = GpuResourceKey {
            namespace: SHADER_ASSET_GPU_NAMESPACE,
            id: source.asset_id().get(),
            generation: source.generation(),
        };
        let lease = self.request_shader(key, asset.clone())?;
        let entry = self
            .shaders
            .get_mut(&key)
            .expect("request_shader inserted or found the key");
        if let Some(existing) = &entry.source_lease {
            if existing != source {
                return Err(format!(
                    "shader generation {key:?} is already pinned to another AssetServer generation"
                ));
            }
        } else {
            entry.source_lease = Some(source.clone());
        }
        Ok(lease)
    }

    /// Queue or share a render pipeline generation depending on a leased shader generation.
    /// The pipeline retains its own shader lease, so callers may release their shader lease as
    /// soon as the pipeline request has been submitted.
    pub fn request_render_pipeline(
        &mut self,
        key: GpuResourceKey,
        label: impl Into<String>,
        recipe_fingerprint: u64,
        shader: &ShaderModuleLease,
        builder: RenderPipelineBuilder,
    ) -> Result<RenderPipelineLease, String> {
        let Some(shader_entry) = self.shaders.get(&shader.key) else {
            return Err(format!(
                "shader generation {:?} is not registered",
                shader.key
            ));
        };
        if !Arc::ptr_eq(&shader_entry.leases, &shader.counter) {
            return Err(format!(
                "shader lease {:?} belongs to another registry",
                shader.key
            ));
        }
        let label = label.into();
        if label.trim().is_empty() {
            return Err("render-pipeline label must not be empty".to_owned());
        }
        if let Some(entry) = self.pipelines.get(&key) {
            if entry.shader.key != shader.key
                || entry.label != label
                || entry.recipe_fingerprint != recipe_fingerprint
            {
                return Err(format!(
                    "pipeline key {key:?} already identifies a different shader or recipe fingerprint; increment generation"
                ));
            }
            return Ok(entry.leases.acquire_pipeline(key));
        }
        let leases = Arc::new(LeaseCounter::default());
        let lease = leases.acquire_pipeline(key);
        self.pipelines.insert(
            key,
            PipelineEntry {
                label,
                recipe_fingerprint,
                shader: shader.clone(),
                builder,
                state: RenderPipelineState::WaitingForShader,
                pipeline: None,
                validation: None,
                leases,
            },
        );
        Ok(lease)
    }

    pub fn shader_state(&self, lease: &ShaderModuleLease) -> Option<&ShaderModuleState> {
        self.shaders
            .get(&lease.key)
            .filter(|entry| Arc::ptr_eq(&entry.leases, &lease.counter))
            .map(|entry| &entry.state)
    }

    pub fn pipeline_state(&self, lease: &RenderPipelineLease) -> Option<&RenderPipelineState> {
        self.pipelines
            .get(&lease.key)
            .filter(|entry| Arc::ptr_eq(&entry.leases, &lease.counter))
            .map(|entry| &entry.state)
    }

    /// Resolve a Ready shader module. The returned reference is valid only until the next mutable
    /// registry operation and must be consumed on the render-owning thread.
    pub fn shader_module(&self, lease: &ShaderModuleLease) -> Option<&wgpu::ShaderModule> {
        let entry = self.shaders.get(&lease.key)?;
        if !Arc::ptr_eq(&entry.leases, &lease.counter) {
            return None;
        }
        (entry.state == ShaderModuleState::Ready)
            .then_some(entry.module.as_ref())
            .flatten()
    }

    /// Resolve a Ready render pipeline. The lease must remain alive while the caller retains any
    /// renderer-side references to the pipeline generation.
    pub fn render_pipeline(&self, lease: &RenderPipelineLease) -> Option<&wgpu::RenderPipeline> {
        let entry = self.pipelines.get(&lease.key)?;
        if !Arc::ptr_eq(&entry.leases, &lease.counter) {
            return None;
        }
        (entry.state == RenderPipelineState::Ready)
            .then_some(entry.pipeline.as_ref())
            .flatten()
    }

    /// Service at most `max_creations` pending GPU creations. One asynchronous error scope is
    /// outstanding at a time, preventing scope-stack ambiguity. Validation completion is polled
    /// without blocking; queued assets remain observable while a scope is pending.
    pub fn pump(&mut self, device: &wgpu::Device, max_creations: usize) {
        if max_creations == 0 {
            self.collect_orphans();
            return;
        }
        let has_validation = self.has_pending_validation();
        if has_validation {
            let _ = device.poll(wgpu::PollType::Poll);
            self.poll_validation();
        }
        self.collect_orphans();
        if self.has_pending_validation() {
            return;
        }

        if let Some((key, label, source)) = self
            .shaders
            .iter()
            .find(|(_, entry)| entry.state == ShaderModuleState::Queued && entry.leases.count() > 0)
            .map(|(key, entry)| (*key, entry.asset.label.clone(), entry.asset.wgsl.clone()))
        {
            device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            device.push_error_scope(wgpu::ErrorFilter::Validation);
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let validation_result = device.pop_error_scope();
            let out_of_memory_result = device.pop_error_scope();
            let future =
                Box::pin(async move { validation_result.await.or(out_of_memory_result.await) });
            if let Some(entry) = self.shaders.get_mut(&key) {
                entry.module = Some(module);
                entry.validation = Some(Mutex::new(future));
                entry.state = ShaderModuleState::Preparing;
            }
            self.creations_started = self.creations_started.saturating_add(1);
            return;
        }

        if let Some((key, shader_key, label, builder)) = self
            .pipelines
            .iter()
            .find(|(_, entry)| {
                entry.state == RenderPipelineState::WaitingForShader && entry.leases.count() > 0
            })
            .map(|(key, entry)| {
                (
                    *key,
                    entry.shader.key,
                    entry.label.clone(),
                    Arc::clone(&entry.builder),
                )
            })
        {
            let shader_ready = self.shaders.get(&shader_key).is_some_and(|entry| {
                entry.state == ShaderModuleState::Ready && entry.module.is_some()
            });
            if let Some(shader_entry) = self.shaders.get(&shader_key) {
                if let ShaderModuleState::Failed(message) = &shader_entry.state {
                    if let Some(entry) = self.pipelines.get_mut(&key) {
                        entry.state = RenderPipelineState::Failed(format!(
                            "shader dependency failed: {message}"
                        ));
                    }
                    return;
                }
            }
            if !shader_ready {
                return;
            }
            let shader_module = self
                .shaders
                .get(&shader_key)
                .and_then(|entry| entry.module.as_ref())
                .expect("ready shader was checked");
            device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            device.push_error_scope(wgpu::ErrorFilter::Validation);
            match builder(device, shader_module) {
                Ok(pipeline) => {
                    let validation_result = device.pop_error_scope();
                    let out_of_memory_result = device.pop_error_scope();
                    let future =
                        Box::pin(
                            async move { validation_result.await.or(out_of_memory_result.await) },
                        );
                    if let Some(entry) = self.pipelines.get_mut(&key) {
                        entry.pipeline = Some(pipeline);
                        entry.validation = Some(Mutex::new(future));
                        entry.state = RenderPipelineState::Preparing;
                    }
                    self.creations_started = self.creations_started.saturating_add(1);
                }
                Err(message) => {
                    // Even when the application-side recipe fails, pop and poll the wgpu scope.
                    // Dropping an unpolled pop future would leave the device's scope stack open.
                    let validation_result = device.pop_error_scope();
                    let out_of_memory_result = device.pop_error_scope();
                    let future =
                        Box::pin(
                            async move { validation_result.await.or(out_of_memory_result.await) },
                        );
                    if let Some(entry) = self.pipelines.get_mut(&key) {
                        entry.validation = Some(Mutex::new(future));
                        entry.state = RenderPipelineState::Failed(format!("{label}: {message}"));
                    }
                    self.validation_failures = self.validation_failures.saturating_add(1);
                }
            }
        }
    }

    fn has_pending_validation(&self) -> bool {
        self.shaders
            .values()
            .any(|entry| entry.validation.is_some())
            || self
                .pipelines
                .values()
                .any(|entry| entry.validation.is_some())
    }

    fn poll_validation(&mut self) {
        let waker = Waker::noop();
        let mut shader_results = Vec::new();
        for (key, entry) in &mut self.shaders {
            let Some(future) = entry.validation.as_ref() else {
                continue;
            };
            let mut future = future
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut context = Context::from_waker(waker);
            if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
                shader_results.push((*key, result.map(|error| error.to_string())));
            }
        }
        for (key, result) in shader_results {
            if let Some(entry) = self.shaders.get_mut(&key) {
                entry.validation = None;
                match result {
                    Some(message) => {
                        entry.module = None;
                        entry.state = ShaderModuleState::Failed(message);
                        self.validation_failures = self.validation_failures.saturating_add(1);
                    }
                    None => entry.state = ShaderModuleState::Ready,
                }
            }
        }

        let mut pipeline_results = Vec::new();
        for (key, entry) in &mut self.pipelines {
            let Some(future) = entry.validation.as_ref() else {
                continue;
            };
            let mut future = future
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut context = Context::from_waker(waker);
            if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
                pipeline_results.push((*key, result.map(|error| error.to_string())));
            }
        }
        for (key, result) in pipeline_results {
            if let Some(entry) = self.pipelines.get_mut(&key) {
                entry.validation = None;
                if matches!(entry.state, RenderPipelineState::Failed(_)) {
                    if let Some(message) = result {
                        if let RenderPipelineState::Failed(existing) = &mut entry.state {
                            existing.push_str("; wgpu scope: ");
                            existing.push_str(&message);
                        }
                    }
                    continue;
                }
                match result {
                    Some(message) => {
                        entry.pipeline = None;
                        entry.state = RenderPipelineState::Failed(message);
                        self.validation_failures = self.validation_failures.saturating_add(1);
                    }
                    None => entry.state = RenderPipelineState::Ready,
                }
            }
        }
    }

    /// Drop device-owned objects and requeue every leased generation using retained WGSL and
    /// pipeline builders. Must be called on the render-owning thread before dropping the device.
    pub fn reset_after_device_loss(&mut self) {
        for entry in self.pipelines.values_mut() {
            entry.pipeline = None;
            entry.validation = None;
            if entry.leases.count() > 0 {
                entry.state = RenderPipelineState::WaitingForShader;
            }
        }
        for entry in self.shaders.values_mut() {
            entry.module = None;
            entry.validation = None;
            if entry.leases.count() > 0 {
                entry.state = ShaderModuleState::Queued;
            }
        }
    }

    /// Release unused generations in dependency order (pipelines before their shader modules).
    /// This is called by `pump` on the renderer thread, so wgpu destruction uses the right owner.
    pub fn collect_orphans(&mut self) {
        let old_pipeline_count = self.pipelines.len();
        self.pipelines
            .retain(|_, entry| entry.leases.count() > 0 || entry.validation.is_some());
        self.released_generations = self
            .released_generations
            .saturating_add((old_pipeline_count - self.pipelines.len()) as u64);
        let old_shader_count = self.shaders.len();
        self.shaders
            .retain(|_, entry| entry.leases.count() > 0 || entry.validation.is_some());
        self.released_generations = self
            .released_generations
            .saturating_add((old_shader_count - self.shaders.len()) as u64);
    }

    pub fn stats(&self) -> ShaderPipelineRegistryStats {
        let mut stats = ShaderPipelineRegistryStats {
            shader_modules: self.shaders.len(),
            render_pipelines: self.pipelines.len(),
            creations_started: self.creations_started,
            validation_failures: self.validation_failures,
            released_generations: self.released_generations,
            ..ShaderPipelineRegistryStats::default()
        };
        for entry in self.shaders.values() {
            stats.retained_wgsl_bytes = stats
                .retained_wgsl_bytes
                .saturating_add(entry.asset.wgsl.len() as u64);
            stats.source_asset_leases += usize::from(entry.source_lease.is_some());
            stats.leased_shader_modules += usize::from(entry.leases.count() > 0);
            stats.shader_modules_ready += usize::from(entry.state == ShaderModuleState::Ready);
            stats.shader_modules_failed +=
                usize::from(matches!(entry.state, ShaderModuleState::Failed(_)));
            stats.pending_validations += usize::from(entry.validation.is_some());
        }
        for entry in self.pipelines.values() {
            stats.leased_render_pipelines += usize::from(entry.leases.count() > 0);
            stats.render_pipelines_ready += usize::from(entry.state == RenderPipelineState::Ready);
            stats.render_pipelines_failed +=
                usize::from(matches!(entry.state, RenderPipelineState::Failed(_)));
            stats.pending_validations += usize::from(entry.validation.is_some());
        }
        stats
    }
}

#[derive(Default)]
struct ShaderAssetLoaderInstalled(bool);

/// Install the WGSL AssetServer loader once the optional AssetPlugin is available.
pub fn ensure_shader_asset_loader(world: &mut ribbon_app::World) {
    if world.try_resource::<ShaderAssetLoaderInstalled>().is_none() {
        world.insert_resource(ShaderAssetLoaderInstalled::default());
    }
    if world.resource::<ShaderAssetLoaderInstalled>().0
        || world.try_resource::<AssetServer>().is_none()
    {
        return;
    }
    world.resource_scope::<AssetServer, _>(|world, assets| {
        assets.register_loader(ShaderSourceLoader);
        world.resource_mut::<ShaderAssetLoaderInstalled>().0 = true;
    });
}

#[cfg(test)]
mod tests {
    use super::{
        RenderPipelineBuilder, RenderPipelineState, ShaderModuleAsset, ShaderModuleState,
        ShaderPipelineRegistry, ShaderSourceLoader,
    };
    use crate::gpu_streaming::GpuResourceKey;
    use ribbon_assets::{AssetServer, MemorySource};
    use std::sync::Arc;

    fn key(id: u64, generation: u32) -> GpuResourceKey {
        GpuResourceKey {
            namespace: 0x5348_4452,
            id,
            generation,
        }
    }

    fn shader() -> ShaderModuleAsset {
        ShaderModuleAsset {
            label: "test shader".to_owned(),
            wgsl: concat!(
                "@vertex fn vs_main(@builtin(vertex_index) i: u32) -> ",
                "@builtin(position) vec4<f32> { ",
                "var p = array<vec2<f32>, 3>(",
                "vec2<f32>(0.0, 0.5), vec2<f32>(-0.5, -0.5), ",
                "vec2<f32>(0.5, -0.5)); ",
                "return vec4<f32>(p[i], 0.0, 1.0); } ",
                "@fragment fn fs_main() -> @location(0) vec4<f32> { ",
                "return vec4<f32>(1.0); }",
            )
            .to_owned(),
        }
    }

    fn builder() -> RenderPipelineBuilder {
        Arc::new(|device, shader| {
            Ok(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("registry test pipeline"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: Some("vs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: Some("fs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview: None,
                    cache: None,
                }),
            )
        })
    }

    #[test]
    fn wgsl_asset_loader_and_registry_keep_the_asset_generation_pinned() {
        let source = MemorySource::default();
        source.insert("shaders/test.wgsl", shader().wgsl.into_bytes());
        let mut assets = AssetServer::default();
        assets.add_source(0, source);
        assets.register_loader(ShaderSourceLoader);
        let asset_lease = assets
            .load_ref::<ShaderModuleAsset>("shaders/test.wgsl")
            .unwrap();
        let mut registry = ShaderPipelineRegistry::default();
        let source_handle = asset_lease.handle();
        let shader_lease = registry
            .request_shader_asset(&assets, &asset_lease)
            .unwrap();
        assert_eq!(shader_lease.key().generation, asset_lease.generation());
        // `AssetRef::clone` shares one strong lease family (one independent pin), rather than
        // incrementing AssetServer's independent-lease counter. The registry's clone must keep
        // that pin alive after the caller drops its copy.
        assert_eq!(assets.strong_ref_count(source_handle), 1);
        drop(asset_lease);
        assert_eq!(assets.strong_ref_count(source_handle), 1);
        assert!(registry.stats().retained_wgsl_bytes > 0);
        // Releasing the registry must release the final clone and therefore the AssetServer pin.
        drop(registry);
        assert_eq!(assets.strong_ref_count(source_handle), 0);
    }

    #[test]
    fn shader_generations_are_distinct_and_identical_requests_share_a_lease_family() {
        let mut registry = ShaderPipelineRegistry::default();
        let first = registry.request_shader(key(1, 1), shader()).unwrap();
        let shared = registry.request_shader(key(1, 1), shader()).unwrap();
        let replacement = registry.request_shader(key(1, 2), shader()).unwrap();
        assert_eq!(first.key(), shared.key());
        assert_ne!(first.key(), replacement.key());
        assert_eq!(registry.stats().shader_modules, 2);
        assert_eq!(registry.stats().leased_shader_modules, 2);
        assert!(registry
            .request_shader(
                key(1, 1),
                ShaderModuleAsset {
                    label: "different".to_owned(),
                    wgsl: "invalid".to_owned(),
                }
            )
            .is_err());
    }

    #[test]
    fn pipeline_lease_retains_shader_generation_after_caller_releases_shader_lease() {
        let mut registry = ShaderPipelineRegistry::default();
        let shader_lease = registry.request_shader(key(2, 4), shader()).unwrap();
        let pipeline = registry
            .request_render_pipeline(key(9, 1), "test pipeline", 0x9001, &shader_lease, builder())
            .unwrap();
        drop(shader_lease);
        registry.collect_orphans();
        assert_eq!(registry.stats().shader_modules, 1);
        assert_eq!(
            registry.pipeline_state(&pipeline),
            Some(&RenderPipelineState::WaitingForShader)
        );
        drop(pipeline);
        registry.collect_orphans();
        assert_eq!(registry.stats().shader_modules, 0);
        assert_eq!(registry.stats().render_pipelines, 0);
    }

    fn wait_until_pipeline_ready(
        registry: &mut ShaderPipelineRegistry,
        context: &crate::GpuContext,
        shader: &super::ShaderModuleLease,
        pipeline: &super::RenderPipelineLease,
    ) {
        for _ in 0..128 {
            registry.pump(context.device(), 1);
            let _ = context.device().poll(wgpu::PollType::Wait);
            registry.pump(context.device(), 1);
            if registry.shader_state(shader) == Some(&ShaderModuleState::Ready)
                && registry.pipeline_state(pipeline) == Some(&RenderPipelineState::Ready)
            {
                return;
            }
        }
        panic!("shader and pipeline did not become Ready on the selected GPU adapter");
    }

    /// Run explicitly on a Windows machine with a real wgpu adapter:
    /// `cargo test -p ribbon_plugin_render3d real_adapter_recreation_rebuilds_retained_shader_and_pipeline -- --ignored --nocapture`
    #[test]
    #[ignore = "requires an actual wgpu adapter and validates device recreation on hardware"]
    fn real_adapter_recreation_rebuilds_retained_shader_and_pipeline() {
        let first = crate::pollster::block_on(crate::GpuContext::new_offscreen(16, 16))
            .expect("a real wgpu adapter must be available for this hardware test");
        let first_adapter_info = first.adapter().get_info();
        eprintln!("initial adapter: {first_adapter_info:?}");
        assert_ne!(
            first_adapter_info.device_type,
            wgpu::DeviceType::Cpu,
            "this test requires a physical GPU adapter, not a software CPU adapter"
        );

        let mut registry = ShaderPipelineRegistry::default();
        let shader_lease = registry.request_shader(key(0xD001, 1), shader()).unwrap();
        let pipeline_lease = registry
            .request_render_pipeline(
                key(0xD002, 1),
                "hardware recreation pipeline",
                0xD002_0001,
                &shader_lease,
                builder(),
            )
            .unwrap();
        wait_until_pipeline_ready(&mut registry, &first, &shader_lease, &pipeline_lease);

        // Simulate device destruction after all device-owned registry objects are dropped.
        // This validates the registry replay contract against an actual adapter; the platform
        // window's DeviceLost callback/surface recovery still needs the interactive stress run.
        registry.reset_after_device_loss();
        first.device().destroy();
        drop(first);

        let recreated = crate::pollster::block_on(crate::GpuContext::new_offscreen(16, 16))
            .expect("wgpu adapter must be recreated for the second device");
        let recreated_adapter_info = recreated.adapter().get_info();
        eprintln!("recreated adapter: {recreated_adapter_info:?}");
        assert_ne!(
            recreated_adapter_info.device_type,
            wgpu::DeviceType::Cpu,
            "recreated context selected a software CPU adapter"
        );
        assert_eq!(
            (recreated_adapter_info.vendor, recreated_adapter_info.device),
            (first_adapter_info.vendor, first_adapter_info.device),
            "device recreation test must target the same physical adapter"
        );
        wait_until_pipeline_ready(&mut registry, &recreated, &shader_lease, &pipeline_lease);
        assert_eq!(
            registry.shader_state(&shader_lease),
            Some(&ShaderModuleState::Ready)
        );
        assert_eq!(
            registry.pipeline_state(&pipeline_lease),
            Some(&RenderPipelineState::Ready)
        );
    }

    #[test]
    fn pipeline_generation_rejects_a_changed_recipe_fingerprint() {
        let mut registry = ShaderPipelineRegistry::default();
        let shader_lease = registry.request_shader(key(8, 2), shader()).unwrap();
        let _pipeline = registry
            .request_render_pipeline(
                key(8, 5),
                "stable pipeline",
                0x1111,
                &shader_lease,
                builder(),
            )
            .unwrap();
        assert!(registry
            .request_render_pipeline(
                key(8, 5),
                "stable pipeline",
                0x2222,
                &shader_lease,
                builder(),
            )
            .is_err());
    }

    #[test]
    fn device_loss_requeues_leased_shader_and_pipeline_recipes() {
        let mut registry = ShaderPipelineRegistry::default();
        let shader_lease = registry.request_shader(key(3, 1), shader()).unwrap();
        let pipeline = registry
            .request_render_pipeline(key(4, 1), "test pipeline", 0x4001, &shader_lease, builder())
            .unwrap();
        registry.reset_after_device_loss();
        assert_eq!(
            registry.shader_state(&shader_lease),
            Some(&ShaderModuleState::Queued)
        );
        assert_eq!(
            registry.pipeline_state(&pipeline),
            Some(&RenderPipelineState::WaitingForShader)
        );
    }
}
