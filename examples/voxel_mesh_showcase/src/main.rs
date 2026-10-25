//! Greedy-meshed voxel chunks drawn through the native 3D mesh renderer.
//!
//! Run with `cargo run -p voxel_mesh_showcase`. Use `-- --frames N` for bounded capture runs.
//! Optional profile capture: `cargo run -p voxel_mesh_showcase --release -- --frames 1800 --profile-csv target/profiles/voxel-mesh-showcase.csv`.

use ribbon_app::{App, AppExit, Stage, Time, World};
use ribbon_events::WorldEventsExt;
use ribbon_platform::input::Key;
use ribbon_platform::{Input, PlatformConfig, PlatformPlugin};
use ribbon_plugin_render3d::glam::Vec3;
use ribbon_plugin_render3d::{
    BuiltinMesh, Camera3D, MeshInstance, Render3dPlugin, SunLight, Transform3D, VoxelMeshInstance3D,
};
use ribbon_plugin_voxel::{ChunkCoord, ChunkMeshUpdate, ChunkedVoxelWorld, VoxelWorld};
use ribbon_plugins::AppPluginExt;
use ribbon_render::gpu_window::{
    ClearColor, GpuWindow, GpuWindowPlugin, WindowTextOverlay, WindowTextOverlayPass,
};
use ribbon_render::profiling::RenderDiagnostics;
use ribbon_render::profiling::RenderProfilePlugin;
use ribbon_scene::ScenePlugin;
use std::error::Error;

const EMPTY: u8 = 0;
const FLOOR: u8 = 1;
const WORLD_WIDTH: usize = 16;
const WORLD_HEIGHT: usize = 12;
const WORLD_DEPTH: usize = 8;
const CHUNK_SIZE: [usize; 3] = [8, 6, 8];
const LOOK_TARGET: Vec3 = Vec3::new(8.0, 2.0, -5.5);
const INITIAL_TARGET: [i32; 3] = [7, 5, 0];
const INITIAL_ORBIT_ANGLE: f32 = 0.844_153_94;
const INITIAL_ELEVATION: f32 = 10.0;
const INITIAL_DISTANCE: f32 = 12.041_595;

struct FrameLimit(Option<u64>);

struct DynamicVoxelResource(ChunkedVoxelWorld);

#[derive(Clone, Copy)]
struct ChunkMeshOwner {
    coord: ChunkCoord,
}

#[derive(Clone, Copy)]
struct TargetMarker;

struct VoxelShowcaseState {
    target: [i32; 3],
    selected_block: u8,
    edit_count: u64,
    last_remesh_count: usize,
    last_action: String,
    orbit_angle: f32,
    elevation: f32,
    distance: f32,
}

impl Default for VoxelShowcaseState {
    fn default() -> Self {
        Self {
            target: INITIAL_TARGET,
            selected_block: 2,
            edit_count: 0,
            last_remesh_count: 0,
            last_action: "READY".to_owned(),
            orbit_angle: INITIAL_ORBIT_ANGLE,
            elevation: INITIAL_ELEVATION,
            distance: INITIAL_DISTANCE,
        }
    }
}

fn block_color(block: u8) -> [f32; 3] {
    match block {
        1 => [0.36, 0.62, 0.42],
        2 => [0.88, 0.36, 0.16],
        3 => [0.20, 0.47, 0.82],
        4 => [0.88, 0.71, 0.24],
        _ => [0.8, 0.8, 0.8],
    }
}

fn set_block(
    blocks: &mut [u8],
    dimensions: (usize, usize),
    cell: (usize, usize, usize),
    block: u8,
) {
    let (x, y, z) = cell;
    blocks[VoxelWorld::idx(dimensions.0, dimensions.1, z, y, x)] = block;
}

