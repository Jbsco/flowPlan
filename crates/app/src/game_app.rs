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
use flowplan_render::{DebugLineBatch, Renderer, SceneParams, ShadingParams, Telemetry};
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

use crate::debug_gizmo;
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

/// Cube size for the placeholder vehicle (meters). Used when the selected variant's
/// glTF mesh is missing or malformed.
const CUBE_SIZE: f32 = 2.0;

/// Workspace-relative directory that holds the aircraft catalog (one `.ron` per
/// aircraft, matching `.glb` next to it). `AircraftCatalog::scan` walks this at
/// startup.
const AIRCRAFT_DIR: &str = "data/aircraft";

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

/// Water tessellation. Each vertex is a Gerstner anchor — wavelet visibility requires
/// vertex spacing ≤ wavelength/Nyquist. With chop wavelengths down to 2.5 m, vertex
/// spacing should be < 1.5 m for clean wavelet rendering. The previous 64×64 over
/// 8 km (= 125 m/vertex) made chop completely under-sampled (visible only as a
/// barely-perceptible noise texture). Current 512×512 over 2 km (= ~4 m/vertex)
/// resolves chop at 5 m wavelengths cleanly, with the 2.5 m fine-chop near Nyquist
/// (visible as a finer surface tremor). Far-field water is fog-faded, so the
/// reduced extent (was 4 km half) doesn't visibly cap the horizon.
///
/// Vertex count: 263 169 (513² grid). Static mesh, uploaded once at startup; only
/// the shader displaces them per-frame — minimal CPU cost.
const WATER_CELLS: u32 = 512;
const WATER_HALF_EXTENT: f32 = 1000.0;

/// Deep cel-water tint. Cooler than the sky so the horizon reads. Alpha is unused by the
/// cel shader.
const WATER_COLOR: [f32; 4] = [0.18, 0.38, 0.58, 1.0];

// Mean water level (was `WATER_LEVEL = 0.0`) is no longer a min-altitude clamp source.
// Phase 2.5a buoyancy + drag handles water contact directly. The water *render* still
// uses y=0 as its mean surface (in `crates/render/src/water.wgsl` and the CPU sampler);
// keeping that sim/render contract independent of the gameplay clamp.

/// Small clearance added to the floor to avoid surface z-fighting between the vehicle
/// and the terrain / water mesh.
const SURFACE_MARGIN: f32 = 0.05;

pub fn run(mut opts: crate::CliOptions) -> Result<()> {
    // --list-aircraft short-circuits window construction.
    if opts.list_aircraft {
        print_catalog_and_exit();
    }

    // Resolve scenario from CLI flag before opening the window so we can fail
    // fast on typos. When --scenario is supplied, the scenario's own aircraft_id
    // + variants drive the catalog selection (unless the user ALSO passed
    // --aircraft / --variant, in which case their choices win — but must be
    // consistent with the scenario or we error).
    let scenario_opt = if let Some(name) = &opts.scenario_name {
        let scenario = flowplan_sim::flight_bench::BEAVER_SCENARIOS
            .iter()
            .find(|s| s.name == name.as_str())
            .ok_or_else(|| {
                let known: Vec<&str> = flowplan_sim::flight_bench::BEAVER_SCENARIOS
                    .iter()
                    .map(|s| s.name)
                    .collect();
                anyhow::anyhow!("unknown scenario '{name}'. Known: {known:?}")
            })?;

        // If --aircraft wasn't set, default to the scenario's aircraft. If it
        // WAS set, it must match — running a Beaver scenario with a Catalina
        // selected is nonsense.
        match &opts.aircraft_id {
            None => opts.aircraft_id = Some(scenario.aircraft_id.to_string()),
            Some(id) if id != scenario.aircraft_id => {
                anyhow::bail!(
                    "scenario {:?} targets aircraft {:?} but --aircraft is {:?}",
                    scenario.name,
                    scenario.aircraft_id,
                    id,
                );
            },
            Some(_) => {},
        }

        // Likewise for --variant: default to the first declared variant if
        // unset, else verify the user's choice is one of the scenario's
        // applicable variants.
        match &opts.variant_id {
            None => {
                opts.variant_id = scenario.variants.first().map(ToString::to_string);
            },
            Some(v) if !scenario.variants.contains(&v.as_str()) => {
                anyhow::bail!(
                    "scenario {:?} applies to variants {:?} but --variant is {:?}",
                    scenario.name,
                    scenario.variants,
                    v,
                );
            },
            Some(_) => {},
        }

        tracing::info!(
            "scenario playback: {} [{}/{}] ({} ticks, {} inputs, {} assertions)",
            scenario.name,
            opts.aircraft_id.as_deref().unwrap_or("?"),
            opts.variant_id.as_deref().unwrap_or("?"),
            scenario.duration_ticks,
            scenario.inputs.len(),
            scenario.assertions.len(),
        );
        Some(*scenario)
    } else {
        None
    };

    let event_loop = EventLoop::new().context("construct winit event loop")?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = GameApp {
        opts,
        scenario: scenario_opt,
        runtime: None,
    };
    event_loop
        .run_app(&mut app)
        .context("run winit event loop")?;
    Ok(())
}

