//! Winit event loop and renderer-independent application driver.

use std::error::Error;
use std::num::NonZeroU32;
use std::sync::Arc;

use softbuffer::{Context, Surface};
use voxelworks_api::Framebuffer;
use voxelworks_api::Renderer;
use voxelworks_api::{App, Ctx, Flow};
use voxelworks_api::{CursorRequest, FrameInfo, WindowConfig};
use voxelworks_core::time::FixedStepper;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

use crate::cursor;
use crate::keyboard::{KeyboardState, MouseState};
use crate::present::blit_scaled;
use crate::timer::FrameTimer;
use crate::window::{attributes, mouse_position_in_framebuffer, physical_size};

enum Presenter {
    #[cfg(feature = "gpu")]
    Gpu(Box<voxelworks_render_gpu::GpuPresenter>),
    Cpu(SoftwarePresenter),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererMode {
    Auto,
    Cpu,
    Gpu,
}

struct SoftwarePresenter {
    _context: Context<Arc<Window>>,
    surface: Surface<Arc<Window>, Arc<Window>>,
    dimensions: (u32, u32),
    pixels: Vec<u32>,
}

impl SoftwarePresenter {
    fn new(window: Arc<Window>) -> Result<Self, Box<dyn Error>> {
        let context = Context::new(window.clone())?;
        let surface = Surface::new(&context, window.clone())?;
        Ok(Self {
            _context: context,
            surface,
            dimensions: (0, 0),
            pixels: Vec::new(),
        })
    }

    fn present(
        &mut self,
        window: &Window,
        framebuffer: &Framebuffer,
    ) -> Result<(), Box<dyn Error>> {
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        if self.dimensions != (size.width, size.height) {
            self.surface.resize(
                NonZeroU32::new(size.width).expect("nonzero window width"),
                NonZeroU32::new(size.height).expect("nonzero window height"),
            )?;
            self.dimensions = (size.width, size.height);
            self.pixels
                .resize(size.width as usize * size.height as usize, 0);
        }
        blit_scaled(framebuffer, &mut self.pixels, size.width, size.height);
        let mut buffer = self.surface.buffer_mut()?;
        buffer.copy_from_slice(&self.pixels);
        buffer.present()?;
        Ok(())
    }
}

impl Presenter {
    fn new(
        window: Arc<Window>,
        _config: &WindowConfig,
        mode: RendererMode,
        application_name: &str,
    ) -> Result<Self, Box<dyn Error>> {
        if mode == RendererMode::Cpu {
            println!("[{application_name}] CPU reference renderer enabled");
            return Ok(Self::Cpu(SoftwarePresenter::new(window)?));
        }
        #[cfg(feature = "gpu")]
        match voxelworks_render_gpu::GpuPresenter::new(
            window.clone(),
            (_config.width as u32, _config.height as u32),
        ) {
            Ok(gpu) => {
                println!("[{application_name}] GPU presentation enabled");
                Ok(Self::Gpu(Box::new(gpu)))
            }
            Err(error) => {
                if mode == RendererMode::Gpu {
                    return Err(std::io::Error::other(format!(
                        "[{application_name}] GPU renderer required but unavailable: {error}"
                    ))
                    .into());
                }
                eprintln!("[{application_name}] GPU unavailable; falling back to CPU presentation: {error}");
                Ok(Self::Cpu(SoftwarePresenter::new(window)?))
            }
        }
        #[cfg(not(feature = "gpu"))]
        {
            if mode == RendererMode::Gpu {
                return Err(std::io::Error::other(format!(
                    "[{application_name}] GPU renderer required but platform was built without the gpu feature"
                ))
                .into());
            }
            println!("[{application_name}] GPU feature disabled; CPU presentation enabled");
            Ok(Self::Cpu(SoftwarePresenter::new(window)?))
        }
    }

