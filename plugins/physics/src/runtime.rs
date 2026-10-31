use std::collections::{HashMap, HashSet};
use std::sync::{
    mpsc::{channel, Receiver},
    Mutex,
};

use glam::{Quat, Vec3};
use rapier3d::prelude::{
    BroadPhaseBvh, CCDSolver, ChannelEventCollector, ColliderHandle, ColliderSet,
    CollisionEvent as RapierCollisionEvent, ImpulseJointSet, IntegrationParameters, IslandManager,
    MultibodyJointSet, NarrowPhase, PhysicsPipeline, Pose, QueryFilter, Ray, RigidBodyBuilder,
    RigidBodyHandle, RigidBodySet, SoftBodySet, Vector,
};
use ribbon_ecs::{Entity, World};
use ribbon_events::WorldEventsExt;
use ribbon_scene::{propagate_transforms_3d, GlobalTransform3D, Parent, Transform3D};

use crate::body::{RigidBody, RigidBodyKind};
use crate::collider::{validate_scale, Collider, ColliderShape, CollisionLayers};
use crate::config::PhysicsConfig;
use crate::controller::{CharacterController, CharacterMovementIntent, CharacterMovementResult};
use crate::events::{CollisionStarted, CollisionStopped, SensorEntered, SensorExited};
use crate::queries::{RaycastHit, RaycastQuery};

/// A diagnostic emitted by physics validation or synchronization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicsDiagnostic {
    pub entity: Option<Entity>,
    pub message: String,
}

/// Current physics diagnostics. Entries are deduplicated within a frame.
#[derive(Clone, Debug, Default)]
pub struct PhysicsDiagnostics {
    pub entries: Vec<PhysicsDiagnostic>,
    /// CPU nanoseconds spent reconciling ECS collider/body descriptors in the most recent PreUpdate.
    pub last_reconcile_time_ns: u64,
    /// CPU nanoseconds spent running the most recent fixed physics step.
    pub last_fixed_step_time_ns: u64,
    /// Enable one-shot timings for the next physics reconciliation and fixed step.
    /// Kept disabled outside explicit profiling/transition measurement windows.
    pub timing_enabled: bool,
}

impl PhysicsDiagnostics {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn record(&mut self, entity: Option<Entity>, message: impl Into<String>) {
        let message = message.into();
        if !self
            .entries
            .iter()
            .any(|entry| entry.entity == entity && entry.message == message)
        {
            self.entries.push(PhysicsDiagnostic { entity, message });
        }
    }
}

/// Renderer-independent collider data for editor overlays or custom debug renderers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsDebugCollider {
    pub entity: Entity,
    pub shape: ColliderShape,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub sensor: bool,
}

/// Snapshot of current colliders. This crate intentionally does not draw them.
#[derive(Clone, Debug, Default)]
pub struct PhysicsDebugData {
    pub colliders: Vec<PhysicsDebugCollider>,
}

#[derive(Clone, Copy)]
struct DesiredEntity {
    body: RigidBody,
    explicit_body: bool,
    collider: Collider,
    layers: CollisionLayers,
    transform: Transform3D,
    global: GlobalTransform3D,
    parent: Option<Entity>,
}