#[derive(Default)]
struct GameApp {
    opts: crate::CliOptions,
    /// Resolved scenario from `opts.scenario_name`, if any.
    scenario: Option<flowplan_sim::flight_bench::FlightScenario>,
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

    /// Reused CPU batch for the orientation widget's line segments. App-side state;
    /// not part of `SimState` so determinism is preserved (the widget is presentation-
    /// only per [`DESIGN.md § Telemetry derivation`](../../../../DESIGN.md#telemetry-derivation-render-side)).
    gizmo_batch: DebugLineBatch,
    /// Reused CPU batch for the attitude-indicator widget (2.4.4e).
    attitude_batch: DebugLineBatch,

    /// 2.4.8: if present, scenario driving scripted inputs + telemetry capture.
    scenario_playback: Option<ScenarioPlayback>,
}

/// Scenario playback state: scripted-input source + telemetry log.
struct ScenarioPlayback {
    scenario: flowplan_sim::flight_bench::FlightScenario,
    /// Tick counter since playback started.
    elapsed_ticks: u32,
    /// Buffered inputs sorted by tick; drained as ticks advance.
    samples: Vec<flowplan_sim::flight_bench::BenchSample>,
    /// Where to write CSV when playback ends. `None` means don't persist.
    log_dir: Option<std::path::PathBuf>,
    /// If true, exit the app when scenario duration elapses.
    exit_on_end: bool,
    /// Have we finalized (wrote CSV)? Set after first frame past duration so we
    /// don't keep writing forever.
    finalized: bool,
    /// Per-controller PID state. Parallels `scenario.controllers` — owned here
    /// so the visual playback feeds the same closed-loop inputs the headless
    /// runner does (required for turn scenarios and any future closed-loop
    /// scenarios to behave identically visual-vs-bench).
    controller_states: Vec<flowplan_sim::ControlLawState>,
}

