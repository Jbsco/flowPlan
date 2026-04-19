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
use flowplan_render::{Renderer, SceneParams, ShadingParams};
use flowplan_sim::{
    state::FIXED_DT, AircraftSpec, DifficultyDials, InputEvent, InputEvents, SimSnapshot, SimState,
    VehicleState,
};
use flowplan_world::{sun_direction, HeightSampler, Terrain};
use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::debug_hud::DebugHud;
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

/// Terrain patch dimensions. 200 cells at 2000 m half-extent = 4 km x 4 km patch,
/// ~20 m per cell. Large enough that the plane is visibly moving over recognizable
/// terrain at cruise speed; small enough that the vertex buffer (~40 k verts, ~240 k
/// indices) uploads and renders trivially. Phase 2 replaces this with a streamed
/// heightmap tied to the chosen world scope.
const TERRAIN_CELLS: u32 = 200;
const TERRAIN_HALF_EXTENT: f32 = 2000.0;

/// Cube size for the placeholder vehicle (meters). Used only when the glTF aircraft at
/// `VEHICLE_GLB_PATH` is missing or malformed.
const CUBE_SIZE: f32 = 2.0;

/// Workspace-relative path to the MVP aircraft glTF. See AIRCRAFT.md § MVP Reference.
/// Populated by the author via Blender export; absent by default. Load failure is
/// non-fatal -- we fall back to the placeholder cube so the app always runs.
const VEHICLE_GLB_PATH: &str = "data/aircraft/northlander.glb";

/// Workspace-relative path to the MVP aircraft spec sheet.
const VEHICLE_SPEC_PATH: &str = "data/aircraft/northlander.ron";

/// Cel-shading presets for the Phase 1 placeholder scene. The terrain is a large, mostly
/// diffuse surface: many bands (4), high ambient floor (0.5), low rim (0.08). The vehicle
/// is a small object where silhouette reads matter: fewer bands (3), lower ambient (0.35),
/// stronger rim (0.35) with a sharp falloff (exponent 3.0). Tuning will be revisited once
/// the DHC-2 glTF model lands in Milestone 2.3.
const TERRAIN_SHADING: ShadingParams = ShadingParams::new(4.0, 0.5, 0.08, 4.0);
const VEHICLE_SHADING: ShadingParams = ShadingParams::new(3.0, 0.35, 0.35, 3.0);

/// Water shading: 3 bands for a readable surface, moderate ambient floor (calm water
/// reads as lit even in shadow), and a low rim (sun sparkle arrives with Gerstner waves).
const WATER_SHADING: ShadingParams = ShadingParams::new(3.0, 0.55, 0.12, 3.5);

/// Water tessellation. 64 cells across 4000 m half-extent matches the terrain patch so
/// the two meet cleanly. Milestone 2.2.2 uses the existing vertices as Gerstner anchors.
const WATER_CELLS: u32 = 64;
const WATER_HALF_EXTENT: f32 = 4000.0;

/// Deep cel-water tint. Cooler than the sky so the horizon reads. Alpha is unused by the
/// cel shader.
const WATER_COLOR: [f32; 4] = [0.18, 0.38, 0.58, 1.0];

/// Mean water level in world-space Y. Gerstner waves oscillate around this; the passive
/// ground clamp (pre-2.5) uses it as a static floor so the plane can't descend through
/// mean sea level while buoyancy is still not modeled.
const WATER_LEVEL: f32 = 0.0;