struct BodyEntry {
    body_handle: RigidBodyHandle,
    collider_handle: ColliderHandle,
    body: RigidBody,
    explicit_body: bool,
    collider: Collider,
    layers: CollisionLayers,
    parent: Option<Entity>,
    last_local_transform: Transform3D,
    last_world_transform: GlobalTransform3D,
    effective_ccd: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ActivePair {
    a: Entity,
    b: Entity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ActiveSensorPair {
    sensor: Entity,
    other: Entity,
}

#[derive(Clone, Copy)]
struct ColliderOwner {
    entity: Entity,
    sensor: bool,
}

/// Internal Rapier world. Game code interacts through Ribbon components and query functions.
pub(crate) struct PhysicsContext {
    pipeline: PhysicsPipeline,
    integration_parameters: IntegrationParameters,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    soft_bodies: SoftBodySet,
    ccd_solver: CCDSolver,
    event_handler: ChannelEventCollector,
    collision_events: Mutex<Receiver<RapierCollisionEvent>>,
    _contact_force_events: Mutex<Receiver<rapier3d::prelude::ContactForceEvent>>,
    entries: HashMap<Entity, BodyEntry>,
    collider_owners: HashMap<ColliderHandle, ColliderOwner>,
    active_pairs: HashSet<ActivePair>,
    active_sensor_pairs: HashSet<ActiveSensorPair>,
}

impl Default for PhysicsContext {
    fn default() -> Self {
        let (collision_sender, collision_events) = channel();
        let (contact_force_sender, contact_force_events) = channel();
        let (soft_body_tear_sender, _soft_body_tear_events) = channel();
        Self {
            pipeline: PhysicsPipeline::new(),
            integration_parameters: IntegrationParameters::default(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            soft_bodies: SoftBodySet::new(),
            ccd_solver: CCDSolver::new(),
            event_handler: ChannelEventCollector::new(
                collision_sender,
                contact_force_sender,
                soft_body_tear_sender,
            ),
            collision_events: Mutex::new(collision_events),
            _contact_force_events: Mutex::new(contact_force_events),
            entries: HashMap::new(),
            collider_owners: HashMap::new(),
            active_pairs: HashSet::new(),
            active_sensor_pairs: HashSet::new(),
        }
    }
}

impl PhysicsContext {
    pub(crate) fn reconcile(&mut self, world: &mut World) {
        let started = world
            .resource::<PhysicsDiagnostics>()
            .timing_enabled
            .then(std::time::Instant::now);
        // Make 3D world poses current before inspecting entity descriptors, even when the frame
        // does not accumulate a fixed step. ScenePlugin still runs its normal PostUpdate pass.
        propagate_transforms_3d(world);
        world.resource_mut::<PhysicsDiagnostics>().clear();
        let config = *world.resource::<PhysicsConfig>();
        if let Err(message) = config.validate() {
            self.diagnostic(world, None, message);
        }
        let default_ccd = config.default_ccd_enabled;
        let mut desired = HashMap::new();
        let raw_entities: Vec<_> = world
            .query::<(
                &Collider,
                Option<&RigidBody>,
                Option<&Transform3D>,
                Option<&CollisionLayers>,
                Option<&Parent>,
            )>()
            .iter()
            .map(|(entity, (collider, body, transform, layers, parent))| {
                (
                    entity,
                    *collider,
                    body.copied(),
                    transform.copied(),
                    layers.copied(),
                    parent.map(|value| value.0),
                )
            })
            .collect();

        let rigid_body_only_entities: Vec<_> = world
            .query::<&RigidBody>()
            .iter()
            .map(|(entity, _)| entity)
            .filter(|entity| world.get::<Collider>(*entity).is_none())
            .collect();
        for entity in rigid_body_only_entities {
            self.diagnostic(
                world,
                Some(entity),
                "RigidBody without Collider is ignored; add a Collider to participate in physics",
            );
        }

        for (entity, collider, body, transform, layers, parent) in raw_entities {
            let Some(transform) = transform else {
                self.diagnostic(
                    world,
                    Some(entity),
                    "a physics Collider requires Transform3D; entity was not registered",
                );
                continue;
            };
            let global = world
                .get::<GlobalTransform3D>(entity)
                .map(|value| *value)
                .unwrap_or_else(|| GlobalTransform3D::from(transform));
            // Scene's 3D propagation follows only ancestors that have Transform3D. A Parent link
            // to a non-3D entity is metadata, not part of the resolved 3D pose.
            let parent = parent.filter(|parent| world.get::<Transform3D>(*parent).is_some());
            if let Err(message) = validate_parent_chain(world, parent) {
                self.diagnostic(world, Some(entity), message);
                continue;
            }
            let descriptor = DesiredEntity {
                body: body.unwrap_or_else(RigidBody::fixed),
                explicit_body: body.is_some(),
                collider,
                layers: layers.unwrap_or_default(),
                transform,
                global,
                parent,
            };

            if let Err(message) = validate_pose(&descriptor.global) {
                self.diagnostic(world, Some(entity), message);
                continue;
            }
            if let Err(message) = descriptor.body.validate() {
                self.diagnostic(world, Some(entity), message);
                continue;
            }
            if let Err(message) = descriptor.collider.validate() {
                self.diagnostic(world, Some(entity), message);
                continue;
            }
            if let Err(message) = descriptor
                .collider
                .validate_for_scale(descriptor.global.scale)
            {
                self.diagnostic(world, Some(entity), message);
                continue;
            }
            desired.insert(entity, descriptor);
        }

        let obsolete: Vec<_> = self
            .entries
            .keys()
            .filter(|entity| !desired.contains_key(*entity))
            .copied()
            .collect();
        let mut obsolete = obsolete;
        obsolete.sort_unstable_by_key(|entity| entity.to_bits().get());
        for entity in obsolete {
            self.remove_entity(world, entity);
        }

        let mut desired_entities: Vec<_> = desired.into_iter().collect();
        desired_entities.sort_unstable_by_key(|(entity, _)| entity.to_bits().get());
        for (entity, descriptor) in desired_entities {
            let effective_ccd = descriptor.body.ccd_enabled.unwrap_or(default_ccd);
            let recreate = self.entries.get(&entity).is_some_and(|entry| {
                entry.body != descriptor.body
                    || entry.explicit_body != descriptor.explicit_body
                    || entry.collider != descriptor.collider
                    || entry.layers != descriptor.layers
                    || entry.last_world_transform.scale != descriptor.global.scale
                    || entry.effective_ccd != effective_ccd
            });
            if recreate {
                self.remove_entity(world, entity);
            }
            if !self.entries.contains_key(&entity) {
                if let Err(message) = self.insert_entity(entity, descriptor, effective_ccd) {
                    self.diagnostic(world, Some(entity), message);
                    continue;
                }
            } else {
                let has_character_controller = world.get::<CharacterController>(entity).is_some();
                if self.synchronize_entity_transform(entity, descriptor, has_character_controller) {
                    // External teleports happen during ECS reconciliation, before the fixed-step
                    // character query. Keep Rapier's attached collider pose and query BVH in sync
                    // immediately rather than waiting for PhysicsPipeline::step.
                    self.refresh_query_aabb(entity);
                }
            }
        }

        self.refresh_debug_data(world);
        if let Some(started) = started {
            world
                .resource_mut::<PhysicsDiagnostics>()
                .last_reconcile_time_ns =
                started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        }
    }

    fn insert_entity(
        &mut self,
        entity: Entity,
        descriptor: DesiredEntity,
        effective_ccd: bool,
    ) -> Result<(), &'static str> {
        let mut builder = match descriptor.body.kind {
            RigidBodyKind::Fixed => RigidBodyBuilder::fixed(),
            RigidBodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            RigidBodyKind::KinematicPositionBased => RigidBodyBuilder::kinematic_position_based(),
            RigidBodyKind::KinematicVelocityBased => RigidBodyBuilder::kinematic_velocity_based(),
        };
        builder = builder
            .translation(to_rapier_vector(descriptor.global.translation))
            .rotation(to_rapier_angvel(descriptor.global.rotation))
            .linvel(to_rapier_vector(descriptor.body.linear_velocity))
            .angvel(to_rapier_vector(descriptor.body.angular_velocity))
            .linear_damping(descriptor.body.linear_damping)
            .angular_damping(descriptor.body.angular_damping)
            .ccd_enabled(effective_ccd);
        let body_handle = self.bodies.insert(builder.build());
        let collider_builder = match descriptor
            .collider
            .to_builder(descriptor.global.scale, descriptor.layers)
        {
            Ok(builder) => builder,
            Err(message) => {
                self.bodies.remove(
                    body_handle,
                    &mut self.islands,
                    &mut self.colliders,
                    &mut self.impulse_joints,
                    &mut self.multibody_joints,
                    &mut self.soft_bodies,
                    true,
                );
                return Err(message);
            }
        };
        let collider_handle = self.colliders.insert_with_parent(
            collider_builder.build(),
            body_handle,
            &mut self.bodies,
        );
        self.collider_owners.insert(
            collider_handle,
            ColliderOwner {
                entity,
                sensor: descriptor.collider.sensor,
            },
        );
        // The regular broad-phase update runs inside PhysicsPipeline::step, after character
        // movement queries. Index new colliders now so they participate in that first query.
        if let Some(collider) = self.colliders.get(collider_handle) {
            let aabb =
                collider.compute_broad_phase_aabb(&self.integration_parameters, &self.bodies);
            self.broad_phase
                .set_aabb(&self.integration_parameters, collider_handle, aabb);
        }
        self.entries.insert(
            entity,
            BodyEntry {
                body_handle,
                collider_handle,
                body: descriptor.body,
                explicit_body: descriptor.explicit_body,
                collider: descriptor.collider,
                layers: descriptor.layers,
                parent: descriptor.parent,
                last_local_transform: descriptor.transform,
                last_world_transform: descriptor.global,
                effective_ccd,
            },
        );
        Ok(())
    }

    fn synchronize_entity_transform(
        &mut self,
        entity: Entity,
        desired: DesiredEntity,
        has_character_controller: bool,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(&entity) else {
            return false;
        };
        let local_was_edited = desired.transform != entry.last_local_transform;
        let parent_changed = desired.parent != entry.parent;
        let world_pose_changed = pose_changed(&desired.global, &entry.last_world_transform);
        let user_teleported = local_was_edited || parent_changed || world_pose_changed;
        let mut refresh_query_aabb = false;

        match entry.body.kind {
            RigidBodyKind::Dynamic => {
                // Dynamic poses are authoritative in Rapier. Only treat a changed ECS pose as a
                // teleport, never reapply the last ECS pose every fixed step.
                if user_teleported {
                    if let Some(body) = self.bodies.get_mut(entry.body_handle) {
                        body.set_position(
                            to_rapier_pose(desired.global.translation, desired.global.rotation),
                            true,
                        );
                    }
                    entry.last_world_transform = desired.global;
                    refresh_query_aabb = true;
                }
            }
            RigidBodyKind::KinematicPositionBased => {
                if user_teleported {
                    if let Some(body) = self.bodies.get_mut(entry.body_handle) {
                        let pose =
                            to_rapier_pose(desired.global.translation, desired.global.rotation);
                        if has_character_controller {
                            // Character movement queries the current body pose before the Rapier
                            // step. A kinematic target would leave that pose stale and then be
                            // overwritten by the movement intent in the same fixed step.
                            body.set_position(pose, true);
                            refresh_query_aabb = true;
                        } else {
                            body.set_next_kinematic_position(pose);
                        }
                    }
                    entry.last_world_transform = desired.global;
                }
            }
            RigidBodyKind::Fixed | RigidBodyKind::KinematicVelocityBased => {
                if user_teleported {
                    if let Some(body) = self.bodies.get_mut(entry.body_handle) {
                        body.set_position(
                            to_rapier_pose(desired.global.translation, desired.global.rotation),
                            true,
                        );
                    }
                    entry.last_world_transform = desired.global;
                    refresh_query_aabb = true;
                }
            }
        }
        entry.last_local_transform = desired.transform;
        entry.parent = desired.parent;
        refresh_query_aabb
    }

    fn refresh_query_aabb(&mut self, entity: Entity) {
        let Some(entry) = self.entries.get(&entity) else {
            return;
        };
        let collider_handle = entry.collider_handle;
        // set_position marks the body as changed. Propagate that pose to its attached collider
        // before recomputing its broad-phase bounds for immediate scene queries.
        self.bodies
            .propagate_modified_body_positions_to_colliders(&mut self.colliders);
        if let Some(collider) = self.colliders.get(collider_handle) {
            let aabb =
                collider.compute_broad_phase_aabb(&self.integration_parameters, &self.bodies);
            self.broad_phase
                .set_aabb(&self.integration_parameters, collider_handle, aabb);
        }
    }

    fn remove_entity(&mut self, world: &mut World, entity: Entity) {
        let Some(entry) = self.entries.remove(&entity) else {
            return;
        };
        self.collider_owners.remove(&entry.collider_handle);

        let mut ended: Vec<_> = self
            .active_pairs
            .iter()
            .filter(|pair| pair.a == entity || pair.b == entity)
            .copied()
            .collect();
        ended.sort_unstable_by_key(|pair| (pair.a.to_bits().get(), pair.b.to_bits().get()));
        for pair in ended {
            self.active_pairs.remove(&pair);
            world.send_event(CollisionStopped {
                entity_a: pair.a,
                entity_b: pair.b,
            });
        }
        let mut ended_sensors: Vec<_> = self
            .active_sensor_pairs
            .iter()
            .filter(|pair| pair.sensor == entity || pair.other == entity)
            .copied()
            .collect();
        ended_sensors
            .sort_unstable_by_key(|pair| (pair.sensor.to_bits().get(), pair.other.to_bits().get()));
        for pair in ended_sensors {
            self.active_sensor_pairs.remove(&pair);
            world.send_event(SensorExited {
                sensor: pair.sensor,
                other: pair.other,
            });
        }

        let _ = self.bodies.remove(
            entry.body_handle,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            true,
        );
    }

    pub(crate) fn step(&mut self, world: &mut World, delta_seconds: f32) {
        let started = world
            .resource::<PhysicsDiagnostics>()
            .timing_enabled
            .then(std::time::Instant::now);
        let config = *world.resource::<PhysicsConfig>();
        if let Err(message) = config.validate() {
            self.diagnostic(world, None, message);
            Self::record_step_time(world, started);
            return;
        }
        if !delta_seconds.is_finite() || delta_seconds <= 0.0 {
            Self::record_step_time(world, started);
            return;
        }

        // The application already decided whether a fixed step is due. Reuse that delta rather
        // than accumulating time here or consulting a render-frame duration.
        self.integration_parameters.dt = delta_seconds;
        self.integration_parameters.num_solver_iterations = config.solver_iterations;
        self.apply_character_intents(world, delta_seconds);

        let gravity = to_rapier_vector(config.gravity);
        self.pipeline.step(
            gravity,
            &self.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd_solver,
            &(),
            &self.event_handler,
        );
        self.flush_collision_events(world);
        self.write_back_simulated_poses(world);
        self.refresh_debug_data(world);
        Self::record_step_time(world, started);
    }

    fn record_step_time(world: &mut World, started: Option<std::time::Instant>) {
        if let Some(started) = started {
            world
                .resource_mut::<PhysicsDiagnostics>()
                .last_fixed_step_time_ns =
                started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        }
    }

    fn apply_character_intents(&mut self, world: &mut World, delta_seconds: f32) {
        let requests: Vec<_> = world
            .query::<(&CharacterController, &CharacterMovementIntent)>()
            .iter()
            .map(|(entity, (controller, intent))| (entity, *controller, *intent))
            .collect();
        for (entity, controller, intent) in requests {
            let result = if let Err(message) = controller.validate() {
                self.diagnostic(world, Some(entity), message);
                CharacterMovementResult::default()
            } else if !intent.desired_displacement.is_finite() {
                self.diagnostic(
                    world,
                    Some(entity),
                    "character movement displacement must be finite",
                );
                CharacterMovementResult::default()
            } else if let Some((body_kind, collider_shape)) = self
                .entries
                .get(&entity)
                .map(|entry| (entry.body.kind, entry.collider.shape))
            {
                if body_kind != RigidBodyKind::KinematicPositionBased {
                    self.diagnostic(
                        world,
                        Some(entity),
                        "CharacterController requires a position-based kinematic rigid body",
                    );
                    CharacterMovementResult::default()
                } else if !matches!(collider_shape, ColliderShape::CapsuleY { .. }) {
                    self.diagnostic(
                        world,
                        Some(entity),
                        "CharacterController currently requires a vertical capsule collider",
                    );
                    CharacterMovementResult::default()
                } else {
                    self.move_character(
                        entity,
                        controller,
                        intent.desired_displacement,
                        delta_seconds,
                    )
                    .unwrap_or_else(|| {
                        self.diagnostic(
                            world,
                            Some(entity),
                            "character movement could not query the current physics scene",
                        );
                        CharacterMovementResult::default()
                    })
                }
            } else {
                self.diagnostic(
                    world,
                    Some(entity),
                    "CharacterController entity must have a registered Collider and RigidBody",
                );
                CharacterMovementResult::default()
            };
            let _ = world.insert(entity, result);
            // Intent is a one-fixed-step command. Games should write a new intent each step.
            let _ = world.insert(entity, CharacterMovementIntent::default());
        }
    }

    fn move_character(
        &mut self,
        entity: Entity,
        config: CharacterController,
        desired_displacement: Vec3,
        delta_seconds: f32,
    ) -> Option<CharacterMovementResult> {
        let entry = self.entries.get(&entity)?;
        let body_handle = entry.body_handle;
        let collider_handle = entry.collider_handle;
        let layers = entry.layers;
        let pose = *self.bodies.get(body_handle)?.position();
        let (target, applied, grounded, contact_count) = {
            let collider = self.colliders.get(collider_handle)?;
            // Sensors generate overlap events but must not become invisible walls for the
            // kinematic character shape cast (e.g. ATHAR's brass-key interaction sensor).
            let filter = QueryFilter::default()
                .exclude_rigid_body(body_handle)
                .exclude_sensors()
                .groups(layers.rapier_groups());
            let query_pipeline = self.broad_phase.as_query_pipeline(
                self.narrow_phase.query_dispatcher(),
                &self.bodies,
                &self.colliders,
                filter,
            );
            let controller = rapier3d::control::KinematicCharacterController {
                up: Vector::new(0.0, 1.0, 0.0),
                offset: rapier3d::control::CharacterLength::Absolute(config.offset),
                slide: config.slide,
                max_slope_climb_angle: config.max_slope_climb_angle,
                min_slope_slide_angle: config.min_slope_slide_angle,
                snap_to_ground: config
                    .snap_to_ground
                    .map(rapier3d::control::CharacterLength::Absolute),
                ..Default::default()
            };
            let mut contact_count = 0;
            let movement = controller.move_shape(
                delta_seconds,
                &query_pipeline,
                collider.shape(),
                &pose,
                to_rapier_vector(desired_displacement),
                |_| contact_count += 1,
            );
            let applied = from_rapier_vector(movement.translation);
            let mut target = pose;
            target.translation += movement.translation;
            (target, applied, movement.grounded, contact_count)
        };
        self.bodies
            .get_mut(body_handle)?
            .set_next_kinematic_position(target);
        Some(CharacterMovementResult::new(
            desired_displacement,
            applied,
            grounded,
            contact_count,
        ))
    }

    fn flush_collision_events(&mut self, world: &mut World) {
        // The ECS resource contract requires Sync, while std's mpsc Receiver is only Send.
        // Access is serialized by the resource's mutable borrow, so the mutex is not contended.
        let events = self
            .collision_events
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_iter()
            .collect::<Vec<_>>();
        for event in events {
            match event {
                RapierCollisionEvent::Started(first_handle, second_handle, _) => {
                    let (Some(first), Some(second)) = (
                        self.collider_owners.get(&first_handle).copied(),
                        self.collider_owners.get(&second_handle).copied(),
                    ) else {
                        continue;
                    };
                    if first.entity == second.entity {
                        continue;
                    }
                    let pair = ordered_pair(first.entity, second.entity);
                    if self.active_pairs.insert(pair) {
                        world.send_event(CollisionStarted {
                            entity_a: pair.a,
                            entity_b: pair.b,
                        });
                    }
                    if first.sensor {
                        let sensor_pair = ActiveSensorPair {
                            sensor: first.entity,
                            other: second.entity,
                        };
                        if self.active_sensor_pairs.insert(sensor_pair) {
                            world.send_event(SensorEntered {
                                sensor: sensor_pair.sensor,
                                other: sensor_pair.other,
                            });
                        }
                    }
                    if second.sensor {
                        let sensor_pair = ActiveSensorPair {
                            sensor: second.entity,
                            other: first.entity,
                        };
                        if self.active_sensor_pairs.insert(sensor_pair) {
                            world.send_event(SensorEntered {
                                sensor: sensor_pair.sensor,
                                other: sensor_pair.other,
                            });
                        }
                    }
                }
                RapierCollisionEvent::Stopped(first_handle, second_handle, _) => {
                    let (Some(first), Some(second)) = (
                        self.collider_owners.get(&first_handle).copied(),
                        self.collider_owners.get(&second_handle).copied(),
                    ) else {
                        continue;
                    };
                    if first.entity == second.entity {
                        continue;
                    }
                    let pair = ordered_pair(first.entity, second.entity);
                    if self.active_pairs.remove(&pair) {
                        world.send_event(CollisionStopped {
                            entity_a: pair.a,
                            entity_b: pair.b,
                        });
                    }
                    for sensor_pair in [
                        first.sensor.then_some(ActiveSensorPair {
                            sensor: first.entity,
                            other: second.entity,
                        }),
                        second.sensor.then_some(ActiveSensorPair {
                            sensor: second.entity,
                            other: first.entity,
                        }),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if self.active_sensor_pairs.remove(&sensor_pair) {
                            world.send_event(SensorExited {
                                sensor: sensor_pair.sensor,
                                other: sensor_pair.other,
                            });
                        }
                    }
                }
            }
        }
        // Contact-force collection is intentionally not part of the public API yet. Drain it so
        // the channel remains bounded in practice if Rapier emits optional force notifications.
        let _ = self
            ._contact_force_events
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_iter()
            .count();
    }

    fn write_back_simulated_poses(&mut self, world: &mut World) {
        let entities: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(&entity, entry)| {
                (matches!(
                    entry.body.kind,
                    RigidBodyKind::Dynamic | RigidBodyKind::KinematicVelocityBased
                ) || (entry.body.kind == RigidBodyKind::KinematicPositionBased
                    && world.get::<CharacterController>(entity).is_some()))
                .then_some((entity, entry.body_handle, entry.parent))
            })
            .collect();

        for (entity, handle, parent) in entities {
            let Some(body) = self.bodies.get(handle) else {
                continue;
            };
            let pose = body.position();
            let world_translation = from_rapier_vector(pose.translation);
            let world_rotation = from_rapier_rotation(pose.rotation);
            let mut local = world
                .get::<Transform3D>(entity)
                .map(|value| *value)
                .unwrap_or_default();

            if let Some(parent_entity) = parent {
                let Some(mut parent_world) = world
                    .get::<GlobalTransform3D>(parent_entity)
                    .map(|value| *value)
                else {
                    self.diagnostic(
                        world,
                        Some(entity),
                        "parented physics body has no GlobalTransform3D; pose was not written back",
                    );
                    continue;
                };
                if let Some(parent_entry) = self.entries.get(&parent_entity) {
                    if let Some(parent_body) = self.bodies.get(parent_entry.body_handle) {
                        parent_world.translation =
                            from_rapier_vector(parent_body.position().translation);
                        parent_world.rotation =
                            from_rapier_rotation(parent_body.position().rotation);
                    }
                }
                if !parent_world.scale.is_finite() || parent_world.scale.min_element() <= 0.0 {
                    self.diagnostic(
                        world,
                        Some(entity),
                        "parent scale is invalid; cannot convert physics pose to local space",
                    );
                    continue;
                }
                let parent_rotation = normalized(parent_world.rotation);
                let parent_scaled_offset =
                    parent_rotation.conjugate() * (world_translation - parent_world.translation);
                local.translation = parent_scaled_offset / parent_world.scale;
                local.rotation = (parent_rotation.conjugate() * world_rotation).normalize();
            } else {
                local.translation = world_translation;
                local.rotation = world_rotation;
            }

            let _ = world.insert(entity, local);
            if let Some(entry) = self.entries.get_mut(&entity) {
                entry.last_local_transform = local;
            }
        }

        // The scene plugin's normal PostUpdate propagation remains in place. This pre-render pass
        // makes the physics-updated world transforms visible to systems that run after physics.
        propagate_transforms_3d(world);
        for (&entity, entry) in &mut self.entries {
            if let Some(global) = world.get::<GlobalTransform3D>(entity) {
                entry.last_world_transform = *global;
            }
        }
    }

    fn refresh_debug_data(&self, world: &mut World) {
        let mut debug = Vec::with_capacity(self.entries.len());
        for (&entity, entry) in &self.entries {
            let Some(collider) = self.colliders.get(entry.collider_handle) else {
                continue;
            };
            let pose = collider.position();
            debug.push(PhysicsDebugCollider {
                entity,
                shape: entry.collider.shape,
                position: from_rapier_vector(pose.translation),
                rotation: from_rapier_rotation(pose.rotation),
                scale: entry.last_world_transform.scale,
                sensor: entry.collider.sensor,
            });
        }
        debug.sort_unstable_by_key(|collider| collider.entity.to_bits().get());
        world.resource_mut::<PhysicsDebugData>().colliders = debug;
    }

    fn diagnostic(&self, world: &mut World, entity: Option<Entity>, message: impl Into<String>) {
        world
            .resource_mut::<PhysicsDiagnostics>()
            .record(entity, message);
    }

    pub(crate) fn raycast(&self, query: RaycastQuery) -> Option<RaycastHit> {
        let direction = query.direction.normalize();
        let mut filter = QueryFilter::default().groups(query.layers.rapier_groups());
        if let Some(entity) = query.exclude_entity {
            if let Some(entry) = self.entries.get(&entity) {
                filter = filter.exclude_rigid_body(entry.body_handle);
            }
        }
        if !query.include_sensors {
            filter = filter.exclude_sensors();
        }
        let ray = Ray::new(to_rapier_vector(query.origin), to_rapier_vector(direction));
        let query_pipeline = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            filter,
        );
        let (handle, hit) =
            query_pipeline.cast_ray_and_get_normal(&ray, query.max_distance, true)?;
        let entity = self.collider_owners.get(&handle)?.entity;
        Some(RaycastHit {
            entity,
            position: query.origin + direction * hit.time_of_impact,
            normal: from_rapier_vector(hit.normal).normalize_or_zero(),
            distance: hit.time_of_impact,
        })
    }
}