    fn resize(&mut self, _window_size: PhysicalSize<u32>) {
        #[cfg(feature = "gpu")]
        if let Self::Gpu(gpu) = self {
            gpu.resize_surface(_window_size.width, _window_size.height);
        }
    }

    fn render_and_present<R: Renderer>(
        &mut self,
        window: &Window,
        renderer: &mut R,
        framebuffer: &mut Framebuffer,
        plan: &voxelworks_api::FramePlan<'_>,
        materials: &voxelworks_scene::material::MaterialTable,
    ) -> Result<(), Box<dyn Error>> {
        match self {
            #[cfg(feature = "gpu")]
            Self::Gpu(gpu) => gpu
                .present_frame(framebuffer, plan, materials)
                .map_err(std::io::Error::other)
                .map_err(Into::into),
            Self::Cpu(cpu) => {
                renderer.render_frame(plan, materials);
                renderer.present(framebuffer);
                cpu.present(window, framebuffer)
            }
        }
    }
}

pub fn run<A: App, R: Renderer>(
    app: A,
    application_name: &str,
    config: WindowConfig,
    renderer: R,
) -> Result<(), Box<dyn Error>> {
    run_with_mode(app, application_name, config, renderer, RendererMode::Auto)
}

pub fn run_with_mode<A: App, R: Renderer>(
    app: A,
    application_name: &str,
    config: WindowConfig,
    renderer: R,
    renderer_mode: RendererMode,
) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.listen_device_events(winit::event_loop::DeviceEvents::Always);
    event_loop.set_control_flow(ControlFlow::Poll);
    let application_name = if application_name.trim().is_empty() {
        if config.title.trim().is_empty() {
            "Application".to_owned()
        } else {
            config.title.clone()
        }
    } else {
        application_name.to_owned()
    };
    let mut config = config;
    config.title = application_name.clone();
    let mut runner = Runner {
        app,
        config: config.clone(),
        application_name,
        renderer,
        framebuffer: Framebuffer::new(config.width, config.height),
        window: None,
        presenter: None,
        keys: KeyboardState::default(),
        mouse: MouseState::default(),
        mouse_delta: (0.0, 0.0),
        cursor_position: None,
        timer: FrameTimer::new(),
        stepper: FixedStepper::new(config.fixed_hz, config.max_steps),
        frame: 0,
        window_active: false,
        cursor_captured: false,
        grab_mode: None,
        failure: None,
        renderer_mode,
    };
    event_loop.run_app(&mut runner)?;
    if let Some(error) = runner.failure {
        return Err(std::io::Error::other(error).into());
    }
    Ok(())
}

struct Runner<A, R> {
    app: A,
    application_name: String,
    config: WindowConfig,
    renderer: R,
    framebuffer: Framebuffer,
    window: Option<Arc<Window>>,
    presenter: Option<Presenter>,
    keys: KeyboardState,
    mouse: MouseState,
    mouse_delta: (f32, f32),
    cursor_position: Option<PhysicalPosition<f64>>,
    timer: FrameTimer,
    stepper: FixedStepper,
    frame: u32,
    window_active: bool,
    cursor_captured: bool,
    grab_mode: Option<cursor::GrabMode>,
    failure: Option<String>,
    renderer_mode: RendererMode,
}