/// Small clearance added to the floor to avoid surface z-fighting between the vehicle
/// and the terrain / water mesh.
const SURFACE_MARGIN: f32 = 0.05;

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

    // World (kept on the app side; sim doesn't depend on terrain in Phase 1).
    terrain: Terrain,
    /// Depth (meters) from the vehicle mesh's CG/origin down to its lowest point
    /// (floats / wheels). Used to lift the spawn altitude and each-tick `min_altitude`
    /// so those contact points clear terrain / water.
    vehicle_float_offset: f32,

    // Linear (non-sRGB) view format for the egui HUD pass. See `linear_variant`.
    hud_format: wgpu::TextureFormat,

    // Sim.
    sim: SimState,
    difficulty: DifficultyDials,
    tick_budget: f32,
    last_frame_time: Instant,
    /// Wall-clock Instant captured at startup; fed to `SceneParams.time_seconds`.
    start_time: Instant,

    // Sim -> render handoff.
    snapshots: SnapshotBuffer,

    // Input.
    input_queue: Arc<ArrayQueue<InputEvent>>,
    key_state: KeyState,

    // Debug HUD (egui overlay -- Phase 1 debug only).
    debug_hud: DebugHud,
    /// Number of fixed-step ticks executed in the last frame, exposed to the HUD.
    ticks_last_frame: u32,
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
        // Debug HUD sees every event first (for scale factor, scroll, etc.). It never
        // consumes game input in Phase 1; the sim still receives subsequent processing.
        rt.debug_hud.on_window_event(&rt.window, &event);
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
    // wgpu initialization is a linear descriptor-chain; splitting it doesn't improve
    // readability, so allow the slight overrun on this function's line count.
    #[allow(clippy::too_many_lines)]
    fn init(event_loop: &ActiveEventLoop) -> Result<Self> {
        // Window.
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("flowPlan")
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
        // Linear (non-sRGB) counterpart for the egui HUD pass. egui's shader output is
        // already in sRGB space; writing through an sRGB view would re-encode and
        // produce slightly washed-out colors. Declaring this in `view_formats` allows
        // making a linear `TextureView` of the same swapchain texture.
        let hud_format = linear_variant(surface_format);
        let view_formats = if hud_format == surface_format {
            Vec::new()
        } else {
            vec![hud_format]
        };
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
            view_formats,
        };
        surface.configure(&device, &config);

        // Build terrain + vehicle meshes. The vehicle comes from `VEHICLE_GLB_PATH` when
        // available; otherwise we fall back to the placeholder cube.
        let terrain = Terrain::from_seed(WORLD_SEED);
        let terrain_mesh = CpuMesh::terrain_patch(TERRAIN_CELLS, TERRAIN_HALF_EXTENT, |x, z| {
            terrain.height(x, z)
        });
        let vehicle_mesh = load_vehicle_mesh();
        // Depth of the lowest mesh point below the origin (CG). Used below to lift the
        // spawn altitude so the floats clear terrain. Zero when the mesh sits at or
        // above y=0.
        let vehicle_float_offset = (-vehicle_mesh.bbox_min_y()).max(0.0);
        let water_mesh = CpuMesh::water_grid(WATER_CELLS, WATER_HALF_EXTENT, WATER_COLOR);
        let aircraft_spec = load_aircraft_spec();

        let renderer = Renderer::new(
            &device,
            &queue,
            surface_format,
            (config.width, config.height),
            &terrain_mesh,
            TERRAIN_SHADING,
            &water_mesh,
            WATER_SHADING,
            &vehicle_mesh,
            VEHICLE_SHADING,
        );

        // Sim.
        let mut sim = SimState::new(WORLD_SEED);
        sim.aircraft = aircraft_spec;
        // Start the vehicle a comfortable distance above the procedural terrain,
        // already in cruise forward velocity so placeholder-wing lift balances gravity
        // from frame zero. This avoids the "I pressed throttle but fell" confusion
        // that a dead-stop start produces with only thrust and gravity.
        sim.vehicle = VehicleState::at_origin();
        // CG-centered mesh means `position.y` is the CG height. Lift by the float-to-CG
        // distance so the floats clear terrain peaks by the same 25 m margin the old
        // bottom-anchored mesh had.
        let start_y = terrain.max_height() + vehicle_float_offset + 25.0;
        sim.vehicle.position = glam::Vec3::new(0.0, start_y, 0.0);
        sim.vehicle.velocity = glam::Vec3::new(0.0, 0.0, -28.0); // ~28 m/s forward (-Z)
        sim.vehicle.controls.throttle = 0.4; // idle cruise
                                             // Seed the camera at a chase-valid pose behind+above the vehicle so the first
                                             // frame shows a sensible view rather than converging from the Camera::default
                                             // position (which is far from the vehicle on startup).
        sim.camera.position = sim.vehicle.position
            + glam::Vec3::new(0.0, sim.camera.chase.height, sim.camera.chase.distance);
        sim.camera
            .follow(sim.vehicle.position, sim.vehicle.orientation, 1.0);

        let snapshots = SnapshotBuffer::new(SimSnapshot::from_state(&sim));
        let input_queue = Arc::new(ArrayQueue::new(INPUT_QUEUE_CAP));
        let debug_hud = DebugHud::new(&device, hud_format, &window);

        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            terrain,
            vehicle_float_offset,
            hud_format,
            sim,
            difficulty: DifficultyDials::default(),
            tick_budget: 0.0,
            last_frame_time: Instant::now(),
            start_time: Instant::now(),
            snapshots,
            input_queue,
            debug_hud,
            ticks_last_frame: 0,
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
        #[cfg(feature = "shader-hot-reload")]
        self.renderer.maybe_reload_shaders(&self.device);

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
            // Min-altitude floor: highest of (terrain at vehicle's XZ, mean water level)
            // plus float_offset plus a small margin to avoid surface z-fighting. Wave
            // crests can still clip floats briefly -- resolved by the Phase 2.5
            // buoyancy model, not this passive clamp.
            let pos = self.sim.vehicle.position;
            let terrain_here = self.terrain.height(pos.x, pos.z);
            let floor = terrain_here.max(WATER_LEVEL);
            let min_altitude = floor + self.vehicle_float_offset + SURFACE_MARGIN;
            self.sim
                .tick(&batch, &self.difficulty, FIXED_DT, min_altitude);
            // Post-tick terrain-aware camera clamp: prevents chase camera from
            // clipping through hills. Kept outside `sim.tick` so sim stays pure.
            let cam_ground = self
                .terrain
                .height(self.sim.camera.position.x, self.sim.camera.position.z);
            self.sim.camera.clamp_above(cam_ground + 2.0);
            self.snapshots.publish(SimSnapshot::from_state(&self.sim));
            self.tick_budget -= FIXED_DT;
            tick_count += 1;
        }
        self.ticks_last_frame = tick_count;
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
        // Main (sRGB) view for the scene pass -- hardware applies linear->sRGB on write.
        let main_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        // Linear view for the HUD pass -- egui's shader output is already in sRGB space,
        // so writing through a non-sRGB view preserves its intended colors.
        let hud_view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("hud-linear-view"),
            format: Some(self.hud_format),
            ..Default::default()
        });

        let (prev, curr) = self.snapshots.read();

        // Drive the scene uniform from the sim's time-of-day: sun direction updates as
        // the day advances. Fog / ambient / sun color stay at render defaults for now;
        // per-biome tuning arrives alongside the weather system. Wall-clock seconds
        // since startup feed water Gerstner phase.
        let scene_params = SceneParams {
            sun_dir: sun_direction(curr.time_of_day),
            time_seconds: self.start_time.elapsed().as_secs_f32(),
            ..SceneParams::default()
        };
        self.renderer.set_scene(&self.queue, &scene_params);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("phase1-encoder"),
            });
        self.renderer.render(
            &self.queue,
            &mut encoder,
            &main_view,
            self.size(),
            &prev,
            &curr,
            alpha,
            CLEAR_COLOR,
        );
        // Debug HUD pass on top of the main scene, rendered via the linear view.
        self.debug_hud.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &hud_view,
            &self.window,
            &curr,
            &self.sim,
            self.ticks_last_frame,
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

