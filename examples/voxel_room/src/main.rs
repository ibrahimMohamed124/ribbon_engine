use engine_app::{App, Time};
use engine_assets::{
    AssetLoader, AssetServer, LoadContext, LoadError, LoadErrorKind, MemorySource,
};
use engine_ecs::Entity;
use engine_math::{Angles, Vect};
use engine_platform::PlatformPlugin;
use engine_plugins::AppPluginExt;
use engine_render::Framebuffer;
use engine_scene::{instantiate, Name, SceneDoc, ScenePlugin, Transform};
use plugin_rendering::ecs::{
    ActiveCamera, Camera, EntityRegistryRes, LightingRes, MaterialTableRes, RenderEntity,
    RenderSettings, RenderStyleRes, RenderingPlugin,
};
use plugin_rendering::entities::{
    EntityAnimation, EntityDefinition, EntityKind, EntityRegistry, EntityRenderMode, EntityShadow,
};
use plugin_rendering::{LightingParams, RenderStyle, EMPTY_MATERIAL_TABLE};
use plugin_voxel::{VoxelWorld, VoxelWorldRes};
use std::error::Error;
use std::time::Duration;

struct SceneLoader;
impl AssetLoader for SceneLoader {
    type Asset = SceneDoc;
    fn extensions(&self) -> &[&str] {
        &["ron"]
    }
    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        ron::de::from_bytes(bytes).map_err(|error| {
            LoadError::new(LoadErrorKind::Decode, &context.path, error.to_string())
        })
    }
}

static NO_BOXES: [plugin_rendering::entities::EntityBox; 0] = [];
static DEFINITIONS: [EntityDefinition; 1] = [EntityDefinition {
    kind: EntityKind(1),
    bounds: None,
    boxes: &NO_BOXES,
    render_mode: EntityRenderMode::Boxes,
    animation: EntityAnimation::None,
    shadow: EntityShadow::None,
}];
static ENTITY_REGISTRY: EntityRegistry = EntityRegistry::new(&DEFINITIONS);

fn main() -> Result<(), Box<dyn Error>> {
    let mut requested = String::from("cpu");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--renderer" {
            requested = args.next().ok_or("--renderer expects cpu or gpu")?;
        } else if arg != "--headless" {
            return Err(format!("unknown argument `{arg}`").into());
        }
    }
    if requested != "cpu" && requested != "gpu" {
        return Err("renderer must be cpu or gpu".into());
    }
    if requested == "gpu" {
        eprintln!("ECS voxel GPU integration is pending; running the deterministic CPU scene for this example.");
    }

    let source = MemorySource::default();
    source.insert(
        "voxel_room.ron",
        include_bytes!("../../../assets/voxel_room.ron").to_vec(),
    );
    let mut assets = AssetServer::default();
    assets.add_source(10, source);
    assets.register_loader(SceneLoader);
    let handle = assets.load::<SceneDoc>("voxel_room.ron")?;
    let document = assets
        .get(handle)
        .ok_or("loaded prefab disappeared")?
        .clone();

    let mut blocks = vec![0; 8 * 8 * 4];
    blocks[75] = 1;
    let mut app = App::new();
    app.insert_resource(RenderSettings {
        backend: plugin_rendering::ecs::RenderBackend::Cpu,
        width: 160,
        height: 100,
    });
    app.insert_resource(VoxelWorldRes(VoxelWorld::new(8, 8, 4, blocks), 0));
    app.insert_resource(MaterialTableRes(&EMPTY_MATERIAL_TABLE));
    app.insert_resource(EntityRegistryRes(&ENTITY_REGISTRY));
    app.insert_resource(RenderStyleRes(RenderStyle::default()));
    app.insert_resource(LightingRes(LightingParams {
        light_level: 1.0,
        intensity: 1.0,
        frame: 0,
        style: RenderStyle::default(),
    }));
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(RenderingPlugin)?;

    let entities = app
        .world
        .resource_scope::<engine_reflect::TypeRegistry, _>(|world, registry| {
            instantiate(world, registry, &document.prefab, None)
        })?;
    let mut camera_entity: Option<Entity> = None;
    for entity in entities {
        let name = app
            .world
            .get::<Name>(entity)
            .map(|name| name.0.clone())
            .unwrap_or_default();
        if name == "Camera" {
            app.world.insert(
                entity,
                Transform {
                    translation: Vect::new(1.5, 1.5, 1.5),
                    rotation: Angles::default(),
                    scale: 1.0,
                },
            )?;
            app.world.insert(entity, Camera { hfov: 1.1 })?;
            app.world.insert(entity, ActiveCamera)?;
            camera_entity = Some(entity);
        } else if name == "Marker" {
            app.world.insert(
                entity,
                Transform {
                    translation: Vect::new(4.0, 1.5, 1.5),
                    rotation: Angles::default(),
                    scale: 1.0,
                },
            )?;
            app.world.insert(
                entity,
                RenderEntity {
                    kind: EntityKind(1),
                    params: [0.0; 4],
                },
            )?;
        }
    }
    if camera_entity.is_none() {
        return Err("prefab has no camera entity".into());
    }
    let mut app = app.build()?;
    app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
    app.update();
    let frame = app
        .world
        .resource::<plugin_rendering::ecs::CpuBackend<plugin_rendering::ecs::DefaultShader>>();
    println!(
        "voxel_room renderer={requested} checksum={:016x}",
        checksum(&frame.framebuffer)
    );
    Ok(())
}

fn checksum(framebuffer: &Framebuffer) -> u64 {
    framebuffer
        .pixels
        .iter()
        .fold(0xcbf29ce484222325, |hash, pixel| {
            (hash ^ u64::from(*pixel)).wrapping_mul(0x100000001b3)
        })
}