impl ApplicationHandler for GameApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.runtime.is_some() {
            return;
        }
        match Runtime::init(event_loop, self.scenario, &self.opts) {
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(rt) = &self.runtime {
            // Scenario auto-exit: once the scripted scenario has finalized its CSV
            // and `exit_on_scenario_end` is set, request the event loop to exit so
            // batch-driven runs return to the shell without manual intervention.
            if let Some(pb) = &rt.scenario_playback {
                if pb.finalized && pb.exit_on_end {
                    event_loop.exit();
                    return;
                }
            }
            rt.window.request_redraw();
        }
    }

    /// Called after `event_loop.exit()` is requested but BEFORE winit tears down its
    /// internal event sources. Explicitly dropping the `Runtime` here releases the
    /// `Arc<Window>` / `wgpu::Surface` / `egui_winit::State` (and on Wayland, the
    /// underlying calloop sources they reference) in a deterministic order. Without
    /// this, calloop can log "Received an event for non-existence source" when
    /// events queued before teardown are dispatched to sources our still-alive
    /// references helped remove.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if self.runtime.take().is_some() {
            tracing::info!("exiting: runtime dropped");
        }
    }

    /// Background / sleep transition. Not exercised on desktop X11/Wayland in
    /// practice, but the winit 0.30 API expects both `resumed` and `suspended` for
    /// correctness on platforms that do pause (mobile, some compositors). We drop
    /// the surface here so we don't hold a backbuffer across a suspend; the next
    /// `resumed` rebuilds the runtime from scratch.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if self.runtime.take().is_some() {
            tracing::info!("suspended: runtime dropped");
        }
    }
}