/// Attempt to load the MVP aircraft mesh from `VEHICLE_GLB_PATH`; fall back to the
/// placeholder cube on missing file or parse failure. Log the chosen path so startup is
/// self-documenting.
fn load_vehicle_mesh() -> CpuMesh {
    match std::fs::read(VEHICLE_GLB_PATH) {
        Ok(bytes) => match flowplan_assets::load_mesh(&bytes) {
            Ok(mesh) => {
                // The mesh is rendered at `vehicle.position` and rotated around it.
                // That pivot is the physical CG; the mesh's local origin must therefore
                // be at the CG too. We trust the Blender-authored origin rather than
                // auto-shifting -- an automatic shift-to-bottom would push the pivot
                // down to the floats, which makes roll / pitch feel wrong. If the
                // floats visually sit below the terrain, that's a sign the Blender
                // origin needs to move up (or the spawn altitude needs tuning).
                //
                // The bbox dump below lets you verify the axis convention. For a
                // DHC-2-analogue authored at real scale and with Y-up export: expect
                // X extent ~wingspan (±7 m), Y extent tall (~2-3 m with CG near
                // wing-root height), Z extent fuselage length (~±4.5 m). If Y and Z
                // extents look swapped, the Blender `+Y Up` export option is off.
                let (mn, mx) = mesh.bbox();
                tracing::info!(
                    "loaded aircraft mesh: {} ({} verts, {} tris) bbox \
                     x[{:+6.2}, {:+6.2}] y[{:+6.2}, {:+6.2}] z[{:+6.2}, {:+6.2}]",
                    VEHICLE_GLB_PATH,
                    mesh.vertex_count(),
                    mesh.index_count() / 3,
                    mn[0],
                    mx[0],
                    mn[1],
                    mx[1],
                    mn[2],
                    mx[2],
                );
                mesh
            },
            Err(e) => {
                tracing::warn!("failed to parse {VEHICLE_GLB_PATH}: {e}; using placeholder cube");
                placeholder_cube()
            },
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("no {VEHICLE_GLB_PATH} on disk; using placeholder cube");
            placeholder_cube()
        },
        Err(e) => {
            tracing::warn!("read {VEHICLE_GLB_PATH} failed: {e}; using placeholder cube");
            placeholder_cube()
        },
    }
}