fn validate_pose(transform: &GlobalTransform3D) -> Result<(), &'static str> {
    validate_scale(transform.scale)?;
    if !transform.translation.is_finite() {
        return Err("physics transform translation must be finite");
    }
    if !transform.rotation.is_finite() || transform.rotation.length_squared() <= 1.0e-12 {
        return Err("physics transform rotation must be finite and non-zero");
    }
    Ok(())
}

fn pose_changed(a: &GlobalTransform3D, b: &GlobalTransform3D) -> bool {
    a.translation != b.translation || normalized(a.rotation) != normalized(b.rotation)
}

fn ordered_pair(a: Entity, b: Entity) -> ActivePair {
    if a.to_bits().get() <= b.to_bits().get() {
        ActivePair { a, b }
    } else {
        ActivePair { a: b, b: a }
    }
}

fn to_rapier_vector(value: Vec3) -> Vector {
    Vector::new(value.x, value.y, value.z)
}

fn from_rapier_vector(value: Vector) -> Vec3 {
    Vec3::new(value.x, value.y, value.z)
}

fn to_rapier_rotation(value: Quat) -> rapier3d::math::Rotation {
    let value = normalized(value);
    rapier3d::math::Rotation::from_xyzw(value.x, value.y, value.z, value.w)
}

fn from_rapier_rotation(value: rapier3d::math::Rotation) -> Quat {
    normalized(Quat::from_xyzw(value.x, value.y, value.z, value.w))
}

