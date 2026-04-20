<!-- L0: authoritative progress log for flowPlan; read first after CLAUDE.md on cold-start -->
# flowPlan -- Progress Log

Authoritative snapshot of what is built, what conventions are locked, and what comes next. Read this after [CLAUDE.md](CLAUDE.md) and before making any changes. Designed to survive session compaction / agent handoff -- if a detail isn't here, ask or re-derive from the specs and source.

Last updated: **2026-04-20** (Phase 2 in progress; 2.1 / 2.6 / 2.2 core + 2.3 core shipped; Milestone 2.4 flight model substantially complete -- 2.4.1/2.4.2/2.4.2b/2.4.2c/2.4.3/2.4.4a/2.4.4b/2.4.4c/2.4.4d/2.4.4e/2.4.5 shipped, 2.4.6 secondary effects / 2.4.7 density altitude / 2.4.8 feel tune + 2.5 contact physics remaining).

## Current status

- **Phase 0 (Skeleton)**: COMPLETE 2026-04-19
- **Phase 1 (Core Loop)**: COMPLETE 2026-04-19 (locked; tuning frozen at Phase 1 close, superseded inside Phase 2 as each milestone lands)
- **Phase 2 (Feel Core)**: IN PROGRESS
    - Milestone 2.1 (cel shading + outline + hot-reload + insta harness): COMPLETE
    - Milestone 2.6 (procedural sky + fog + sim time-of-day): COMPLETE
    - Milestone 2.3 (plane model + glTF loader + spec sheet): CORE SHIPPED; 2.3.4 back-face culling deferred pending chase-camera / cockpit work
    - Milestone 2.2 (water): CORE SHIPPED (flat quad, Gerstner waves, CPU height sampler, terrain amplitude tune, pre-2.5 ground clamp); 2.2.4 shoreline foam and 2.2.5b distant-water LOD pending
    - Milestone 2.4 (flight model): SUBSTANTIALLY COMPLETE. Shipped: 2.4.1 skeleton, 2.4.2 airfoil forces, 2.4.2b lift perpendicular to velocity, 2.4.2c telemetry + orientation widget, 2.4.3 engine power curve + fuel, 2.4.4a keyboard slew ramp, 2.4.4b moment-based angular dynamics, 2.4.4c moment HUD, 2.4.4d real stall physics + α-based stall alert, 2.4.4e attitude indicator widget, 2.4.5 4-position flaps. Still pending: 2.4.6 adverse yaw / P-factor / torque / ground effect, 2.4.7 density altitude, 2.4.8 feel-tuning pass vs POH.
    - Milestone 2.5 (ground + water contact physics): QUEUED (needs 2.2 water height sampler for buoyancy; replaces the pre-2.5 passive altitude clamp with per-part normal force + hydrodynamic drag + wheel friction)
