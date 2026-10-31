use super::*;

pub(super) fn update_ui_and_collect_input(world: &mut World) {
    world.resource_scope::<StudsState, _>(|world, state| {
        world.resource_scope::<PlatformInput, _>(|world, platform_input| {
            let pressed_keys: Vec<Key> = ALL_KEYS
                .iter()
                .copied()
                .filter(|key| platform_input.just_pressed(*key))
                .collect();
            let framebuffer_size = (
                state.settings.render_width.max(1),
                state.settings.render_height.max(1),
            );
            let window_size = world
                .try_resource::<WindowHandle>()
                .map(|window| {
                    let size = window.0.inner_size();
                    (size.width, size.height)
                })
                .or_else(|| {
                    world
                        .try_resource::<PlatformConfig>()
                        .map(|config| (config.width.max(1), config.height.max(1)))
                })
                .unwrap_or((framebuffer_size.0 as u32, framebuffer_size.1 as u32));
            let viewport = ribbon_platform::window::viewport(window_size, framebuffer_size);
            let mouse_position = platform_input.mouse_position.and_then(|position| {
                ribbon_platform::window::window_to_framebuffer(
                    position,
                    &viewport,
                    framebuffer_size,
                )
            });
            let ui_input = UiInput::from_keys(pressed_keys.clone()).with_mouse(
                mouse_position,
                platform_input.mouse_buttons.left,
                platform_input.mouse_scroll,
                framebuffer_size,
            );
            let outcome = state.ui.update(&ui_input, &mut state.settings);
            if outcome.quit {
                world.send_event(AppExit);
                return;
            }
            if outcome.settings_changed {
                let _ = state.settings.save(&state.settings_path);
                state.input.set_bindings(state.settings.bindings.clone());
                state
                    .input
                    .set_mouse_sensitivity(state.settings.mouse_sensitivity);
                state.input.set_invert_y(state.settings.invert_y);
            }
            if outcome.window_changed {
                let width = state.settings.render_width.max(1);
                let height = state.settings.render_height.max(1);
                let scale = state.settings.window_scale.max(1);
                world.insert_resource(PlatformConfig {
                    title: "STUDS".to_owned(),
                    width: (width as u32).saturating_mul(scale),
                    height: (height as u32).saturating_mul(scale),
                });
                world.insert_resource(RenderSettings {
                    width: width as u32,
                    height: height as u32,
                });
            }
            if outcome.start_game {
                state.pending_input.clear();
                let difficulty = outcome
                    .difficulty
                    .unwrap_or_else(|| world.resource::<RunState>().difficulty);
                let mut game = Game::new_with_difficulty(seed_now(), difficulty);
                game.flashlight_on = state.settings.flashlight_on_start;
                game_ecs::install_game(world, game);
            } else if outcome.restart_game {
                state.pending_input.clear();
                let (difficulty, flashlight_on) = {
                    let run = world.resource::<RunState>();
                    (run.difficulty, run.flashlight_on)
                };
                let mut game = Game::new_with_difficulty(seed_now(), difficulty);
                game.flashlight_on = flashlight_on;
                game_ecs::install_game(world, game);
            }

            let is_playing =
                state.ui.is_playing() && world.resource::<RunState>().state == State::Playing;
            if is_playing {
                let capture_mouse = state.input.mouse_look_active() && platform_input.focused;
                let mut mouse = FrameMouse(platform_input.mouse_delta);
                let frame_input =
                    state
                        .input
                        .poll_with_mouse_source(platform_input, &mut mouse, capture_mouse);
                if frame_input.quit {
                    world.send_event(AppExit);
                    return;
                }
                state
                    .pending_input
                    .accumulate_with(frame_input, |pending, latest| pending.merge(latest));
                platform_input.cursor_grab = if capture_mouse {
                    CursorGrabState::Locked
                } else {
                    CursorGrabState::Released
                };
            } else {
                state.pending_input.clear();
                state.repeat_input = InputFrame::default();
                state.input.synchronize_keys(platform_input);
                platform_input.cursor_grab = CursorGrabState::Released;
            }
            state.render_frame = world.resource::<Time>().frame as u32;
        });
    });
}

/// Select the buffered input for this fixed step and advance run-level clocks.
pub(super) fn begin_fixed_step(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    world.resource_scope::<StudsState, _>(|world, state| {
        let ui_playing = state.ui.is_playing();
        let step = world.resource_scope::<RunState, _>(|_, run| {
            if ui_playing && run.state == State::Playing {
                let input = if state.repeat_step {
                    state.repeat_input
                } else {
                    let first = state.pending_input.pop().unwrap_or_default();
                    state.repeat_input = first.level_only();
                    state.repeat_step = true;
                    first
                };
                let enemy_awake_before_time_advance = run.enemy_awake();
                if input.toggle_flashlight {
                    run.flashlight_on = !run.flashlight_on;
                }
                run.time += dt;
                RunStep {
                    active: true,
                    input,
                    enemy_awake_before_time_advance,
                }
            } else {
                if run.state != State::Playing {
                    run.state_t += dt;
                }
                RunStep::default()
            }
        });
        world.insert_resource(step);
    });
}

pub(super) fn finish_game_state(world: &mut World) {
    world.resource_scope::<StudsState, _>(|world, state| {
        world.resource_scope::<RunState, _>(|_, run| {
            if state.ui.is_playing() && run.state != State::Playing {
                if run.state == State::Won
                    && state.settings.record_best_time(run.difficulty, run.time)
                {
                    let _ = state.settings.save(&state.settings_path);
                }
                state.ui.show_result();
            }
        });
        // FixedUpdate may execute more than once per displayed frame. Repeat held controls
        // within that frame, then let the next PreUpdate supply fresh input edges and deltas.
        state.repeat_step = false;
    });
}

pub(super) fn update_window_title(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    let fps = if dt > 0.000_01 { 1.0 / dt } else { 0.0 };
    world.resource_scope::<StudsState, _>(|world, state| {
        state.title_timer += dt;
        if state.title_timer > 0.4 {
            state.title_timer = 0.0;
            let title = match state.ui.screen() {
                Screen::MainMenu => "STUDS | main menu".to_owned(),
                Screen::Difficulty => "STUDS | choose difficulty".to_owned(),
                Screen::Pause => "STUDS | paused".to_owned(),
                Screen::Settings => "STUDS | settings".to_owned(),
                Screen::Controls => "STUDS | controls".to_owned(),
                Screen::ConfirmMainMenu => "STUDS | confirm".to_owned(),
                Screen::Result | Screen::Playing => world.resource::<RunState>().window_title(fps),
            };
            if let Some(config) = world.try_resource::<PlatformConfig>().cloned() {
                world.insert_resource(PlatformConfig { title, ..config });
            }
        }
    });
}