/// Persist scenario telemetry to CSV + log assertion results. Called once per
/// scenario, the first frame after the scenario's duration elapses.
fn finalize_scenario_playback(pb: &mut ScenarioPlayback) {
    pb.finalized = true;
    let results = flowplan_sim::flight_bench::check_assertions(&pb.scenario, &pb.samples);
    let passed = results.iter().filter(|r| r.passed).count();
    tracing::info!(
        "scenario '{}' complete: {}/{} assertions passed",
        pb.scenario.name,
        passed,
        results.len(),
    );
    for r in &results {
        let mark = if r.passed { "PASS" } else { "FAIL" };
        tracing::info!("  [{mark}] {:?}: {}", r.assertion, r.message);
    }
    if let Some(dir) = &pb.log_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!("create log_dir {}: {e}", dir.display());
            return;
        }
        let csv = flowplan_sim::flight_bench::log_to_csv(&pb.samples);
        let path = dir.join(format!("{}.csv", pb.scenario.name));
        if let Err(e) = std::fs::write(&path, csv) {
            tracing::warn!("write {}: {e}", path.display());
        } else {
            tracing::info!("wrote telemetry CSV: {}", path.display());
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
    fn init(
        event_loop: &ActiveEventLoop,
        scenario: Option<flowplan_sim::flight_bench::FlightScenario>,
        opts: &crate::CliOptions,
    ) -> Result<Self> {
        // Window. 1080p baseline gives the debug widgets (orientation BR + attitude BL
        // + top-left / top-right / bottom-left text panels) comfortable room without
        // overlapping the scene. User remap / fullscreen toggle lands in Phase 4's
        // settings UI; for now the size is fixed at startup.
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("flowPlan")
                        .with_inner_size(winit::dpi::LogicalSize::new(1920.0, 1080.0)),
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
        //
        // Scenario-playback mode uses `Terrain::flat()` (height = 0 everywhere) so the
        // visual test matches the headless bench exactly — the bench always passes
        // `min_altitude = 0.0` to `SimState::tick`, and here we do the same. Without
        // this, the visual could diverge from the CSV when the aircraft's horizontal
        // track crosses terrain higher than sea level, breaking the determinism
        // invariant `(seed, inputs, dials, dt, min_altitude trace) → bit-identical`.
        // Resolve aircraft selection from CLI/env/catalog before loading the mesh;
        // mesh path + spec + config all come from the catalog entry. The
        // selected variant's config also drives terrain choice below
        // (flat-land for wheels, flat-water for floats).
        let selected = resolve_aircraft_selection(opts);
        let (vehicle_mesh, aircraft_spec, vehicle_config) = selected.as_ref().map_or_else(
            || {
                (
                    placeholder_cube(),
                    None,
                    flowplan_sim::VehicleConfig::default(),
                )
            },
            |sel| {
                (
                    load_vehicle_mesh(&sel.variant.mesh_path),
                    Some(sel.spec.clone()),
                    sel.variant.config,
                )
            },
        );

        // Scenario mode picks between two flat terrains based on the selected
        // variant's contact physics: `flat()` for floats (terrain at y=-10, water
        // at y=0), `flat_land()` for wheels (terrain at y=0, water suppressed
        // below). Free-flight mode keeps the procedural terrain.
        let terrain = if scenario.is_some() {
            if vehicle_config == flowplan_sim::VehicleConfig::Wheels {
                Terrain::flat_land()
            } else {
                Terrain::flat()
            }
        } else {
            Terrain::from_seed(WORLD_SEED)
        };
        let terrain_mesh = CpuMesh::terrain_patch(TERRAIN_CELLS, TERRAIN_HALF_EXTENT, |x, z| {
            terrain.height(x, z)
        });

        // Depth of the lowest mesh point below the origin (CG). Used below to lift the
        // spawn altitude so the floats clear terrain. Zero when the mesh sits at or
        // above y=0.
        let vehicle_float_offset = (-vehicle_mesh.bbox_min_y()).max(0.0);
        let water_mesh = CpuMesh::water_grid(WATER_CELLS, WATER_HALF_EXTENT, WATER_COLOR);

        let mut renderer = Renderer::new(
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
        // Suppress water rendering for wheels-variant scenario playback so the
        // y=0 flat-land terrain isn't occluded by a y=0 water mesh.
        if scenario.is_some() && vehicle_config == flowplan_sim::VehicleConfig::Wheels {
            renderer.set_water_visible(false);
        }

        // Sim. Scenario-mode init uses the scenario's ScenarioInitial so playback is
        // deterministic and reproducible. Normal-mode init is the existing cruise-
        // above-terrain seed.
        let mut sim = SimState::new(WORLD_SEED);
        let initial_fuel_gal = aircraft_spec.as_ref().map_or(0.0, |s| s.fuel_capacity_gal);
        sim.aircraft = aircraft_spec;
        sim.vehicle = VehicleState::at_origin();
        sim.vehicle.fuel_gal = initial_fuel_gal;
        sim.vehicle.config = vehicle_config;

        if let Some(s) = scenario.as_ref() {
            // Scenario playback: seed from the scenario's initial state. Aircraft
            // spec was already set above from the default load path; scenario
            // references the same spec file so this is consistent.
            sim.vehicle.position = s.initial.position;
            sim.vehicle.velocity = s.initial.velocity;
            sim.vehicle.orientation = s.initial.orientation;
            sim.vehicle.angular_velocity = s.initial.angular_velocity;
            sim.vehicle.controls.throttle = s.initial.throttle;
            sim.vehicle.controls.flaps = s.initial.flaps;
            sim.vehicle.fuel_gal = s.initial.fuel_gal;
        } else {
            // Normal (free-flight) mode: cruise above terrain.
            let start_y = terrain.max_height() + vehicle_float_offset + 25.0;
            sim.vehicle.position = glam::Vec3::new(0.0, start_y, 0.0);
            sim.vehicle.velocity = glam::Vec3::new(0.0, 0.0, -55.0);
            sim.vehicle.controls.throttle = 0.55;
        }
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
            gizmo_batch: DebugLineBatch::new(),
            attitude_batch: DebugLineBatch::new(),
            scenario_playback: scenario.map(|s| ScenarioPlayback {
                scenario: s,
                elapsed_ticks: 0,
                samples: Vec::with_capacity(s.duration_ticks as usize),
                log_dir: opts.log_dir.clone(),
                exit_on_end: opts.exit_on_scenario_end,
                finalized: false,
                controller_states: vec![
                    flowplan_sim::ControlLawState::default();
                    s.controllers.len()
                ],
            }),
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

    #[allow(clippy::too_many_lines)] // Linear frame pipeline -- splitting hurts readability.
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
            // Gather input for this tick. In scenario mode the scripted source
            // takes over -- keyboard is ignored for flight controls (still works
            // for Esc / reset / view). In normal mode, keyboard feeds the queue
            // through the standard ramp / axis pipeline.
            let mut batch = InputEvents::default();
            if let Some(pb) = self.scenario_playback.as_mut() {
                // Inject scripted events whose `tick` matches the current scenario
                // elapsed count. Scenarios use small, sparse input traces so the
                // linear scan is cheap.
                for input in pb.scenario.inputs {
                    if input.tick == pb.elapsed_ticks {
                        batch.push(input.event);
                    }
                }
                // Controllers run every tick — same as `run_scenario_headless`.
                // Without this, closed-loop scenarios (turn_30, turn_45, etc.)
                // diverge from the bench because PID outputs are missing.
                for (ctrl, state) in pb
                    .scenario
                    .controllers
                    .iter()
                    .zip(pb.controller_states.iter_mut())
                {
                    batch.push(ctrl.step(state, &self.sim, FIXED_DT));
                }
                // Discard anything the user might have typed -- keyboard events
                // still pushed via apply_key_event() would leak into the queue.
                while self.input_queue.pop().is_some() {}
            } else {
                self.key_state.push_axis_events(&self.input_queue, FIXED_DT);
                while let Some(ev) = self.input_queue.pop() {
                    batch.push(ev);
                }
            }
            // Min-altitude floor: terrain at vehicle's XZ, plus float_offset, plus a
            // small margin. Over water (terrain below sea level), the clamp drops
            // with the terrain — Phase 2.5a's buoyancy physics handles water contact
            // directly, so the clamp no longer needs to hold the aircraft above mean
            // water level (that was the pre-2.5 passive altitude clamp). 2.5b will
            // replace the terrain portion with proper per-wheel ground contact.
            //
            // Scenario-playback mode: pass `min_altitude = 0.0` exactly, matching
            // `crates/sim/tests/flight_bench.rs`. This guarantees bench CSV and visual
            // CSV are byte-identical for the same scenario (determinism invariant
            // `(seed, inputs, dials, dt, min_altitude trace) → bit-identical`).
            let min_altitude = if self.scenario_playback.is_some() {
                0.0
            } else {
                let pos = self.sim.vehicle.position;
                let terrain_here = self.terrain.height(pos.x, pos.z);
                match self.sim.vehicle.config {
                    // Floats: CG clamp keeps pontoons above terrain. The
                    // `vehicle_float_offset` (mesh-derived pontoon depth)
                    // adds the pontoon-to-CG distance so buoyancy never
                    // samples negative compression through terrain.
                    flowplan_sim::VehicleConfig::Floats => {
                        terrain_here + self.vehicle_float_offset + SURFACE_MARGIN
                    },
                    // Wheels: ground-contact physics handles gear spring +
                    // damping directly. `min_altitude` is passed to
                    // `compute_ground_contact` as `ground_y`, so it must be
                    // the ACTUAL terrain — adding float offset here would
                    // simulate an elevated phantom ground 1.69 m above the
                    // rendered terrain, preventing the gear from reaching
                    // contact on descent. The CG hard-clamp using the same
                    // value is a safety net; gear spring arrests descent
                    // long before CG reaches raw terrain.
                    flowplan_sim::VehicleConfig::Wheels => terrain_here + SURFACE_MARGIN,
                }
            };
            // Sample weather at the aircraft's current position and feed it
            // to the sim. Scenario-playback mode uses still air (weather is
            // a real-world-distraction for deterministic scenario benching);
            // free-flight mode gets the full W1 + W2 + W3 composed state.
            let weather = if self.scenario_playback.is_some() {
                flowplan_sim::WeatherInputs::default()
            } else {
                let pos = self.sim.vehicle.position;
                let wind = flowplan_world::weather::wind_at_with_weather(
                    WORLD_SEED,
                    &self.terrain,
                    pos.x,
                    pos.y,
                    pos.z,
                    self.sim.wave_time,
                );
                let turbulence = flowplan_world::weather::turbulence_at_with_weather(
                    WORLD_SEED,
                    &self.terrain,
                    pos.x,
                    pos.y,
                    pos.z,
                    self.sim.wave_time,
                );
                // Chop scalar from horizontal surface-wind magnitude. Drives
                // physics wave-amplitude multiplier (buoyancy) and visual
                // wave-amplitude via SceneParams below. Closes the 2.5a.7
                // chop hook.
                let wind_mag = wind.x.hypot(wind.z);
                self.sim.chop_intensity = flowplan_world::chop_intensity_from_wind(wind_mag);
                flowplan_sim::WeatherInputs {
                    wind,
                    turbulence_intensity: turbulence,
                }
            };
            self.sim
                .tick_with_weather(&batch, &self.difficulty, FIXED_DT, min_altitude, weather);
            // Post-tick terrain-aware camera clamp: prevents chase camera from
            // clipping through hills. Kept outside `sim.tick` so sim stays pure.
            let cam_ground = self
                .terrain
                .height(self.sim.camera.position.x, self.sim.camera.position.z);
            self.sim.camera.clamp_above(cam_ground + 2.0);

            // Scenario telemetry capture + auto-exit.
            if let Some(pb) = self.scenario_playback.as_mut() {
                if pb.elapsed_ticks < pb.scenario.duration_ticks {
                    let sample = flowplan_sim::flight_bench::BenchSample::capture(
                        u64::from(pb.elapsed_ticks),
                        &self.sim,
                        &self.sim.last_flight,
                    );
                    pb.samples.push(sample);
                }
                pb.elapsed_ticks = pb.elapsed_ticks.saturating_add(1);
                if !pb.finalized && pb.elapsed_ticks >= pb.scenario.duration_ticks {
                    finalize_scenario_playback(pb);
                }
            }

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
            // Mirror sim-side chop on the visual waves so buoyancy response
            // and rendered surface amplitude stay aligned.
            chop_intensity: curr.chop_intensity,
            ..SceneParams::default()
        };
        self.renderer.set_scene(&self.queue, &scene_params);

        // Derive telemetry once per frame from the current snapshot. Both the 3D gizmo
        // and the debug HUD read from this; later phases add the hand-rolled HUD and
        // 3D cockpit instruments as additional consumers of the same struct.
        let telemetry = Telemetry::derive(&curr);

        // Rebuild both debug widgets every frame (always on; debug-first instruments
        // that inform the real HUD later).
        debug_gizmo::build(&mut self.gizmo_batch, &telemetry);
        debug_gizmo::build_attitude(&mut self.attitude_batch, &telemetry);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("phase1-encoder"),
            });
        self.renderer.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &main_view,
            self.size(),
            &prev,
            &curr,
            alpha,
            CLEAR_COLOR,
            Some(&self.gizmo_batch),
            Some(&self.attitude_batch),
        );
        // Debug HUD pass on top of the main scene, rendered via the linear view.
        self.debug_hud.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &hud_view,
            &self.window,
            &telemetry,
            self.ticks_last_frame,
            self.sim.aircraft.as_ref(),
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