- Milestone order taken (deviates from ROADMAP's numeric order): **2.1 -> 2.6 -> 2.3 -> 2.2 -> 2.4 -> 2.5**. Sky landed before plane because it made visual tuning of the Beaver easier; water came before flight model because buoyancy (2.5) needs it. Within 2.4: 2.4.4 jumped ahead of 2.4.3 because the moment model fundamentally changes handling feel and was a prerequisite for sensible tuning against any thrust model.
- Gate: green. `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo bench --bench frame_budget -- --test`, and the hardware snapshot tests all pass.
- Test count: **161 total** (up from 140 at 2.4.4c, 117 at 2.4.2c, 102 mid-Phase-2, 81 at Phase 1 lock). Three hardware snapshots committed: `scene_minimal`, `beaver_preview`, `scene_extended` (1024x640; uses the real `Terrain::from_seed` so it reflects in-game terrain faithfully). Reference photos `ref1.png`, `ref2.png`, `ref3.png` in `crates/render/tests/snapshots/` as aesthetic targets (coastal Alaska / fjord floatplane photography).

## Quick commands

```bash
# Full gate (non-negotiable before marking any phase complete)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo bench --bench frame_budget -- --test

# Run the app (release build recommended; dev build is slower but valid)
cargo run -p flowplan-app --release

# Brief smoke-run (3 s; useful for CI-style sanity)
timeout 3 env RUST_LOG=info cargo run -p flowplan-app --release
```

Controls in the window: **W/S** throttle | **↑** dive / **↓** climb (stick convention) | **←/→** roll | **Q/E** yaw | **F/C** flaps extend / retract | **1/2/3** view first-person / chase / orbit | **V** cycle view | **R** reset vehicle | **Esc** quit.

## Tech stack (locked versions)

Pinned in workspace `Cargo.toml`. Do not bump without amendment to [DESIGN.md § Tech Stack](DESIGN.md#tech-stack).

| Crate | Version | Role |
| --- | --- | --- |
| rustc | 1.93 stable | Compiler (pinned via `rust-toolchain.toml`) |
| wgpu | 22 | GPU (Vulkan backend on test hardware: AMD RX 7900 GRE / RADV) |
| winit | 0.30 | Windowing + `ApplicationHandler` pattern |
| egui / egui-wgpu / egui-winit | 0.29 | Debug HUD only (per DESIGN.md non-goals) |
| glam | 0.29 | Math (with `bytemuck` + `serde` features) |
| bytemuck | 1.19 | POD casts for GPU upload |
| rand | 0.8 | RNG frontend |
| rand_pcg | 0.3 | Pcg64Mcg seeded deterministic generator |
| parking_lot | 0.12 | Short-hold mutex for snapshot buffer |
| rtrb | 0.3 | Lock-free SPSC ring (not yet wired; audio crate stub) |
| crossbeam-queue | 0.3 | Bounded input event queue |
| serde / ron / toml | 1 / 0.8 / 0.8 | Data files + workspace-rules parsing |
| tracing / tracing-subscriber | 0.1 / 0.3 | Structured logging |
| anyhow / thiserror | 1 / 1 | Errors (anyhow app-layer only) |
| pollster | 0.4 | Block-on for wgpu adapter/device init |
| proptest (dev) | 1 | Property-driven determinism sweeps |
| criterion (dev) | 0.5 | Frame-budget benches |

## Phase 0 deliverables (COMPLETE)

- Rust workspace with 10 crates (`core`, `assets`, `sim`, `progression`, `world`, `render`, `audio`, `app`, `tests/workspace-rules`, `benches`) wired per DAG from [DESIGN.md § Dependency Rules](DESIGN.md#dependency-rules).
- `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `.gitignore`, `.rustfmt.toml` configured.
- Window opens, clears to sky-blue, exits cleanly.
- 9 dependency-DAG enforcement tests in `tests/workspace-rules/tests/dependency_dag.rs`.
- Criterion smoke bench at `benches/frame_budget.rs`.
- 30 tests passing at the time of Phase 0 completion.

## Phase 1 deliverables (COMPLETE)

### Simulation
- Fixed 120 Hz tick at `crates/sim/src/state.rs`. `SimState::tick(inputs, difficulty, dt)` mutates state deterministically.
- Per-seed `Rng` (Pcg64Mcg via `crates/core/src/rng.rs`), stream-separable via `Rng::stream(seed, id)`.
- `state_hash()` in `crates/sim/src/hash.rs` (FNV-1a) for deterministic comparison.
- Proptest-driven determinism sweeps (`prop_same_seed_same_trace_same_hash`, `prop_different_seed_different_hash`), 32 cases each.
- `TickCount` (u64) and `Seconds` (f64) newtypes to keep sim time and wall time distinct.

### Flight model (Phase 1 placeholder; SUPERSEDED by Phase 2.4)

> **Historical section.** The constants and dynamics documented here were the frozen Phase 1 values. All of them have been replaced by Phase 2.4 work:
> - Translational forces: see "Milestone 2.4 state" below (2.4.2 airfoil lift/drag, 2.4.2b perpendicular-to-velocity lift).
> - Rotational dynamics: see 2.4.4a (keyboard slew ramp) + 2.4.4b (moment-based angular accel) + 2.4.4c (moment HUD). The `PITCH_RATE_MAX` / `YAW_RATE_MAX` / `ROLL_RATE_MAX` / `ANG_RESPONSE` constants are gone.
> - Ground clamp: `GROUND_Y = 0.5` replaced by per-tick `min_altitude` from `max(terrain, water_level) + float_offset + margin`.
>
> The Phase 1 numbers are kept here for historical reference so a future reader can understand how far the model has moved.

Phase 1 constants (former, in `crates/sim/src/state.rs`):

- `THRUST_ACCEL_MAX = 32.0` m/s² at full throttle (now in `crates/sim/src/flight.rs` as `3.5`)
- `LIFT_COEFF = 0.344` (m/s² per m/s of horizontal forward airspeed; superseded by full airfoil `Cl(α)` table)
- `LIN_DAMPING = 0.05` (superseded by drag-polar `Cd`)
- `QUAD_DRAG = 0.014` (superseded by drag-polar `Cd`)
- `ROLL_RATE_MAX = 1.6`, `PITCH_RATE_MAX = 1.0`, `YAW_RATE_MAX = 0.6` rad/s (superseded by moment model)
- `ANG_RESPONSE = 10.0` (superseded by moment/inertia Euler integration)
- `GROUND_Y = 0.5` m (superseded by `min_altitude` parameter)

Phase 1 dynamics: thrust along body forward; lift = +Y × *horizontal* forward airspeed × `LIFT_COEFF`; drag = linear + quadratic on `|v|`; gravity 9.81 Y; single-step explicit Euler integration; quaternion body-frame rotation composition; commanded-rate angular tracking.

Starting state (former): `(0, terrain_max + 25)` with `(0, 0, -28)` initial velocity and `throttle = 0.4`. Current: `(0, terrain_max + 25)` with `(0, 0, -55)` and `throttle = 0.55` (see 2.4.2b tuning).

### Rotation sign conventions (documented in `crates/sim/src/state.rs`)
Right-handed, Y-up, forward = −Z. Body-frame angular velocity components:
- `pitch` positive = nose up → rotate **+X** (positive scaled axis)
- `yaw` positive = nose right → rotate **−Y** (inverted; positive rotation around +Y is yaw-LEFT)
- `roll` positive = roll right → rotate **−Z** (inverted; positive rotation around +Z is roll-LEFT)

Input-map convention (stick style): arrow UP = stick forward = nose DOWN (dive); arrow DOWN = stick back = nose UP (climb). The held-state field names in `KeyState` document *intent* (`pitch_up`, `pitch_down`), and the arrow → field mapping is deliberately inverted from key symbols. Phase 4 adds user remapping.

### Camera
- `Camera` in `crates/sim/src/camera.rs` with `ViewKind::{FirstPerson, ChaseThirdPerson, ExteriorOrbit}`.
- Body-frame chase (bank-tracking) with exponential smoothing; orbit uses polar offset; first-person pins to a seat offset from the vehicle.
- Ground clamp via `Camera::clamp_above(min_y)`, called from `game_app` post-tick with terrain-sampled ground height + 2 m margin. Kept outside `sim.tick` so determinism tests stay terrain-agnostic.
- **`look_at_quat` uses `right = forward × up`** (NOT `up × forward` -- that is the bug that flipped the camera upside-down). Regression-tested in `camera::tests::chase_up_stays_above` and `orbit_up_stays_above`.

### World
- `Terrain::from_seed(u64)` in `crates/world/src/terrain.rs` builds a 4-octave sum-of-sines heightmap.
- Octave params: `(0.03, 4.5)`, `(0.07, 2.0)`, `(0.18, 0.9)`, `(0.42, 0.35)`. Max amplitude ~7.75 m. Frequencies chosen for 15-200 m wavelength spread on a 4 km patch.
- `HeightSampler` trait in same file; `Terrain` implements it.

### Assets
- `CpuMesh` + `Vertex` (POD, `#[repr(C)]`, bytemuck-derived) in `crates/assets/src/mesh.rs`.
- Builders: `cube(size, color)` and `terrain_patch(cells, half_extent, sample_height_fn)`.

### Render
- Flat-shaded forward pipeline in `crates/render/src/renderer.rs` + WGSL at `shader.wgsl`.
- Depth buffer `Depth32Float`. Back-face culling **disabled** in Phase 1 (`cull_mode: None`) because hand-authored cube/terrain winding isn't trusted. Re-enable in Phase 2 when glTF import provides known-good winding.
- Camera + per-draw model uniforms; two draw calls per frame (terrain + cube).
- Snapshot interpolation: lerp position, slerp orientation, alpha = tick_budget / FIXED_DT.

### App
- `GameApp` / `Runtime` in `crates/app/src/game_app.rs`: winit 0.30 `ApplicationHandler`, fixed-timestep accumulator capped at 250 ms / 8 ticks per frame, `ArrayQueue<InputEvent>` input plumbing, double-buffered `SnapshotBuffer` (`parking_lot::Mutex`), keyboard via `input_map::KeyState`.
- Terrain stored at app level; sim doesn't depend on terrain in Phase 1.
- `DebugHud` in `crates/app/src/debug_hud.rs` renders an egui overlay each frame with tick/frame/FPS/view/pos/vel/speed/altitude/attitude/throttle/controls + cheat sheet.

### Workspace lint allowlist (root `Cargo.toml`)
Added during Phase 0-1 to keep pedantic+nursery useful without fighting it on stylistic noise:

- `module_name_repetitions`, `missing_errors_doc`, `missing_panics_doc`, `similar_names` — standard pedantic noise
- `multiple_crate_versions` — upstream transitive noise; audit manually
- `missing_const_for_fn` — nursery, too noisy in exploratory game code
- `suboptimal_flops` — `mul_add` isn't always readable
- `cast_lossless` — `as` is idiomatic in math code
- `inline_always` — allowed for tight hot-path hints
- `must_use_candidate` — too aggressive; `#[must_use]` added by judgment

Per-crate `#![allow(...)]` uses (documented at site):
- `crates/assets/src/lib.rs`: `cast_possible_truncation`, `cast_precision_loss` for mesh-gen index/UV casts
- `crates/render/src/lib.rs`: `cast_*` + `too_many_lines` + `too_many_arguments` for wgpu descriptor chains
- `crates/app/src/main.rs`: `unreachable_pub` because binary crates don't expose APIs
- `crates/app/src/game_app.rs::Runtime::init`: `too_many_lines` (linear wgpu init)
- `crates/app/src/debug_hud.rs::DebugHud::render`: `too_many_arguments` (HUD bag is legitimately wide)
- `crates/app/src/input_map.rs::KeyState`: `struct_excessive_bools` (8 one-per-axis-direction flags)

### Bugs fixed during Phase 1 (do not re-introduce)

1. **Yaw sign**: positive `controls.yaw` intended as "nose right" was rotating nose left. Fix: negate the Y component in the body-frame angular velocity expression. Sign conventions now commented in `tick_dynamics`.
2. **Camera upside-down**: `look_at_quat` was computing `right = up × forward`, which flipped the camera's local +Y vector. Result: world +Y (altitude) rendered as screen −Y. Fix: `right = forward × up`. Regression tests fire on orientation-level inversion.
3. **Infinite climb**: lift used `velocity.dot(forward).max(0.0)` which stayed strong when diving (velocity aligned with downward forward). Fix: project velocity and forward onto the world XZ plane and scale lift by that horizontal airspeed.
4. **Unbounded forward speed**: linear damping alone produced a ~640 m/s terminal velocity with 40% throttle. Fix: added quadratic drag bounding terminal speed to ~28 m/s at 40% throttle.
5. **Non-body-frame rotation composition**: integrated angular velocity had to account for the orientation so body-frame rotation compounds correctly. The pattern used (`dq = from_scaled_axis(orient * angvel * dt); orient = dq * orient`) produces the same result as the textbook `orient = orient * from_scaled_axis(angvel * dt)` by conjugation identity.
6. **egui-wgpu sRGB double-encoding**: egui's shader output is already in sRGB space; writing through an sRGB surface view (e.g. `Rgba8UnormSrgb`) caused re-encoding and slight color shift. Fix: declare the linear counterpart in `SurfaceConfiguration::view_formats`, create a second `TextureView` with the linear format, and pass that view (plus the linear format) to `egui-wgpu`. Main scene pass continues to use the default sRGB view so the hardware's linear→sRGB conversion stays in place for it. Helper: `linear_variant` in `crates/app/src/game_app.rs`.

### Known remaining issues (non-blocking)

- None tracked. All Phase 1 warnings have been addressed.

## Phase 2 (IN PROGRESS)

Milestone order taken: **2.1 -> 2.6 -> 2.3 -> 2.2 -> 2.4 -> 2.5**. See [ROADMAP.md § Phase 2](ROADMAP.md#phase-2-feel-core) for the original ordering.

### Tech-stack additions in Phase 2

Pinned via `Cargo.toml`. Still within the approved list in [DESIGN.md § Tech Stack](DESIGN.md#tech-stack).

| Crate | Version | Role |
| --- | --- | --- |
| gltf | 1.4 | glTF 2.0 import in `flowplan-assets`. |
| thiserror | 1 | Error enums on the asset loader. |
| notify | 6 (optional) | Dev-only shader hot-reload; feature `shader-hot-reload` on `flowplan-render`. |
| insta | 1.34 (dev) | Hardware visual snapshot harness. |
| image | 0.25 (dev) | PNG encoding for snapshots. |
| pollster | 0.4 (dev) | Headless wgpu adapter init for snapshot tests. |

Feature flags introduced:
- `flowplan-render/shader-hot-reload` -- pulls in `notify` + reads `shader.wgsl` / `outline.wgsl` / `sky.wgsl` from disk at runtime; dev only. `flowplan-app/shader-hot-reload` forwards.
- `flowplan-render/hardware-tests` -- gates the hardware snapshot test in `crates/render/tests/visual_snapshot.rs`.

### Milestone 2.1 deliverables (COMPLETE)

- `ShadingParams { bands, ambient_floor, rim_strength, rim_exponent }` uniform attached per-material to the scene pipeline's `ModelUniform.shading: vec4`. App sets distinct terrain/vehicle presets.
- Screen-space sobel outline pass on depth, `outline.wgsl`, separate pipeline, alpha-blended after scene.
- Rim light in the scene fragment shader, per-material strength + exponent.
- Shader hot-reload via `notify` behind `shader-hot-reload` feature. `Renderer::maybe_reload_shaders` re-reads WGSL from `env!("CARGO_MANIFEST_DIR")/src/*.wgsl` and rebuilds the scene + outline pipelines.
- `cargo-insta` visual snapshot harness in `crates/render/tests/visual_snapshot.rs`, baseline `scene_minimal.snap.png` committed. Hardware-gated.

### Milestone 2.6 deliverables (COMPLETE)

- `SceneParams { sun_dir, sun_color, ambient, fog_color, fog_density }` bound at `@group(2)` for the scene pipeline. Default reproduces the Phase 1 hard-coded light direction.
- Procedural sky in `sky.wgsl`: full-screen triangle pre-scene pass, reconstructs world-space view rays via `inverse_view_proj` (added to `CameraUniform`). Zenith-to-horizon gradient, warm sun-bearing band, sun disc + halo, below-horizon darkening.
- Sun-tinted exponential distance fog in the scene fragment shader. Fog mixes toward `sun_color` along the view-to-sun axis. Default density 0.0015/m.
- `SimState.time_of_day: f32` in `[0, 1)`, default 0.5 (noon), advances `dt / DAY_LENGTH_SECONDS` per tick with `rem_euclid(1)` wrap. Folded into `state_hash` so determinism covers it.
- `world::sun_direction(t)` in new `crates/world/src/sky.rs`. Great-circle with fixed `Z = -0.3` tilt: 0 = midnight (below horizon), 0.25 = dawn east, 0.5 = noon overhead, 0.75 = dusk west.
- App derives `SceneParams.sun_dir` from the snapshot's `time_of_day` each frame.

### Milestone 2.2 deliverables (CORE SHIPPED; depth-compare foam + distant LOD pending; terrain rebuild planned for Phase 3+)

- **Flat water plane (2.2.1):** `CpuMesh::water_grid(cells, half_extent, color)` in `flowplan-assets` builds a tessellated grid at y=0 with uniform blue color. App instantiates 64x64 at 4000 m half-extent. Renderer treats it as a separate DrawModel drawn between terrain and vehicle in the scene pass.
- **Gerstner GPU waves (2.2.2):** new `water.wgsl` with 3 wave components (wavelengths 7-18 m, amplitudes 0.08-0.25 m). Dedicated `water_pipeline` built against the same camera+model+scene layout. Per-vertex normal reconstruction via the GPU Gems 2 Ch. 1 simplified form. Sun-glint specular spike bundled in the fragment shader.
- **CPU water-height sampler (2.2.3):** `flowplan_world::water_height(x, z, t)` in `crates/world/src/water.rs`. Mirrors the GPU wave set (same directions / wavelengths / amplitudes / speeds). Y-only height; trochoidal X/Z displacement omitted because Phase 2.5 buoyancy only needs immersion depth. `max_amplitude()` = 0.45 m exposed for conservative clearance checks. `water_max_amplitude` re-exported from the crate root.
- **Whitecap foam (2.2.4 EX):** value-noise-based foam mask in `water.wgsl` (2D value noise at 20 m + 5 m wavelengths, multiplied into a `smoothstep(0.20, 0.26, y)` crest threshold). Produces discrete foam clusters on wave tops rather than sweeping tints. Pattern rides the waves because it samples the displaced world position. **Depth-compare shoreline foam deferred** -- would require a render pass that samples a prior pass's depth buffer; revisit with Phase 2.5 contact physics or Phase 3 scene_standard bench work.
- **Terrain at mountain scale (2.2.5 + polish iterations):** 5-octave sum-of-sines in `world::Terrain`. Current amplitudes: regional ±100 m (`1 km` wavelength), coarse ±120 m, mid ±40 m, fine ±12 m, detail ±3 m. Max |height| ±275 m. Regional halved from an initial 200 m to let the beach flatten produce visible shore flats rather than clifflike transitions. Color ramp in `terrain_color_for_height` blends sand (below 6 m) / field / forest / rock / snow (above 320 m).
- **Beach flattening:** post-process `raw * smoothstep²(|raw|/120)` in `Terrain::height` compresses terrain within ±120 m of sea level toward zero. Scene_extended test uses the real `Terrain::from_seed` (flowplan-world added as a render dev-dep) so the snapshot reflects in-game terrain.
- **Pre-2.5 terrain/water ground clamp:** `SimState::tick` signature gained a `min_altitude: f32` parameter. App computes `min_altitude = max(terrain.height(x, z), WATER_LEVEL=0.0) + vehicle_float_offset + SURFACE_MARGIN=0.05` every tick. Prevents through-terrain flight and mean-water submersion. Determinism invariant extended: `(seed, input trace, dials, tick count, min_altitude)` -> identical state.
- **Mesh bounding-box helper:** `CpuMesh::bbox() -> ([f32; 3], [f32; 3])` and startup log line `"bbox x[..] y[..] z[..]"` for axis-convention verification on freshly-exported glTFs.
- **Fog tuning:** default `SceneParams.fog_density` dropped from `0.0015/m` to `0.0005/m` so distant peaks on the ±275 m terrain read through atmospheric haze without disappearing.

### Milestone 2.4 state (2.4.1 through 2.4.5 shipped; 2.4.6 secondary + 2.4.7 density altitude + 2.4.8 feel tune remaining)

- **2.4.1 (shipped):** `sim::flight::compute_forces(state, spec, dt) -> FlightOutputs` extracted from the old `tick_dynamics`. Pure function; no behavior change from Phase 1 placeholder. `SimState.aircraft: Option<AircraftSpec>` wired from app startup; `SimState.last_flight` caches the last forces for the debug HUD.
- **2.4.2 (shipped):** real airfoil physics for *translational* forces. Lift `L = 0.5 ρ V² S Cl(α)` with a NACA 64A416-shaped `CL_TABLE` (`Cl_max = 1.65` at α=16°, whole-wing adjusted above the ~1.4 2D section value to approximate fuselage + vortex contributions). Drag polar `Cd = Cd0 + Cl²/(π·AR·e)` with a post-stall quadratic spike. Angle of attack α and sideslip β computed from body-frame velocity. Mass from `AircraftSpec.max_mass_wheels_kg` converts force to acceleration. No-spec fallback path retained for pure-sim tests.
- **2.4.2b (shipped, in test):** lift direction corrected from body +Y to perpendicular-to-velocity (in the plane containing velocity and body-up). The body-+Y placeholder produced a spurious `L·sin(α)` backward component at high α -- decelerating the plane every time the pilot pulled up and cascading into stall behavior with no effective control authority. Now lift is orthogonal to the relative wind by construction; backward force on the airframe comes only from proper Cd. **In-test feedback:** coordinated roll turns now feel realistic (lift vector correctly tilts into the turn). Full throttle still climbs; full pitch still behaves sensibly. Remaining feel issues traced to the rotational model below, not the force model. Closes `TODO(2.4.2b)` pending more flight hours.
- **Tuning shipped with 2.4.2b:** `THRUST_ACCEL_MAX` 2.0 → 3.5 m/s² (middle of the R-985 takeoff / cruise thrust band; keeps climb margin without overshooting Vne -- proper power curve lands in 2.4.3). Spawn velocity 50 → 55 m/s so the plane starts with ~2.4× Vs margin -- enough to sustain a 60° bank (which demands 2× Cl) without pushing α past stall.
- **2.4.4a (shipped) Keyboard input slew-rate ramp.** Pitch / roll / yaw axes slew toward held-key targets at 1.5 full-deflection/s (full travel in ~0.67 s, symmetric on release). Throttle keeps its existing per-tick delta model. Analog input sources (gamepad, HOTAS) bypass the ramp when added in Phase 4. Eliminates the "mashed full rudder in one frame" pathology that made the fiat rate model's stall-on-yaw behavior worse. Tests cover slew convergence, release-to-neutral, direction reversal, throttle preservation.
- **2.4.4b (shipped) Moment-based rotational dynamics.** The fiat stick-to-body-rate model is gone. Per-axis aerodynamic moments now drive angular acceleration: `M = q·S·L·(Cm_δ·δ + Cm_stab·α_or_β + Cm_damp·ω·L/(2V))`. `FlightOutputs` replaces `commanded_angular_vel` + `ang_response_rate` with `angular_accel` + per-axis `MomentBreakdown { control, stability, damping }`. `state::tick_dynamics` Euler-integrates angular velocity from the acceleration. Moments of inertia (I_xx=8000, I_yy=12000, I_zz=5000) and all moment coefficients (Cm_α=-0.25, Cm_δe=0.05, Cm_q=-25, Cl_δa=0.08, Cl_p=-0.4, Cn_β=0.08, Cn_δr=0.05, Cn_r=-0.15) are hardcoded Beaver placeholders; migration to `AircraftSpec` is a 2.4.x follow-up. Recovered behaviors: control authority scales with airspeed (no more "identical yaw at any speed"), weathervane restores the nose toward velocity under sideslip, α-stability pitches back toward trim, damping prevents runaway rates. Six new physics tests cover control-axis moments, damping, weathervaning, α-stability, and q-scaling.
- **2.4.4c (shipped) Telemetry + debug HUD for moments.** `Telemetry` replaces `commanded_angular_vel` / `ang_response_rate` with `angular_accel` + three `MomentBreakdown` fields. Dynamics HUD's "Angular" section now shows actual rate + angular accel; new "Moments" section shows total M/L/N with control / stability / damping columns per axis, so "why isn't the plane responding?" is diagnosable at a glance (low q, damping dominating, stick not through ramp, etc.).
- **2.4.5 (shipped) Slotted flaps (4 positions).** `FlapPosition` enum (UP / CLIMB / TAKEOFF / LANDING) on `Controls`, keyed to per-position aero deltas in `flap_aero()`: `Δ Cl` raises the lift curve (camber increase), `Δ Cd0` adds parasite drag, `Δ Cm` adds nose-down pitching moment. Landing flap (58°): `Cl_max = 1.65 + 0.37 = 2.02` → stall speed drops `23 m/s → 20.8 m/s` matching Beaver POH `Vso = 21 m/s`; zero-lift drag bumps 89% making the approach descend aggressively. Input events `ExtendFlaps` / `RetractFlaps` are edge-triggered (no auto-repeat) and clamp at endpoints; bound to `F` (extend one notch) and `C` (retract one notch). `ControlsSnapshot` carries flap position as a `u32` discriminant (stays `Pod` / `Zeroable`); Telemetry decodes back to enum for HUD display. Top-left Controls panel shows current notch + deflection angle. Five new tests cover monotonic deltas across positions, Cl / Cd / pitch-moment change with landing flaps, and end-of-travel clamping.
- **2.4.4d (shipped) Real stall physics + α-based stall alert.** Three changes: (1) a nose-down pitching moment activates past α_stall (16°) at slope `CM_STALL_PITCH_SLOPE = -2.0 per rad past stall`, modeling the aft-shift of center of lift -- this is what lets a stable aircraft self-recover from an inadvertent stall. (2) `wing_drop_roll_moment()` samples `Cl(α)` at each wing's quarter-span with a roll-rate-induced α perturbation and returns the non-linear portion of the left/right differential as a rolling moment. Below ~12.8° α (APPROACH threshold) it's near-zero (linear region). Past stall, the stalled wing's Cl drops while the unstalled side keeps lift → asymmetry reinforces any roll rate into wing-drop / spin-entry behavior. (3) HUD stall alert switched from airspeed-heuristic to α-based `StallState` enum (`Normal` / `Approach` / `Stall`), set from the flight model's current α each tick. Accelerated stall now reads correctly: a high-G pull hits `Cl_max` at higher IAS so α saturates at stall even though airspeed is well above Vs. `FlightOutputs` + `Telemetry` gain `stall_state: StallState`; HUD alert shows current α vs the 16° threshold with the same NORMAL / APPROACH / STALL color scheme. Four new tests: `StallState` classification, post-stall pitching moment direction, normal-α wing-drop near-zero, past-stall wing-drop non-zero.
- **2.4.3 (shipped) Engine power curve + prop efficiency + fuel model.** Replaced the constant `THRUST_ACCEL_MAX` with a physical model: shaft power `P(throttle)` ranges 20 kW (idle) to 335 kW (450 hp max) linearly in throttle; prop efficiency piecewise-linear vs airspeed (0.30 static, 0.82 peak around 55-70 m/s, 0.65 at Vne, 0.45 at 120 m/s); thrust `T = η·P/V` with V clamped at `V_MIN_FOR_THRUST=12 m/s` to prevent divergence (static thrust at max power ~9 kN, matches the Beaver's "P(hp)·4.5 lbf" rule of thumb). Fuel tracking: `VehicleState.fuel_gal` decremented each tick by `fuel_flow_gph × dt / 3600`, where `fuel_flow_gph = P_kw × 0.124` (calibrated from 27 gph at cruise). Zero fuel cuts engine power to zero regardless of throttle (flameout). `FlightOutputs` / `Telemetry` expose `engine_power_kw`, `prop_efficiency`, `fuel_flow_gph`, plus derived `fuel_gal` and `endurance_seconds`. HUD dynamics panel gains an "Engine" section with kW/hp, η, gph, remaining gallons, and endurance at current burn. No-spec path still uses the legacy `THRUST_ACCEL_MAX_FALLBACK` for tests without an `AircraftSpec`. Eight new tests: engine power curve (idle/max/no-fuel), prop efficiency peak, thrust finite at V=0, thrust magnitude at cruise, fuel flow scaling with throttle, and full-state fuel-depletion integration test over 60 sim-seconds.
- **2.4.4e (shipped) Attitude indicator / glide-slope widget (bottom-left).** Standard-AI style: yellow aircraft symbol fixed in display, horizon line rotates with bank and translates with pitch, pitch ladder ticks at ±5°/±10°/±20°, green flight-path-vector crosshair offset from the aircraft symbol by `(β, -α) × scale`. FPV shows where the plane is actually going relative to attitude -- above horizon = climbing, below = descending, separation from aircraft symbol = α+β. Readouts in a bottom-left-anchored egui panel: FPA, pitch, α, β, VS (m/s + ft/min), L/D ratio. `Telemetry` gains `flight_path_angle_rad` + `glide_ratio`; renderer generalizes `widget_view_proj` with a `WidgetAnchor` enum + second camera uniform/bind-group, and adds per-widget `set_scissor_rect` so the horizon line doesn't bleed outside the widget at extreme pitch. Same `DebugLinesRenderer` pipeline serves both widgets.
- **Pre-2.4.4 "input aggression" TODO (resolved by 2.4.4a).** Kept here as a reference to what was fixed: keyboard step inputs now pass through a 1.5 / s linear slew before reaching the moment model. Analog input sources bypass it.
- **2.4.2c (shipped) Telemetry plumbing + cel-shaded orientation widget.** Single `Telemetry` struct (`flowplan_render::Telemetry`) derived once per frame from `SimSnapshot` (which now carries the tick-boundary `FlightOutputs` plus `VehicleSnapshot.angular_velocity` / `ControlsSnapshot` for full display coverage). SI units internally. Debug HUD refactored to read from Telemetry only -- no more `&SimState` on the HUD boundary. First visual consumer is a screen-space **orientation widget** anchored to the bottom-right (180×180 px, 20 px margin): unit-length direction vectors rotated with the aircraft -- body axes (+X red, +Y green, -Z blue), velocity (white), thrust (yellow), lift (cyan), drag (orange), gravity (grey), total accel (magenta). All vectors unit-length so directions compare at a glance; magnitudes read from the top-right dynamics HUD. Always on (no toggle). Arrows are **cel-shaded**: each segment emits a ~4 px dark outline quad behind a ~2.5 px colored fill quad, giving a crisp cel-style border slightly thinner than the main scene's sobel outline so the widget doesn't compete with scene edges. A fixed "isometric" display rotation (≈25° pitch + 32° yaw) is composed with the aircraft orientation so all three body axes always have a visible XY projection regardless of attitude (nose-up, inverted, pure bank). Rendered via a new `TriangleList` pipeline (`debug_lines.rs` + `debug_lines.wgsl`) driven by a dedicated `widget_camera_buf` holding a screen-space ortho view-proj recomputed per frame from viewport size; no depth attachment so the widget always overlays. Determinism preserved: widget state is app-side (`gizmo_batch: DebugLineBatch`), never in `SimState`. Architectural boundary documented in [DESIGN.md § Telemetry derivation (render side)](DESIGN.md#telemetry-derivation-render-side); consumer progression and field list in [FEATURES.md § Telemetry & Instrumentation Data Flow](FEATURES.md#telemetry--instrumentation-data-flow). Infrastructure dividend: the same quad-widening pipeline + widget ortho pattern is reused in Phase 4 for HUD ladders (pitch ladder, bank indicator ticks), artificial-horizon tick marks, and other screen-space instrument lines. A depth-aware variant for world-space wireframes (collision debug, flight-path vector) needs a clip-space vertex shader for widening (deferred until the first world-space consumer arrives; 2D widget widening in the XY plane is fine for now because the widget uses an ortho projection down +Z).
- **Pitch-feel observation (resolved by 2.4.4b).** Static pitch stability -- disturbed α produces restoring nose-down moment via `Cm_α` -- now emerges naturally from the moment model. Recorded here for historical context since it drove the 2.4.4 priority.
- **Dynamics HUD (top-right):** shows airspeeds (total + horizontal-fwd, m/s + kt), α / β / Cl / Cd, airspeed-based stall alert (NORMAL / APPROACH / STALL vs `AircraftSpec.speeds.vs_clean_mps`), per-force breakdown (thrust / lift / drag / gravity), total acceleration, actual angular velocity + angular acceleration, per-axis moment breakdown (control / stability / damping), active aircraft spec.
- **Current moment coefficients (Beaver placeholders, tuned by flight-test feel).** `Cm_α=-0.25, Cm_δe=0.24, Cm_q=-25` (pitch); `Cl_δa=0.075, Cl_p=-0.5` (roll); `Cn_β=0.08, Cn_δr=0.04, Cn_r=-0.6` (yaw). Moments of inertia: `I_xx=8000, I_yy=12000, I_zz=5000 kg·m²`. Reference lengths: `c=1.52 m, b=14.63 m`. All in `crates/sim/src/flight.rs`. Final reconciliation against POH targets (cruise, Vne, climb rate, best-glide, stall speeds) is the 2.4.8 feel-tuning pass once 2.4.6 + 2.4.7 land.
- **Engine model (2.4.3).** `ENGINE_POWER_MAX_KW=335` (450 hp), `ENGINE_POWER_IDLE_KW=20`, linear in throttle. Prop efficiency piecewise-linear from 0.30 static → 0.82 at 55-70 m/s → 0.45 at 120 m/s. `V_MIN_FOR_THRUST=12` clamps the denominator at low airspeed so static thrust stays finite (~9 kN, matches Beaver "P(hp)×4.5 lbf" rule of thumb). Fuel BSFC constant `0.124 gph/kW`; fuel state lives in `VehicleState.fuel_gal`, decrements each tick, flameout at zero.
- **Flap model (2.4.5).** 4 positions with `Δ Cl`, `Δ Cd0`, `Δ Cm` deltas per detent. LANDING (58°) gives `Cl_max ≈ 2.02` → Vso ≈ 20.8 m/s (matches POH `Vso=21`); Cd0 +89%; noticeable but pilot-manageable nose-down pitching moment (`Δ Cm = -0.030`). Keys: `F` extend, `C` retract.
- **Flight-model TODOs** live at the code site in `crates/sim/src/flight.rs` as `TODO(phase-N):` markers per CODING_RULES. Open: engine power curve + fuel burn + prop model (2.4.3), real stall / wing-drop / accelerated stall / α-based stall alert (2.4.4d), flaps (2.4.5), ground effect / P-factor / adverse yaw / torque (2.4.6), density altitude (2.4.7), spin / gyroscopic precession / slipstream / slip-skid / icing (post-2.4).
- **Known HUD heuristic false-positives (pending 2.4.4d):**
  - The stall alert compares horizontal forward airspeed against Vs_clean. It false-positives in a steep dive (vertical velocity doesn't count toward the ratio even when real α is well clear of stall) and misses accelerated / G-loaded stalls in tight turns (stall at higher IAS). `TODO(2.4.4d)` in `debug_hud.rs::build_ui` and `flight.rs` documents both. Proper α-based stall detection lands with 2.4.4d wing-drop / accelerated-stall modeling.
  - The stall alert doesn't know about flaps; at Vso ~ 21 m/s with flaps down a real aircraft isn't stalling, but the HUD would show APPROACH against Vs-clean = 23.25 m/s.

### Milestone 2.3 deliverables (CORE SHIPPED; 2.3.4 culling deferred)

- `flowplan_assets::load_mesh(bytes) -> Result<CpuMesh, GltfLoadError>` handles `.glb` and embedded-buffer `.gltf`. First-mesh / first-primitive / triangle topology. POSITION required; NORMAL / TEXCOORD_0 / COLOR_0 / indices default when absent.
- `CpuMesh::bbox_min_y` and `CpuMesh::translate` for per-load origin adjustment.
- App tries `data/aircraft/northlander.glb` at startup; falls back to the placeholder cube on missing-file or parse-error. Auto-shifts the mesh so bottom-Y sits at 0.
- `AircraftSpec { name, empty_mass_kg, max_mass_wheels_kg, max_mass_floats_kg, wing_area_m2, aspect_ratio, airfoil_root, airfoil_tip, engine, fuel_capacity_gal, speeds }` in `flowplan-sim`, serde+ron parsed. `data/aircraft/northlander.ron` committed, authored from AIRCRAFT.md § MVP Reference with SI-normalized speeds.
- Renderer's `vehicle_cube_size` removed; meshes now render at authored scale. The cube placeholder is built at 2 m directly in the app.
- `beaver_preview.snap.png` committed as a visual regression baseline for the loaded Beaver.
- **Deferred:** 2.3.4 back-face culling. Attempted; snapshot views remained clean but interior chase-camera / first-person views made the Beaver's exterior shell disappear (see-through plane effect). Culling stays off until a cockpit model + chase-camera clearance rework lands.

### Phase 2 tech-stack additions in Cargo.toml

| Crate | Version | Role |
| --- | --- | --- |
| gltf | 1.4 | glTF 2.0 import in `flowplan-assets`. |
| thiserror | 1 | Loader error enum. |
| notify | 6 (optional) | Dev-only filesystem watch for hot-reload. |

### Bugs fixed during Phase 2 (do not re-introduce)

1. **Scene uniform visibility**: when the fragment stage started reading `camera.view_pos` for the rim-light view direction, the camera-BGL visibility had to move from `VERTEX` to `VERTEX_FRAGMENT` -- wgpu validation error otherwise. Applies to the scene BGL too (water vertex shader reads `scene.time.x` for Gerstner phase, so scene-BGL visibility must also be `VERTEX_FRAGMENT`).
2. **Duplicate `@group` declarations in one WGSL module**: the outline shader's `@group(0)` conflicted with the scene shader's camera `@group(0)`. Fix: separate `shader.wgsl` / `outline.wgsl` / `sky.wgsl` / `water.wgsl` modules.
3. **Full-screen triangle vertex math**: the initial oversized-triangle formula was correct in NDC but an intermediate commit had duplicate unused `x` / `y` let-bindings that confused the reader. Current form in `sky.wgsl` and `outline.wgsl`: `x_ndc = f32(idx & 1) * 4.0 - 1.0; y_ndc = f32(idx >> 1) * 4.0 - 1.0;`.
4. **insta dev-dep lint pollution**: `cargo test` without the `hardware-tests` feature tripped `unused_crate_dependencies` on dev-deps consumed only by the feature-gated test. Fix: `use bytemuck as _;` / `use image as _;` / etc. markers at the top of `visual_snapshot.rs`, plus a feature-conditional `#[cfg(feature = "shader-hot-reload")] use notify as _;` line.
5. **Blender STL orientation**: STL export preserves Blender's Z-up + -Y-forward; glTF wants Y-up + -Z-forward. Re-export with `+Y Up` under `Transform` in the glTF export dialog, and `Object -> Apply -> All Transforms` before export. STL imports create disconnected per-triangle vertex soup; needs `Mesh -> Merge by Distance` + `Recalculate Normals Outside` + `Shade Auto Smooth` in Blender before the glTF export looks right.
6. **Back-face culling vs. interior views**: culling correctly removes back-facing geometry from the exterior, but chase / first-person camera poses that dip inside the vehicle's bounding volume see the mesh as hollow. Keep `cull_mode: None` in the scene pipeline until cockpit interior + chase-camera clearance are proper.
7. **Mesh origin = rotation pivot = CG**: auto-shifting the mesh so bottom-Y sits at origin put the rotation pivot at the floats, giving unrealistic roll / pitch feel. Fix: trust the Blender-authored origin (placed at CG) and compute `vehicle_float_offset = (-mesh.bbox_min_y()).max(0.0)` to lift the spawn altitude and the `min_altitude` clamp by the float depth.
8. **Tick `min_altitude` parameter**: before Milestone 2.2 the sim had a hard-coded `GROUND_Y = 0.5` in `tick_dynamics`. The vehicle flew through terrain hills and submerged through water. Replaced with a per-tick `min_altitude` parameter the app supplies from `max(terrain.height, water_level) + float_offset + margin`. Determinism invariant now covers `(seed, inputs, dials, tick count, min_altitude)`.

### Known remaining issues in Phase 2 (deferred to 2.5 or later)

- **Ground / water contact is a passive altitude clamp only -- no rotation constraint, no friction, no drag, no buoyancy, no per-part collision.** The `min_altitude` clamp keeps the CG above `max(terrain, water_level) + float_offset` but doesn't resolve anything else. Known consequences still present: (1) the plane can roll / pitch while "on" the ground, including attitudes that would physically drive the floats or wingtips into the ground; (2) contact with ground or water imparts no drag or deceleration -- landing "on" a runway at cruise speed doesn't slow the plane; (3) the plane can fly horizontally into a hillside without collision response (altitude clamp only affects Y); (4) wave crests clip the floats because the clamp uses mean water level only (wave-height clamp would undermine determinism). All four land together in Milestone 2.5 ground + water contact physics -- see [ROADMAP.md § Milestone 2.5](ROADMAP.md#milestone-25-ground--water-contact-physics) for the full breakdown into 2.5a (water / floats), 2.5b (ground / wheels / tailwheel), and 2.5c (invariants + tests).
- **Back-face culling off.** See Phase 2 bug #6.
- **Terrain is placeholder-at-scale.** Sum-of-sines produces mountain-scale geometry but cannot independently tune amplitude and gradient -- genuinely gentle beach flats beneath steep mountain backdrops (per ref1/2/3 aesthetic targets) are not achievable without a different representation. Related gaps: no elevation lakes, no rivers, no drainage basins, no shore classification (sand / gravel / rock / cliff / delta), no erosion-shaped coastlines. Tracked under [FEATURES.md § Terrain & Static Landscape](FEATURES.md#terrain--static-landscape) for a proper hydrology-aware terrain rebuild in Phase 3+ alongside the world-scope decision.
- **Flight model covers 2.4.1 / 2.4.2 / 2.4.2b / 2.4.4 (rotation); 2.4.3 / 2.4.4d / 2.4.5+ still open.** *Translational* forces are real airfoil physics (`0.5 ρ V² S Cl(α)`, polar `Cd0 + Cl²/(π·AR·e)`, lift perpendicular to velocity) -- coordinated banked turns feel plausible in flight testing. *Rotational* dynamics are moment-based -- control authority scales with q, weathervane stability restores toward velocity, α-stability pitches back toward trim, damping prevents runaway rates. Thrust is still a throttle-proportional constant (no RPM / MAP / altitude; engine power curve is 2.4.3). No flaps (2.4.5). Post-stall `Cl` drop and drag spike exist in the table but no nose-down pitching moment on stall, no wing-drop, no accelerated-stall speed correction -- 2.4.4d. No ground effect / P-factor / adverse yaw / torque roll (2.4.6). No density altitude (2.4.7). Each gap is marked with a `TODO(phase-N):` at its code site in `crates/sim/src/flight.rs` and closes as its milestone lands.

### New invariants / conventions locked in Phase 2

- **Scene uniform lives at `@group(2)`** in the scene and water pipelines. Sky pipeline uses `@group(0) = camera, @group(1) = scene` (its own layout; reuses the same bind groups via layout compatibility).
- **`CameraUniform.inverse_view_proj`** is now part of the uniform's stable layout. Sky pass depends on it.
- **`SceneUniform.ambient.a`** is reserved for celestial-sphere intensity (Phase 5 hook).
- **`SceneUniform.time.x`** = wall-clock seconds since app start. Drives Gerstner wave phase (and future animated effects). Non-deterministic by design -- render is presentation-only.
- **Mesh origin at CG.** Blender authors the origin at the physical CG; the app does not auto-shift. `vehicle_float_offset = (-bbox_min_y).max(0.0)` lifts spawn altitude and `min_altitude` clamp so floats clear surfaces.
- **`SimState::tick` takes `min_altitude`**: the per-tick floor the CG cannot cross. Determinism invariant is `(seed, input trace, dials, tick count, min_altitude)` -> identical state.
- **Water wave parameters are a two-sided contract.** `crates/render/src/water.wgsl` (`wave_dir_and_params` / `wave_speed_and_steep`) and `crates/world/src/water.rs` (`WAVES` constant) must stay in sync when tuning. Only the Y height matters CPU-side; X/Z trochoidal displacement is GPU-only.
- **Time-of-day is a first-class `SimState` field** and part of `state_hash`. Any future sky / weather / wildlife driver reads it from the snapshot.
- **Snapshot tests are hardware-tier**, feature-gated `hardware-tests`. Baselines live at `crates/render/tests/snapshots/visual_snapshot__*.snap.png`. Byte-exact comparison; tolerant of reference-hardware parity, not cross-driver.

Gate criteria for Phase 2 close (unchanged from ROADMAP): per-phase quality gate + visual snapshot tests + frame-budget bench on `scene_standard` under reference-tier budget + playtest that the core feel "clicks."

## File map (quick reference)

```
crates/
  core/src/{time,handle,rng}.rs            # Newtypes, generational handle, seeded RNG
  assets/src/
    mesh.rs                                 # Vertex, CpuMesh, cube / terrain / water_grid builders + bbox + translate
    gltf_load.rs                            # glTF 2.0 loader (Phase 2.3)
  sim/src/
    state.rs                                # SimState, tick(), dynamics constants, time_of_day
    camera.rs                               # Camera + views + look_at_quat (WATCH THE SIGN)
    vehicle.rs                              # VehicleState
    controls.rs                             # Controls (pitch/yaw/roll/throttle)
    input.rs                                # InputEvent enum, InputEvents batch
    difficulty.rs                           # DifficultyDials placeholder
    snapshot.rs                             # SimSnapshot (includes time_of_day) / VehicleSnapshot / CameraSnapshot
    hash.rs                                 # state_hash + proptest determinism (covers time_of_day)
    aircraft_spec.rs                        # AircraftSpec / EngineSpec / SpeedsSpec, serde+ron (Phase 2.3)
  world/src/
    terrain.rs                              # HeightSampler, seeded Terrain (amplitudes tuned Phase 2.2.5)
    sky.rs                                  # sun_direction(time_of_day) (Phase 2.6)
    water.rs                                # water_height(x, z, t) Gerstner sampler (Phase 2.2.3)
  render/src/
    renderer.rs                             # Renderer + SceneParams + ShadingParams + scene/water/outline/sky pipelines
    shader.wgsl                             # Scene pipeline: cel diffuse + rim + fog (Phase 2.1 + 2.6)
    water.wgsl                              # Water pipeline: Gerstner vertex + cel fragment + sunglint (Phase 2.2.2)
    outline.wgsl                            # Sobel outline full-screen pass (Phase 2.1)
    sky.wgsl                                # Procedural sky full-screen pre-pass (Phase 2.6)
    hot_reload.rs                           # Dev-only filesystem watcher (Phase 2.1, feature-gated)
  app/src/
    main.rs                                 # Entry point + tracing init
    game_app.rs                             # Runtime, accumulator, tick loop, glb + ron loaders
    input_map.rs                            # KeyState, arrow/WASD/etc. bindings
    snapshot_buf.rs                         # Double-buffered SimSnapshot
    debug_hud.rs                            # egui overlay (Phase 1 debug only)
tests/workspace-rules/
  src/lib.rs + tests/dependency_dag.rs      # 9 DAG enforcement tests
crates/render/tests/
  visual_snapshot.rs                        # Hardware-gated insta snapshots (scene_minimal, beaver_preview)
  snapshots/                                # Committed PNG baselines
data/aircraft/
  northlander.glb                           # Beaver mesh (Blender-exported, STL origin)
  northlander.ron                           # Beaver spec sheet
benches/
  frame_budget.rs                           # criterion smoke
```

## Verification before any Phase 2 work

Before touching Phase 2 sub-steps, run:

```bash
cargo fmt --all -- --check && \
cargo clippy --workspace --all-targets -- -D warnings && \
cargo test --workspace && \
cargo bench -p flowplan-benches --bench frame_budget -- --test
```

Plus the hardware-gated snapshot tier:

```bash
cargo test -p flowplan-render --features hardware-tests --test visual_snapshot
```

All five must be green. Feature-matrix spot-checks (run periodically; not on every iteration):

```bash
cargo clippy --workspace --all-targets --features flowplan-app/shader-hot-reload -- -D warnings
cargo clippy --workspace --all-targets --features flowplan-render/hardware-tests -- -D warnings
cargo clippy --workspace --all-targets \
    --features flowplan-app/shader-hot-reload,flowplan-render/hardware-tests -- -D warnings
```