impl<A: App, R: Renderer> Runner<A, R> {
    fn draw_frame(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let sample = self.timer.begin_frame();
        let fixed_steps = self.stepper.advance(sample.dt);
        self.frame = self.frame.wrapping_add(1);
        let position = self.cursor_position.and_then(|pos| {
            mouse_position_in_framebuffer(
                pos,
                window.inner_size(),
                (self.framebuffer.width, self.framebuffer.height),
            )
        });
        let mouse = self
            .mouse
            .snapshot(std::mem::take(&mut self.mouse_delta), position);
        let pressed = self.keys.take_pressed();
        let mut ctx = Ctx::new(
            FrameInfo {
                dt: sample.dt,
                frame: self.frame,
                fps: sample.fps,
                fixed_dt: self.stepper.step(),
                fixed_steps,
                alpha: self.stepper.alpha(),
            },
            &self.keys,
            &pressed,
            mouse,
            self.window_active,
        );
        if self.app.update(&mut ctx) == Flow::Quit {
            cursor::release(&window);
            event_loop.exit();
            return;
        }
        if let Some(request) = ctx.requests.window {
            self.config.width = request.width;
            self.config.height = request.height;
            self.config.scale = request.scale.max(1);
            self.framebuffer = Framebuffer::new(request.width, request.height);
            self.renderer
                .resize(request.width as u32, request.height as u32);
            let _ = window.request_inner_size(physical_size(&self.config));
        }
        let plan = self.app.frame_plan(&ctx);
        if let Some(presenter) = &mut self.presenter {
            let result = presenter.render_and_present(
                &window,
                &mut self.renderer,
                &mut self.framebuffer,
                &plan,
                self.app.materials(),
            );
            if let Err(error) = result {
                self.failure = Some(error.to_string());
                event_loop.exit();
                return;
            }
        }

        let should_capture = ctx.requests.cursor == CursorRequest::Capture && self.window_active;
        if should_capture && !self.cursor_captured {
            self.grab_mode = cursor::capture(&window);
            self.cursor_captured = self.grab_mode.is_some();
        } else if !should_capture && self.cursor_captured {
            cursor::release(&window);
            self.cursor_captured = false;
            self.grab_mode = None;
        }
        if let Some(title) = ctx.requests.title {
            window.set_title(&format!("{} — {title}", self.application_name));
        }
        self.timer.cap_frame();
    }
}

impl<A: App, R: Renderer> ApplicationHandler for Runner<A, R> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match event_loop.create_window(attributes(&self.config)) {
            Ok(window) => {
                let window = Arc::new(window);
                self.window_active = true;
                self.renderer
                    .resize(self.config.width as u32, self.config.height as u32);
                match Presenter::new(
                    window.clone(),
                    &self.config,
                    self.renderer_mode,
                    &self.application_name,
                ) {
                    Ok(presenter) => {
                        if self.renderer_mode == RendererMode::Auto
                            && matches!(&presenter, Presenter::Cpu(_))
                        {
                            self.app.on_cpu_fallback();
                        }
                        self.presenter = Some(presenter);
                    }
                    Err(error) => {
                        self.failure =
                            Some(format!("could not create CPU fallback presenter: {error}"));
                        event_loop.exit();
                        return;
                    }
                }
                self.window = Some(window.clone());
                window.request_redraw();
            }
            Err(error) => {
                self.failure = Some(format!("could not create window: {error}"));
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.as_ref().cloned() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.draw_frame(event_loop),
            WindowEvent::Resized(size) => {
                if let Some(presenter) = &mut self.presenter {
                    presenter.resize(size);
                }
            }
            WindowEvent::Focused(focused) => {
                self.window_active = focused;
                if !focused {
                    self.keys.clear();
                    self.mouse.clear();
                    self.mouse_delta = (0.0, 0.0);
                    if self.cursor_captured {
                        cursor::release(&window);
                    }
                    self.cursor_captured = false;
                    self.grab_mode = None;
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.keys.process_winit_key(code, event.state, event.repeat);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = Some(position);
                if self.grab_mode == Some(cursor::GrabMode::Confined) {
                    cursor::recenter_confined(&window, position);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse.process_button(button, state);
            }
            WindowEvent::MouseWheel { delta, .. } => self.mouse.process_scroll(delta),
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(presenter) = &mut self.presenter {
                    presenter.resize(window.inner_size());
                }
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
        if self.cursor_captured {
            if let DeviceEvent::MouseMotion { delta } = event {
                self.mouse_delta.0 += delta.0 as f32;
                self.mouse_delta.1 += delta.1 as f32;
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
