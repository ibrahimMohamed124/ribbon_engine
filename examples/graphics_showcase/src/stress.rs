use super::*;

pub(super) fn stress_asset_path(index: usize) -> String {
    format!("models/stress_{index:04}.glb")
}

pub(super) fn spawn_large_scene_grid(
    world: &mut World,
    models: &[(ribbon_assets::Handle<ModelAsset>, Vec3)],
    side: usize,
    generation: u64,
) -> usize {
    if models.is_empty() {
        return 0;
    }
    let offset = (side.saturating_sub(1) as f32) * STRESS_GRID_SPACING * 0.5;
    for z_index in 0..side {
        for x_index in 0..side {
            let x = x_index as f32 * STRESS_GRID_SPACING - offset;
            let z = z_index as f32 * STRESS_GRID_SPACING - offset;
            let cell_index = z_index.saturating_mul(side).saturating_add(x_index);
            let (model, local_center) = models[cell_index % models.len()];
            let scale = 0.72 + ((x_index + z_index * 3) % 4) as f32 * 0.045;
            let rotation = Quat::from_rotation_y(
                ((x_index + z_index) % 8) as f32 * std::f32::consts::FRAC_PI_8,
            );
            let center = Vec3::new(x, 0.55, z);
            world.spawn((
                Transform3D {
                    translation: center - rotation * (local_center * scale),
                    rotation,
                    scale: Vec3::splat(scale),
                },
                ModelInstance { model },
                StressPlacement { generation },
            ));
        }
    }
    side.saturating_mul(side)
}

pub(super) fn despawn_large_scene_grid(world: &mut World, generation: u64) -> usize {
    let entities: Vec<_> = world
        .query::<&StressPlacement>()
        .iter()
        .filter(|(_, placement)| placement.generation == generation)
        .map(|(entity, _)| entity)
        .collect();
    let count = entities.len();
    for entity in entities {
        let _ = world.despawn(entity);
    }
    count
}

pub(super) fn request_stress_leases(
    world: &mut World,
    paths: &[String],
) -> Result<Vec<AssetRef<ModelAsset>>, String> {
    let server = world.resource_mut::<AssetServer>();
    let mut leases = Vec::with_capacity(paths.len());
    for path in paths {
        let lease = if matches!(server.state::<ModelAsset>(path), Some(LoadState::Failed(_))) {
            let handle = server
                .retry_with_priority::<ModelAsset>(path, ACTIVE_ASSET_PRIORITY)
                .map_err(|error| format!("could not retry stress model `{path}`: {error}"))?;
            server.strong_ref(handle).ok_or_else(|| {
                format!("could not pin the retry handle for stress model `{path}`")
            })?
        } else {
            server
                .request_ref_with_priority::<ModelAsset>(path, ACTIVE_ASSET_PRIORITY)
                .map_err(|error| format!("could not request stress model `{path}`: {error}"))?
        };
        leases.push(lease);
    }
    Ok(leases)
}

