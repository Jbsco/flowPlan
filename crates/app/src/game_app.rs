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
use flowplan_world::{local_time_of_day, sun_direction, HeightSampler, Terrain};
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

/// Compile-time absolute path to the aircraft catalog directory. Resolved
/// from the app crate's `CARGO_MANIFEST_DIR` (at `crates/app`) → up two
/// levels → `data/aircraft`. Used as fallback when the CWD-relative path
/// doesn't resolve (e.g. when the release binary is invoked from outside
/// the project root).
const AIRCRAFT_DIR_FALLBACK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/aircraft");

/// Resolve the aircraft catalog directory, trying CWD-relative first
/// (matches cargo-run-from-root + workspace dev workflow), then falling
/// back to the compile-time absolute path (release binary run from any
/// CWD). Either path may still fail to exist; caller handles that as
/// "empty catalog → cube fallback".
fn aircraft_dir_path() -> std::path::PathBuf {
    let cwd = std::path::Path::new(AIRCRAFT_DIR);
    if cwd.is_dir() {
        cwd.to_path_buf()
    } else {
        std::path::PathBuf::from(AIRCRAFT_DIR_FALLBACK)
    }
}

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
    /// Streaming terrain mesh (Phase 3.6.3c.5b) — recenters on the active
    /// camera as it moves, replacing the fixed-origin mesh used in earlier
    /// phases. None for scenario playback (uses fixed `Terrain::flat*`).
    streaming_terrain: Option<crate::streaming::StreamingTerrain>,
    /// Streaming water mesh — same pattern, follows active camera.
    streaming_water: Option<crate::streaming::StreamingWater>,
    /// Streaming Voronoi-cell active set (Phase 3.6.3c.5b Layer 2).
    /// Refreshed per frame from the active camera position. Consumers
    /// (feature instances, future LOD, hub markers, weather overlays,
    /// audio sources) read `active()` / `entered()` / `exited()` slices.
    /// None for scenario playback.
    streaming_cells: Option<crate::streaming::StreamingCellSet>,
    /// Sub-biome cell catalog — baked once, queried per frame by
    /// `streaming_cells.refresh`. None for scenario playback.
    sub_biome_catalog: Option<std::sync::Arc<flowplan_world::SubBiomeCellCatalog>>,
    /// Eager-baked per-cell feature assignments + instance lists
    /// (Phase 3.6.3c.5c.1). Built once at app init; the active-cell-set
    /// upload loop reads `instances_at_cell(cell_id)` for each entered
    /// cell. None for scenario playback + during the 33 s bake window
    /// when the optional is taken by a background builder (5c.2+).
    baked_assignments: Option<flowplan_world::baked_assignments::BakedAssignments>,
    /// Hydrology graph — retained after bake. `Arc` so the background
    /// ambient worker holds a read-only clone. Kept on main thread
    /// too for future water-surface queries (audio, feature streaming,
    /// debug HUD). `None` outside `FLOWPLAN_FEATURE_INSTANCES` mode.
    #[allow(dead_code)] // retained for future main-thread water queries
    hydrology: Option<std::sync::Arc<flowplan_world::HydrologyGraph>>,
    /// Background ambient-generation worker (Phase 3.6.3c.5c.2.c.12d).
    /// Spawned alongside `baked_assignments` + `hydrology`. Main thread
    /// enqueues `AmbientJob` on cell-enter with priority =
    /// distance-to-camera; worker drains nearest-first and sends back
    /// `AmbientResult`. `None` in scenario mode or without
    /// `FLOWPLAN_FEATURE_INSTANCES`.
    ambient_worker: Option<crate::ambient_worker::AmbientWorker>,
    /// Per-cell lazy ambient cache (Phase 3.6.3c.5c.2.c.11 polish —
    /// streaming-hitch fix 2026-04-24). Key = active-set `CellId`,
    /// value = the ambient `FeatureInstance` list generated at
    /// cell-enter. Populated from `entered()`, evicted from `exited()`,
    /// looked up during the active-set upload walk. Prevents the
    /// every-delta regeneration of ~300 cells × ~300 candidates per
    /// camera cross of a cell boundary.
    ambient_cache: std::collections::HashMap<
        flowplan_world::SubBiomeCellId,
        Vec<flowplan_world::feature_instances::FeatureInstance>,
    >,
    /// Most-recent terrain mesh-rebuild CPU time (milliseconds). Includes
    /// `CpuMesh::terrain_patch_at` — the parallel height-sampling pass.
    /// Persists between rebuilds so the debug HUD can display stale-but-
    /// useful values. 3.6.3c.5b instrumentation; read by 3.6.3c.5a HUD.
    last_terrain_rebuild_cpu_ms: f32,
    /// Most-recent terrain mesh-rebuild GPU upload time (milliseconds).
    /// Includes VBO + IBO buffer creation + data transfer.
    last_terrain_rebuild_gpu_ms: f32,
    /// Water rebuild timings — same pattern.
    last_water_rebuild_cpu_ms: f32,
    last_water_rebuild_gpu_ms: f32,
    /// Debug-fly camera (Phase 3.6.3c.5a). Always allocated; only drives
    /// rendering when `key_state.debug_active` is true. See
    /// `crate::debug_camera::DebugCamera`.
    debug_camera: crate::debug_camera::DebugCamera,
    /// Mouse-delta accumulator (pixels). Cleared each frame after
    /// applying to `debug_camera`.
    debug_mouse_delta: (f32, f32),
    /// True while the OS cursor is captured + hidden for debug mouse-look.
    /// Entering debug mode captures; ESC releases; re-clicking in the
    /// window can re-capture (future polish).
    mouse_captured: bool,
    /// EMA of frame times (seconds) for HUD readout. Alpha = 0.1 →
    /// stable on short timescales while responsive to sustained changes.
    frametime_ema_s: f32,
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
                // ESC quits the app in normal mode; in debug-camera mode
                // it releases mouse capture (handled via KeyState flags
                // + tick_and_render). We check debug_active BEFORE
                // dispatching so the exit path only fires when debug is
                // inactive.
                if is_escape_pressed(&key_event) && !rt.key_state.debug_active {
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

    /// Raw device events — used to capture mouse-motion deltas (dx, dy)
    /// for the debug-fly camera.
    ///
    /// Regular `WindowEvent::CursorMoved` gives absolute positions (jumps
    /// when cursor leaves window), but `DeviceEvent::MouseMotion` gives
    /// raw relative motion even when the cursor is locked + hidden
    /// (FPS-game pattern).
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        let Some(rt) = self.runtime.as_mut() else {
            return;
        };
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            // Only accumulate when debug mode is active AND the cursor
            // is captured — otherwise the mouse is for regular GUI use.
            // Mouse delta is f64 (subpixel precision from the OS); cast
            // to f32 for our math. Per-frame values are tiny (<100 px),
            // so truncation is harmless.
            #[allow(clippy::cast_possible_truncation)]
            if rt.key_state.debug_active && rt.mouse_captured {
                rt.debug_mouse_delta.0 += delta.0 as f32;
                rt.debug_mouse_delta.1 += delta.1 as f32;
            }
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
        // Depth of the lowest mesh point below the origin (CG). Used below to lift the
        // spawn altitude so the floats clear terrain. Zero when the mesh sits at or
        // above y=0.
        let vehicle_float_offset = (-vehicle_mesh.bbox_min_y()).max(0.0);

        // Sub-biome cell catalog (Phase 3.6.3c.5b Layer 2). Baked once
        // per session. Consumed both by the streaming cell active-set
        // refresh and by the feature-instance bake below. Initialized
        // BEFORE streaming-terrain/water so spawn-near-instance can
        // reposition the initial mesh center (avoiding a ~1 s async
        // rebuild at frame 0 when teleporting far from origin).
        let sub_biome_catalog = if scenario.is_none() {
            Some(std::sync::Arc::new(
                flowplan_world::SubBiomeCellCatalog::bake(WORLD_SEED),
            ))
        } else {
            None
        };

        // 3.6.3c.5c.1: eager bake of feature assignments + per-cell
        // instance lists. Gated behind `FLOWPLAN_FEATURE_INSTANCES=1`
        // because the full 3.9 M-cell bake is ~40 s (hydrology + hub
        // + assignments). Dev iteration stays fast by default; instance
        // rendering opts in when needed. Serialized preload is a 3.7
        // save-format target.
        let instances_enabled = std::env::var("FLOWPLAN_FEATURE_INSTANCES")
            .map(|v| v == "1")
            .unwrap_or(false);
        let (baked_assignments, hydrology) = match (
            scenario.is_none(),
            instances_enabled,
            sub_biome_catalog.as_ref(),
        ) {
            (true, true, Some(catalog)) => {
                eprintln!(
                    "flowplan: FLOWPLAN_FEATURE_INSTANCES=1 — baking feature assignments (~30-60 s)..."
                );
                let t0 = std::time::Instant::now();
                let hydrology = flowplan_world::HydrologyGraph::bake(WORLD_SEED, &terrain);
                eprintln!(
                    "flowplan: hydrology baked in {:.1} s",
                    t0.elapsed().as_secs_f32()
                );
                let t1 = std::time::Instant::now();
                let hub_catalog =
                    flowplan_world::HubCatalog::bake(WORLD_SEED, &terrain, &hydrology);
                eprintln!(
                    "flowplan: hub catalog baked in {:.1} s",
                    t1.elapsed().as_secs_f32()
                );
                let t2 = std::time::Instant::now();
                let baked = flowplan_world::baked_assignments::BakedAssignments::bake(
                    catalog,
                    &terrain,
                    &hydrology,
                    &hub_catalog,
                );
                eprintln!(
                    "flowplan: baked assignments in {:.1} s — {} total instances across {} cells ({} populated of {} world)",
                    t2.elapsed().as_secs_f32(),
                    baked.total_instance_count(),
                    baked.populated_cell_count(),
                    baked.populated_cell_count(),
                    baked.stats().cell_count,
                );
                // Print ~8 sample populated-cell coordinates nearest the
                // spawn (0, 0) so the player has a target to fly to.
                // Uses the first instance's position (x, y, z); y is
                // terrain-anchored at bake time so it tells you what
                // altitude to fly at. Distance uses toroidal metric
                // because the world wraps at WORLD_EXTENT_M.
                let toroidal_dist_from_origin = |x: f32, z: f32| -> f32 {
                    let dx = x.min(flowplan_core::WORLD_EXTENT_M - x);
                    let dz = z.min(flowplan_core::WORLD_EXTENT_M - z);
                    dx.hypot(dz)
                };
                let mut samples: Vec<(f32, f32, f32, f32)> = baked
                    .populated_cells()
                    .filter_map(|(_, insts)| {
                        insts
                            .first()
                            .map(|i| (i.position.x, i.position.y, i.position.z))
                    })
                    .map(|(x, y, z)| (x, y, z, toroidal_dist_from_origin(x, z)))
                    .collect();
                samples.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal));
                eprintln!(
                    "flowplan: nearest {} populated cells to spawn:",
                    samples.len().min(8)
                );
                for (x, y, z, d) in samples.iter().take(8) {
                    eprintln!(
                        "  ({:>+9.0}, y={:>+6.0}, {:>+9.0})  ~{:>6.1} km from spawn",
                        x,
                        y,
                        z,
                        d / 1000.0
                    );
                }
                (Some(baked), Some(std::sync::Arc::new(hydrology)))
            },
            _ => (None, None),
        };

        // Spawn background ambient worker alongside the bake (5c.2.c.12d).
        // Worker needs hydrology + sub-biome catalog (for the per-cell
        // Voronoi-polygon clip — 5c.2.c.12i). Only spawn when both are
        // present.
        let ambient_worker = match (hydrology.as_ref(), sub_biome_catalog.as_ref()) {
            (Some(hydro), Some(catalog)) => Some(crate::ambient_worker::AmbientWorker::spawn(
                WORLD_SEED,
                terrain.clone(),
                std::sync::Arc::clone(hydro),
                std::sync::Arc::clone(catalog),
            )),
            _ => None,
        };

        // 3.6.3c.5c.1 diagnostic: `FLOWPLAN_SPAWN_NEAR_INSTANCE=1` puts
        // the debug camera 200 m south + 100 m above the populated cell
        // CLOSEST TO ORIGIN (by toroidal distance), pitched slightly
        // down, with debug-fly mode pre-activated. Nearest-to-origin
        // keeps the world-coord magnitudes small so f32 precision in
        // the view-matrix matmul doesn't produce z-fighting / flicker
        // at extreme coords. Only honored when `FLOWPLAN_FEATURE_INSTANCES=1`
        // also yielded a bake.
        let spawn_near_instance = std::env::var("FLOWPLAN_SPAWN_NEAR_INSTANCE")
            .map(|v| v == "1")
            .unwrap_or(false);
        let initial_debug_camera_pose: Option<(glam::Vec3, f32, f32)> = match (
            spawn_near_instance,
            baked_assignments.as_ref(),
        ) {
            (true, Some(baked)) => {
                // Toroidal distance from origin (0, 0) to (x, z) in a
                // 5000 km world with coords in [0, WORLD_EXTENT_M).
                // Minimum of the direct wrap (x) and the "other way"
                // (WORLD_EXTENT - x).
                let toroidal_dist = |x: f32, z: f32| -> f32 {
                    let dx = x.min(flowplan_core::WORLD_EXTENT_M - x);
                    let dz = z.min(flowplan_core::WORLD_EXTENT_M - z);
                    dx.hypot(dz)
                };
                let best = baked
                    .populated_cells()
                    .filter_map(|(cell_id, insts)| insts.first().map(|inst| (cell_id, inst)))
                    .min_by(|a, b| {
                        let da = toroidal_dist(a.1.position.x, a.1.position.z);
                        let db = toroidal_dist(b.1.position.x, b.1.position.z);
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    });
                best.map(|(cell_id, inst)| {
                    let eye = glam::Vec3::new(
                        inst.position.x,
                        inst.position.y + 100.0,
                        inst.position.z + 200.0,
                    );
                    // yaw=0 looks -Z (toward cluster since eye.z > inst.z).
                    // pitch=-0.4 rad ≈ -23° (cluster is ~100 m below + ~200 m
                    // away, angular offset ~26°).
                    let yaw = 0.0;
                    let pitch = -0.4;
                    let d_km = toroidal_dist(inst.position.x, inst.position.z) / 1000.0;
                    eprintln!(
                        "flowplan: SPAWN_NEAR_INSTANCE — cell {cell_id} is closest ({:.1} km from origin); teleporting debug camera to ({:+.0}, {:+.0}, {:+.0}) toward instance at ({:+.0}, {:+.0}, {:+.0})",
                        d_km,
                        eye.x, eye.y, eye.z,
                        inst.position.x, inst.position.y, inst.position.z,
                    );
                    (eye, yaw, pitch)
                })
            },
            _ => None,
        };

        // Streaming terrain + water (Phase 3.6.3c.5b). For free-flight mode (no
        // scenario), we use the streaming meshes so the camera can fly anywhere
        // in the 5000 km world. For scenario mode, we keep the small fixed mesh
        // because scenarios are bounded test cases that don't need streaming.
        //
        // Initial center = debug-camera spawn-near pose if set (avoids
        // a ~1 s async rebuild at frame 0 when teleporting far from
        // origin); otherwise world origin.
        let initial_center = initial_debug_camera_pose.map_or(glam::Vec2::ZERO, |(pos, _, _)| {
            glam::Vec2::new(pos.x, pos.z)
        });
        let (terrain_mesh, streaming_terrain) = if scenario.is_none() {
            let st = crate::streaming::StreamingTerrain::new(
                crate::streaming::TERRAIN_HALF_EXTENT_M,
                crate::streaming::TERRAIN_CELLS,
                initial_center,
                &terrain,
            );
            (st.cpu_mesh().clone(), Some(st))
        } else {
            let m = CpuMesh::terrain_patch(TERRAIN_CELLS, TERRAIN_HALF_EXTENT, |x, z| {
                terrain.height(x, z)
            });
            (m, None)
        };
        let (water_mesh, streaming_water) = if scenario.is_none() {
            let sw = crate::streaming::StreamingWater::new(
                crate::streaming::WATER_HALF_EXTENT_M,
                crate::streaming::WATER_CELLS,
                initial_center,
                WATER_COLOR,
            );
            (sw.cpu_mesh().clone(), Some(sw))
        } else {
            let m = CpuMesh::water_grid(WATER_CELLS, WATER_HALF_EXTENT, WATER_COLOR);
            (m, None)
        };
        let streaming_cells = if scenario.is_none() {
            Some(crate::streaming::StreamingCellSet::new(
                crate::streaming::STREAM_RADIUS_M,
            ))
        } else {
            None
        };

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
        // 3.6.3c.5b: free-flight uses streaming water whose vertices are
        // already in world coordinates. Suppress the per-frame camera-
        // follow model-matrix translation (which would double-translate
        // and make the water plane drift visibly with the camera).
        // Scenarios keep the old behavior (fixed-origin mesh + follow).
        if scenario.is_none() {
            renderer.set_water_follows_camera(false);
        }
        // Spawn-near mode: debug camera is pre-activated with the
        // vehicle hidden (same contract as the interactive backtick
        // toggle inside `tick_and_render`).
        if initial_debug_camera_pose.is_some() {
            renderer.set_vehicle_visible(false);
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
            streaming_terrain,
            streaming_water,
            streaming_cells,
            sub_biome_catalog,
            baked_assignments,
            hydrology,
            ambient_worker,
            ambient_cache: std::collections::HashMap::new(),
            last_terrain_rebuild_cpu_ms: 0.0,
            last_terrain_rebuild_gpu_ms: 0.0,
            last_water_rebuild_cpu_ms: 0.0,
            last_water_rebuild_gpu_ms: 0.0,
            debug_camera: match initial_debug_camera_pose {
                Some((pos, yaw, pitch)) => {
                    let mut cam = crate::debug_camera::DebugCamera::spawn_at(pos);
                    cam.yaw = yaw;
                    cam.pitch = pitch;
                    cam
                },
                None => crate::debug_camera::DebugCamera::default(),
            },
            debug_mouse_delta: (0.0, 0.0),
            mouse_captured: false,
            frametime_ema_s: 1.0 / 60.0,
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
            key_state: {
                let mut ks = KeyState::default();
                // Activate debug mode from frame 0 when spawn-near
                // is engaged so the pre-posed debug camera drives
                // rendering immediately.
                if initial_debug_camera_pose.is_some() {
                    ks.debug_active = true;
                }
                ks
            },
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

    /// Grab / hide the OS cursor for FPS-style mouse-look.
    ///
    /// `true` = entering debug mode / re-capturing after a previous
    /// release. `false` = releasing (on ESC or exit of debug mode).
    ///
    /// Swallows errors because cursor-grab support varies by platform
    /// (X11 can fail transiently; Wayland works differently); we degrade
    /// to "no cursor grab" gracefully rather than crashing.
    fn window_set_cursor_captured(&self, captured: bool) {
        use winit::window::CursorGrabMode;
        if captured {
            // Try Locked first (hides cursor + confines to window); fall
            // back to Confined if Locked isn't supported (some Wayland).
            let locked = self.window.set_cursor_grab(CursorGrabMode::Locked);
            if locked.is_err() {
                drop(self.window.set_cursor_grab(CursorGrabMode::Confined));
            }
            self.window.set_cursor_visible(false);
        } else {
            drop(self.window.set_cursor_grab(CursorGrabMode::None));
            self.window.set_cursor_visible(true);
        }
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

        // Track frame time via EMA for the debug HUD readout (3.6.3c.5a).
        // Alpha 0.1 = roughly 10-frame smoothing at 60 Hz, responsive but
        // not jittery.
        self.frametime_ema_s = 0.9 * self.frametime_ema_s + 0.1 * elapsed;

        // ---- Debug-camera edge-triggered events (3.6.3c.5a) ----
        // Backtick toggle: enter / leave debug mode. On entry, snap the
        // debug camera to the current sim-camera pose so the view is
        // seamless; on exit, we keep the stored pose for the next entry.
        if self.key_state.debug_toggle_pending {
            self.key_state.debug_toggle_pending = false;
            if self.key_state.debug_active {
                // Leaving debug mode.
                self.key_state.debug_active = false;
                self.key_state.clear_debug_inputs();
                self.mouse_captured = false;
                self.window_set_cursor_captured(false);
                self.renderer.set_vehicle_visible(true);
                tracing::info!(target: "debug_camera", "exit debug mode");
            } else {
                // Entering debug mode. Snap to sim-camera pose.
                let (_, curr) = self.snapshots.read();
                self.debug_camera =
                    crate::debug_camera::DebugCamera::spawn_at(curr.camera.position);
                self.key_state.debug_active = true;
                self.key_state.clear_aircraft_inputs();
                self.debug_mouse_delta = (0.0, 0.0);
                self.mouse_captured = true;
                self.window_set_cursor_captured(true);
                self.renderer.set_vehicle_visible(false);
                tracing::info!(
                    target: "debug_camera",
                    pos_x = curr.camera.position.x,
                    pos_y = curr.camera.position.y,
                    pos_z = curr.camera.position.z,
                    "enter debug mode at sim-camera pose"
                );
            }
        }
        if self.key_state.debug_cycle_speed_pending {
            self.key_state.debug_cycle_speed_pending = false;
            self.debug_camera.cycle_speed();
            tracing::info!(
                target: "debug_camera",
                speed_mps = self.debug_camera.current_speed_mps(),
                "cycle speed tier"
            );
        }
        if self.key_state.debug_release_mouse_pending {
            self.key_state.debug_release_mouse_pending = false;
            self.mouse_captured = false;
            self.window_set_cursor_captured(false);
            tracing::info!(target: "debug_camera", "mouse released (ESC)");
        }

        // ---- Debug-camera per-frame motion update ----
        if self.key_state.debug_active {
            // Apply accumulated mouse delta since last frame.
            let (dx, dy) = self.debug_mouse_delta;
            self.debug_mouse_delta = (0.0, 0.0);
            if self.mouse_captured {
                self.debug_camera.apply_mouse_delta(dx, dy);
            }
            // Apply WASD / Shift / Ctrl motion.
            let (fb, rl, ud) = self.key_state.debug_motion();
            if fb != 0.0 || rl != 0.0 || ud != 0.0 {
                self.debug_camera.integrate(fb, rl, ud, elapsed);
            }
        }

        // 3.6.3c.5a: when debug camera is active, the aircraft is
        // "carried" with the camera — physics paused, position snapped
        // to camera each frame. Avoids the "crashed while I was away"
        // problem AND keeps the sim's implicit center-of-focus (which
        // gameplay systems will eventually query — NPCs, weather, audio)
        // co-located with where the player is actually LOOKING. On exit
        // the aircraft resumes from the camera position with zero
        // velocity (will fall per gravity unless pilot throttles up).
        if self.key_state.debug_active {
            // Debug camera stores position in f64 for precision at
            // large world coords; vehicle.position is f32. Narrow at
            // the boundary — static ~0.25 m quantization error at 5 Mm
            // is absorbed into the render (no per-frame flicker).
            self.sim.vehicle.position = self.debug_camera.position.as_vec3();
            self.sim.vehicle.velocity = glam::Vec3::ZERO;
            self.sim.vehicle.angular_velocity = glam::Vec3::ZERO;
            // Also snap sim camera to track the moved vehicle. Without
            // this, sim.camera is stale at its pre-debug location
            // (normally updated by sim ticks which are paused while
            // debug is active) — on debug exit, the snapshot would
            // read back a camera position hundreds of km from the
            // aircraft, triggering a streaming rebuild cascade. Large
            // dt forces exponential smoothing to converge instantly.
            self.sim.camera.follow(
                self.sim.vehicle.position,
                self.sim.vehicle.orientation,
                10.0,
            );
            // Zero out time budget so we don't catch up on a storm of
            // paused ticks when returning to normal mode.
            self.tick_budget = 0.0;
            // Publish a fresh snapshot so the renderer + HUD see the
            // updated aircraft pose.
            let snap = SimSnapshot::from_state(&self.sim);
            self.snapshots.publish(snap);
            self.ticks_last_frame = 0;
        }

        // Advance the sim by fixed-timestep ticks while we have budget.
        // Skipped entirely while debug camera is active (block above
        // handled that case).
        let mut tick_count = 0u32;
        while !self.key_state.debug_active
            && self.tick_budget >= FIXED_DT
            && tick_count < MAX_TICKS_PER_FRAME
        {
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

        // Build ActiveCamera early — it's the single source of truth
        // for both render and streaming. In normal play it wraps the
        // sim camera; in debug mode it wraps the debug-fly camera.
        let active_camera = if self.key_state.debug_active {
            crate::streaming::ActiveCamera {
                position: self.debug_camera.position.as_vec3(),
                orientation: self.debug_camera.orientation(),
                fov_y_radians: self.debug_camera.fov_y_radians,
                debug_active: true,
            }
        } else {
            crate::streaming::ActiveCamera::from_sim(curr.camera)
        };
        let active_cam_pos_xz = active_camera.position_xz();

        // 3.6.3c.5a: if debug camera is active, synthesize prev/curr
        // snapshots with the debug camera pose replacing the sim camera.
        // Renderer interpolates between prev.camera and curr.camera; using
        // the same pose in both means no interpolation drift, which is
        // what we want for a free-fly camera (user motion is frame-exact,
        // not smoothed). Reads orientation + fov_y_radians fields from
        // ActiveCamera (the single-source-of-truth per 3.6.3c.5b design).
        let (prev, curr) = if active_camera.debug_active {
            let debug_snap = flowplan_sim::CameraSnapshot::from_state(&flowplan_sim::Camera {
                position: active_camera.position,
                orientation: active_camera.orientation,
                fov_y_radians: active_camera.fov_y_radians,
                ..flowplan_sim::Camera::default()
            });
            let mut prev_overridden = prev;
            let mut curr_overridden = curr;
            prev_overridden.camera = debug_snap;
            curr_overridden.camera = debug_snap;
            (prev_overridden, curr_overridden)
        } else {
            (prev, curr)
        };

        // Drive the scene uniform from the sim's time-of-day: sun direction updates as
        // the day advances. Fog / ambient / sun color stay at render defaults for now;
        // per-biome tuning arrives alongside the weather system. Wall-clock seconds
        // since startup feed water Gerstner phase.
        //
        // Phase 3.6.0: longitude-varying local time. Different world-x positions
        // see different solar phases — fly east to chase sunset, west to race
        // sunrise. At session start the aircraft is near origin, so the effect
        // only kicks in after traversal.
        let local_tod = local_time_of_day(curr.time_of_day, curr.vehicle.position.x);
        let scene_params = SceneParams {
            sun_dir: sun_direction(local_tod),
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

        // `active_camera` + `active_cam_pos_xz` were built earlier from
        // the snapshot read. In debug mode they follow the debug camera;
        // otherwise the sim camera. Streaming recenter uses them below.
        if let Some(st) = self.streaming_terrain.as_mut() {
            // Main-thread cost is now just channel probe + possible
            // worker spawn — sub-millisecond. The worker measures its
            // own CPU cost + reports via `last_rebuild_cpu_ms`.
            st.maybe_rebuild(active_cam_pos_xz, &self.terrain);
            if st.just_rebuilt() {
                let t_gpu = Instant::now();
                self.renderer
                    .update_terrain_mesh(&self.device, st.cpu_mesh());
                let gpu_ms = t_gpu.elapsed().as_secs_f32() * 1000.0;
                let cpu_ms = st.last_rebuild_cpu_ms();
                self.last_terrain_rebuild_cpu_ms = cpu_ms;
                self.last_terrain_rebuild_gpu_ms = gpu_ms;
                tracing::info!(
                    target: "streaming",
                    cpu_ms = cpu_ms,
                    gpu_ms = gpu_ms,
                    center_x = st.center().x,
                    center_z = st.center().y,
                    "terrain mesh rebuild (async worker)"
                );
            }
        }
        if let Some(sw) = self.streaming_water.as_mut() {
            let t_cpu = Instant::now();
            sw.maybe_rebuild(active_cam_pos_xz);
            let cpu_ms = t_cpu.elapsed().as_secs_f32() * 1000.0;
            if sw.just_rebuilt() {
                let t_gpu = Instant::now();
                self.renderer.update_water_mesh(&self.device, sw.cpu_mesh());
                let gpu_ms = t_gpu.elapsed().as_secs_f32() * 1000.0;
                self.last_water_rebuild_cpu_ms = cpu_ms;
                self.last_water_rebuild_gpu_ms = gpu_ms;
                tracing::info!(
                    target: "streaming",
                    cpu_ms = cpu_ms,
                    gpu_ms = gpu_ms,
                    center_x = sw.center().x,
                    center_z = sw.center().y,
                    "water mesh rebuild"
                );
            }
        }
        // Layer 2 cell active-set refresh (cheap; ~80 cells in radius).
        // Consumers (feature instance streaming 3.6.3c.5c+, hub markers,
        // weather overlays, audio sources) read from `.active()` slice.
        if let (Some(cells), Some(catalog)) = (
            self.streaming_cells.as_mut(),
            self.sub_biome_catalog.as_ref(),
        ) {
            cells.refresh(active_cam_pos_xz, catalog);

            // Drain any worker-completed ambient results every frame
            // so late results (for cells still active) land in the
            // cache even at standstill. Stale results (cell has since
            // exited) are dropped here.
            let mut any_ambient_drained = false;
            if let Some(worker) = self.ambient_worker.as_ref() {
                while let Some(result) = worker.try_recv() {
                    if cells.active().binary_search(&result.cell_id).is_ok() {
                        self.ambient_cache
                            .insert(result.cell_id, result.instances);
                        any_ambient_drained = true;
                    }
                }
            }

            // 3.6.3c.5c.1 + 5c.2.a: feature-instance upload. Rebuild
            // the active-set instance lists (grouped per PrimitiveKind)
            // and upload per kind. Full rebuild is O(active cells ×
            // per-cell instance lookup) = cheap; per-frame incremental
            // ranges are a 5c.2+ optimization.
            if let Some(baked) = self.baked_assignments.as_ref() {
                if !cells.entered().is_empty()
                    || !cells.exited().is_empty()
                    || any_ambient_drained
                {
                    use flowplan_world::feature_instances::PrimitiveKind;

                    // Phase 1 — ambient cache maintenance.
                    //
                    // Evict exited cells. For newly-entered cells,
                    // enqueue an `AmbientJob` on the background worker
                    // (5c.2.c.12d). Results arrive asynchronously and
                    // are drained in Phase 1b below. Cells without a
                    // cache entry yet render feature instances only;
                    // the ambient fills in within a frame or two.
                    for &cell_id in cells.exited() {
                        self.ambient_cache.remove(&cell_id);
                    }
                    if !cells.entered().is_empty() {
                        if let Some(worker) = self.ambient_worker.as_ref() {
                            let libs = flowplan_world::sub_biome::SubBiomeLibraries::shared();
                            // Toroidal-wrap fix (5c.2.c.12m):
                            // `cells_within_radius` returns cells via
                            // wrap-distance, so cells near the player
                            // can have literal seed coords near
                            // WORLD_EXTENT_M (e.g., 4.99M when player
                            // is at -7000). The downstream renderer
                            // works with camera-relative positions —
                            // an instance at literal world (4.99M, 0)
                            // with camera at (-7000, 0) projects far
                            // off-screen. Normalize seed coords to be
                            // within ±EXTENT/2 of camera before
                            // generating, so all candidate world
                            // coords land near the camera.
                            let half_world = flowplan_core::WORLD_EXTENT_M * 0.5;
                            for &cell_id in cells.entered() {
                                let raw = catalog.seeds()[cell_id as usize].position;
                                let mut sx = raw.0;
                                let mut sz = raw.1;
                                let dx_raw = sx - active_cam_pos_xz.x;
                                let dz_raw = sz - active_cam_pos_xz.y;
                                if dx_raw > half_world {
                                    sx -= flowplan_core::WORLD_EXTENT_M;
                                } else if dx_raw < -half_world {
                                    sx += flowplan_core::WORLD_EXTENT_M;
                                }
                                if dz_raw > half_world {
                                    sz -= flowplan_core::WORLD_EXTENT_M;
                                } else if dz_raw < -half_world {
                                    sz += flowplan_core::WORLD_EXTENT_M;
                                }
                                let seed_pos = (sx, sz);
                                let host_biome = flowplan_world::biome::biome_at(
                                    WORLD_SEED,
                                    seed_pos.0,
                                    seed_pos.1,
                                );
                                let ambient_entries = libs.ambient_for_biome(host_biome);
                                if ambient_entries.is_empty() {
                                    continue;
                                }
                                let feat_insts = baked.instances_at_cell(cell_id).to_vec();
                                let assignment = baked.feature_at_cell(cell_id);
                                let feature = libs
                                    .find(
                                        assignment.source.feature_source_biome(),
                                        &assignment.feature_id,
                                    )
                                    .or_else(|| {
                                        libs.oceanic()
                                            .iter()
                                            .find(|f| f.id == assignment.feature_id)
                                    });
                                let ambient_control =
                                    feature.and_then(|f| f.ambient_control);
                                let dx = seed_pos.0 - active_cam_pos_xz.x;
                                let dz = seed_pos.1 - active_cam_pos_xz.y;
                                worker.enqueue(crate::ambient_worker::AmbientJob {
                                    cell_id,
                                    seed_pos,
                                    biome: host_biome,
                                    feat_insts,
                                    ambient_control,
                                    priority_dist_m: dx.hypot(dz),
                                });
                            }
                        }
                    }
                    // Phase 2 — active-set bucket rebuild. Feature
                    // instances are cheap `Vec`-index lookups; ambient
                    // is a `HashMap` hit served from the cache.
                    let mut boxes: Vec<flowplan_render::InstanceGpuData> = Vec::new();
                    let mut cones: Vec<flowplan_render::InstanceGpuData> = Vec::new();
                    let mut cylinders: Vec<flowplan_render::InstanceGpuData> = Vec::new();
                    let mut spheres: Vec<flowplan_render::InstanceGpuData> = Vec::new();
                    let mut half_cylinders: Vec<flowplan_render::InstanceGpuData> = Vec::new();
                    let mut flat_quads: Vec<flowplan_render::InstanceGpuData> = Vec::new();
                    let mut populated_in_active = 0_usize;
                    let mut ambient_uploaded = 0_usize;
                    for &cell_id in cells.active() {
                        let insts = baked.instances_at_cell(cell_id);
                        if !insts.is_empty() {
                            populated_in_active += 1;
                        }
                        for inst in insts {
                            let gpu = flowplan_render::InstanceGpuData::from_fields(
                                inst.position.to_array(),
                                inst.rotation_y_rad,
                                inst.scale.to_array(),
                                inst.color,
                            );
                            match inst.primitive {
                                PrimitiveKind::Box => boxes.push(gpu),
                                PrimitiveKind::Cone => cones.push(gpu),
                                PrimitiveKind::Cylinder => cylinders.push(gpu),
                                PrimitiveKind::Sphere => spheres.push(gpu),
                                PrimitiveKind::HalfCylinder => half_cylinders.push(gpu),
                                PrimitiveKind::FlatQuad => flat_quads.push(gpu),
                            }
                        }
                        if let Some(ambient) = self.ambient_cache.get(&cell_id) {
                            ambient_uploaded += ambient.len();
                            for inst in ambient {
                                let gpu = flowplan_render::InstanceGpuData::from_fields(
                                    inst.position.to_array(),
                                    inst.rotation_y_rad,
                                    inst.scale.to_array(),
                                    inst.color,
                                );
                                match inst.primitive {
                                    PrimitiveKind::Box => boxes.push(gpu),
                                    PrimitiveKind::Cone => cones.push(gpu),
                                    PrimitiveKind::Cylinder => cylinders.push(gpu),
                                    PrimitiveKind::Sphere => spheres.push(gpu),
                                    PrimitiveKind::HalfCylinder => half_cylinders.push(gpu),
                                    PrimitiveKind::FlatQuad => flat_quads.push(gpu),
                                }
                            }
                        }
                    }
                    let total_uploaded = boxes.len()
                        + cones.len()
                        + cylinders.len()
                        + spheres.len()
                        + half_cylinders.len()
                        + flat_quads.len();
                    tracing::info!(
                        target: "streaming",
                        active = cells.active().len(),
                        entered = cells.entered().len(),
                        exited = cells.exited().len(),
                        populated_in_active = populated_in_active,
                        instances_uploaded = total_uploaded,
                        ambient_uploaded = ambient_uploaded,
                        boxes = boxes.len(),
                        cones = cones.len(),
                        cylinders = cylinders.len(),
                        spheres = spheres.len(),
                        half_cylinders = half_cylinders.len(),
                        flat_quads = flat_quads.len(),
                        cam_x = active_cam_pos_xz.x,
                        cam_z = active_cam_pos_xz.y,
                        "instance-buffer rebuild"
                    );
                    self.renderer.upload_instances_for(
                        &self.device,
                        &self.queue,
                        PrimitiveKind::Box,
                        &boxes,
                    );
                    self.renderer.upload_instances_for(
                        &self.device,
                        &self.queue,
                        PrimitiveKind::Cone,
                        &cones,
                    );
                    self.renderer.upload_instances_for(
                        &self.device,
                        &self.queue,
                        PrimitiveKind::Cylinder,
                        &cylinders,
                    );
                    self.renderer.upload_instances_for(
                        &self.device,
                        &self.queue,
                        PrimitiveKind::Sphere,
                        &spheres,
                    );
                    self.renderer.upload_instances_for(
                        &self.device,
                        &self.queue,
                        PrimitiveKind::HalfCylinder,
                        &half_cylinders,
                    );
                    self.renderer.upload_instances_for(
                        &self.device,
                        &self.queue,
                        PrimitiveKind::FlatQuad,
                        &flat_quads,
                    );
                }
            }
        }

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
        // In debug-camera mode we substitute the aircraft-HUD panels with
        // the debug-camera overlay per 3.6.3c.5a.
        let debug_overlay = if self.key_state.debug_active {
            Some(crate::debug_hud::DebugCameraOverlay {
                position: self.debug_camera.position.as_vec3(),
                heading_deg: self.debug_camera.heading_deg(),
                pitch_deg: self.debug_camera.pitch * 180.0 / core::f32::consts::PI,
                speed_mps: self.debug_camera.current_speed_mps(),
                active_cell_count: self
                    .streaming_cells
                    .as_ref()
                    .map_or(0, crate::streaming::StreamingCellSet::active_count),
                instance_count: self.renderer.instance_count(),
                last_terrain_cpu_ms: self.last_terrain_rebuild_cpu_ms,
                last_terrain_gpu_ms: self.last_terrain_rebuild_gpu_ms,
                rebuild_in_flight: self
                    .streaming_terrain
                    .as_ref()
                    .is_some_and(crate::streaming::StreamingTerrain::rebuild_in_flight),
                terrain_mesh_center: self
                    .streaming_terrain
                    .as_ref()
                    .map_or(glam::Vec2::ZERO, crate::streaming::StreamingTerrain::center),
                _phantom: core::marker::PhantomData,
            })
        } else {
            None
        };
        self.debug_hud.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &hud_view,
            &self.window,
            &telemetry,
            self.ticks_last_frame,
            self.sim.aircraft.as_ref(),
            debug_overlay.as_ref(),
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
    let dir = aircraft_dir_path();
    let (catalog, errors) = flowplan_sim::AircraftCatalog::scan(&dir);
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
    let dir = aircraft_dir_path();
    let (catalog, errors) = flowplan_sim::AircraftCatalog::scan(&dir);
    for err in &errors {
        eprintln!("catalog scan warning: {err}");
    }
    if catalog.is_empty() {
        println!(
            "(catalog empty — no parseable RON files in {})",
            dir.display()
        );
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
    // Try CWD-relative first, then fall back to compile-time workspace-
    // root path. Catalog RON files store `mesh_path` as workspace-root-
    // relative (`data/aircraft/foo.glb`) so this matches the
    // `aircraft_dir_path` resolution pattern.
    let resolved: std::path::PathBuf = {
        let cwd = std::path::Path::new(path);
        if cwd.is_file() {
            cwd.to_path_buf()
        } else {
            std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../")).join(path)
        }
    };
    match std::fs::read(&resolved) {
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
