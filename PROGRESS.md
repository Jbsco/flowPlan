<!-- L0: authoritative progress log for flowPlan; read first after CLAUDE.md on cold-start -->
# flowPlan -- Progress Log

Authoritative snapshot of what is built, what conventions are locked, and what comes next. Read this after [CLAUDE.md](CLAUDE.md) and before making any changes. Designed to survive session compaction / agent handoff -- if a detail isn't here, ask or re-derive from the specs and source.

Last updated: **2026-04-19** (Phase 2 in progress; Milestones 2.1, 2.6, 2.3 core shipped; 2.2 Water is NEXT).

## Current status

- **Phase 0 (Skeleton)**: COMPLETE 2026-04-19
- **Phase 1 (Core Loop)**: COMPLETE 2026-04-19 (locked; tuning frozen)
- **Phase 2 (Feel Core)**: IN PROGRESS
    - Milestone 2.1 (cel shading + outline + hot-reload + insta harness): COMPLETE
    - Milestone 2.6 (procedural sky + fog + sim time-of-day): COMPLETE
    - Milestone 2.3 (plane model + glTF loader + spec sheet): CORE SHIPPED; 2.3.4 back-face culling deferred pending chase-camera / cockpit work
    - Milestone 2.2 (water): NEXT
    - Milestone 2.4 (flight model): QUEUED (consumes the spec from 2.3)
    - Milestone 2.5 (pontoon buoyancy): QUEUED (needs 2.2 water height sampler)
- Milestone order taken (deviates from ROADMAP's numeric order): **2.1 -> 2.6 -> 2.3 -> 2.2 -> 2.4 -> 2.5**. Sky landed before plane because it made visual tuning of the Beaver easier; water comes before flight model because buoyancy (2.5) needs it.
- Gate: green. `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo bench --bench frame_budget -- --test`, and the hardware snapshot tests all pass.
- Test count: **94 total** (up from 81 at Phase 1 lock). Breakdown: +7 sim/world (time-of-day, sky math, aircraft_spec), +4 assets (mesh bbox/translate), +2 hardware (scene_minimal, beaver_preview).

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

Controls in the window: **W/S** throttle | **↑** dive / **↓** climb (stick convention) | **←/→** roll | **Q/E** yaw | **1/2/3** view first-person / chase / orbit | **V** cycle view | **R** reset vehicle | **Esc** quit.

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

### Flight model (Phase 1 placeholder; **not** the final Beaver model)
Constants in `crates/sim/src/state.rs`. These are the frozen Phase 1 values:

- `THRUST_ACCEL_MAX = 32.0` m/s² at full throttle
- `LIFT_COEFF = 0.344` (m/s² per m/s of horizontal forward airspeed; level flight at ~28 m/s)
- `LIN_DAMPING = 0.05` (1/s; low-speed stability)
- `QUAD_DRAG = 0.014` (1/m; bounds terminal velocity)
- `ROLL_RATE_MAX = 1.6`, `PITCH_RATE_MAX = 1.0`, `YAW_RATE_MAX = 0.6` rad/s
- `ANG_RESPONSE = 10.0` (1/s; rate-tracking exponential)
- `GROUND_Y = 0.5` m (flat-plane clamp; terrain integration is Phase 2)

Dynamics: thrust along body forward; lift = +Y × *horizontal* forward airspeed × `LIFT_COEFF`; drag = linear + quadratic on `|v|`; gravity 9.81 Y; single-step explicit Euler integration; quaternion body-frame rotation composition.

Starting state: `(0, terrain_max + 25)` with `(0, 0, -28)` initial velocity and `throttle = 0.4` so the cube is in level cruise from frame zero.

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

1. **Scene uniform visibility**: when the fragment stage started reading `camera.view_pos` for the rim-light view direction, the camera-BGL visibility had to move from `VERTEX` to `VERTEX_FRAGMENT` -- wgpu validation error otherwise. Applies to the scene BGL too.
2. **Duplicate `@group` declarations in one WGSL module**: the outline shader's `@group(0)` conflicted with the scene shader's camera `@group(0)`. Fix: separate `shader.wgsl` / `outline.wgsl` / `sky.wgsl` modules.
3. **Full-screen triangle vertex math**: the initial oversized-triangle formula was correct in NDC but an intermediate commit had duplicate unused `x` / `y` let-bindings that confused the reader. Current form in `sky.wgsl` and `outline.wgsl`: `x_ndc = f32(idx & 1) * 4.0 - 1.0; y_ndc = f32(idx >> 1) * 4.0 - 1.0;`.
4. **insta dev-dep lint pollution**: `cargo test` without the `hardware-tests` feature tripped `unused_crate_dependencies` on dev-deps consumed only by the feature-gated test. Fix: `use bytemuck as _;` / `use image as _;` / etc. markers at the top of `visual_snapshot.rs`, plus a feature-conditional `#[cfg(feature = "shader-hot-reload")] use notify as _;` line.
5. **Blender STL orientation**: STL export preserves Blender's Z-up + -Y-forward; glTF wants Y-up + -Z-forward. Re-export with `+Y Up` under `Transform` in the glTF export dialog, and `Object -> Apply -> All Transforms` before export. STL imports create disconnected per-triangle vertex soup; needs `Mesh -> Merge by Distance` + `Recalculate Normals Outside` + `Shade Auto Smooth` in Blender before the glTF export looks right.
6. **Back-face culling vs. interior views**: culling correctly removes back-facing geometry from the exterior, but chase / first-person camera poses that dip inside the vehicle's bounding volume see the mesh as hollow. Keep `cull_mode: None` in the scene pipeline until cockpit interior + chase-camera clearance are proper.

### New invariants / conventions locked in Phase 2

- **Scene uniform lives at `@group(2)`** in the scene pipeline. Sky pipeline uses `@group(0) = camera, @group(1) = scene` (its own layout; reuses the same bind groups via layout compatibility).
- **`CameraUniform.inverse_view_proj`** is now part of the uniform's stable layout. Sky pass depends on it.
- **`SceneUniform.ambient.a`** is reserved for celestial-sphere intensity (Phase 5 hook).
- **Mesh origin shift at load** is the convention: the app's `load_vehicle_mesh` lifts the mesh so bottom-Y sits at 0 regardless of Blender pivot. Applies to any glTF-imported vehicle.
- **Time-of-day is a first-class `SimState` field** and part of `state_hash`. Any future sky / weather / wildlife driver reads it from the snapshot.
- **Snapshot tests are hardware-tier**, feature-gated `hardware-tests`. Baselines live at `crates/render/tests/snapshots/visual_snapshot__*.snap.png`. Byte-exact comparison; tolerant of reference-hardware parity, not cross-driver.

Gate criteria for Phase 2 close (unchanged from ROADMAP): per-phase quality gate + visual snapshot tests + frame-budget bench on `scene_standard` under reference-tier budget + playtest that the core feel "clicks."

## File map (quick reference)

```
crates/
  core/src/{time,handle,rng}.rs            # Newtypes, generational handle, seeded RNG
  assets/src/
    mesh.rs                                 # Vertex, CpuMesh, cube / terrain builders + bbox_min_y / translate
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
    terrain.rs                              # HeightSampler, seeded Terrain
    sky.rs                                  # sun_direction(time_of_day) (Phase 2.6)
  render/src/
    renderer.rs                             # Renderer + SceneParams + ShadingParams + scene/outline/sky pipelines
    shader.wgsl                             # Scene pipeline: cel diffuse + rim + fog (Phase 2.1 + 2.6)
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
