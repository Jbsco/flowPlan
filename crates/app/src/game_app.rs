//! Phase 1 game app: winit 0.30 `ApplicationHandler` with a fixed-timestep sim loop,
//! a double-buffered snapshot for render interpolation, and the flat-shaded renderer.
//!
//! Architecture follows [`DESIGN.md § Threading Model`](../../../../DESIGN.md#threading-model).
//! Phase 1 runs the sim on the main thread; Phase 3 moves it to a worker thread without
//! touching the `SnapshotBuffer` API.

#![deny(unsafe_code)]

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use crossbeam_queue::ArrayQueue;
use flowplan_assets::CpuMesh;
use flowplan_render::Renderer;
use flowplan_sim::{
    state::FIXED_DT, DifficultyDials, InputEvent, InputEvents, SimSnapshot, SimState, VehicleState,
};
use flowplan_world::{HeightSampler, Terrain};
use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::input_map::KeyState;
use crate::snapshot_buf::SnapshotBuffer;

/// Sky-blue clear color; placeholder until Phase 2 procedural sky lands.
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.52,
    g: 0.71,
    b: 0.88,
    a: 1.0,
};

/// Size of the input event queue. One frame of events rarely exceeds a dozen.
const INPUT_QUEUE_CAP: usize = 256;

/// World seed for the deterministic procedural terrain + sim. Arbitrary; a loadable
/// world seed lands with save/load in Phase 4.
const WORLD_SEED: u64 = 0x1337_CAFE_B0A7_5EAF;

/// Maximum number of ticks to advance in a single frame. Prevents a long pause from
/// producing a sim storm on resume.
const MAX_TICKS_PER_FRAME: u32 = 8;

/// Terrain patch dimensions. 96 cells at 400 m half-extent = fly over a meaningful area
/// while still fitting in a tiny vertex buffer.
const TERRAIN_CELLS: u32 = 96;
const TERRAIN_HALF_EXTENT: f32 = 400.0;

/// Cube size for the placeholder vehicle (meters).
const CUBE_SIZE: f32 = 2.0;

pub fn run() -> Result<()> {
    let event_loop = EventLoop::new().context("construct winit event loop")?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = GameApp::default();
    event_loop
        .run_app(&mut app)
        .context("run winit event loop")?;
    Ok(())
}

#[derive(Default)]
struct GameApp {
    runtime: Option<Runtime>,
}

struct Runtime {
    // wgpu / window.
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    // Render.
    renderer: Renderer,

    // Sim.
    sim: SimState,
    difficulty: DifficultyDials,
    tick_budget: f32,
    last_frame_time: Instant,

    // Sim -> render handoff.
    snapshots: SnapshotBuffer,

    // Input.
    input_queue: Arc<ArrayQueue<InputEvent>>,
    key_state: KeyState,
}

impl ApplicationHandler for GameApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.runtime.is_some() {
            return;
        }
        match Runtime::init(event_loop) {
            Ok(rt) => {
                let (w, h) = rt.size();
                tracing::info!("window + wgpu ready ({w}x{h}); sim + render online");
                self.runtime = Some(rt);
            },
            Err(e) => {
                tracing::error!("failed to initialize runtime: {e:#}");
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
        let Some(rt) = self.runtime.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("close requested");
                event_loop.exit();
            },
            WindowEvent::Resized(size) => {
                rt.resize(size.width, size.height);
            },
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                if is_escape_pressed(&key_event) {
                    event_loop.exit();
                    return;
                }
                rt.key_state.apply_key_event(&key_event, &rt.input_queue);
            },
            WindowEvent::RedrawRequested => {
                match rt.tick_and_render() {
                    Ok(()) => {},
                    Err(FrameError::Lost) => {
                        let (w, h) = rt.size();
                        rt.resize(w, h);
                    },
                    Err(FrameError::OutOfMemory) => {
                        tracing::error!("GPU out of memory -- exiting");
                        event_loop.exit();
                    },
                    Err(FrameError::Other(e)) => {
                        tracing::warn!("transient render error: {e:?}");
                    },
                }
                rt.window.request_redraw();
            },
            _ => {},
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(rt) = &self.runtime {
            rt.window.request_redraw();
        }
    }
}

