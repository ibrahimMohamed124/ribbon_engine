from __future__ import annotations
from pathlib import Path
import re, struct, sys

ROOT = Path(__file__).resolve().parents[1]
checks = []
def check(label: str, ok: bool, detail: str = '') -> None:
    checks.append((label, bool(ok), detail))

def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding='utf-8')

main = read('games/athar/src/main.rs')
gameplay = read('games/athar/src/scripts/gameplay.rs')
mods = read('games/athar/src/scripts/mod.rs')
village = read('games/athar/src/scripts/village.rs')
render = read('plugins/render3d/src/lib.rs')
render_runtime = render.split('#[cfg(test)]\nmod model_warmup_integration_tests', 1)[0]
preload = read('plugins/render3d/src/preload.rs')
model = read('plugins/render3d/src/model.rs')
extract = read('plugins/render3d/src/extract.rs')
window = read('crates/render/src/gpu_window.rs')
weather = read('plugins/weather/src/lib.rs')
physics = read('plugins/physics/src/runtime.rs')
csv = read('crates/render/src/profiling/csv.rs')

paths = re.findall(r'"(village_[a-z]+\.glb)"', village.split('];', 1)[0])
check('seven unique village model paths', len(paths) == 7 and len(set(paths)) == 7, str(paths))
check('village models are described generically and requested through the streaming AssetServer path', '.streaming_models()' in main and 'with_model(model)' in main and 'retry_ref_with_priority::<ModelAsset>' in read('plugins/streaming/src/coordinator.rs'))
check('generic coordinator request starts village preparation at startup', 'coordinator.request_scene(&SceneId::new(scripts::village::VILLAGE_SCENE_ID))?' in main and 'StreamingPlugin::new' in main)
check('Region CPU/GPU readiness and failure are explicit', 'RegionState::Failed(reason)' in gameplay and 'RegionState::Ready' in gameplay and 'gpu_ready_models' in gameplay)
check('scene commit gated on VillageReady and room phase', 'phase == GamePhase::Room && matches!(state, VillageTransitionState::VillageReady)' in gameplay)
enter = gameplay[gameplay.index('fn enter_village('):gameplay.index('/// Consume one E press')]
check('scene and Region readiness plus collider readiness precede old room teardown', enter.index('scene_state') < enter.index('state.room_entities.drain') and 'SceneState::Active' in enter and 'RegionState::Active' in enter and 'collider_count' in enter)
check('village readiness includes completed GPU models and visual entities', 'all_models_ready' in gameplay and 'gpu_ready_models' in gameplay and 'preview_ready' in gameplay)
cache_liveness_helper = (
    render_runtime.split('fn collect_live_model_keys(', 1)[1].split('\n}', 1)[0]
    if 'fn collect_live_model_keys(' in render_runtime
    else ''
)
check(
    'renderer cache liveness includes visible placements and non-failed warmups independent of CPU residency',
    bool(cache_liveness_helper)
    and re.search(r'extracted\s*\.models\s*\.iter\(\)\s*\.map\(\|placement\|\s*placement\.model\)', cache_liveness_helper) is not None
    and re.search(r'warmup_snapshots\s*\.iter\(\)\s*\.filter_map', cache_liveness_helper) is not None
    and 'server.get(' not in cache_liveness_helper
    and 'model_liveness_includes_visible_placements_and_pending_warmups' in render,
)
check('per-frame GPU model upload cap remains bounded at two by default', re.search(r'max_models_per_frame:\s*2', preload) is not None)
check('GPU readiness polls both validation and OOM scopes', 'let validation_error = validation_result.await;' in render and 'let out_of_memory_error = out_of_memory_result.await;' in render)
check('warmup GPU scope polling is nonblocking', 'device.poll(wgpu::PollType::Poll)' in render_runtime and 'pollster::block_on' not in render_runtime)
check('invalid model resources fail before GPU upload', 'validate_model_for_upload(model)' in render or 'validate_model_for_upload(model)' in model)
check('unplaced primitives are not uploaded into orphan cache entries', 'used_primitives' in model and 'meshes[node.primitive]' in model)
check('warmup states enter renderer diagnostics', all(x in render for x in ['model_warmup_requested', 'model_warmup_waiting_for_cpu', 'model_warmup_waiting_for_gpu', 'model_warmup_ready', 'model_warmup_failed']))
check('warmup profiling columns appended to CSV schema', all(f'"{x}"' in csv for x in ['model_warmup_requested', 'model_warmup_waiting_for_cpu', 'model_warmup_waiting_for_gpu', 'model_warmup_ready', 'model_warmup_failed', 'model_gpu_uploads_this_frame']) and 'row[80] = resources.model_gpu_uploads_this_frame.to_string();' in csv)
check('scene extraction profile is conditional on profiling', 'RenderProfileRecorder' in extract and 'extraction_cpu_time_ns' in extract)
check('physics timings are explicitly gated to a short window', 'timing_enabled' in physics and '.then(std::time::Instant::now)' in physics and 'last_reconcile_time_ns' in physics and 'last_fixed_step_time_ns' in physics)
check('transition arms one-shot physics timing for next reconcile/fixed step', 'diagnostics.timing_enabled = true;' in gameplay and 'diagnostics.last_reconcile_time_ns = 0;' in gameplay)
check('rain allocation timing is measured only on creation batches', 'if created_count == 0' in weather and 'Instant::now()' in weather and 'RainDiagnostics' in weather)
check('ATHAR reports first village rain allocation batch', 'village_rain_profile_logged' in gameplay and 'rain particle batch' in gameplay)
check('window clear executes every frame independently of scene draws', 'struct WindowClearNode' in window and 'clear_view(target.device, target.queue, target.view, color);' in window)
check('transition regression test is present', 'village_commit_requires_ready_state_and_room_phase' in gameplay)
check('transition regression rejects reentrant commit states', 'VillageTransitionState::CommittingTransition' in gameplay and 'VillageTransitionState::VillageActive' in gameplay)
check('GPU warmup has a no-placement and cache-handoff integration test', 'model_warmup_is_gpu_ready_without_placement_and_cache_survives_handoff' in render)
check('empty-scene render pass preserves previously cleared output test exists', 'empty_scene_mesh_pass_preserves_the_window_clear_color' in render)
check('early-return frames reset per-frame model upload diagnostic', 'self.model_gpu_uploads_this_frame = 0;' in render_runtime and 'early_return_frame_resets_per_frame_model_upload_diagnostic' in render)
check('weather allocation regression test is present', 'rain_particle_creation_batch_is_measured_once_and_then_reuses_entities' in weather)
check('model upload cache-lifetime regression test is present', 'model_upload_does_not_allocate_gpu_meshes_for_unplaced_primitives' in model)
check('village spawn preflights before entity creation', 'missing_village_handle_fails_preflight_without_spawning_any_entities' in village)
check('generic streaming render staging is present and used for Region-owned entities', 'StagedRender' in render and 'StagedRender' in read('plugins/render3d/src/extract.rs') and 'RegionState::Ready' in gameplay)
check('preview visibility is gated by door progress and Region visual readiness', 'door_progress >= VILLAGE_PREVIEW_DOOR_PROGRESS' in gameplay and 'village_preview_is_resident' in gameplay and 'region.visual_entity_count' in gameplay)
check('Region activation requires authored Rapier colliders to be observed', 'with_collider(collider)' in main and 'physics_entities_registered(world, id)' in read('plugins/streaming/src/coordinator.rs') and 'required Rapier colliders are registered' in read('plugins/streaming/src/coordinator.rs'))
check('commit activates the prepared Scene through the coordinator without manual duplicate spawns', 'activate_scene(&SceneId::new(village::VILLAGE_SCENE_ID))?' in gameplay and 'finish_village_activation_if_ready' in gameplay and 'village::spawn_village(' not in gameplay)
check('opening door reveals non-black outdoor clear color while preparation continues', 'OUTDOOR_CLEAR_COLOR: [f64; 4] = [0.24, 0.29, 0.34, 1.0]' in gameplay and 'village_door_view_is_revealed' in gameplay)
check('outdoor lighting/far plane are activated after streaming preview is resident', 'if preview_ready && !state.village_preview_lighting_active' in gameplay and 'camera.far = 260.0;' in gameplay)
check('preview ground and road avoid coplanar room-floor overlap', 'VILLAGE_GROUND_Y: f32 = -0.02' in village and 'VILLAGE_ROAD_Y: f32 = -0.014' in village)
check('preview staging regression covers Region readiness and fully spawned visuals', 'streamed_preview_is_visible_only_after_region_readiness_and_all_visual_spawns' in gameplay)
check('door backdrop and delayed preview lighting have a regression test', 'open_door_reveals_backdrop_without_switching_lighting_before_streamed_preview' in gameplay)
check('scene plugin installs one-shot 3D transform timing diagnostics', 'TransformPropagationDiagnostics::default()' in read('crates/scene/src/lib.rs') and 'capture_next_3d' in read('crates/scene/src/hierarchy.rs') and 'requested_3d_transform_propagation_timing_is_one_shot' in read('crates/scene/src/hierarchy.rs'))
check('ATHAR arms and reports transition transform propagation timing', 'diagnostics.capture_next_3d = true;' in gameplay and 'first village 3D transform propagation' in gameplay)
check('render profile separates CPU-side model GPU resource creation/submission', 'model GPU resource creation/submission' in render and 'model GPU resource creation/submission' in render.split('#[cfg(test)]\nmod model_warmup_integration_tests', 1)[1])
check('CSV regression protects existing schema and warmup tail', 'csv_keeps_existing_fields_and_appends_warmup_columns_without_reordering' in csv and 'row[80] = resources.model_gpu_uploads_this_frame.to_string();' in csv)
check('one-shot scene transform timing is an opt-in CPU phase', '3D transform propagation (one-shot)' in render and 'sample_pending' in render)

