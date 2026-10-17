//! Winit event loop adapter for the engine's application runner contract.

use crate::runtime::{
    CursorGrabState, InputMouseButton, KeyboardEvent, MouseButtonEvent, MouseMotion, PendingInput,
    PlatformConfig, PlatformEvent, WindowResized,
};
use crate::{cursor, keyboard, window};
use engine_app::{App, AppExit, LogBuffer, LogLevel, Runner, Time};
use engine_events::{EventCursor, WorldEventsExt};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

/// Desktop runner that pumps winit events and advances the app each frame.
#[derive(Default)]
pub struct WinitRunner;

impl Runner for WinitRunner {
    fn run(self: Box<Self>, app: App) {
        let event_loop = match EventLoop::new() {
            Ok(event_loop) => event_loop,
            Err(error) => {
                eprintln!("could not start the desktop event loop: {error}");
                return;
            }
        };
        event_loop.set_control_flow(ControlFlow::Poll);
        let mut driver = DesktopDriver::new(app);
        if let Err(error) = event_loop.run_app(&mut driver) {
            if let Some(app) = driver.app.as_mut() {
                app.world.resource_mut::<LogBuffer>().push(
                    LogLevel::Error,
                    format!("desktop event loop failed: {error}"),
                );
            }
        }
    }
}

struct DesktopDriver {
    app: Option<App>,
    window: Option<Window>,
    window_id: Option<WindowId>,
    exit_cursor: EventCursor<AppExit>,
    last_frame: Instant,
    cursor_state_applied: CursorGrabState,
}

impl DesktopDriver {
    fn new(app: App) -> Self {
        Self {
            app: Some(app),
            window: None,
            window_id: None,
            exit_cursor: EventCursor::default(),
            last_frame: Instant::now(),
            cursor_state_applied: CursorGrabState::Released,
        }
    }

    fn queue(&mut self, event: PlatformEvent) {
        if let Some(app) = self.app.as_mut() {
            app.world.resource_mut::<PendingInput>().events.push(event);
        }
    }

    fn push_mouse_position(&mut self, position: PhysicalPosition<f64>) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        let input = window::mouse_position_in_framebuffer(
            position,
            size,
            (size.width as usize, size.height as usize),
        );
        self.queue(PlatformEvent::MouseMotion(MouseMotion {
            delta: (0.0, 0.0),
            position: input,
            scroll: 0.0,
        }));
    }
}

impl ApplicationHandler for DesktopDriver {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let config = self
            .app
            .as_ref()
            .and_then(|app| app.world.try_resource::<PlatformConfig>())
            .cloned()
            .unwrap_or_default();
        let attributes = Window::default_attributes()
            .with_title(config.title)
            .with_inner_size(PhysicalSize::new(config.width.max(1), config.height.max(1)));
        match event_loop.create_window(attributes) {
            Ok(window) => {
                self.window_id = Some(window.id());
                self.window = Some(window);
                self.last_frame = Instant::now();
            }
            Err(error) => {
                if let Some(app) = self.app.as_mut() {
                    app.world.resource_mut::<LogBuffer>().push(
                        LogLevel::Error,
                        format!("could not create the window: {error}"),
                    );
                }
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if Some(window_id) != self.window_id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => self.queue(PlatformEvent::CloseRequested),
            WindowEvent::Focused(focused) => self.queue(PlatformEvent::Focused(focused)),
            WindowEvent::Resized(size) => {
                self.queue(PlatformEvent::WindowResized(WindowResized {
                    width: size.width,
                    height: size.height,
                }));
            }
            WindowEvent::CursorMoved { position, .. } => self.push_mouse_position(position),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if let Some(key) = keyboard::from_winit(code) {
                        self.queue(PlatformEvent::Keyboard(KeyboardEvent {
                            key,
                            pressed: event.state == ElementState::Pressed,
                            repeat: event.repeat,
                        }));
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(button) = map_mouse_button(button) {
                    self.queue(PlatformEvent::MouseButton(MouseButtonEvent {
                        button,
                        pressed: state == ElementState::Pressed,
                    }));
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 / 40.0,
                };
                self.queue(PlatformEvent::MouseMotion(MouseMotion {
                    delta: (0.0, 0.0),
                    position: None,
                    scroll,
                }));
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.queue(PlatformEvent::MouseMotion(MouseMotion {
                delta: (delta.0 as f32, delta.1 as f32),
                position: None,
                scroll: 0.0,
            }));
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(app) = self.app.as_mut() else {
            event_loop.exit();
            return;
        };
        let now = Instant::now();
        app.world.resource_mut::<Time>().delta = now.saturating_duration_since(self.last_frame);
        self.last_frame = now;
        app.update();
        let requested_cursor = app.world.resource::<crate::runtime::Input>().cursor_grab;
        if requested_cursor != self.cursor_state_applied {
            if let Some(window) = self.window.as_ref() {
                self.cursor_state_applied = apply_cursor(window, requested_cursor);
                app.world
                    .resource_mut::<crate::runtime::Input>()
                    .cursor_grab = self.cursor_state_applied;
            }
        }
        if self
            .exit_cursor
            .read(app.world.events::<AppExit>())
            .next()
            .is_some()
        {
            event_loop.exit();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(app) = self.app.as_mut() {
            app.shutdown();
        }
    }
}

fn map_mouse_button(button: MouseButton) -> Option<InputMouseButton> {
    Some(match button {
        MouseButton::Left => InputMouseButton::Left,
        MouseButton::Right => InputMouseButton::Right,
        MouseButton::Middle => InputMouseButton::Middle,
        MouseButton::Back | MouseButton::Forward => return None,
        MouseButton::Other(index) => InputMouseButton::Other(index),
    })
}

/// Convert a winit scroll event into a neutral event value.
pub fn scroll_event(delta: MouseScrollDelta) -> MouseMotion {
    MouseMotion {
        delta: (0.0, 0.0),
        position: None,
        scroll: match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(position) => position.y as f32 / 40.0,
        },
    }
}

/// Convert a winit mouse button value to the platform-neutral button enum.
pub fn mouse_button(button: MouseButton) -> Option<InputMouseButton> {
    map_mouse_button(button)
}

/// Change the cursor capture state using the existing Locked-to-Confined fallback.
pub fn apply_cursor(window: &Window, requested: CursorGrabState) -> CursorGrabState {
    match requested {
        CursorGrabState::Released => {
            cursor::release(window);
            CursorGrabState::Released
        }
        CursorGrabState::Locked => {
            match cursor::capture_preferred(window, cursor::GrabMode::Locked) {
                Some(cursor::GrabMode::Locked) => CursorGrabState::Locked,
                Some(cursor::GrabMode::Confined) => CursorGrabState::Confined,
                None => CursorGrabState::Released,
            }
        }
        CursorGrabState::Confined => {
            match cursor::capture_preferred(window, cursor::GrabMode::Confined) {
                Some(cursor::GrabMode::Locked) => CursorGrabState::Locked,
                Some(cursor::GrabMode::Confined) => CursorGrabState::Confined,
                None => CursorGrabState::Released,
            }
        }
    }
}
