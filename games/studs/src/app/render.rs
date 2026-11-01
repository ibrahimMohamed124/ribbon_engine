use super::*;

pub(super) fn render_game_frame(world: &mut World) {
    world.resource_scope::<StudsState, _>(|world, state| {
        world.resource_scope::<StudsRenderer, _>(|world, renderer| {
            let width = state.settings.render_width.max(1);
            let height = state.settings.render_height.max(1);
            renderer.resize(width, height);

            let has_game_scene = matches!(
                state.ui.screen(),
                Screen::Playing
                    | Screen::Result
                    | Screen::Pause
                    | Screen::Settings
                    | Screen::Controls
                    | Screen::ConfirmMainMenu
            );
            let apply_brightness = state.ui.is_playing()
                || matches!(
                    state.ui.screen(),
                    Screen::Result
                        | Screen::Pause
                        | Screen::Settings
                        | Screen::Controls
                        | Screen::ConfirmMainMenu
                );

            world.resource_scope::<MazeResource, _>(|world, maze| {
                world.resource_scope::<RunState, _>(|world, run| {
                    let Some((player, enemy, (pages, got))) = game_ecs::read_view_parts(world)
                    else {
                        return;
                    };
                    let style = {
                        let mut style = state
                            .settings
                            .visual_style
                            .render_style(state.settings.quality_preset);
                        style.limit_flicker = state.settings.reduce_flashing;
                        style
                    };
                    let view = GameReadView {
                        difficulty: run.difficulty,
                        world: &maze.0,
                        player: &player,
                        enemy: &enemy,
                        pages: &pages,
                        got: &got,
                        collected: run.collected,
                        time: run.time,
                        state: run.state,
                        state_t: run.state_t,
                        flashlight_on: run.flashlight_on,
                        enemy_awake_since: run.enemy_awake_since,
                    };
                    let scene = if has_game_scene {
                        view.scene_styled(state.render_frame, state.settings.hfov_radians(), style)
                    } else {
                        None
                    };
                    let mut world_layer = DrawList::new(width, height);
                    if apply_brightness {
                        view.draw_overlay(&mut world_layer, state.render_frame);
                    }
                    let ui_model: GameUiState = view.ui_state();
                    let mut ui_layer = DrawList::new(width, height);
                    if !state.ui.is_playing() {
                        state.ui.draw_model(
                            &mut ui_layer,
                            state.render_frame,
                            ui_model,
                            &state.settings,
                        );
                    } else {
                        state.ui.draw_game_hints_model(&mut ui_layer, ui_model);
                    }
                    let brightness = if apply_brightness {
                        state.settings.brightness
                    } else {
                        1.0
                    };

                    #[cfg(feature = "gpu")]
                    if renderer.backend == StudsRendererMode::Gpu {
                        // Only copy presentation-neutral camera/light data from the legacy
                        // scene adapter. The voxel world, chunking, greedy meshing and GPU
                        // mesh lifecycle are all owned by engine crates/plugins.
                        let (camera, flashlight) = scene
                            .as_ref()
                            .map(|scene| {
                                (
                                    studs_camera(scene.camera, width, height),
                                    scene.lighting.light_level,
                                )
                            })
                            .map_or((None, 0.0), |(camera, light)| (Some(camera), light));
                        // `view` has no destructor; its borrows end at its final use above.
                        // Drop the scene adapter explicitly because it can retain the world borrow.
                        drop(scene);

                        let objects = if camera.is_some() {
                            studs_scene_objects(&enemy, &pages, &got, run.time, run.enemy_awake())
                        } else {
                            Vec::new()
                        };
                        // `SunLight::direction` describes where light travels; the shader
                        // reverses it to obtain the surface-to-light vector.
                        let light_direction = camera
                            .map(|camera| camera.forward)
                            .unwrap_or(Vec3::new(-0.35, -0.8, -0.45));
                        let sun = SunLight {
                            direction: light_direction.normalize_or_zero(),
                            color: Vec3::splat(flashlight.max(0.0) * 3.0),
                            ambient: Vec3::splat(match state.settings.visual_style {
                                VisualStyle::Moody => 0.025,
                                VisualStyle::Classic => 0.10,
                                VisualStyle::Realistic => 0.065,
                            }),
                        };
                        let clear_color = match state.settings.visual_style {
                            VisualStyle::Moody => [0.002, 0.006, 0.004, 1.0],
                            VisualStyle::Classic => [0.018, 0.018, 0.018, 1.0],
                            VisualStyle::Realistic => [0.008, 0.009, 0.008, 1.0],
                        };
                        let palette = studs_voxel_palette(state.settings.visual_style);
                        let error = renderer.render_voxel_world(
                            maze.0.chunked_world_mut(),
                            camera,
                            &palette,
                            sun,
                            &objects,
                            clear_color,
                        );
                        if let Err(error) = error {
                            world.resource_mut::<ribbon_app::LogBuffer>().push(
                                ribbon_app::LogLevel::Warn,
                                format!(
                                    "STUDS mesh renderer frame failed: {error}; falling back to CPU"
                                ),
                            );
                            renderer.output.pixels.fill(0);
                            renderer.fall_back_to_cpu();
                            if let Some(settings) = world.try_resource::<RenderSettings>().copied()
                            {
                                world.insert_resource(RenderSettings {
                                    width: settings.width.max(1),
                                    height: settings.height.max(1),
                                });
                            }
                        }
                        studs_cpu_renderer::cpu::renderer::composite_frame_layers(
                            &mut renderer.output,
                            &world_layer,
                            brightness,
                            &ui_layer,
                        );
                    } else {
                        let error =
                            render_cpu_frame(renderer, scene, world_layer, brightness, ui_layer);
                        if let Some(error) = error {
                            world.resource_mut::<ribbon_app::LogBuffer>().push(
                                ribbon_app::LogLevel::Warn,
                                format!("STUDS renderer frame failed: {error}"),
                            );
                        }
                    }

                    #[cfg(not(feature = "gpu"))]
                    {
                        let error =
                            render_cpu_frame(renderer, scene, world_layer, brightness, ui_layer);
                        if let Some(error) = error {
                            world.resource_mut::<ribbon_app::LogBuffer>().push(
                                ribbon_app::LogLevel::Warn,
                                format!("STUDS renderer frame failed: {error}"),
                            );
                        }
                    }
                    if world.try_resource::<WindowFrame>().is_some() {
                        world.resource_mut::<WindowFrame>().set_pixels(
                            renderer.output.width,
                            renderer.output.height,
                            &renderer.output.pixels,
                        );
                    }
                });
            });
        });
    });
}

