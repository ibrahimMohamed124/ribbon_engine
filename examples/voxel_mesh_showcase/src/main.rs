//! Greedy-meshed voxel chunks drawn through the native 3D mesh renderer.
//!
//! Run with `cargo run -p voxel_mesh_showcase` or capture a fixed run with
//! `cargo run -p voxel_mesh_showcase -- --frames 240`.
//! Optional profile capture: `cargo run -p voxel_mesh_showcase --release -- --frames 1800 --profile-csv target/profiles/voxel-mesh-showcase.csv`.

use engine_app::{App, AppExit, Stage, Time, World};
use engine_events::WorldEventsExt;
use engine_platform::{PlatformConfig, PlatformPlugin};
use engine_plugins::AppPluginExt;
use engine_render::gpu_window::{ClearColor, GpuWindowPlugin};
use engine_render::profiling::RenderProfilePlugin;
use engine_scene::ScenePlugin;
use plugin_render3d::glam::Vec3;
use plugin_render3d::{Camera3D, Render3dPlugin, SunLight, Transform3D, VoxelMeshInstance3D};
use plugin_voxel::{ChunkCoord, ChunkMeshUpdate, ChunkedVoxelWorld, VoxelWorld};
use std::error::Error;

const EMPTY: u8 = 0;
const FLOOR: u8 = 1;
const EDIT_X: i32 = 7;
const EDIT_Y: i32 = 5;
const EDIT_Z: i32 = 0;
const CLEAR_BLOCK_FRAME: u64 = 120;
const RESTORE_BLOCK_FRAME: u64 = 180;
const CHUNK_SIZE: [usize; 3] = [8, 6, 8];

struct FrameLimit(Option<u64>);

struct DynamicVoxelResource(ChunkedVoxelWorld);

#[derive(Clone, Copy)]
struct ChunkMeshOwner {
    coord: ChunkCoord,
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
    let (width, height, depth) = (16, 12, 8);
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

fn update_dynamic_voxels(world: &mut World) {
    let frame = world.resource::<Time>().frame;
    world.resource_scope::<DynamicVoxelResource, _>(|world, resource| {
        let block_edit = match frame {
            CLEAR_BLOCK_FRAME => Some((EMPTY, "opened a floor voxel on a chunk corner")),
            RESTORE_BLOCK_FRAME => Some((FLOOR, "restored the floor voxel")),
            _ => None,
        };
        if let Some((block, description)) = block_edit {
            if resource
                .0
                .set_block(EDIT_X, EDIT_Y, EDIT_Z, block)
                .is_some()
            {
                println!("dynamic voxel demo: {description} at frame {frame}");
            }
        }

        for update in resource.0.rebuild_dirty_chunks() {
            replace_chunk_meshes(world, update);
        }
    });
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
        title: "Voxelworks | Greedy Voxel Meshes".to_owned(),
        width: 1100,
        height: 720,
    });
    app.insert_resource(ClearColor([0.018, 0.026, 0.04, 1.0]));
    app.insert_resource(FrameLimit(frame_limit));
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
    if let Some(path) = profile_csv {
        app.add_plugin(RenderProfilePlugin::new(path).warmup_frames(profile_warmup))?;
    }

    app.world.spawn((
        Transform3D::looking_at(
            Vec3::new(17.0, 12.0, 2.5),
            Vec3::new(8.0, 2.0, -5.5),
            Vec3::Y,
        ),
        Camera3D {
            far: 150.0,
            ..Camera3D::default()
        },
    ));
    app.add_system(Stage::Update, update_dynamic_voxels);
    app.add_system(Stage::Last, limit_frames);

    println!("Voxelworks Greedy Voxel Mesh Showcase");
    println!(
        "Showing dynamic chunk meshing, boundary invalidation, GPU mesh replacement, HDR/PBR lighting, shadows, and tone mapping."
    );
    println!("At frame {CLEAR_BLOCK_FRAME}, a floor block on a chunk corner is removed; at frame {RESTORE_BLOCK_FRAME}, it is restored.");
    app.build()?.run();
    Ok(())
}
