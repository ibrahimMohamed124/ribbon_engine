//! Deterministic, game-specific layout and spawning for Wet Creek village.

#[cfg(test)]
use super::GameModelHandles;
#[cfg(test)]
use ribbon_app::World;
#[cfg(test)]
use ribbon_assets::Handle;
#[cfg(test)]
use ribbon_ecs::Entity;
#[cfg(test)]
use ribbon_plugin_gltf::ModelAsset;
use ribbon_plugin_physics::{Aabb3, Collider};
use ribbon_plugin_render3d::glam::{Quat, Vec3};
#[cfg(test)]
use ribbon_plugin_render3d::ModelInstance;
use ribbon_plugin_render3d::{PointLight, Transform3D};
use ribbon_plugin_streaming::{RegionCollider, RegionModel, RegionPointLight};

/// Unique model assets required for the complete village scene. Keep this list shared by initial
/// asset requests, renderer warmup, spawn preflight, and transition diagnostics.
pub const VILLAGE_REGION_ID: &str = "athar.wet_creek_village";
pub const VILLAGE_SCENE_ID: &str = "athar.village_scene";

pub const VILLAGE_MODEL_ASSETS: [&str; 7] = [
    "village_house.glb",
    "village_hall.glb",
    "village_tree.glb",
    "village_ground.glb",
    "village_road.glb",
    "village_lamp.glb",
    "village_well.glb",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub asset: &'static str,
    pub position: Vec3,
    pub yaw: f32,
    pub scale: Vec3,
}

#[derive(Clone, Debug)]
pub struct VillageLayout {
    pub houses: Vec<Placement>,
    pub trees: Vec<Placement>,
    pub lamps: Vec<Placement>,
    pub horizontal_roads: Vec<f32>,
    pub vertical_roads: Vec<f32>,
    pub hall: Placement,
    pub well: Placement,
}

impl VillageLayout {
    pub fn deterministic() -> Self {
        let mut houses = Vec::new();
        // Eight blocks in either axis, with a four-house opening around the village square.
        // These positions make the main roads visible and keep every house inside the outer wall.
        let grid = [-98.0_f32, -70.0, -42.0, -14.0, 14.0, 42.0, 70.0, 98.0];
        for z in grid {
            for x in grid {
                if x.abs() == 14.0 && z.abs() == 14.0 {
                    continue;
                }
                houses.push(Placement {
                    asset: "village_house.glb",
                    position: Vec3::new(x, 0.0, z),
                    // The local front door faces -Z. Rotate southern houses toward the square.
                    yaw: if z > 0.0 { 0.0 } else { std::f32::consts::PI },
                    scale: Vec3::ONE,
                });
            }
        }

        let mut trees = Vec::new();
        for step in (-100..=100).step_by(20) {
            let distance = step as f32;
            for position in [
                Vec3::new(-116.0, 0.0, distance),
                Vec3::new(116.0, 0.0, distance),
                Vec3::new(distance, 0.0, -116.0),
                Vec3::new(distance, 0.0, 116.0),
            ] {
                trees.push(Placement {
                    asset: "village_tree.glb",
                    position,
                    yaw: (step as f32 * 0.071).sin() * 0.2,
                    scale: Vec3::splat(0.82 + ((step + 100) % 5) as f32 * 0.055),
                });
            }
        }

        // Lamps flank the main east-west street without blocking the walking lane.
        let lamps = [-100.0_f32, -40.0, 40.0, 100.0]
            .into_iter()
            .flat_map(|x| [-6.0_f32, 6.0].into_iter().map(move |z| (x, z)))
            .map(|(x, z)| Placement {
                asset: "village_lamp.glb",
                position: Vec3::new(x, 0.0, z),
                yaw: 0.0,
                scale: Vec3::ONE,
            })
            .collect();

        Self {
            houses,
            trees,
            lamps,
            horizontal_roads: vec![-84.0, -56.0, -28.0, 0.0, 28.0, 56.0, 84.0],
            // Leave the center open for the village hall and square.
            vertical_roads: vec![-84.0, -56.0, -28.0, 28.0, 56.0, 84.0],
            hall: Placement {
                asset: "village_hall.glb",
                position: Vec3::new(0.0, 0.0, 19.0),
                yaw: 0.0,
                scale: Vec3::ONE,
            },
            well: Placement {
                asset: "village_well.glb",
                position: Vec3::new(-8.0, 0.0, 8.0),
                yaw: 0.0,
                scale: Vec3::ONE,
            },
        }
    }