pub(super) fn update_large_scene_stress(world: &mut World) {
    let frame = world.resource::<Time>().frame;
    let pending = world.try_resource::<LargeSceneStress>().and_then(|stress| {
        stress.pending_activation.then(|| {
            (
                stress
                    .leases
                    .iter()
                    .map(AssetRef::handle)
                    .collect::<Vec<_>>(),
                stress.model_paths.clone(),
                stress.grid_side,
                stress.generation,
                stress.cycle_frames,
            )
        })
    });

    if let Some((handles, paths, side, generation, cycle_frames)) = pending {
        if handles.is_empty() || handles.len() != paths.len() {
            let leases = {
                let stress = world.resource_mut::<LargeSceneStress>();
                stress.pending_activation = false;
                stress.next_cycle_frame = cycle_frames
                    .map(|interval| frame.saturating_add(interval))
                    .unwrap_or(u64::MAX);
                std::mem::take(&mut stress.leases)
            };
            drop(leases);
            eprintln!(
                "[ASSET STRESS] invalid pending activation: active asset paths and leases differ"
            );
            return;
        }

        enum ActivationState {
            Pending,
            Failed(String),
            Ready(Vec<(ribbon_assets::Handle<ModelAsset>, Vec3)>),
        }
        let activation = {
            let server = world.resource::<AssetServer>();
            let mut loaded_models = Vec::with_capacity(handles.len());
            let mut waiting_for_assets = false;
            let mut failure = None;
            for (handle, path) in handles.iter().copied().zip(&paths) {
                if let Some(model) = server.get(handle) {
                    match model.bounds() {
                        Some((min, max)) => loaded_models.push((handle, (min + max) * 0.5)),
                        None => {
                            failure = Some(format!("stress model `{path}` contains no triangles"))
                        }
                    }
                } else if let Some(LoadState::Failed(error)) = server.state::<ModelAsset>(path) {
                    failure = Some(format!("stress model `{path}` failed to load: {error}"));
                } else {
                    waiting_for_assets = true;
                }
            }
            if let Some(error) = failure {
                ActivationState::Failed(error)
            } else if waiting_for_assets || loaded_models.len() != handles.len() {
                ActivationState::Pending
            } else {
                ActivationState::Ready(loaded_models)
            }
        };

        match activation {
            ActivationState::Ready(models) => {
                let count = spawn_large_scene_grid(world, &models, side, generation);
                {
                    let stress = world.resource_mut::<LargeSceneStress>();
                    stress.active = true;
                    stress.pending_activation = false;
                    stress.next_cycle_frame = cycle_frames
                        .map(|interval| frame.saturating_add(interval))
                        .unwrap_or(u64::MAX);
                }
                let identities: Vec<_> = models
                    .iter()
                    .map(|(handle, _)| (handle.id(), handle.generation()))
                    .collect();
                println!(
                    "[ASSET STRESS] activated grid_generation={generation}; active_assets={}; model_asset_id/generations={identities:?}; placements={count}; CPU model leases restored",
                    models.len()
                );
                let cache_stats = world.resource::<AssetServer>().cpu_memory_stats();
                println!(
                    "[ASSET STRESS] CPU cache at activation: resident={} bytes; pinned={} bytes; unpinned={} bytes; budget={:?}; unpinned_over_budget={} bytes; budget_evictions={}",
                    cache_stats.resident_bytes_estimate,
                    cache_stats.pinned_bytes_estimate,
                    cache_stats.unpinned_bytes_estimate,
                    cache_stats.budget_bytes,
                    cache_stats.over_budget_bytes,
                    cache_stats.budget_evictions_cumulative
                );
                return;
            }
            ActivationState::Pending => return,
            ActivationState::Failed(error) => {
                let leases = {
                    let stress = world.resource_mut::<LargeSceneStress>();
                    stress.pending_activation = false;
                    stress.next_cycle_frame = cycle_frames
                        .map(|interval| frame.saturating_add(interval))
                        .unwrap_or(u64::MAX);
                    std::mem::take(&mut stress.leases)
                };
                drop(leases);
                let memory_budget_enabled = world
                    .resource::<AssetServer>()
                    .cpu_memory_stats()
                    .budget_bytes
                    .is_some();
                let (evicted, stats) = {
                    let server = world.resource_mut::<AssetServer>();
                    let evicted = if memory_budget_enabled {
                        server.enforce_cpu_memory_budget()
                    } else {
                        server.evict_unused()
                    };
                    (evicted, server.cpu_memory_stats())
                };
                eprintln!(
                    "[ASSET STRESS] activation failed: {error}; evicted={evicted}; resident_cpu_bytes={}; unpinned_cpu_bytes={}; budget_bytes={:?}",
                    stats.resident_bytes_estimate,
                    stats.unpinned_bytes_estimate,
                    stats.budget_bytes
                );
                return;
            }
        }
    }

    let should_toggle = world
        .try_resource::<LargeSceneStress>()
        .is_some_and(|stress| {
            stress.cycle_frames.is_some()
                && !stress.pending_activation
                && frame >= stress.next_cycle_frame
        });
    if !should_toggle {
        return;
    }

    let active = world.resource::<LargeSceneStress>().active;
    if active {
        let generation = world.resource::<LargeSceneStress>().generation;
        let despawned = despawn_large_scene_grid(world, generation);
        let leases = {
            let stress = world.resource_mut::<LargeSceneStress>();
            stress.active = false;
            stress.pending_activation = false;
            stress.generation = stress.generation.saturating_add(1);
            stress.next_cycle_frame = stress
                .cycle_frames
                .map(|interval| frame.saturating_add(interval))
                .unwrap_or(u64::MAX);
            std::mem::take(&mut stress.leases)
        };
        let asset_identities: Vec<_> = leases
            .iter()
            .map(|lease| (lease.asset_id().get(), lease.generation()))
            .collect();
        drop(leases);
        let (evicted, stats) = {
            let server = world.resource_mut::<AssetServer>();
            let evicted = if server.cpu_memory_stats().budget_bytes.is_some() {
                server.enforce_cpu_memory_budget()
            } else {
                server.evict_unused()
            };
            (evicted, server.cpu_memory_stats())
        };
        println!(
            "[ASSET STRESS] deactivated grid_generation={generation}; assets={asset_identities:?}; despawned={despawned}; CPU cache evictions={evicted}; resident_cpu_bytes={}; pinned_cpu_bytes={}; unpinned_cpu_bytes={}; budget_bytes={:?}; budget_evictions_cumulative={}",
            stats.resident_bytes_estimate,
            stats.pinned_bytes_estimate,
            stats.unpinned_bytes_estimate,
            stats.budget_bytes,
            stats.budget_evictions_cumulative
        );
        return;
    }

    let paths = world.resource::<LargeSceneStress>().model_paths.clone();
    match request_stress_leases(world, &paths) {
        Ok(leases) => {
            let generation = {
                let stress = world.resource_mut::<LargeSceneStress>();
                stress.leases = leases;
                stress.pending_activation = true;
                stress.next_cycle_frame = u64::MAX;
                stress.generation
            };
            println!(
                "[ASSET STRESS] reload requested for generation={generation}; active_assets={}; waiting for worker decode",
                paths.len()
            );
        }
        Err(error) => {
            eprintln!("[ASSET STRESS] {error}");
            let stress = world.resource_mut::<LargeSceneStress>();
            stress.next_cycle_frame = stress
                .cycle_frames
                .map(|interval| frame.saturating_add(interval))
                .unwrap_or(u64::MAX);
        }
    }
}