/// The catalog entry + variant the app is running this session. Built once at
/// startup from the CLI flags / env var / catalog defaults; held on `Runtime` so
/// the HUD and future in-session menu can read what's loaded.
#[derive(Clone, Debug)]
pub struct SelectedAircraft {
    /// Stable catalog id (`"northlander"`, `"catalina"`, …). Intentionally kept
    /// even though no in-session path reads it yet — future menu UI (Phase 4)
    /// and future save-load (Phase 5+) both reference aircraft by this id, and
    /// keeping the field stable now means the settings UI doesn't force a
    /// struct rev on the app crate when it lands.
    #[allow(dead_code)]
    pub aircraft_id: String,
    /// Selected variant within that catalog entry. Owned so the `Runtime` isn't
    /// pinned to the catalog lifetime.
    pub variant: flowplan_sim::AircraftVariant,
    /// Full spec — fed to `SimState.aircraft`.
    pub spec: AircraftSpec,
}

/// Scan `data/aircraft/`, resolve the CLI + env selection against it, and return
/// the chosen entry. Logs at each fallback so startup traces what's loaded and
/// why.
///
/// Selection precedence (highest wins):
/// 1. `--aircraft <id>` / `--variant <id>` CLI flags
/// 2. `FLOWPLAN_AIRCRAFT=<id>[:<variant>]` env var (applied in `parse_cli`)
/// 3. Catalog's first entry, id-sorted, for the aircraft; entry's
///    `default_variant` for the variant
///
/// Returns `None` when the catalog is empty — caller falls back to the
/// placeholder cube with a warning.
fn resolve_aircraft_selection(opts: &crate::CliOptions) -> Option<SelectedAircraft> {
    let dir = std::path::Path::new(AIRCRAFT_DIR);
    let (catalog, errors) = flowplan_sim::AircraftCatalog::scan(dir);
    for err in &errors {
        tracing::warn!("catalog scan: {err}");
    }
    if catalog.is_empty() {
        tracing::warn!("catalog empty; falling back to placeholder cube");
        return None;
    }

    let entry = opts.aircraft_id.as_deref().map_or_else(
        || catalog.default_entry(),
        |id| {
            catalog.find(id).or_else(|| {
                tracing::warn!("requested aircraft {id:?} not in catalog; falling back to default");
                catalog.default_entry()
            })
        },
    )?;

    let variant = opts.variant_id.as_deref().map_or_else(
        || entry.default_variant(),
        |id| {
            entry.find_variant(id).or_else(|| {
                tracing::warn!(
                    "requested variant {id:?} not in {:?}; falling back to default_variant",
                    entry.id
                );
                entry.default_variant()
            })
        },
    )?;

    tracing::info!(
        "selected aircraft: {} / {} ({} → {})",
        entry.id,
        variant.id,
        variant.display_name,
        variant.mesh_path,
    );

    Some(SelectedAircraft {
        aircraft_id: entry.id.clone(),
        variant: variant.clone(),
        spec: entry.spec.clone(),
    })
}