    /// Translate the deterministic village layout into renderer-independent streaming records.
    /// The coordinator requests each unique model once, warms it through render3d, then spawns
    /// these placements only after every required model and collider is ready.
    pub fn streaming_models(&self) -> Vec<RegionModel> {
        let mut models = Vec::with_capacity(128);
        let mut push = |asset: &'static str, position: Vec3, yaw: f32, scale: Vec3| {
            models.push(RegionModel::new(
                asset,
                Transform3D {
                    translation: position,
                    rotation: Quat::from_rotation_y(yaw),
                    scale,
                },
            ));
        };

        push(
            "village_ground.glb",
            Vec3::new(0.0, VILLAGE_GROUND_Y, 0.0),
            0.0,
            Vec3::ONE,
        );
        for z in &self.horizontal_roads {
            push(
                "village_road.glb",
                Vec3::new(0.0, VILLAGE_ROAD_Y, *z),
                0.0,
                Vec3::ONE,
            );
        }
        for x in &self.vertical_roads {
            push(
                "village_road.glb",
                Vec3::new(*x, VILLAGE_ROAD_Y, 0.0),
                std::f32::consts::FRAC_PI_2,
                Vec3::ONE,
            );
        }
        for placement in &self.houses {
            push(
                placement.asset,
                placement.position,
                placement.yaw,
                placement.scale,
            );
        }
        push(
            self.hall.asset,
            self.hall.position,
            self.hall.yaw,
            self.hall.scale,
        );
        push(
            self.well.asset,
            self.well.position,
            self.well.yaw,
            self.well.scale,
        );
        for placement in &self.trees {
            push(
                placement.asset,
                placement.position,
                placement.yaw,
                placement.scale,
            );
        }
        for placement in &self.lamps {
            push(
                placement.asset,
                placement.position,
                placement.yaw,
                placement.scale,
            );
        }
        models
    }

    /// Convert the existing authoritative village AABBs to Rapier cuboid descriptors.
    pub fn streaming_colliders(&self) -> Vec<RegionCollider> {
        self.colliders()
            .into_iter()
            .map(|bounds| {
                RegionCollider::new(
                    Transform3D::from_translation(Vec3::from_array(bounds.center)),
                    Collider::cuboid(Vec3::from_array(bounds.half_extents)),
                )
            })
            .collect()
    }

    /// Lights are separate from model placements so the coordinator can count and budget them
    /// independently while retaining the same region lifetime.
    pub fn streaming_point_lights(&self) -> Vec<RegionPointLight> {
        self.lamps
            .iter()
            .map(|lamp| {
                RegionPointLight::new(PointLight {
                    position: lamp.position + Vec3::Y * 3.15,
                    color: Vec3::new(1.0, 0.56, 0.24),
                    intensity: 22.0,
                    range: 17.0,
                })
            })
            .collect()
    }

    pub fn colliders(&self) -> Vec<Aabb3> {
        let mut colliders = Vec::with_capacity(self.houses.len() + self.trees.len() + 12);
        for house in &self.houses {
            colliders.push(Aabb3::new(
                [house.position.x, 2.45, house.position.z],
                [3.4, 2.45, 3.4],
            ));
        }
        colliders.push(Aabb3::new(
            [self.hall.position.x, 3.2, self.hall.position.z],
            [5.8, 3.2, 4.9],
        ));
        colliders.push(Aabb3::new(
            [self.well.position.x, 0.48, self.well.position.z],
            [1.45, 0.48, 1.45],
        ));
        for tree in &self.trees {
            // Tree model instances use per-tree uniform scale; keep their fixed collider proxies
            // scaled to the same footprint and height rather than using one size for every tree.
            colliders.push(Aabb3::new(
                [tree.position.x, 2.7 * tree.scale.y, tree.position.z],
                [0.55 * tree.scale.x, 2.7 * tree.scale.y, 0.55 * tree.scale.z],
            ));
        }
        // A real engine collider under the visible ground lets Rapier report grounded state and
        // keeps future vertical movement supported instead of relying on a kinematic Y invariant.
        colliders.push(Aabb3::new([0.0, -0.1, 0.0], [130.0, 0.1, 130.0]));
        // Invisible bounds make the village feel large without allowing an endless walk away
        // from the authored scene or exposing the edge of its ground plane.
        colliders.extend([
            Aabb3::new([129.0, 3.2, 0.0], [1.0, 3.2, 130.0]),
            Aabb3::new([-129.0, 3.2, 0.0], [1.0, 3.2, 130.0]),
            Aabb3::new([0.0, 3.2, 129.0], [130.0, 3.2, 1.0]),
            Aabb3::new([0.0, 3.2, -129.0], [130.0, 3.2, 1.0]),
        ]);
        colliders
    }
}

