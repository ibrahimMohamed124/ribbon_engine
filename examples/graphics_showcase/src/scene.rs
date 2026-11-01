use super::*;

pub(super) fn spawn_cube(
    world: &mut World,
    position: Vec3,
    scale: Vec3,
    color: [f32; 3],
    motion: Option<CubeMotion>,
) {
    let transform = Transform3D {
        translation: position,
        scale,
        ..Transform3D::default()
    };
    let mesh = MeshInstance {
        mesh: BuiltinMesh::Cube,
        color,
    };
    if let Some(motion) = motion {
        world.spawn((transform, mesh, motion));
    } else {
        world.spawn((transform, mesh));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_model(
    world: &mut World,
    model: ribbon_assets::Handle<ModelAsset>,
    local_center: Vec3,
    world_center: Vec3,
    scale: f32,
    radians_per_second: f32,
    phase: f32,
    tilt: f32,
) {
    // Transform3D is the outer transform. Offset it so the GLB's actual bounds
    // center (which may not be at the asset origin) stays on the desired pivot.
    let transform = Transform3D {
        translation: world_center - local_center * scale,
        scale: Vec3::splat(scale),
        ..Transform3D::default()
    };
    world.spawn((
        transform,
        ModelInstance { model },
        ModelMotion {
            world_center,
            local_center,
            radians_per_second,
            phase,
            tilt,
        },
    ));
}

pub(super) fn animate_showcase(world: &mut World) {
    let delta = world.resource::<Time>().delta.as_secs_f32();
    let elapsed = world.resource::<Time>().elapsed.as_secs_f32();
    let frame = world.resource::<Time>().frame;
    let (space, left, right, up, down, reset) = {
        let input = world.resource::<Input>();
        (
            input.just_pressed(Key::Space),
            input.is_down(Key::Left),
            input.is_down(Key::Right),
            input.is_down(Key::Up),
            input.is_down(Key::Down),
            input.just_pressed(Key::R),
        )
    };

    let (orbit_angle, camera_height, camera_distance, paused, frame_limit) = {
        let state = world.resource_mut::<ShowcaseState>();
        if reset {
            state.orbit_angle = 0.35;
            state.orbit_speed = 0.24;
            state.camera_height =
                (state.camera_distance * 0.31).clamp(5.0, state.camera_height_max);
            state.paused = false;
        }
        if space {
            state.paused = !state.paused;
        }
        // Let the user change the orbit rate, including reversing its direction.
        let speed_delta = delta * 0.45;
        if left {
            state.orbit_speed -= speed_delta;
        }
        if right {
            state.orbit_speed += speed_delta;
        }
        state.orbit_speed = state.orbit_speed.clamp(-1.5, 1.5);
        if up {
            state.camera_height =
                (state.camera_height + delta * 2.0).clamp(1.5, state.camera_height_max);
        }
        if down {
            state.camera_height =
                (state.camera_height - delta * 2.0).clamp(1.5, state.camera_height_max);
        }
        if !state.paused {
            state.orbit_angle += delta * state.orbit_speed;
        }
        (
            state.orbit_angle,
            state.camera_height,
            state.camera_distance,
            state.paused,
            state.frame_limit,
        )
    };

    if !paused {
        for (_, (transform, motion)) in world.query_mut::<(&mut Transform3D, &CubeMotion)>() {
            if motion.radians_per_second.abs() > f32::EPSILON {
                let axis = motion.axis.normalize_or_zero();
                if axis != Vec3::ZERO {
                    transform.rotation =
                        Quat::from_axis_angle(axis, motion.radians_per_second * delta)
                            * transform.rotation;
                }
            }
            transform.translation = motion.base_position
                + Vec3::Y * (elapsed * 1.55 + motion.phase).sin() * motion.bob_amplitude;
        }
        for (_, (transform, motion)) in world.query_mut::<(&mut Transform3D, &ModelMotion)>() {
            let rotation =
                Quat::from_rotation_y(elapsed * motion.radians_per_second + motion.phase)
                    * Quat::from_rotation_x(motion.tilt);
            transform.rotation = rotation;
            transform.translation =
                motion.world_center - rotation * (transform.scale * motion.local_center);
        }

        // A gently moving directional light makes the mesh normals and depth visibly useful.
        let sun_phase = elapsed * 0.22;
        let sun = world.resource_mut::<SunLight>();
        sun.direction =
            Vec3::new(sun_phase.cos() * 0.55, -0.78, sun_phase.sin() * 0.55).normalize();
    }

    let target = Vec3::new(0.0, 0.75, 0.0);
    let eye = target
        + Vec3::new(
            orbit_angle.sin() * camera_distance,
            camera_height,
            orbit_angle.cos() * camera_distance,
        );
    for (_, (transform, _)) in world.query_mut::<(&mut Transform3D, &Camera3D)>() {
        *transform = Transform3D::looking_at(eye, target, Vec3::Y);
    }

    if frame_limit.is_some_and(|limit| frame >= limit) {
        world.send_event(AppExit);
    }
}

pub(super) fn report_render_extraction(world: &mut World) {
    let (frame, has_camera, mesh_count, model_count) = {
        let extracted = world.resource::<ExtractedScene3D>();
        (
            extracted.frame,
            extracted.camera.is_some(),
            extracted.meshes.len(),
            extracted.models.len(),
        )
    };
    if frame == 0 {
        return;
    }
    let renderable_count = mesh_count + model_count;
    let (is_stress_scene, activation_pending, stress_active) = world
        .try_resource::<LargeSceneStress>()
        .map(|stress| {
            (
                stress.grid_side > 0,
                stress.pending_activation,
                stress.active,
            )
        })
        .unwrap_or((false, false, false));

    // Multi-asset stress startup is intentionally asynchronous. Do not report the expected empty
    // extraction before model decode as a renderer failure; wait for activation and a few frames
    // for the extraction stage to observe newly spawned placements. Activation failures are
    // reported by update_large_scene_stress and still result in a final extraction failure.
    if renderable_count == 0 && is_stress_scene && (activation_pending || stress_active) {
        if activation_pending {
            return;
        }
        let state = world.resource_mut::<ShowcaseState>();
        if state.extraction_reported {
            return;
        }
        state.extraction_empty_frames = state.extraction_empty_frames.saturating_add(1);
        if state.extraction_empty_frames < MAX_STRESS_EXTRACT_EMPTY_FRAMES {
            return;
        }
    }

    let state = world.resource_mut::<ShowcaseState>();
    if state.extraction_reported {
        return;
    }
    state.extraction_reported = true;
    if has_camera && renderable_count > 0 {
        println!(
            "[RENDER EXTRACT] PASS: frame={frame}, camera=1, built-in meshes={mesh_count}, glTF placements={model_count}"
        );
    } else {
        println!(
            "[RENDER EXTRACT] FAIL: frame={frame}, camera={has_camera}, built-in meshes={mesh_count}, glTF placements={model_count}"
        );
    }
}
