//! CPU simulation and render-batch extraction.
use crate::rng::Pcg32;
use crate::{
    EmitterShape, GroundResponse, ParticleBudget, ParticleDiagnostics, ParticleEmitter,
    SimulationSpace,
};
use glam::{Quat, Vec3};
use ribbon_app::{Time, World};
use ribbon_render_contracts::{ParticleBatch, ParticleInstance, ParticleRenderData};
use ribbon_scene::{GlobalTransform3D, Transform3D};
use std::collections::HashMap;
use std::time::Instant;

/// Per-emitter runtime particle storage. Instances are stored as structure-of-arrays.
pub struct ParticleEmitterState {
    pos: Vec<Vec3>,
    vel: Vec<Vec3>,
    age: Vec<f32>,
    lifetime: Vec<f32>,
    rot: Vec<f32>,
    ang_vel: Vec<f32>,
    size: Vec<f32>,
    color0: Vec<[f32; 4]>,
    spawn_accumulator: f32,
    elapsed: f32,
    burst_index: usize,
    rng: Pcg32,
    finished: bool,
    seed: u64,
    max_particles: u32,
    prewarmed: bool,
    validated: bool,
    validation_failed: bool,
}
impl ParticleEmitterState {
    fn new(seed: u64, max: u32) -> Self {
        Self {
            pos: vec![],
            vel: vec![],
            age: vec![],
            lifetime: vec![],
            rot: vec![],
            ang_vel: vec![],
            size: vec![],
            color0: vec![],
            spawn_accumulator: 0.0,
            elapsed: 0.0,
            burst_index: 0,
            rng: Pcg32::new(seed, 0xC0FFEE),
            finished: false,
            seed,
            max_particles: max,
            prewarmed: false,
            validated: false,
            validation_failed: false,
        }
    }
    fn len(&self) -> usize {
        self.pos.len()
    }
    fn clear(&mut self, seed: u64, max: u32) {
        *self = Self::new(seed, max)
    }
}
#[derive(Clone, Copy)]
struct Pose {
    position: Vec3,
    rotation: Quat,
}