#[cfg(test)]
fn get_model(handles: &GameModelHandles, asset: &str) -> Option<Handle<ModelAsset>> {
    handles.0.get(asset).copied()
}

#[cfg(test)]
fn spawn_model(
    world: &mut World,
    handles: &GameModelHandles,
    asset: &str,
    position: Vec3,
    yaw: f32,
    scale: Vec3,
    spawned: &mut Vec<Entity>,
) {
    let Some(model) = get_model(handles, asset) else {
        eprintln!("village model handle missing: {asset}");
        return;
    };
    let entity = world.spawn((
        Transform3D {
            translation: position,
            rotation: Quat::from_rotation_y(yaw),
            scale,
        },
        ModelInstance { model },
    ));
    spawned.push(entity);
}

const VILLAGE_GROUND_Y: f32 = -0.02;
const VILLAGE_ROAD_Y: f32 = -0.014;

/// Spawn the renderable village preview without activating collision geometry.
///
/// This is used while the room is still active so the exterior is already part of the extracted
/// scene when the door opens. The ground and road surfaces stay a little below the room floor's
/// y=0 surface: the room floor occludes them indoors, while the roads remain 6 mm above the
/// village ground outdoors. This prevents coincident surfaces and roads appearing inside the room.
#[cfg(test)]
pub fn spawn_village_visuals(
    world: &mut World,
    handles: &GameModelHandles,
) -> Result<Vec<Entity>, String> {
    // Validate the complete visual dependency set before creating any entities. A missing model
    // must not leave a partial preview next to the active room.
    for asset in VILLAGE_MODEL_ASSETS {
        if !handles.0.contains_key(asset) {
            return Err(format!("village model handle missing for `{asset}`"));
        }
    }

    let layout = VillageLayout::deterministic();
    let mut entities = Vec::with_capacity(136);
    spawn_model(
        world,
        handles,
        "village_ground.glb",
        Vec3::new(0.0, VILLAGE_GROUND_Y, 0.0),
        0.0,
        Vec3::ONE,
        &mut entities,
    );

    for z in &layout.horizontal_roads {
        spawn_model(
            world,
            handles,
            "village_road.glb",
            Vec3::new(0.0, VILLAGE_ROAD_Y, *z),
            0.0,
            Vec3::ONE,
            &mut entities,
        );
    }
    for x in &layout.vertical_roads {
        spawn_model(
            world,
            handles,
            "village_road.glb",
            Vec3::new(*x, VILLAGE_ROAD_Y, 0.0),
            std::f32::consts::FRAC_PI_2,
            Vec3::ONE,
            &mut entities,
        );
    }
    for house in &layout.houses {
        spawn_model(
            world,
            handles,
            house.asset,
            house.position,
            house.yaw,
            house.scale,
            &mut entities,
        );
    }
    spawn_model(
        world,
        handles,
        layout.hall.asset,
        layout.hall.position,
        layout.hall.yaw,
        layout.hall.scale,
        &mut entities,
    );
    spawn_model(
        world,
        handles,
        layout.well.asset,
        layout.well.position,
        layout.well.yaw,
        layout.well.scale,
        &mut entities,
    );
    for tree in &layout.trees {
        spawn_model(
            world,
            handles,
            tree.asset,
            tree.position,
            tree.yaw,
            tree.scale,
            &mut entities,
        );
    }
    for lamp in &layout.lamps {
        spawn_model(
            world,
            handles,
            lamp.asset,
            lamp.position,
            lamp.yaw,
            lamp.scale,
            &mut entities,
        );
        entities.push(world.spawn((PointLight {
            position: lamp.position + Vec3::Y * 3.15,
            color: Vec3::new(1.0, 0.56, 0.24),
            intensity: 22.0,
            range: 17.0,
        },)));
    }

    Ok(entities)
}

/// Create the fixed Rapier proxies only when the gameplay transition commits.
///
/// Preview geometry deliberately has no collider so the room's collision layout remains unchanged
/// while the player is still indoors.
#[cfg(test)]
pub fn spawn_village_physics(world: &mut World) -> Vec<Entity> {
    let layout = VillageLayout::deterministic();
    let mut entities = Vec::with_capacity(layout.colliders().len());
    spawn_village_colliders(world, &layout, &mut entities);
    entities
}