fn make_voxel_world() -> VoxelWorld {
    let (width, height, depth) = (WORLD_WIDTH, WORLD_HEIGHT, WORLD_DEPTH);
    let mut blocks = vec![EMPTY; width * height * depth];
    // Broad floor. The voxel grid is Z-up; the renderer's mesher converts it to Y-up.
    for x in 0..width {
        for y in 0..height {
            set_block(&mut blocks, (width, height), (x, y, 0), FLOOR);
        }
    }
    // A warm tower and a stepped blue structure.
    for z in 1..5 {
        for x in 3..6 {
            for y in 3..6 {
                set_block(&mut blocks, (width, height), (x, y, z), 2);
            }
        }
    }
    for step in 0..5 {
        for dz in 1..=step + 1 {
            for y in 7..10 {
                set_block(&mut blocks, (width, height), (8 + step, y, dz), 3);
            }
        }
    }
    for z in 1..4 {
        for x in 12..14 {
            for y in 2..4 {
                set_block(&mut blocks, (width, height), (x, y, z), 4);
            }
        }
    }
    VoxelWorld::new(width, height, depth, blocks)
}

fn replace_chunk_meshes(world: &mut World, update: ChunkMeshUpdate) {
    // Replace only ECS instances owned by the changed chunk. The renderer extraction
    // pass observes despawns/spawns in the same frame, then releases stale GPU buffers.
    let stale_entities: Vec<_> = world
        .query::<&ChunkMeshOwner>()
        .iter()
        .filter(|(_, owner)| owner.coord == update.coord)
        .map(|(entity, _)| entity)
        .collect();
    for entity in stale_entities {
        let _ = world.despawn(entity);
    }

    for mesh in update.meshes {
        let color = block_color(mesh.block);
        let transform = Transform3D::from_translation(Vec3::from_array(mesh.origin_y_up));
        world.spawn((
            transform,
            VoxelMeshInstance3D { mesh, color },
            ChunkMeshOwner {
                coord: update.coord,
            },
        ));
    }
}

fn move_target(target: &mut [i32; 3], axis: usize, delta: i32) {
    let dims = [WORLD_WIDTH as i32, WORLD_HEIGHT as i32, WORLD_DEPTH as i32];
    if axis < 3 {
        target[axis] = target[axis].saturating_add(delta).clamp(0, dims[axis] - 1);
    }
}