fn is_escape_pressed(ev: &KeyEvent) -> bool {
    matches!(
        (ev.physical_key, ev.state),
        (
            PhysicalKey::Code(KeyCode::Escape),
            winit::event::ElementState::Pressed
        )
    )
}

impl Runtime {
    fn init(event_loop: &ActiveEventLoop) -> Result<Self> {
        // Window.
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("flowPlan (Phase 1 core loop)")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
                )
                .context("create window")?,
        );

        // wgpu init (same as Phase 0).
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

        // Build terrain + cube meshes.
        let terrain = Terrain::from_seed(WORLD_SEED);
        let terrain_mesh = CpuMesh::terrain_patch(TERRAIN_CELLS, TERRAIN_HALF_EXTENT, |x, z| {
            terrain.height(x, z)
        });
        let vehicle_mesh = CpuMesh::cube(1.0, [1.0, 0.82, 0.28, 1.0]);

        let renderer = Renderer::new(
            &device,
            &queue,
            surface_format,
            (config.width, config.height),
            &terrain_mesh,
            &vehicle_mesh,
            CUBE_SIZE,
        );

        // Sim.
        let mut sim = SimState::new(WORLD_SEED);
        // Start the vehicle a comfortable distance above the procedural terrain.
        sim.vehicle = VehicleState::at_origin();
        sim.vehicle.position = glam::Vec3::new(0.0, terrain.max_height() + 40.0, 0.0);

        let snapshots = SnapshotBuffer::new(SimSnapshot::from_state(&sim));
        let input_queue = Arc::new(ArrayQueue::new(INPUT_QUEUE_CAP));

        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            sim,
            difficulty: DifficultyDials::default(),
            tick_budget: 0.0,
            last_frame_time: Instant::now(),
            snapshots,
            input_queue,
            key_state: KeyState::default(),
        })
    }

    fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.renderer.resize(&self.device, (width, height));
    }

    fn tick_and_render(&mut self) -> Result<(), FrameError> {
        // Measure wall time since last frame; cap to avoid sim-storms after a pause.
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        self.tick_budget = (self.tick_budget + elapsed).min(0.25); // cap = 250 ms

        // Advance the sim by fixed-timestep ticks while we have budget.
        let mut tick_count = 0u32;
        while self.tick_budget >= FIXED_DT && tick_count < MAX_TICKS_PER_FRAME {
            // Gather input for this tick: held-axes first, then drain the queue.
            self.key_state.push_axis_events(&self.input_queue, FIXED_DT);
            let mut batch = InputEvents::default();
            while let Some(ev) = self.input_queue.pop() {
                batch.push(ev);
            }
            self.sim.tick(&batch, &self.difficulty, FIXED_DT);
            self.snapshots.publish(SimSnapshot::from_state(&self.sim));
            self.tick_budget -= FIXED_DT;
            tick_count += 1;
        }
        let alpha = self.tick_budget / FIXED_DT;

        // Acquire next surface frame.
        let output = match self.surface.get_current_texture() {
            Ok(o) => o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                return Err(FrameError::Lost);
            },
            Err(wgpu::SurfaceError::OutOfMemory) => {
                return Err(FrameError::OutOfMemory);
            },
            Err(e) => return Err(FrameError::Other(e)),
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let (prev, curr) = self.snapshots.read();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("phase1-encoder"),
            });
        self.renderer.render(
            &self.queue,
            &mut encoder,
            &view,
            self.size(),
            &prev,
            &curr,
            alpha,
            CLEAR_COLOR,
        );
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }
}

enum FrameError {
    Lost,
    OutOfMemory,
    Other(wgpu::SurfaceError),
}