/// Spawn a complete village in one call for tests that need a one-shot village spawn.
#[cfg(test)]
pub fn spawn_village(world: &mut World, handles: &GameModelHandles) -> Result<Vec<Entity>, String> {
    let mut entities = spawn_village_visuals(world, handles)?;
    entities.extend(spawn_village_physics(world));
    Ok(entities)
}

/// Convert the established game-specific bounds into engine-owned fixed Rapier colliders.
/// `Aabb3` remains a compatibility/data type; it no longer resolves player motion.
#[cfg(test)]
fn spawn_village_colliders(
    world: &mut World,
    layout: &VillageLayout,
    tracked_entities: &mut Vec<Entity>,
) {
    for bounds in layout.colliders() {
        let collider = world.spawn((
            Transform3D {
                translation: Vec3::from_array(bounds.center),
                ..Transform3D::default()
            },
            Collider::cuboid(Vec3::from_array(bounds.half_extents)),
        ));
        tracked_entities.push(collider);
    }
}

#[cfg(test)]
mod tests {
    use super::VillageLayout;
    use ribbon_plugin_render3d::glam::Vec3;

    #[test]
    fn streaming_descriptors_match_the_legacy_village_content_counts() {
        use super::VILLAGE_MODEL_ASSETS;
        let layout = VillageLayout::deterministic();
        assert_eq!(layout.streaming_models().len(), 128);
        assert_eq!(layout.streaming_colliders().len(), 111);
        assert_eq!(layout.streaming_point_lights().len(), 8);
        let paths: std::collections::HashSet<_> = layout
            .streaming_models()
            .into_iter()
            .map(|model| model.asset_path)
            .collect();
        assert_eq!(paths.len(), VILLAGE_MODEL_ASSETS.len());
        assert!(VILLAGE_MODEL_ASSETS
            .iter()
            .all(|path| paths.contains(*path)));
    }

    #[test]
    fn village_layout_is_large_deterministic_and_bounded() {
        let a = VillageLayout::deterministic();
        let b = VillageLayout::deterministic();
        assert_eq!(a.houses, b.houses);
        assert_eq!(a.trees, b.trees);
        assert_eq!(a.houses.len(), 60);
        assert!(a.trees.len() >= 40);
        assert_eq!(a.lamps.len(), 8);
        assert!(a
            .houses
            .iter()
            .all(|p| p.position.x.abs() <= 100.0 && p.position.z.abs() <= 100.0));
        assert!(a
            .trees
            .iter()
            .all(|p| p.position.x.abs() <= 120.0 && p.position.z.abs() <= 120.0));
        assert!(a.horizontal_roads.contains(&0.0));
        assert!(!a.vertical_roads.contains(&0.0));
    }

    #[test]
    fn missing_village_handle_fails_preflight_without_spawning_any_entities() {
        use super::{spawn_village, GameModelHandles};
        use ribbon_app::World;
        use ribbon_plugin_physics::Collider;
        use ribbon_plugin_render3d::ModelInstance;

        let mut world = World::new();
        let error = spawn_village(&mut world, &GameModelHandles::default())
            .expect_err("a village with missing model handles must not partially spawn");
        assert!(error.contains("village model handle missing"));
        assert_eq!(world.query::<&ModelInstance>().iter().count(), 0);
        assert_eq!(world.query::<&Collider>().iter().count(), 0);
    }

    #[test]
    fn village_preview_is_renderable_without_collision_and_handoff_does_not_duplicate_models() {
        use super::{
            spawn_village_physics, spawn_village_visuals, GameModelHandles, VILLAGE_MODEL_ASSETS,
        };
        use ribbon_app::World;
        use ribbon_assets::Handle;
        use ribbon_plugin_gltf::ModelAsset;
        use ribbon_plugin_physics::Collider;
        use ribbon_plugin_render3d::{ModelInstance, PointLight, Transform3D};
        use std::collections::HashMap;

        let mut world = World::new();
        let handles = GameModelHandles(HashMap::from_iter(
            VILLAGE_MODEL_ASSETS
                .into_iter()
                .map(|asset| (asset.to_owned(), Handle::<ModelAsset>::default())),
        ));

        let visuals = spawn_village_visuals(&mut world, &handles)
            .expect("all named model handles should stage a preview");
        assert_eq!(
            world.query::<&ModelInstance>().iter().count(),
            128,
            "the whole visible village must be staged before crossing the doorway"
        );
        assert_eq!(world.query::<&PointLight>().iter().count(), 8);
        assert_eq!(world.query::<&Collider>().iter().count(), 0);
        assert_eq!(visuals.len(), 136);

        let transforms: Vec<_> = visuals
            .iter()
            .filter_map(|entity| world.get::<Transform3D>(*entity).map(|t| t.translation))
            .collect();
        assert!(transforms.iter().any(|p| (p.y + 0.02).abs() < 1.0e-6));
        assert!(transforms.iter().any(|p| (p.y + 0.014).abs() < 1.0e-6));

        let physics = spawn_village_physics(&mut world);
        assert_eq!(
            physics.len(),
            VillageLayout::deterministic().colliders().len()
        );
        assert_eq!(world.query::<&ModelInstance>().iter().count(), 128);
        assert_eq!(world.query::<&PointLight>().iter().count(), 8);
        assert_eq!(world.query::<&Collider>().iter().count(), physics.len());
    }