pub(super) fn render_cpu_frame(
    renderer: &mut StudsRenderer,
    scene: Option<Scene<'_>>,
    world_layer: DrawList,
    brightness: f32,
    ui_layer: DrawList,
) -> Option<String> {
    let plan = FramePlan {
        scene,
        world_layer,
        brightness,
        ui_layer,
    };
    let error = renderer.render_frame(&plan, studs_material_table());
    drop(plan);
    renderer.present();
    error
}

#[cfg(feature = "gpu")]
pub(super) fn studs_camera(
    camera: studs_cpu_renderer::Camera,
    width: usize,
    height: usize,
) -> OffscreenCamera3D {
    let forward = ribbon_math::forward_vec(camera.view);
    let aspect = width.max(1) as f32 / height.max(1) as f32;
    let fov_y = 2.0 * ((camera.hfov * 0.5).tan() / aspect.max(1.0e-4)).atan();
    OffscreenCamera3D {
        position: Vec3::new(camera.eye.x, camera.eye.z, -camera.eye.y),
        forward: Vec3::new(forward.x, forward.z, -forward.y),
        up: Vec3::Y,
        fov_y,
        near: 0.08,
        far: 160.0,
    }
}

#[cfg(feature = "gpu")]
pub(super) fn studs_voxel_palette(style: VisualStyle) -> [VoxelBlockColor; 3] {
    use crate::level::{CEIL, FLOOR, WALL};
    let (floor, wall, ceiling) = match style {
        VisualStyle::Classic => ([0.26, 0.26, 0.26], [0.38, 0.38, 0.38], [0.24, 0.24, 0.24]),
        VisualStyle::Moody => (
            [0.025, 0.045, 0.032],
            [0.075, 0.13, 0.095],
            [0.04, 0.06, 0.045],
        ),
        VisualStyle::Realistic => ([0.19, 0.21, 0.19], [0.30, 0.31, 0.28], [0.22, 0.23, 0.21]),
    };
    [
        VoxelBlockColor {
            block: FLOOR,
            color: floor,
        },
        VoxelBlockColor {
            block: WALL,
            color: wall,
        },
        VoxelBlockColor {
            block: CEIL,
            color: ceiling,
        },
    ]
}

#[cfg(feature = "gpu")]
pub(super) fn studs_scene_objects(
    enemy: &Enemy,
    pages: &[Vect],
    got: &[bool],
    time: f32,
    enemy_awake: bool,
) -> Vec<OffscreenDraw3D> {
    let mut objects = Vec::with_capacity(1 + pages.len());
    let enemy_position = Vec3::new(enemy.pos.x, enemy.pos.z + 0.67, -enemy.pos.y);
    let enemy_scale = if enemy_awake {
        Vec3::new(0.52, 1.34, 0.52)
    } else {
        Vec3::new(0.42, 1.08, 0.42)
    };
    objects.push(OffscreenDraw3D {
        model: Mat4::from_scale_rotation_translation(
            enemy_scale,
            Quat::from_rotation_y(enemy.yaw),
            enemy_position,
        ),
        tint: if enemy_awake {
            [0.9, 0.055, 0.035]
        } else {
            [0.16, 0.12, 0.10]
        },
    });

    for (index, page) in pages.iter().enumerate() {
        if got.get(index).copied().unwrap_or(true) {
            continue;
        }
        let phase = time * 0.9 + index as f32 * 1.7;
        let position = Vec3::new(page.x, page.z + 0.38 + phase.sin() * 0.08, -page.y);
        objects.push(OffscreenDraw3D {
            model: Mat4::from_scale_rotation_translation(
                Vec3::splat(0.22),
                Quat::from_rotation_y(phase),
                position,
            ),
            tint: [1.8, 1.35, 0.28],
        });
    }
    objects
}