# Check CSV indexes match the header width and remain appended.
header_match = re.search(r'const HEADER: &\[&str\] = &\[(.*?)\n\s*\];', csv, re.S)
header = re.findall(r'"([^"]+)"', header_match.group(1)) if header_match else []
check('CSV retains the current 152-column schema with model residency pressure appended', len(header) == 152 and header[147:152] == ['model_gpu_cache_bytes_estimate','model_gpu_cache_budget_bytes','model_gpu_cache_over_budget_bytes','model_gpu_cache_pressure_events','model_gpu_cache_pressure_level'], f'header count={len(header)}; tail={header[147:]}')

# Validate packaged game GLB container headers (not full glTF semantic decoding).
glb_files = sorted((ROOT / 'games/athar/assets/models').glob('*.glb'))
glb_ok = True
for path in glb_files:
    data = path.read_bytes()[:12]
    if len(data) != 12 or data[:4] != b'glTF' or struct.unpack('<I', data[8:12])[0] != path.stat().st_size:
        glb_ok = False
        break
sum_bytes = sum(p.stat().st_size for p in glb_files)
check('12 packaged GLBs have consistent container headers', len(glb_files) == 12 and glb_ok, f'{len(glb_files)} files; {sum_bytes/1024/1024:.2f} MiB source bytes')

failed = 0
for label, ok, detail in checks:
    print(('PASS' if ok else 'FAIL') + ': ' + label + (f' ({detail})' if detail else ''))
    failed += not ok
print(f'\n{len(checks)-failed}/{len(checks)} static checks passed')
sys.exit(1 if failed else 0)