/// Update all emitter simulations and write their renderer-neutral batches.
pub fn particle_update_system(world: &mut World) {
    let started = Instant::now();
    let dt = world
        .try_resource::<Time>()
        .map_or(0.0, |t| t.delta.as_secs_f32().clamp(0.0, 0.1));
    let budget = world
        .try_resource::<ParticleBudget>()
        .copied()
        .unwrap_or_default()
        .max_total_particles;
    let ids = world
        .query::<&ParticleEmitter>()
        .iter()
        .map(|(e, _)| e)
        .collect::<Vec<_>>();
    let live_emitters = ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let orphaned_states = world
        .query::<&ParticleEmitterState>()
        .iter()
        .filter_map(|(entity, _)| (!live_emitters.contains(&entity)).then_some(entity))
        .collect::<Vec<_>>();
    for entity in orphaned_states {
        let _ = world.remove::<ParticleEmitterState>(entity);
    }
    let mut poses = HashMap::with_capacity(ids.len());
    for e in &ids {
        let pose = world
            .get::<GlobalTransform3D>(*e)
            .map(|t| Pose {
                position: t.translation,
                rotation: t.rotation,
            })
            .or_else(|| {
                world.get::<Transform3D>(*e).map(|t| Pose {
                    position: t.translation,
                    rotation: t.rotation,
                })
            })
            .unwrap_or(Pose {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            });
        poses.insert(*e, pose);
        if world.get::<ParticleEmitterState>(*e).is_none() {
            let seed = world.get::<ParticleEmitter>(*e).map_or(0, |x| x.seed);
            let max = world
                .get::<ParticleEmitter>(*e)
                .map_or(1, |x| x.max_particles);
            let _ = world.insert(*e, ParticleEmitterState::new(seed, max));
        }
    }
    let mut diagnostics = ParticleDiagnostics {
        emitters: ids.len(),
        ..ParticleDiagnostics::default()
    };
    world.resource_scope::<ParticleRenderData, _>(|world, render| {
        for batch in &mut render.batches {
            batch.instances.clear();
        }
        let mut batch_index = 0usize;
        let mut total = 0usize;
        for (entity, (emitter, state)) in world
            .query_mut::<(&mut ParticleEmitter, &mut ParticleEmitterState)>()
            .into_iter()
        {
            if state.seed != emitter.seed || state.max_particles != emitter.max_particles {
                state.clear(emitter.seed, emitter.max_particles);
            }
            if !state.validated {
                // Validation is deliberately attempted once for this emitter
                // generation. A later seed/max-particle change resets the state
                // and permits one fresh validation attempt.
                let validation = emitter.validate();
                state.seed = emitter.seed;
                state.max_particles = emitter.max_particles;
                state.validated = true;
                state.validation_failed = validation.is_err();
            }
            if state.validation_failed {
                continue;
            }
            let pose = poses.get(&entity).copied().unwrap_or(Pose {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            });
            if !state.prewarmed && emitter.prewarm && emitter.looping {
                let warm = emitter.lifetime.1.clamp(0.0, 10.0);
                let mut left = warm;
                while left > 0.0 {
                    let step = left.min(1.0 / 30.0);
                    simulate_emitter(
                        state,
                        emitter,
                        pose,
                        step,
                        budget.saturating_sub(total),
                        &mut diagnostics,
                    );
                    left -= step;
                }
                state.prewarmed = true;
            }
            simulate_emitter(
                state,
                emitter,
                pose,
                dt,
                budget.saturating_sub(total),
                &mut diagnostics,
            );
            total = total.saturating_add(state.len());
            diagnostics.alive_particles = total;
            if state.len() > 0 {
                if batch_index == render.batches.len() {
                    render.batches.push(ParticleBatch {
                        blend: emitter.blend,
                        shape: emitter.shape_sprite,
                        billboard: emitter.billboard,
                        sort_back_to_front: emitter.sort_back_to_front,
                        instances: Vec::with_capacity(state.len()),
                    });
                }
                let batch = &mut render.batches[batch_index];
                batch.blend = emitter.blend;
                batch.shape = emitter.shape_sprite;
                batch.billboard = emitter.billboard;
                batch.sort_back_to_front = emitter.sort_back_to_front;
                for i in 0..state.len() {
                    let age = (state.age[i] / state.lifetime[i].max(0.001)).clamp(0.0, 1.0);
                    let local = emitter.space == SimulationSpace::Local;
                    let pos = if local {
                        pose.position + pose.rotation * state.pos[i]
                    } else {
                        state.pos[i]
                    };
                    let vel = if local {
                        pose.rotation * state.vel[i]
                    } else {
                        state.vel[i]
                    };
                    let color_mult = emitter.color_over_life.eval(age);
                    let color = std::array::from_fn(|j| state.color0[i][j] * color_mult[j]);
                    let scalar = state.size[i] * emitter.size_over_life.eval(age);
                    batch.instances.push(ParticleInstance {
                        position: pos.to_array(),
                        rotation: state.rot[i],
                        velocity: vel.to_array(),
                        emissive: emitter.emissive,
                        size: [scalar, scalar * emitter.aspect],
                        color,
                    });
                }
                batch_index += 1;
            }
        }
        // Keep inactive trailing batches in the pool with empty instance vectors.
        // The renderer skips empty batches, while future emitters can reuse their
        // allocations instead of reallocating when an effect becomes active again.
        for batch in render.batches.iter_mut().skip(batch_index) {
            batch.instances.clear();
        }
    });
    diagnostics.update_time_ns = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
    if let Some(d) = world.try_resource::<ParticleDiagnostics>() {
        let dropped = d.dropped_by_budget;
        world.insert_resource(ParticleDiagnostics {
            dropped_by_budget: dropped.saturating_add(diagnostics.dropped_by_budget),
            ..diagnostics
        });
    } else {
        world.insert_resource(diagnostics);
    }
}
fn simulate_emitter(
    s: &mut ParticleEmitterState,
    e: &ParticleEmitter,
    pose: Pose,
    dt: f32,
    remaining_budget: usize,
    d: &mut ParticleDiagnostics,
) {
    if e.enabled && !s.finished {
        let mut continuous_dt = dt;
        if e.duration > f32::EPSILON && !e.looping {
            // A non-looping emitter may cross its end during this frame. Only
            // the portion before the end contributes continuous emissions.
            continuous_dt = dt.min((e.duration - s.elapsed).max(0.0));
            s.elapsed += continuous_dt;
            while s.burst_index < e.bursts.len()
                && e.bursts[s.burst_index].time <= s.elapsed
                && e.bursts[s.burst_index].time <= e.duration
            {
                let burst = e.bursts[s.burst_index];
                s.burst_index += 1;
                spawn_n(s, e, pose, burst.count as usize, remaining_budget, d);
            }
            if s.elapsed >= e.duration {
                s.finished = true;
            }
        } else if e.duration > f32::EPSILON && e.looping {
            // Process every loop boundary crossed by dt, including bursts at
            // time zero in a new cycle when a short duration wraps mid-frame.
            while s.burst_index < e.bursts.len()
                && e.bursts[s.burst_index].time <= s.elapsed
                && e.bursts[s.burst_index].time <= e.duration
            {
                let burst = e.bursts[s.burst_index];
                s.burst_index += 1;
                spawn_n(s, e, pose, burst.count as usize, remaining_budget, d);
            }
            let mut remaining_dt = dt;
            while remaining_dt > 0.0 {
                let until_boundary = (e.duration - s.elapsed).max(0.0);
                if until_boundary <= f32::EPSILON {
                    s.elapsed = 0.0;
                    s.burst_index = 0;
                    continue;
                }
                let step = remaining_dt.min(until_boundary);
                s.elapsed += step;
                remaining_dt -= step;
                while s.burst_index < e.bursts.len()
                    && e.bursts[s.burst_index].time <= s.elapsed
                    && e.bursts[s.burst_index].time <= e.duration
                {
                    let burst = e.bursts[s.burst_index];
                    s.burst_index += 1;
                    spawn_n(s, e, pose, burst.count as usize, remaining_budget, d);
                }
                if s.elapsed >= e.duration {
                    s.elapsed = 0.0;
                    s.burst_index = 0;
                }
            }
        } else {
            // Duration zero means an unbounded emitter; bursts still use the
            // elapsed clock and fire once at their authored times.
            s.elapsed += dt;
            while s.burst_index < e.bursts.len() && e.bursts[s.burst_index].time <= s.elapsed {
                let burst = e.bursts[s.burst_index];
                s.burst_index += 1;
                spawn_n(s, e, pose, burst.count as usize, remaining_budget, d);
            }
        }
        if continuous_dt > 0.0 {
            s.spawn_accumulator += e.rate_per_second * continuous_dt;
            let n = s.spawn_accumulator.floor() as usize;
            s.spawn_accumulator -= n as f32;
            spawn_n(s, e, pose, n, remaining_budget, d);
        }
    }
    let mut i = 0;
    while i < s.len() {
        let mut vel = s.vel[i];
        // Gravity and wind are authored in world space. Local-space particles
        // store velocity in emitter-local coordinates, so rotate acceleration
        // into that coordinate frame before integration.
        let world_acceleration = e.gravity * e.gravity_scale + e.wind;
        let acceleration = if e.space == SimulationSpace::Local {
            pose.rotation.inverse() * world_acceleration
        } else {
            world_acceleration
        };
        vel += acceleration * dt;
        vel /= 1.0 + e.drag * dt;
        let mut pos = s.pos[i] + vel * dt;
        let mut dead = false;
        if e.space == SimulationSpace::World {
            if let Some(g) = e.ground {
                if pos.y < g.y {
                    match g.response {
                        GroundResponse::Kill => dead = true,
                        GroundResponse::Stop => {
                            pos.y = g.y;
                            vel = Vec3::ZERO
                        }
                        GroundResponse::Bounce { restitution } => {
                            pos.y = g.y;
                            vel.y = vel.y.abs() * restitution.clamp(0.0, 1.0)
                        }
                    }
                }
            }
        }
        s.age[i] += dt;
        s.rot[i] += s.ang_vel[i] * dt;
        s.pos[i] = pos;
        s.vel[i] = vel;
        if s.age[i] >= s.lifetime[i] {
            dead = true
        }
        if dead {
            remove_particle(s, i)
        } else {
            i += 1
        }
    }
}
fn spawn_n(
    s: &mut ParticleEmitterState,
    e: &ParticleEmitter,
    pose: Pose,
    n: usize,
    remaining: usize,
    d: &mut ParticleDiagnostics,
) {
    let available_emitter = (e.max_particles as usize).saturating_sub(s.len());
    let available_budget = remaining.saturating_sub(s.len());
    let allowed = n.min(available_emitter).min(available_budget);
    if available_budget < available_emitter {
        d.dropped_by_budget = d.dropped_by_budget.saturating_add((n - allowed) as u64);
    }
    for _ in 0..allowed {
        let (local_pos, dir) = sample_shape(&mut s.rng, e);
        let direction = if matches!(e.shape, EmitterShape::Cone { .. }) {
            dir
        } else {
            spread(&mut s.rng, e.direction, e.direction_spread_deg)
        };
        let speed = s.rng.range(e.start_speed.0, e.start_speed.1);
        let local = e.space == SimulationSpace::Local;
        let pos = if local {
            local_pos
        } else {
            pose.position + pose.rotation * local_pos
        };
        let vel = if local {
            direction * speed
        } else {
            pose.rotation * direction * speed
        };
        let c = s.rng.next_f32();
        let color = std::array::from_fn(|j| {
            e.start_color_a[j] + (e.start_color_b[j] - e.start_color_a[j]) * c
        });
        s.pos.push(pos);
        s.vel.push(vel);
        s.age.push(0.0);
        s.lifetime
            .push(s.rng.range(e.lifetime.0, e.lifetime.1).max(0.001));
        s.rot
            .push(s.rng.range(e.start_rotation.0, e.start_rotation.1));
        s.ang_vel
            .push(s.rng.range(e.angular_velocity.0, e.angular_velocity.1));
        s.size.push(s.rng.range(e.start_size.0, e.start_size.1));
        s.color0.push(color);
        d.spawned_this_frame = d.spawned_this_frame.saturating_add(1);
    }
}
fn sample_shape(r: &mut Pcg32, e: &ParticleEmitter) -> (Vec3, Vec3) {
    match e.shape {
        EmitterShape::Point => (Vec3::ZERO, e.direction),
        EmitterShape::Sphere {
            radius,
            surface_only,
        } => (
            if surface_only {
                r.unit_vec3() * radius.max(0.0)
            } else {
                r.in_unit_sphere() * radius.max(0.0)
            },
            e.direction,
        ),
        EmitterShape::Box { half_extents } => (
            Vec3::new(
                r.range(-half_extents.x.abs(), half_extents.x.abs()),
                r.range(-half_extents.y.abs(), half_extents.y.abs()),
                r.range(-half_extents.z.abs(), half_extents.z.abs()),
            ),
            e.direction,
        ),
        EmitterShape::Cone { angle_deg, radius } => {
            let radial = r.next_f32().sqrt() * radius.max(0.0);
            let angle = r.range(0.0, std::f32::consts::TAU);
            (
                Vec3::new(radial * angle.cos(), 0.0, radial * angle.sin()),
                spread(r, e.direction, angle_deg),
            )
        }
    }
}
fn spread(r: &mut Pcg32, base: Vec3, angle: f32) -> Vec3 {
    if angle <= 0.0 {
        return base.normalize_or_zero();
    }
    let axis = base.normalize_or_zero();
    let helper = if axis.y.abs() < 0.99 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let tangent = axis.cross(helper).normalize_or_zero();
    let bitangent = axis.cross(tangent);
    let max = angle.to_radians().clamp(0.0, std::f32::consts::PI);
    let cos: f32 = 1.0 - r.next_f32() * (1.0 - max.cos());
    let sin = (1.0 - cos * cos).max(0.0).sqrt();
    let phi = r.range(0.0, std::f32::consts::TAU);
    (axis * cos + tangent * (sin * phi.cos()) + bitangent * (sin * phi.sin())).normalize_or_zero()
}
fn remove_particle(s: &mut ParticleEmitterState, i: usize) {
    s.pos.swap_remove(i);
    s.vel.swap_remove(i);
    s.age.swap_remove(i);
    s.lifetime.swap_remove(i);
    s.rot.swap_remove(i);
    s.ang_vel.swap_remove(i);
    s.size.swap_remove(i);
    s.color0.swap_remove(i);
}
