//! Winit event loop adapter for the engine's application runner contract.

use crate::runtime::{
    CursorGrabState, InputMouseButton, KeyboardEvent, MouseButtonEvent, MouseMotion, PendingInput,
    PlatformConfig, PlatformEvent, WindowFrame, WindowResized,
};
use crate::{cursor, keyboard, window};
use engine_app::{App, AppExit, LogBuffer, LogLevel, Runner, Time};
use engine_events::{EventCursor, WorldEventsExt};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use softbuffer::{Context, Surface};
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
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    exit_cursor: EventCursor<AppExit>,
    last_frame: Instant,
    cursor_state_applied: CursorGrabState,
    presenter: Option<SoftwarePresenter>,
    requested_window_size: (u32, u32),
    applied_title: String,
    presentation_failed: bool,
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
            presenter: None,
            requested_window_size: (0, 0),
            applied_title: String::new(),
            presentation_failed: false,
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

    fn present_frame(&mut self) {
        let Some(app) = self.app.as_mut() else {
            return;
        };
        if self.presentation_failed {
            return;
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        // PlatformPlugin always registers WindowFrame, including for GPU-only apps. Do not
        // create a softbuffer/GDI surface merely because the resource exists: doing so can make
        // softbuffer and wgpu compete for the same HWND and crash during shutdown on Windows.
        // Wait until a CPU renderer has submitted a complete, non-empty frame before creating it.
        let has_presentable_frame = app
            .world
            .try_resource::<WindowFrame>()
            .is_some_and(|frame| {
                frame.width > 0
                    && frame.height > 0
                    && frame.pixels.len() >= frame.width.saturating_mul(frame.height)
            });
        if !has_presentable_frame {
            return;
        }
        if self.presenter.is_none() {
            match SoftwarePresenter::new(window.clone()) {
                Ok(presenter) => self.presenter = Some(presenter),
                Err(error) => {
                    app.world.resource_mut::<LogBuffer>().push(
                        LogLevel::Warn,
                        format!("CPU window presentation unavailable: {error}"),
                    );
                    self.presentation_failed = true;
                    return;
                }
            }
        }
        let presentation_error = match (
            self.presenter.as_mut(),
            app.world.try_resource::<WindowFrame>(),
        ) {
            (Some(presenter), Some(frame)) => presenter.present(window, frame).err(),
            _ => None,
        };
        if let Some(error) = presentation_error {
            app.world.resource_mut::<LogBuffer>().push(
                LogLevel::Warn,
                format!("CPU window presentation stopped: {error}"),
            );
            self.presentation_failed = true;
            self.presenter = None;
        }
    }
}

/// CPU framebuffer presenter used by rendering plugins that target the platform's
/// renderer-neutral [`WindowFrame`] contract.
struct SoftwarePresenter {
    _context: Context<Arc<Window>>,
    surface: Surface<Arc<Window>, Arc<Window>>,
    dimensions: (u32, u32),
    pixels: Vec<u32>,
}

impl SoftwarePresenter {
    fn new(window: Arc<Window>) -> Result<Self, String> {
        let context = Context::new(window.clone()).map_err(|error| error.to_string())?;
        let surface = Surface::new(&context, window).map_err(|error| error.to_string())?;
        Ok(Self {
            _context: context,
            surface,
            dimensions: (0, 0),
            pixels: Vec::new(),
        })
    }

    fn present(&mut self, window: &Window, frame: &WindowFrame) -> Result<(), String> {
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 || frame.width == 0 || frame.height == 0 {
            return Ok(());
        }
        let required = frame.width.saturating_mul(frame.height);
        if frame.pixels.len() < required {
            return Err("submitted frame has fewer pixels than its dimensions require".to_owned());
        }
        if self.dimensions != (size.width, size.height) {
            self.surface
                .resize(
                    NonZeroU32::new(size.width).ok_or("zero width surface")?,
                    NonZeroU32::new(size.height).ok_or("zero height surface")?,
                )
                .map_err(|error| error.to_string())?;
            self.dimensions = (size.width, size.height);
            self.pixels
                .resize(size.width as usize * size.height as usize, 0);
        }
        scale_frame(
            frame.width,
            frame.height,
            &frame.pixels[..required],
            size.width,
            size.height,
            &mut self.pixels,
        );
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|error| error.to_string())?;
        buffer.copy_from_slice(&self.pixels);
        buffer.present().map_err(|error| error.to_string())
    }
}