fn cycle_block(block: u8) -> u8 {
    if (1..4).contains(&block) {
        block + 1
    } else {
        1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VoxelEditResult {
    OutOfBounds,
    Unchanged { block: u8 },
    Changed { previous: u8, current: u8 },
}

fn apply_voxel_edit(world: &mut ChunkedVoxelWorld, target: [i32; 3], block: u8) -> VoxelEditResult {
    match world.set_block(target[0], target[1], target[2], block) {
        None => VoxelEditResult::OutOfBounds,
        Some(previous) if previous == block => VoxelEditResult::Unchanged { block },
        Some(previous) => VoxelEditResult::Changed {
            previous,
            current: block,
        },
    }
}

fn remember_remesh_count(state: &mut VoxelShowcaseState, remesh_count: usize) {
    if remesh_count > 0 {
        state.last_remesh_count = remesh_count;
    }
}

fn update_dynamic_voxels(world: &mut World) {
    let raw_delta = world.resource::<Time>().delta.as_secs_f32();
    let delta = if raw_delta.is_finite() {
        raw_delta.clamp(0.0, 0.1)
    } else {
        0.0
    };
    let (
        reset,
        select_x,
        select_y,
        select_z,
        cycle,
        place,
        remove,
        orbit_left,
        orbit_right,
        elevation_up,
        elevation_down,
        mouse_drag,
        mouse_scroll,
    ) = {
        let input = world.resource::<Input>();
        (
            input.just_pressed(Key::T),
            (input.just_pressed(Key::D) as i32) - (input.just_pressed(Key::A) as i32),
            (input.just_pressed(Key::W) as i32) - (input.just_pressed(Key::S) as i32),
            (input.just_pressed(Key::R) as i32) - (input.just_pressed(Key::F) as i32),
            input.just_pressed(Key::C),
            input.just_pressed(Key::E),
            input.just_pressed(Key::Q),
            input.is_down(Key::Left),
            input.is_down(Key::Right),
            input.is_down(Key::Up),
            input.is_down(Key::Down),
            input.mouse_buttons.right.then_some(input.mouse_delta),
            input.mouse_scroll,
        )
    };

    let (target, edit_request) = {
        let state = world.resource_mut::<VoxelShowcaseState>();
        if reset {
            *state = VoxelShowcaseState::default();
        } else {
            move_target(&mut state.target, 0, select_x);
            move_target(&mut state.target, 1, select_y);
            move_target(&mut state.target, 2, select_z);
            if cycle {
                state.selected_block = cycle_block(state.selected_block);
                state.last_action = format!("SELECTED BLOCK {}", state.selected_block);
            }
            if orbit_left {
                state.orbit_angle -= delta * 0.9;
            }
            if orbit_right {
                state.orbit_angle += delta * 0.9;
            }
            state.orbit_angle = state.orbit_angle.rem_euclid(std::f32::consts::TAU);
            if elevation_up {
                state.elevation = (state.elevation + delta * 1.5).clamp(1.5, 24.0);
            }
            if elevation_down {
                state.elevation = (state.elevation - delta * 1.5).clamp(1.5, 24.0);
            }
            if let Some((dx, dy)) = mouse_drag {
                if dx.is_finite() && dy.is_finite() {
                    state.orbit_angle =
                        (state.orbit_angle + dx * 0.005).rem_euclid(std::f32::consts::TAU);
                    state.elevation = (state.elevation - dy * 0.04).clamp(1.5, 24.0);
                }
            }
            if mouse_scroll.is_finite() {
                state.distance = (state.distance - mouse_scroll * 0.75).clamp(5.0, 40.0);
            }
        }
        let edit = if reset {
            None
        } else if remove {
            Some(EMPTY)
        } else if place {
            Some(state.selected_block)
        } else {
            None
        };
        (state.target, edit)
    };

    let remesh_count = world.resource_scope::<DynamicVoxelResource, _>(|world, resource| {
        if reset {
            resource.0 = ChunkedVoxelWorld::new(make_voxel_world(), CHUNK_SIZE, EMPTY);
        }
        if let Some(block) = edit_request {
            match apply_voxel_edit(&mut resource.0, target, block) {
                VoxelEditResult::OutOfBounds => {
                    world.resource_mut::<VoxelShowcaseState>().last_action =
                        "EDIT REJECTED: TARGET OUT OF BOUNDS".to_owned();
                }
                VoxelEditResult::Unchanged { block } => {
                    world.resource_mut::<VoxelShowcaseState>().last_action =
                        format!("NO CHANGE: BLOCK ALREADY {block}");
                }
                VoxelEditResult::Changed { previous, current } => {
                    let state = world.resource_mut::<VoxelShowcaseState>();
                    state.edit_count = state.edit_count.saturating_add(1);
                    state.last_action = if current == EMPTY {
                        format!("REMOVED BLOCK {previous}")
                    } else {
                        format!("PLACED BLOCK {current}, REPLACED {previous}")
                    };
                }
            }
        } else if reset {
            world.resource_mut::<VoxelShowcaseState>().last_action = "WORLD RESET".to_owned();
        } else if select_x != 0 || select_y != 0 || select_z != 0 {
            world.resource_mut::<VoxelShowcaseState>().last_action =
                "TARGET MOVED; READY TO EDIT".to_owned();
        }

        let updates = resource.0.rebuild_dirty_chunks();
        let count = updates.len();
        for update in updates {
            replace_chunk_meshes(world, update);
        }
        count
    });
    remember_remesh_count(world.resource_mut::<VoxelShowcaseState>(), remesh_count);

    let (
        target,
        selected_block,
        angle,
        elevation,
        distance,
        edit_count,
        last_remesh_count,
        last_action,
    ) = {
        let state = world.resource::<VoxelShowcaseState>();
        (
            state.target,
            state.selected_block,
            state.orbit_angle,
            state.elevation,
            state.distance,
            state.edit_count,
            state.last_remesh_count,
            state.last_action.clone(),
        )
    };
    let marker_position = Vec3::new(
        target[0] as f32 + 0.5,
        target[2] as f32 + 1.12,
        -(target[1] as f32 + 0.5),
    );
    for (_, (transform, mesh, _)) in
        world.query_mut::<(&mut Transform3D, &mut MeshInstance, &TargetMarker)>()
    {
        transform.translation = marker_position;
        mesh.color = block_color(selected_block);
    }
    let eye = LOOK_TARGET + Vec3::new(angle.sin() * distance, elevation, angle.cos() * distance);
    for (_, (transform, _)) in world.query_mut::<(&mut Transform3D, &Camera3D)>() {
        *transform = Transform3D::looking_at(eye, LOOK_TARGET, Vec3::Y);
    }

    let resource = world.resource::<DynamicVoxelResource>();
    let revision = resource.0.revision();
    let dirty = resource.0.dirty_chunk_count();
    let block_at_target = resource
        .0
        .world()
        .get(target[0], target[1], target[2])
        .unwrap_or(EMPTY);
    let owner =
        resource
            .0
            .chunk_for_voxel(target[0] as usize, target[1] as usize, target[2] as usize);
    let (mesh_cache, uploads, releases) = world
        .try_resource::<RenderDiagnostics>()
        .map(|diagnostics| {
            (
                diagnostics.resources.gpu_voxel_mesh_cache_entries,
                diagnostics.resources.voxel_mesh_gpu_uploads_cumulative,
                diagnostics.resources.voxel_mesh_gpu_releases_cumulative,
            )
        })
        .unwrap_or_default();
    let chunk_line = owner.map_or_else(
        || "CHUNK OUT OF BOUNDS".to_owned(),
        |coord| format!("CHUNK {},{},{}", coord.x, coord.y, coord.z),
    );
    world.resource_mut::<WindowTextOverlay>().lines = vec![
        "VOXEL MESH SHOWCASE".to_owned(),
        "WASD SELECT X/Y | R/F SELECT Z".to_owned(),
        "E PLACE BLOCK | Q REMOVE BLOCK".to_owned(),
        "C CYCLE BLOCK | T RESET WORLD".to_owned(),
        "ARROWS ORBIT/ELEVATION".to_owned(),
        "RIGHT MOUSE DRAG ORBIT | WHEEL ZOOM".to_owned(),
        format!(
            "TARGET {},{},{} BLOCK {} TYPE {}",
            target[0], target[1], target[2], block_at_target, selected_block
        ),
        format!("{chunk_line} REV {revision} DIRTY {dirty} LAST REMESH {last_remesh_count}"),
        format!("EDITS {edit_count} ACTION {last_action}"),
        format!("GPU MESHES {mesh_cache} UPLOADS {uploads} RELEASES {releases}"),
    ];
}

fn limit_frames(world: &mut World) {
    let frame = world.resource::<Time>().frame;
    if world
        .resource::<FrameLimit>()
        .0
        .is_some_and(|limit| frame >= limit)
    {
        world.send_event(AppExit);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut frame_limit = None;
    let mut profile_csv: Option<String> = None;
    let mut profile_warmup = 120_u64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => {
                frame_limit = Some(
                    args.next()
                        .ok_or("--frames expects a number")?
                        .parse::<u64>()?,
                );
            }
            "--profile-csv" => {
                profile_csv = Some(args.next().ok_or("--profile-csv expects a path")?);
            }
            "--profile-warmup" => {
                profile_warmup = args
                    .next()
                    .ok_or("--profile-warmup expects a number")?
                    .parse::<u64>()?;
            }
            "--help" | "-h" => {
                println!(
                    "voxel_mesh_showcase\n\nOptions:\n  --frames <N>  Exit after N engine frames\n  --profile-csv PATH  Write render diagnostics to CSV at shutdown\n  --profile-warmup N  Frames excluded before capture\n  -h, --help    Show this help"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }

    let mut app = App::new();
    app.insert_resource(PlatformConfig {
        title: "Ribbon | Greedy Voxel Meshes".to_owned(),
        width: 1100,
        height: 720,
    });
    app.insert_resource(ClearColor([0.018, 0.026, 0.04, 1.0]));
    app.insert_resource(FrameLimit(frame_limit));
    app.insert_resource(VoxelShowcaseState::default());
    app.insert_resource(WindowTextOverlay {
        lines: vec![
            "VOXEL MESH SHOWCASE".to_owned(),
            "LOADING CONTROLS...".to_owned(),
        ],
        visible: true,
    });
    app.insert_resource(DynamicVoxelResource(ChunkedVoxelWorld::new(
        make_voxel_world(),
        CHUNK_SIZE,
        EMPTY,
    )));
    app.insert_resource(SunLight {
        direction: Vec3::new(-0.45, -0.85, -0.36).normalize(),
        color: Vec3::new(1.0, 0.93, 0.8),
        ambient: Vec3::splat(0.18),
    });
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(GpuWindowPlugin)?;
    app.add_plugin(Render3dPlugin)?;
    app.world
        .resource_mut::<GpuWindow>()
        .add_pass(Box::new(WindowTextOverlayPass::default()));
    if let Some(path) = profile_csv {
        app.add_plugin(RenderProfilePlugin::new(path).warmup_frames(profile_warmup))?;
    }

    app.world.spawn((
        Transform3D::looking_at(
            LOOK_TARGET
                + Vec3::new(
                    INITIAL_ORBIT_ANGLE.sin() * INITIAL_DISTANCE,
                    INITIAL_ELEVATION,
                    INITIAL_ORBIT_ANGLE.cos() * INITIAL_DISTANCE,
                ),
            LOOK_TARGET,
            Vec3::Y,
        ),
        Camera3D {
            far: 150.0,
            ..Camera3D::default()
        },
    ));
    app.world.spawn((
        Transform3D {
            translation: Vec3::new(
                INITIAL_TARGET[0] as f32 + 0.5,
                INITIAL_TARGET[2] as f32 + 1.12,
                -(INITIAL_TARGET[1] as f32 + 0.5),
            ),
            scale: Vec3::splat(0.14),
            ..Transform3D::default()
        },
        MeshInstance {
            mesh: BuiltinMesh::Cube,
            color: block_color(2),
        },
        TargetMarker,
    ));
    app.add_system(Stage::Update, update_dynamic_voxels);
    app.add_system(Stage::Last, limit_frames);

    println!("Ribbon Greedy Voxel Mesh Showcase");
    println!(
        "Showing dynamic chunk meshing, boundary invalidation, GPU mesh replacement, HDR/PBR lighting, shadows, and tone mapping."
    );
    println!("Controls are displayed in-window: WASD selects a cell, R/F selects height, E places, Q removes, C cycles material, T resets; arrows/mouse orbit and the wheel zoom.");
    app.build()?.run();
    Ok(())
}

#[cfg(test)]
mod interactive_controls_tests {
    use super::{
        apply_voxel_edit, block_color, cycle_block, make_voxel_world, move_target,
        remember_remesh_count, VoxelEditResult, VoxelShowcaseState, CHUNK_SIZE, FLOOR,
        INITIAL_TARGET,
    };
    use ribbon_plugin_voxel::ChunkedVoxelWorld;

    const EMPTY_FOR_TEST: u8 = 0;

    #[test]
    fn target_navigation_clamps_all_axes_to_world_bounds() {
        let mut target = INITIAL_TARGET;
        move_target(&mut target, 0, -100);
        assert_eq!(target[0], 0);
        move_target(&mut target, 1, 100);
        assert_eq!(target[1], 11);
        move_target(&mut target, 2, 100);
        assert_eq!(target[2], 7);
        move_target(&mut target, 9, 1);
        assert_eq!(target, [0, 11, 7]);
    }

    #[test]
    fn block_cycle_stays_in_supported_non_empty_palette() {
        assert_eq!(cycle_block(1), 2);
        assert_eq!(cycle_block(2), 3);
        assert_eq!(cycle_block(3), 4);
        assert_eq!(cycle_block(4), 1);
        assert_eq!(cycle_block(EMPTY_FOR_TEST), 1);
    }

    #[test]
    fn all_selectable_blocks_have_distinct_color_palette_entries() {
        let colors = [
            block_color(1),
            block_color(2),
            block_color(3),
            block_color(4),
        ];
        for left in 0..colors.len() {
            for right in left + 1..colors.len() {
                assert_ne!(colors[left], colors[right]);
            }
        }
    }

    #[test]
    fn interactive_edit_rebuilds_boundary_neighbors_and_no_op_edits_do_not_remesh() {
        let mut world = ChunkedVoxelWorld::new(make_voxel_world(), CHUNK_SIZE, EMPTY_FOR_TEST);
        let initial = world.rebuild_dirty_chunks();
        assert_eq!(initial.len(), 4);
        assert_eq!(world.dirty_chunk_count(), 0);

        assert_eq!(
            apply_voxel_edit(&mut world, INITIAL_TARGET, EMPTY_FOR_TEST),
            VoxelEditResult::Changed {
                previous: FLOOR,
                current: EMPTY_FOR_TEST,
            }
        );
        assert_eq!(world.revision(), 1);
        assert_eq!(world.dirty_chunk_count(), 3);
        assert_eq!(world.rebuild_dirty_chunks().len(), 3);
        assert_eq!(world.dirty_chunk_count(), 0);

        assert_eq!(
            apply_voxel_edit(&mut world, INITIAL_TARGET, EMPTY_FOR_TEST),
            VoxelEditResult::Unchanged {
                block: EMPTY_FOR_TEST,
            }
        );
        assert_eq!(world.revision(), 1);
        assert_eq!(world.dirty_chunk_count(), 0);

        assert_eq!(
            apply_voxel_edit(&mut world, INITIAL_TARGET, FLOOR),
            VoxelEditResult::Changed {
                previous: EMPTY_FOR_TEST,
                current: FLOOR,
            }
        );
        assert_eq!(world.revision(), 2);
        assert_eq!(world.dirty_chunk_count(), 3);
        assert_eq!(world.rebuild_dirty_chunks().len(), 3);
    }

    #[test]
    fn interactive_edit_reports_out_of_bounds_without_mutating_the_world() {
        let mut world = ChunkedVoxelWorld::new(make_voxel_world(), CHUNK_SIZE, EMPTY_FOR_TEST);
        let _ = world.rebuild_dirty_chunks();
        assert_eq!(
            apply_voxel_edit(&mut world, [-1, 0, 0], FLOOR),
            VoxelEditResult::OutOfBounds
        );
        assert_eq!(world.revision(), 0);
        assert_eq!(world.dirty_chunk_count(), 0);
    }

    #[test]
    fn last_remesh_count_persists_until_a_later_non_empty_rebuild() {
        let mut state = VoxelShowcaseState::default();
        assert_eq!(state.last_remesh_count, 0);
        remember_remesh_count(&mut state, 3);
        assert_eq!(state.last_remesh_count, 3);
        remember_remesh_count(&mut state, 0);
        assert_eq!(state.last_remesh_count, 3);
        remember_remesh_count(&mut state, 2);
        assert_eq!(state.last_remesh_count, 2);
    }

    #[test]
    fn reset_world_is_deterministic_and_contains_the_chunk_boundary_target() {
        let first = make_voxel_world();
        let second = make_voxel_world();
        assert_eq!(first.dimensions(), second.dimensions());
        assert_eq!(first.blocks(), second.blocks());
        assert_eq!(
            first.get(INITIAL_TARGET[0], INITIAL_TARGET[1], INITIAL_TARGET[2]),
            Some(1)
        );
        assert!(CHUNK_SIZE.iter().all(|size| *size > 0));
    }
}