    #[test]
    fn all_village_models_are_packaged_for_offline_play() {
        let model_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("models");
        for filename in [
            "village_ground.glb",
            "village_road.glb",
            "village_house.glb",
            "village_hall.glb",
            "village_well.glb",
            "village_tree.glb",
            "village_lamp.glb",
        ] {
            assert!(
                model_root.join(filename).is_file(),
                "missing packaged model: {filename}"
            );
        }
    }

    #[test]
    fn village_bounds_are_reconciled_into_rapier_and_ground_is_queryable() {
        use super::spawn_village_colliders;
        use ribbon_app::{App, Time};
        use ribbon_plugin_physics::{
            raycast, Collider, PhysicsDebugData, PhysicsPlugin, RaycastQuery,
        };
        use ribbon_plugins::AppPluginExt;
        use ribbon_scene::ScenePlugin;
        use std::time::Duration;

        let layout = VillageLayout::deterministic();
        let expected = layout.colliders().len();
        let mut app = App::new();
        app.add_plugin(ScenePlugin).expect("scene plugin");
        app.add_plugin(PhysicsPlugin).expect("physics plugin");
        let mut tracked = Vec::new();
        spawn_village_colliders(&mut app.world, &layout, &mut tracked);

        assert_eq!(tracked.len(), expected);
        assert_eq!(app.world.query::<&Collider>().iter().count(), expected);
        assert!(tracked
            .iter()
            .all(|entity| app.world.get::<Collider>(*entity).is_some()));

        app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
        app.update();
        assert_eq!(
            app.world.resource::<PhysicsDebugData>().colliders.len(),
            expected,
            "all village collision proxies must exist in the Rapier backend"
        );
        let hit = raycast(
            &app.world,
            RaycastQuery::new(Vec3::new(3.0, 5.0, 3.0), -Vec3::Y, 10.0),
        )
        .expect("village ground must be queryable by Rapier");
        assert!(tracked.contains(&hit.entity));
        assert!(
            hit.position.y.abs() < 0.05,
            "unexpected ground hit: {hit:?}"
        );

        for entity in tracked {
            app.world.despawn(entity).expect("tracked collider exists");
        }
        app.update();
        assert!(app
            .world
            .resource::<PhysicsDebugData>()
            .colliders
            .is_empty());
    }

    #[test]
    fn village_colliders_include_all_houses_trees_landmarks_and_bounds() {
        let layout = VillageLayout::deterministic();
        let colliders = layout.colliders();
        assert_eq!(
            colliders.len(),
            layout.houses.len() + layout.trees.len() + 7
        );
        assert!(colliders
            .iter()
            .any(|c| { c.center == [0.0, -0.1, 0.0] && c.half_extents == [130.0, 0.1, 130.0] }));
        // Collider data is ordered as houses, hall, well, then trees. Per-tree scale must be
        // reflected in the Rapier proxy dimensions, not just in the rendered ModelInstance.
        let tree_colliders =
            &colliders[layout.houses.len() + 2..layout.houses.len() + 2 + layout.trees.len()];
        for (tree, collider) in layout.trees.iter().zip(tree_colliders) {
            assert!((collider.center[1] - 2.7 * tree.scale.y).abs() < 1.0e-5);
            assert!((collider.half_extents[0] - 0.55 * tree.scale.x).abs() < 1.0e-5);
            assert!((collider.half_extents[1] - 2.7 * tree.scale.y).abs() < 1.0e-5);
            assert!((collider.half_extents[2] - 0.55 * tree.scale.z).abs() < 1.0e-5);
        }
        assert!(colliders.iter().any(|c| c.center[0] == 129.0));
        assert!(colliders.iter().any(|c| c.center[2] == -129.0));
    }
}