/// Print the catalog contents to stdout and exit. Used by `--list-aircraft`.
pub fn print_catalog_and_exit() -> ! {
    let dir = std::path::Path::new(AIRCRAFT_DIR);
    let (catalog, errors) = flowplan_sim::AircraftCatalog::scan(dir);
    for err in &errors {
        eprintln!("catalog scan warning: {err}");
    }
    if catalog.is_empty() {
        println!("(catalog empty — no parseable RON files in {AIRCRAFT_DIR})");
    } else {
        println!("Aircraft catalog ({} entries):", catalog.entries().len());
        for entry in catalog.entries() {
            println!("  {} — {}", entry.id, entry.spec.name);
            for v in &entry.spec.variants {
                let marker = if v.id == entry.spec.default_variant {
                    " (default)"
                } else {
                    ""
                };
                println!("    {}: {}{}", v.id, v.display_name, marker);
            }
        }
    }
    std::process::exit(0)
}

/// Load the aircraft mesh at `path`; fall back to the placeholder cube on missing
/// file or parse failure. Log the chosen path so startup is self-documenting.
fn load_vehicle_mesh(path: &str) -> CpuMesh {
    match std::fs::read(path) {
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
                    path,
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
                tracing::warn!("failed to parse {path}: {e}; using placeholder cube");
                placeholder_cube()
            },
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("no {path} on disk; using placeholder cube");
            placeholder_cube()
        },
        Err(e) => {
            tracing::warn!("read {path} failed: {e}; using placeholder cube");
            placeholder_cube()
        },
    }
}

fn placeholder_cube() -> CpuMesh {
    CpuMesh::cube(CUBE_SIZE, [1.0, 0.82, 0.28, 1.0])
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