fn to_rapier_angvel(value: Quat) -> Vector {
    let (axis, angle) = normalized(value).to_axis_angle();
    Vector::new(axis.x * angle, axis.y * angle, axis.z * angle)
}

fn to_rapier_pose(translation: Vec3, rotation: Quat) -> Pose {
    let mut pose = Pose::translation(translation.x, translation.y, translation.z);
    pose.rotation = to_rapier_rotation(rotation);
    pose
}

fn validate_parent_chain(world: &World, mut parent: Option<Entity>) -> Result<(), &'static str> {
    let mut visited = HashSet::new();
    while let Some(entity) = parent {
        if !visited.insert(entity) {
            return Err("physics entity belongs to a cyclic Transform3D parent chain");
        }
        if !world.contains(entity) {
            return Err("physics entity has a stale parent entity");
        }
        if world.get::<Transform3D>(entity).is_none() {
            break;
        }
        // Physics reconciliation runs in PreUpdate, while ScenePlugin propagates global
        // transforms in PostUpdate. On the first frame a valid ancestor may therefore not
        // have GlobalTransform3D yet. Validate its local transform first so an unsupported
        // scale is rejected immediately instead of being reported as a missing global pose.
        let local = world
            .get::<Transform3D>(entity)
            .map(|value| *value)
            .ok_or("physics parent is missing Transform3D")?;
        validate_pose(&GlobalTransform3D::from(local))?;
        if !is_uniform_scale(local.scale) {
            return Err(
                "parented physics entities require uniform scale across Transform3D ancestors",
            );
        }
        // If propagation has already run, validate the resolved pose too. The local checks
        // above remain authoritative for scale validation, including on the first frame.
        if let Some(global) = world.get::<GlobalTransform3D>(entity).map(|value| *value) {
            validate_pose(&global)?;
        }
        parent = world
            .get::<Parent>(entity)
            .map(|value| value.0)
            .filter(|ancestor| world.get::<Transform3D>(*ancestor).is_some());
    }
    Ok(())
}

fn is_uniform_scale(scale: Vec3) -> bool {
    (scale.x - scale.y).abs() <= 1.0e-4 && (scale.x - scale.z).abs() <= 1.0e-4
}

fn normalized(rotation: Quat) -> Quat {
    if rotation.is_finite() && rotation.length_squared() > 1.0e-12 {
        rotation.normalize()
    } else {
        Quat::IDENTITY
    }
}