fn scale_frame(
    source_width: usize,
    source_height: usize,
    source: &[u32],
    destination_width: u32,
    destination_height: u32,
    destination: &mut [u32],
) {
    destination.fill(0);
    if source_width == 0 || source_height == 0 || destination_width == 0 || destination_height == 0
    {
        return;
    }
    let scale = (destination_width as f32 / source_width as f32)
        .min(destination_height as f32 / source_height as f32);
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let offset_x = (destination_width as f32 - source_width as f32 * scale) * 0.5;
    let offset_y = (destination_height as f32 - source_height as f32 * scale) * 0.5;
    let image_right = offset_x + source_width as f32 * scale;
    let image_bottom = offset_y + source_height as f32 * scale;
    for y in 0..destination_height {
        let py = y as f32 + 0.5;
        if py < offset_y || py >= image_bottom {
            continue;
        }
        let sy = (((py - offset_y) / scale).floor() as usize).min(source_height - 1);
        for x in 0..destination_width {
            let px = x as f32 + 0.5;
            if px < offset_x || px >= image_right {
                continue;
            }
            let sx = (((px - offset_x) / scale).floor() as usize).min(source_width - 1);
            let source_index = sy * source_width + sx;
            let destination_index = y as usize * destination_width as usize + x as usize;
            if let (Some(&pixel), Some(output)) = (
                source.get(source_index),
                destination.get_mut(destination_index),
            ) {
                *output = pixel & 0x00ff_ffff;
            }
        }
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
        let attributes = window::with_engine_icons(
            Window::default_attributes()
                .with_title(config.title.clone())
                .with_inner_size(PhysicalSize::new(config.width.max(1), config.height.max(1))),
        );
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window = Arc::new(window);
                self.window_id = Some(window.id());
                if let Some(app) = self.app.as_mut() {
                    app.world
                        .insert_resource(crate::runtime::WindowHandle(window.clone()));
                    app.world.resource_mut::<crate::runtime::Input>().focused = true;
                    app.world
                        .resource_mut::<PendingInput>()
                        .events
                        .push(PlatformEvent::Focused(true));
                }
                self.requested_window_size = (config.width.max(1), config.height.max(1));
                self.applied_title = config.title.clone();
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
            WindowEvent::RedrawRequested => self.present_frame(),
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

        // Keep native window metadata under platform configuration control. Game and editor
        // plugins change resources; they do not call winit directly.
        if let Some(config) = app.world.try_resource::<PlatformConfig>().cloned() {
            if config.title != self.applied_title {
                if let Some(window) = self.window.as_ref() {
                    window.set_title(&config.title);
                }
                self.applied_title = config.title.clone();
            }
            let requested_size = (config.width.max(1), config.height.max(1));
            if requested_size != self.requested_window_size {
                if let Some(window) = self.window.as_ref() {
                    let _ = window
                        .request_inner_size(PhysicalSize::new(requested_size.0, requested_size.1));
                }
                self.requested_window_size = requested_size;
            }
        }

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
        } else if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(app) = self.app.as_mut() {
            app.shutdown();
        }
        // Tear down while the event loop is still alive and in dependency order: CPU presenter,
        // then the app (GPU surface/device/passes), then the window they were created from.
        // Dropping the window first, or letting it outlive the loop, crashes wgpu on Windows.
        self.presenter = None;
        self.app = None;
        self.window = None;
        self.window_id = None;
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
