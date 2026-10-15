//! Desktop event loop for engine applications.

use std::error::Error;

use blocks_engine::app::{
    App, Ctx, CursorRequest, Flow, FrameInfo, MouseButtons, MouseInput, WindowConfig,
};
use blocks_engine::gfx::Framebuffer;
use blocks_engine::input::MouseSource;
use blocks_engine::time::FixedStepper;

use crate::cursor::CursorMouse;
use crate::keys::snapshot;
use crate::timer::FrameTimer;
use crate::window::{
    create_window, mouse_buttons, mouse_position_in_framebuffer, pressed_keys, scroll_y,
};

pub fn run<A: App>(mut app: A, mut config: WindowConfig) -> Result<(), Box<dyn Error>> {
    let mut window = create_window(&config)?;
    window.set_cursor_visibility(true);
    let mut framebuffer = Framebuffer::new(config.width, config.height);
    let mut mouse = CursorMouse::new();
    let mut timer = FrameTimer::new();
    let mut stepper = FixedStepper::new(config.fixed_hz, config.max_steps);
    let mut frame = 0u32;

    while window.is_open() {
        let sample = timer.begin_frame();
        let fixed_steps = stepper.advance(sample.dt);
        frame = frame.wrapping_add(1);
        let pressed = pressed_keys(&mut window);
        let keys = snapshot(&window);
        let mouse_delta = mouse.delta();
        let (left, right, middle) = mouse_buttons(&window);
        let mut ctx = Ctx::new(
            FrameInfo {
                dt: sample.dt,
                frame,
                fps: sample.fps,
                fixed_dt: stepper.step(),
                fixed_steps,
                alpha: stepper.alpha(),
            },
            &keys,
            &pressed,
            MouseInput {
                delta: mouse_delta,
                position: mouse_position_in_framebuffer(
                    &window,
                    (framebuffer.width, framebuffer.height),
                ),
                scroll: scroll_y(&window),
                buttons: MouseButtons {
                    left,
                    right,
                    middle,
                },
            },
            window.is_active(),
        );

        if app.update(&mut ctx) == Flow::Quit {
            mouse.release(&mut window);
            break;
        }

        if let Some(request) = ctx.requests.window {
            mouse.release(&mut window);
            config.width = request.width;
            config.height = request.height;
            config.scale = request.scale;
            window = create_window(&config)?;
            window.set_cursor_visibility(true);
            framebuffer = Framebuffer::new(config.width, config.height);
        }

        app.render(&mut framebuffer, &ctx);
        window.update_with_buffer(&framebuffer.pixels, framebuffer.width, framebuffer.height)?;

        let should_capture = ctx.requests.cursor == CursorRequest::Capture && window.is_active();
        mouse.update(&mut window, should_capture);
        if let Some(title) = ctx.requests.title {
            window.set_title(&title);
        }
        timer.cap_frame();
    }

    mouse.release(&mut window);
    Ok(())
}