fn placeholder_cube() -> CpuMesh {
    CpuMesh::cube(CUBE_SIZE, [1.0, 0.82, 0.28, 1.0])
}

/// Load the MVP aircraft spec sheet. Returns `None` if the file is absent or malformed;
/// the sim keeps running in that case (spec feeds the Phase 2.4 flight model, not the
/// Phase 1 placeholder dynamics).
fn load_aircraft_spec() -> Option<AircraftSpec> {
    match std::fs::read_to_string(VEHICLE_SPEC_PATH) {
        Ok(s) => match AircraftSpec::from_ron(&s) {
            Ok(spec) => {
                tracing::info!(
                    "loaded aircraft spec: {} ({}) MTOW {:.0} kg wheels / {:.0} kg floats, \
                     {} @ {:.0} hp",
                    spec.name,
                    spec.inspiration,
                    spec.max_mass_wheels_kg,
                    spec.max_mass_floats_kg,
                    spec.engine.name,
                    spec.engine.power_hp,
                );
                Some(spec)
            },
            Err(e) => {
                tracing::warn!("parse {VEHICLE_SPEC_PATH}: {e}");
                None
            },
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("no {VEHICLE_SPEC_PATH}; aircraft spec unavailable");
            None
        },
        Err(e) => {
            tracing::warn!("read {VEHICLE_SPEC_PATH}: {e}");
            None
        },
    }
}

/// Map an sRGB surface format to its linear counterpart for the egui HUD view. wgpu 22
/// does not expose `TextureFormat::remove_srgb_suffix` as a stable helper, so this match
/// covers the cases we actually encounter on the target platforms. Unmapped formats are
/// returned unchanged.
const fn linear_variant(format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    match format {
        wgpu::TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8Unorm,
        other => other,
    }
}
