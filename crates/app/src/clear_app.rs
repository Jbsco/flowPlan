//! Phase 0 window-clearing app: opens a window, initializes wgpu, clears each frame.
//!
//! Uses winit 0.30's `ApplicationHandler` pattern and wgpu 22's surface model.
//! Runtime state (`State`) is created lazily on `resumed` and torn down on window close.

#![deny(unsafe_code)]

use std::sync::Arc;

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Clear color: a calm cel-shaded sky-blue. Placeholder for the sky model arriving in
/// Phase 2 per [`ROADMAP.md § Milestone 2.6`](../../../../ROADMAP.md#milestone-26-sky-daytime-foundation).
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.52,
    g: 0.71,
    b: 0.88,
    a: 1.0,
};

/// Entry point for the Phase 0 clear-app.
pub fn run() -> Result<()> {
    let event_loop = EventLoop::new().context("construct winit event loop")?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = ClearApp::default();
    event_loop
        .run_app(&mut app)
        .context("run winit event loop")?;
    Ok(())
}

/// winit application handler. The window and wgpu state are created on `resumed` (per
/// the winit 0.30 lifecycle) so this struct is `Default`-constructible up front.
#[derive(Default)]
struct ClearApp {
    state: Option<State>,
}

impl ApplicationHandler for ClearApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop) {
            Ok(s) => {
                tracing::info!("window + wgpu ready ({}x{})", s.size().0, s.size().1);
                self.state = Some(s);
            },
            Err(e) => {
                tracing::error!("failed to initialize state: {e:#}");
                event_loop.exit();
            },
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("close requested");
                event_loop.exit();
            },
            WindowEvent::Resized(new_size) => {
                state.resize(new_size.width, new_size.height);
            },
            WindowEvent::RedrawRequested => {
                match state.render() {
                    Ok(()) => {},
                    Err(RenderError::Lost) => {
                        let (w, h) = state.size();
                        state.resize(w, h);
                    },
                    Err(RenderError::OutOfMemory) => {
                        tracing::error!("GPU out of memory -- exiting");
                        event_loop.exit();
                    },
                    Err(RenderError::Other(e)) => {
                        tracing::warn!("transient render error: {e:?}");
                    },
                }
                state.window.request_redraw();
            },
            _ => {},
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}

/// Owned runtime state: window (arced so the surface can hold it), wgpu device+queue,
/// surface config. Placeholder until render/sim crates take over in Phase 1+.
struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

impl State {
    fn new(event_loop: &ActiveEventLoop) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("flowPlan (Phase 0 skeleton)")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
                )
                .context("create window")?,
        );

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance
            .create_surface(Arc::clone(&window))
            .context("create surface")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .context("no suitable wgpu adapter found")?;

        tracing::info!("adapter: {:?}", adapter.get_info());

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("flowplan-device"),
                required_features: wgpu::Features::empty(),
                required_limits:
                    wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .context("request wgpu device")?;

        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: surface_caps
                .present_modes
                .iter()
                .copied()
                .find(|p| *p == wgpu::PresentMode::Mailbox)
                .unwrap_or(wgpu::PresentMode::AutoVsync),
            desired_maximum_frame_latency: 2,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: Vec::new(),
        };
        surface.configure(&device, &config);

        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
        })
    }

    const fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn render(&self) -> Result<(), RenderError> {
        let output = match self.surface.get_current_texture() {
            Ok(o) => o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                return Err(RenderError::Lost);
            },
            Err(wgpu::SurfaceError::OutOfMemory) => {
                return Err(RenderError::OutOfMemory);
            },
            Err(e) => {
                return Err(RenderError::Other(e));
            },
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flowplan-clear-encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("flowplan-clear-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }
}

enum RenderError {
    Lost,
    OutOfMemory,
    Other(wgpu::SurfaceError),
}
