# flowPlan Roadmap

This document is the phased execution plan for flowPlan. Each phase delivers a discrete capability; each has a testable gate that must pass before the next phase begins.

The roadmap **must** adhere to the architecture in [DESIGN.md](DESIGN.md), the rules in [CODING_RULES.md](CODING_RULES.md), and the test plan in [TEST_PLAN.md](TEST_PLAN.md). This is not a prototype -- it is the foundation that v1 builds on.

## Core Principle

Follow the migration plan: **skeleton -> feel core -> systems -> polish**. Each phase ends with something that can be flown or heard. Nothing moves to the next phase until the gate passes.

## Contents

- [Prerequisites](#prerequisites)
- [Per-Phase Quality Gate](#per-phase-quality-gate)
- [Phase 0: Skeleton](#phase-0-skeleton)
- [Phase 1: Core Loop](#phase-1-core-loop)
- [Phase 2: Feel Core](#phase-2-feel-core)
- [Phase 3: Systems](#phase-3-systems)
- [Phase 4: Delivery Loop](#phase-4-delivery-loop)
- [Phase 5: Polish & v1](#phase-5-polish--v1)
- [Future Considerations (Post-v1)](#future-considerations-post-v1)
- [Non-Goals (Reminder)](#non-goals-reminder)

---

## Prerequisites

Before implementation begins:

- Rust stable toolchain (pinned in `rust-toolchain.toml`)
- `wgpu`-capable GPU with Vulkan/Metal/DX12 drivers current
- A working `cpal` audio stack (PipeWire/PulseAudio on Linux, CoreAudio on macOS, WASAPI on Windows)
- `cargo-insta` and `cargo-criterion` installed for snapshot and benchmark testing
- Tracy client (`Tracy` profiler) installed on the development machine

Optional but recommended:

- A 144 Hz+ display for visual verification of high-refresh targets
- A gamepad for input testing before HOTAS support (deferred post-v1)

---

## Per-Phase Quality Gate

Every phase ends with the same gate. **A phase is not complete until the gate passes; do not proceed to the next phase until it does.**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo bench --bench frame_budget -- --test   # runs the budget check in CI mode
```

Additionally, per-phase gates (below) list any phase-specific verification on top of the generic gate.

**During Phase 0-2 there is no CI**; the local gate is the only gate. CI lands in Phase 3 (see [Phase 3](#phase-3-systems)). Once CI is in place, developers can rehearse pipeline locally with `act` before pushing.

---

## Phase 0: Skeleton -- COMPLETE 2026-04-19

**Goal:** a pip-installable cargo workspace with the directory structure from [DESIGN.md](DESIGN.md#directory-structure), an entry point that opens a window and clears it, and the full lint/test toolchain wired up.

> **Status:** Gate passed. See [PROGRESS.md § Phase 0 deliverables](PROGRESS.md#phase-0-deliverables-complete) for the built artifacts.

### Steps

1. **Hand-craft the workspace skeleton.** Do not run `cargo new` at the workspace root; create the file layout directly:
    - `Cargo.toml` (workspace, `[workspace]` with `members = [...]`, shared `[workspace.dependencies]`)
    - `rust-toolchain.toml` (pinned stable channel, components: rustfmt, clippy)
    - `.rustfmt.toml`, `.clippy.toml` matching the rules in [CODING_RULES.md](CODING_RULES.md)
    - `.gitignore` (target/, .tracy/, asset-cache/, IDE noise)
    - `crates/{core,assets,sim,world,render,audio,app}/` each with its own `Cargo.toml` and empty `src/lib.rs` (or `src/main.rs` for `app`)
    - `tests/`, `benches/`, `tools/` dirs

2. **Wire up shared toolchain config:**
    - `Cargo.toml` workspace-level `[profile.release]` (LTO thin, codegen-units 1, strip=symbols), `[profile.dev]` (opt-level 1 for usable FPS while debugging)
    - `[workspace.lints.rust]` deny-list: unused_crate_dependencies, let_underscore_drop
    - `[workspace.lints.clippy]` configured per [CODING_RULES.md](CODING_RULES.md#lints-and-formatting)

3. **`crates/core`** minimal content:
    - `src/lib.rs` with `#![forbid(unsafe_code)]` and crate-level docs pointing to DESIGN.md
    - Stub `time.rs` (newtypes `TickCount`, `Seconds`), `handle.rs` (generic `Handle<T>`), `rng.rs` (seeded PCG wrapper)
    - Tests for each stub

4. **`crates/app`** opens a window and clears it:
    - `winit` event loop setup
    - `wgpu::Instance` + adapter + device
    - Clear to a known color each frame
    - Exits cleanly on window close

5. **Architectural enforcement tests** (per [TEST_PLAN.md](TEST_PLAN.md)):
    - Import-linting test that scans each crate's `Cargo.toml` and rejects forbidden deps
    - "Hot path" marker tests (stubs that will enforce no-alloc once hot paths exist)

6. **Bench harness:** `benches/frame_budget.rs` with a `criterion` smoke benchmark (no real scene yet; validates the plumbing)

7. **Run the gate.** `cargo fmt --check && cargo clippy -D warnings && cargo test`. Fix until green.

### Deliverable

Running `cargo run -p flow-app` opens a window showing a cleared color, exits cleanly, and the quality gate passes. Directory structure matches DESIGN.md. Import-linting test enforces the dependency DAG.

> **Gate:** Phase 0 is complete only when the quality gate passes and `cargo run -p flow-app` displays a window. Do not start Phase 1 until it does.

---

## Phase 1: Core Loop -- COMPLETE 2026-04-19

**Goal:** fixed-timestep simulation, interpolated rendering, input plumbing, basic camera, flat terrain. Fly a cube over a heightmap.

> **Status:** Gate passed and phase locked. 81 tests pass. See [PROGRESS.md § Phase 1 deliverables](PROGRESS.md#phase-1-deliverables-complete) for built artifacts, frozen tuning constants, sign conventions, and bugs fixed during the phase (do not re-introduce).

### Milestone 1.1: Fixed-timestep loop

- [ ] Accumulator in `app` drives `sim.tick(dt_fixed)` at 120 Hz
- [ ] `SimState` triple-buffered; render reads front while sim writes back
- [ ] Interpolation alpha passed to render each frame
- [ ] Property test: deterministic state for fixed input sequence and seed
- [ ] Property test: swap correctness under concurrent tick/render

### Milestone 1.2: Input

- [ ] winit events -> `InputEvent` enum in main thread
- [ ] Bounded `crossbeam::queue::ArrayQueue<InputEvent>` drained at tick start
- [ ] Gamepad via `gilrs` (approved dep added in this phase if not in DESIGN.md's stack yet; amend DESIGN.md first)
- [ ] Test: simulated input event sequence produces expected sim state

### Milestone 1.3: Camera

- [ ] `Camera` in `sim` (position, orientation, fov, view-kind tag)
- [ ] View kinds implemented: first-person cockpit (placeholder rig until a plane model lands in Phase 2), third-person chase, exterior orbit. Tail / wingtip / cowl views are Phase 2+ once the plane model provides reference points.
- [ ] Free switching between view kinds at any time, bound to keys / gamepad face buttons
- [ ] Short 60-120 ms camera transition tween with hold-to-hard-cut option
- [ ] Chase-camera controller that tracks a target entity with configurable stiffness/damping
- [ ] Camera interpolation uses the same alpha as scene interpolation
- [ ] Right-stick look-around inside a view (POV-hat emulation) with center-snap-on-release; mouse-look on modifier
- [ ] Per-view FOV setting and inertia tuning

### Milestone 1.4: Terrain

- [ ] Heightmap loaded from a 16-bit PNG / RAW in `assets`
- [ ] Terrain mesh generated on CPU into a static vertex/index buffer
- [ ] Flat-shaded render pass (no cel shading yet; just verify geometry)
- [ ] Terrain sample function in `world` exposed to `sim` (for future buoyancy)

### Milestone 1.5: Test / bench

- [ ] Unit tests for `sim` tick determinism, input queue bounds, camera damping
- [ ] Frame-budget bench on a flat terrain + cube scene: must hit 240 Hz on reference hardware

### Deliverable

A cube can be flown (WASD/gamepad) over a terrain. Frame budget check passes at 240 Hz with headroom.

> **Gate:** Phase 1 is complete only when the quality gate passes, the input determinism property test passes, and the frame-budget bench meets [DESIGN.md targets](DESIGN.md#performance-budget).

---

## Phase 2: Feel Core

**Goal:** the game looks and feels like flowPlan. Cel shading, water, plane, buoyancy. If Phase 1 proves the engine works, Phase 2 proves the design works.

### Milestone 2.1: Cel shading -- COMPLETE

- [x] Banded diffuse in the opaque forward pass (3-4 bands, configurable per-material via `ShadingParams`)
- [x] Outline pass: screen-space sobel on depth (normal-buffer MRT deferred; depth-only catches silhouettes for Phase 2 scenes). Lives in `outline.wgsl`.
- [x] Rim light term (per-material in `ShadingParams`)
- [x] Shader hot-reload (dev only) via `notify`, feature-gated as `shader-hot-reload` on `flowplan-render` / `flowplan-app`
- [x] Visual snapshot tests via `cargo-insta` on a reference scene (`scene_minimal`); hardware-gated behind `hardware-tests` feature.

### Milestone 2.2: Water -- IN PROGRESS (depth-compare foam + distant LOD pending)

- [x] Gerstner wave displacement in vertex shader (3 components) per GPU Gems 2, Ch. 1. `water.wgsl`, separate pipeline. Sun-glint specular highlight bundled. Wave amplitudes 0.1-0.25 m so crests never engulf the plane.
- [x] Whitecap foam via value-noise mask on wave crests. Proper depth-compare shoreline foam (per the original brief) deferred until the render pass can sample a prior pass's depth; revisit with 2.5 contact physics or the Phase 3 scene_standard bench work.
- [ ] Normal-mapped distant water; decide transition distance during this phase (Milestone 2.2.5b, deferred to Phase 3 scene_standard work).
- [x] Water height sampling on CPU for buoyancy (`flowplan_world::water_height(x, z, t)`); same 3 wave components as the shader. Consumed by the Phase 2.5 pontoon buoyancy model.
- [ ] Bench: water on full screen meets frame budget (scene_standard bench is Phase 3 deliverable).
- [x] **Ancillary in this milestone (terrain polish):** 5-octave sum-of-sines at mountain scale (±275 m max); beach flattening `raw * smoothstep²(|raw|/120)`; 5-tier color ramp (sand / field / forest / rock / snow); pre-2.5 terrain+water ground clamp (`SimState::tick` takes `min_altitude` each step; app supplies `max(terrain, water_level) + float_offset + margin`).
- **Note: sum-of-sines terrain has limits.** It cannot independently tune amplitude and gradient, so genuinely gentle beach flats beneath steep mountain backdrops (per ref1/2/3 photos) require a different representation. Tracked under [FEATURES.md § Terrain & Static Landscape](FEATURES.md#terrain--static-landscape) for a Phase 3+ terrain rebuild with proper hydrology (elevation lakes, rivers, drainage basins, shore-type classification). Phase 2 terrain is placeholder-at-scale -- enough to verify the look and feel boundaries without committing to the final world system.

### Milestone 2.3: Plane model (DHC-2 Beaver reference) -- IN PROGRESS (core shipped; back-face culling deferred)

- [x] glTF import in `assets` (`flowplan_assets::load_mesh` handles `.glb` + embedded-buffer `.gltf`, first-mesh first-primitive, optional NORMAL / UV / COLOR fallbacks)
- [x] DHC-2 Beaver reference aircraft, hand-authored in Blender -- low-poly, cel-shaded, sanitized, no branding. Committed at `data/aircraft/northlander.glb`.
- [ ] Wheels + straight floats variants modeled at minimum; amphib floats in the same milestone if feasible, otherwise Milestone 2.5 *(current model bundles both as the STL source; per-part swap comes later)*
- [x] Plane rendered with cel shading + outline
- [x] In-game name placeholder committed: **Northlander**
- [x] Spec sheet (`.ron` data) authored: `data/aircraft/northlander.ron` from AIRCRAFT.md § MVP Reference, SI-normalized. Loaded via `AircraftSpec::from_ron` at startup; consumed by the flight model in Milestone 2.4.
- [x] Mesh origin auto-shift at load so bottom-Y sits at 0 (`CpuMesh::bbox_min_y` / `translate`), eliminating Blender pivot-offset fiddliness.
- [ ] **2.3.4 Re-enable back-face culling** -- attempted; reverted pending chase-camera and cockpit refinements (interior views cull the exterior shell and read as "see-through plane"). Revisit during Milestone 2.4/2.5 feel work.

### Milestone 2.4: Flight model (Beaver-tuned reference)

- [x] **2.4.1 Skeleton**: `sim::flight::compute_forces` pure function; `AircraftSpec` wired; `FlightOutputs` diagnostics.
- [x] **2.4.2 Airfoil physics (translational)**: lift `0.5 ρ V² S Cl(α)`, drag polar `Cd0 + Cl²/(π·AR·e)`, NACA-shaped `Cl(α)` table with whole-wing `Cl_max ≈ 1.65`, α and β computed from body-frame velocity.
- [x] **2.4.2b Lift direction**: perpendicular to velocity (not body-+Y). Coordinated banked turns feel realistic in flight test.
- [x] **2.4.2c Telemetry plumbing + 3D debug gizmo**: extract a `Telemetry` struct (pure derivation from `SimSnapshot + FlightOutputs`, render-rate, SI internally) as the single source for every display surface. First consumer is a 3D debug gizmo -- body axes + velocity + force vectors drawn as colored 3D lines at the aircraft position via a new `LineList` render pipeline (reused later for HUD ladders and cockpit face lines). Debug HUD refactored to read from the new `Telemetry` struct so it no longer reaches into `SimState` directly. See [FEATURES.md § Telemetry & Instrumentation Data Flow](FEATURES.md#telemetry--instrumentation-data-flow) for the progression and invariants. Worth doing before 2.4.4 because the moment-based rotation model will want α / β / load-factor / sideslip readouts for tuning, and every later phase's HUD and cockpit gauges plug into the same layer.
- [x] **2.4.3 Engine + prop**: R-985 power curve (`T = η_prop × P / V`), published cruise fuel burn (27 gph), fixed-pitch prop efficiency curve peaking at cruise. Fuel depletion cuts power to zero. Per-aircraft migration to `AircraftSpec` is a later sub-step; RPM / MAP / altitude-density effects land with 2.4.7. Constant-speed prop remains post-MVP.
- [x] **2.4.4a Keyboard input slew-rate ramp**: pitch / roll / yaw axes slew toward held-key targets at 1.5 full-deflection/s. Throttle unchanged. Analog sources bypass.
- [x] **2.4.4b Moment-based rotational dynamics**: `FlightOutputs` emits per-axis aerodynamic moments (control + static-stability + damping) and `angular_accel`; `state::tick_dynamics` Euler-integrates angular velocity. Control authority now scales with q; rudder creates β which creates restoring yaw; elevator creates pitch which α-stability restores; damping prevents runaway rates. Moments of inertia + coefficients hardcoded (Beaver placeholders; migrate to `AircraftSpec` in 2.4.x follow-up).
- [x] **2.4.4c Telemetry + debug HUD for moments**: `Telemetry` drops `commanded_angular_vel` / `ang_response_rate` in favor of `angular_accel` + per-axis `MomentBreakdown`. Dynamics HUD's angular section shows actual rate + angular acceleration; new "Moments" section shows total M/L/N with control / stability / damping columns, so "why isn't the plane responding" is diagnosable at a glance.
- [x] **2.4.4e Attitude indicator / glide-slope widget**: standard-AI style in the bottom-left (fixed aircraft symbol + rotating horizon + pitch ladder + flight-path-vector crosshair), text readouts for FPA / pitch / α / β / VS / L/D. First instrument widget; the AI layout translates directly to the Phase 4 cockpit attitude-indicator gauge.
- [ ] **2.4.4d Real stall + α-based stall alert**: post-stall `Cl` drop already in the table; add nose-down pitching moment from center-of-lift shift, wing-drop via asymmetric `Cl(α)` on each wing half (ailerons lose authority in stall, rudder retains -- PARE recovery works), accelerated-stall handling. Replace the Vs-heuristic HUD stall alert with α-based detection.
- [ ] **2.4.5 Flaps**: 4 slotted positions matching POH lift-boost (higher `Cl_max`, Vs → Vso) and drag-penalty (ΔCd ~ 0.02 per notch) figures.
- [ ] **2.4.6 Secondary effects**: ground effect (~30% induced-drag drop within 1 wingspan), P-factor (right-rudder-required at low airspeed + high power), adverse yaw (aileron → opposite yaw via induced-drag asymmetry), torque roll (engine torque opposing prop rotation on power changes). All these couple naturally into 2.4.4's moment model.
- [ ] **2.4.7 Density altitude**: `ρ(h, T)` drives both lift and engine power; Vne, cruise speed, and climb rate degrade with altitude.
- [ ] **2.4.8 Feel tuning**: decide arcade-dial-default vs sim-lite-default by playtest; determinism invariant preserved under any dial setting.
- [ ] Property tests (close at end of 2.4):
    - Level flight at cruise power holds altitude within tolerance under zero wind
    - Published cruise speed achievable at published power setting within tolerance
    - Stall speeds (clean / flaps / floats) match POH figures within tolerance
    - Stall recovery within documented time at configured input sequence
    - Determinism: same seed + same input trace + same dial state -> bit-identical state
- [ ] Validate against DHC-2 POH performance charts (takeoff distance, climb rate, cruise fuel burn) before milestone close
- [ ] **Scratch-branch prototype of the flight-model feel belongs in Phase 1**; this milestone is where feel locks. If feel has not clicked by 4-6 weeks into this milestone, trim scope per MVP re-planning in [skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md).

### Milestone 2.5: Pontoon buoyancy (Beaver floats)

- [ ] Hydrodynamic model for Beaver-sized floats (Edo 4930 analogue geometry)
- [ ] Per-pontoon water height sample + buoyancy force
- [ ] Horizontal drag when in contact with water, tuned to produce realistic plowing and on-step behavior
- [ ] Step transition ("on step") as a deliberate feel moment; porpoising risk modeled and avoidable with technique
- [ ] Water rudder for low-speed taxi; weather-vaning when retracted
- [ ] Wake spawn: particle + decal trail
- [ ] Spray effects obstructing forward view at high speed or chop
- [ ] Touchdown moment feels good in playtest (subjective, required)

### Milestone 2.6: Sky (daytime foundation) -- COMPLETE

- [x] Procedural sky: analytic zenith-to-horizon gradient + warm sun band + sun disc / halo in `sky.wgsl`. Full Preetham / Hosek-Wilkie deferred; current stylized model fits cel-shading aesthetic and ships cheap. Pre-scene pass.
- [x] Sun direction tied to sim time-of-day: `SimState.time_of_day: f32` in `[0, 1)` advances per tick at `DAY_LENGTH_SECONDS = 1200` (20 min symmetric cycle); `world::sun_direction(t)` derives the unit vector; app writes into `SceneParams.sun_dir` each frame.
- [x] Exponential height fog with sun tint in the scene fragment shader (distance-based `1 - exp(-density * dist)`, biased toward `sun_color` along the view-to-sun axis).
- [x] Celestial-sphere hook placed (reserved slot `SceneUniform.ambient.a`); populated in Phase 5 Milestone 5.6.

### Deliverable

A plane can be flown, landed on water, taxied, taken off again. Cel-shaded look is locked. Water is the centerpiece. Frame budget holds at 240 Hz on the reference hardware.

> **Gate:** Phase 2 is complete only when the quality gate passes, visual snapshot tests match approved references, the frame-budget bench holds, and a playtest confirms the core feel. Flight model arcade vs. sim-lite decision recorded as an amendment in [DESIGN.md](DESIGN.md#open-design-questions).

---

## Phase 3: Systems

**Goal:** audio spine, weather state, dedicated sim thread, CI pipeline. The game becomes a living system.

### Milestone 3.1: Audio spine

- [ ] `cpal` stream in `audio` crate
- [ ] `glicol_synth` graph construction at startup
- [ ] `rtrb` SPSC ring between sim and audio
- [ ] Engine sound: Glicol patch modulated by RPM, load, airspeed. **Reference aircraft (MVP) is the DHC-2 Beaver's Pratt & Whitney R-985 radial** -- 9-cylinder pattern, distinctive low-RPM rumble, flagship audio example for the project. Other engine types (flat-six, turbine) become patch variants once the radial signature is locked.
- [ ] Wind noise: filtered noise patch keyed to airspeed
- [ ] Water lapping: procedural patch near shoreline
- [ ] Callback invariants verified by hot-path allocation test and logging-free lint

### Milestone 3.2: Weather state

- [ ] Wind vector in `sim` (speed, direction, gustiness, terrain-induced turbulence)
- [ ] Pressure / temperature / humidity / visibility as sim-level state
- [ ] Precipitation regime enum and visuals (drizzle -> heavy_rain -> squall; light_snow -> blizzard in winter biomes)
- [ ] Storm state machine (clear -> building cumulus -> scattered thunderheads -> squall line / organized storm / frontal passage)
- [ ] Fog / mist state ladder (ground mist, wingtip mist, layer fog, dense fog)
- [ ] Wind shear at terrain edges and gust fronts
- [ ] Water chop per water body (calm / ripple / moderate / heavy / breakers) driven by wind fetch + duration
- [ ] Season overlay (spring / summer / autumn / winter) -- static-per-region acceptable for Phase 3; cycling is an open question tracked in [FEATURES.md](FEATURES.md#remaining-open-questions)
- [ ] Debris regime (dust, leaves, ash/smoke, blowing snow)
- [ ] Time-of-day advances with sim clock; dawn/dusk palette shifts land in Phase 5 polish
- [ ] Weather affects flight: wind on ground track, gust attitude perturbation, density altitude effect on performance, icing probability at cold+moist conditions
- [ ] Weather forecast at hubs with accuracy decay by range and time

### Milestone 3.3: Sim worker thread

- [ ] Move `sim.tick()` from main thread to a dedicated `std::thread`
- [ ] Main thread pushes input; sim thread publishes snapshots
- [ ] Determinism test still passes (scheduling must not introduce nondeterminism)
- [ ] No perceptible input-to-photon latency regression

### Milestone 3.4: CI

- [ ] GitHub Actions workflow runs the quality gate on every push
- [ ] Linux and Windows runners (macOS later)
- [ ] Cache `cargo target/` keyed on `Cargo.lock`
- [ ] Upload per-phase frame-budget bench results

### Milestone 3.5: Seeded deterministic soundtrack

- [ ] Master soundtrack seed + derivation function `(seed, season, weather, biome, time-of-day, mission_state) -> Glicol param schedule`
- [ ] Core Glicol graph shapes: ambient pad, motive/melodic, rhythm/percussive, environmental, radio
- [ ] Deterministic additions/subtractions of layers driven by state changes
- [ ] Shared tonal key per biome; cross-fades smoothed within the Glicol graph
- [ ] Per-aircraft engine synth patches (RPM + load + airspeed + damage-hum)
- [ ] Wildlife audio placement with stereo panning
- [ ] Landing / takeoff musical flourishes at milestones
- [ ] No pre-recorded music stems

### Deliverable

The plane has a voice (engine, wind, water, splash). Weather actively shapes flight. CI runs on every push.

> **Gate:** Phase 3 is complete when the quality gate passes in CI, determinism test passes with the sim worker thread active, and the audio callback passes the no-alloc/no-log verification.

---

## Phase 4: Delivery Loop

**Goal:** a complete playable loop. Hub -> destination -> return, with cargo affecting flight, damage persisting, economy and reputation progressing, and discovery unlocking content. No polish yet.

**Prerequisite (world-scope decision):** resolve the world-scope open question (see [FEATURES.md § World Scope & Map Layout](FEATURES.md#world-scope--map-layout)) *before* Phase 4 begins. Phase 4 content depends on it. Prototyping may happen in a Phase 1-2 scratch branch.

### Milestone 4.1: Hubs and routes

- [ ] Chosen world scope implemented (single large map / 4 regional maps / procedural with flight-log memory)
- [ ] Hub feature model: surfaces, services, vendors, mission sources, population tier -- all data-driven
- [ ] At least one full-service airfield, one full-service seaplane base, one mixed amphibious hub, two minor resupply hubs, and two remote mission-only hubs for Phase 4 acceptance
- [ ] Cartographer / map-memory integration for discovered hubs
- [ ] Route planning UI stub (HUD-only, no separate map screen)

### Milestone 4.2: Cargo, inventory, and loading

- [ ] Cargo types (data-driven `.ron`): weight, size, fragility, time sensitivity, passenger/non-passenger
- [ ] Aircraft stowage layout (data-driven per aircraft): fore / mid / aft / wing bays with mass limits and CG moments
- [ ] Loading plan UI with CG indicator; passenger seating "forward of non-passenger" rule
- [ ] Inventory persistence across flights (supplies, cargo held, tackle, firewood)
- [ ] Fuel burn model: power setting * time + density-altitude penalty + wind penalty

### Milestone 4.3: Damage and wear

- [ ] Zone-localized damage state (rear control, forward/lower gear/hull, prop, left/right wing structure and tip, canopy, fuselage, powerplant)
- [ ] Damage -> flight-model modifiers (airfoil performance loss, control authority loss, thrust loss)
- [ ] Visible cel-shaded damage: bent prop, torn canopy, asymmetric float scuffs, wing wrinkles
- [ ] Wear accretion over hours; cheap to address at repair hubs
- [ ] Recovery paths: field repair (supplies), tow/trailer to hub, write-off with barn-find replacement
- [ ] "Critter assist" stub (humor hook) -- data entry only in Phase 4; animation / interaction polish in Phase 5

### Milestone 4.4: Economy and reputation

- [ ] Credits ledger with deterministic transaction history
- [ ] Loan system: advance, passive interest, auto-apply mission rewards
- [ ] Hub-tier-based prices; remoteness multipliers
- [ ] Reputation per hub / region; mission-tier gating
- [ ] Persistent community supply contracts (major <-> remote)
- [ ] Community events (salmon run, fire season, freeze-up, break-up, festival) shift demand

### Milestone 4.5: Missions and template progression

- [ ] Mission templates: delivery, passenger, survey, supply drop, mail, SAR, medevac, scientific, fire support, cartography, photo
- [ ] Template parameterization from world state (weather, cargo supply, season, time, reputation)
- [ ] Sequential progression: completing a template unlocks a harder variant (load, distance, multi-stop, time pressure, adverse weather)
- [ ] Mission board at hub (and radio-board for remote hubs)
- [ ] Mission log with waypoints and optional objectives
- [ ] Wildlife runs as a distinct non-financial category; rewards: liveries, LUTs, map annotations, logbook moments, paint textures, soundtrack stems

### Milestone 4.6: Character foundation (minimal)

- [ ] `character/` subsystem scaffold (part of `progression/` crate group)
- [ ] Minimal character creator (cel-shaded style, modest personalization; avoid blocky look)
- [ ] Disembark / embark state machine
- [ ] On-foot controller (walk/jog; no combat; no stamina)
- [ ] Menu-based interactions: vendors, mission providers, campfire, aircraft walk-around
- [ ] Time-skip action: "camp until morning" anywhere survivable

### Milestone 4.7: Cockpit controls and autopilot

- [ ] Full primary + secondary controls (aileron / elevator / rudder / throttle / flaps / trim / mixture)
- [ ] Switches: parking brake, water rudder, retract, starter, magnetos, fuel selector, carb heat, pitot heat, tire pressure (ground only)
- [ ] Prop feathering for constant-speed/feathering-equipped aircraft
- [ ] Autopilot tiers (upgrade-gated): hold altitude, hold vertical speed, hold roll, hold turn, hold heading, hold all, full-auto to waypoint + circle
- [ ] AP authority respects turbulence; graceful disengage with audio cue

### Milestone 4.7b: Control Bindings & Mapping

- [ ] Full in-game binding UI with live input capture
- [ ] Supported input kinds at v1: keyboard, mouse (incl. wheel), gamepad (button / axis / hat / trigger). Joystick / HOTAS infrastructure present but curated profiles post-v1.
- [ ] Two-layer modifier system (`Mod1`, `Mod2`); chord bindings for rare actions (feather engage, emergency gear, etc.)
- [ ] Per-axis tuning: inversion, inner/outer deadzone, curve (linear / mild / aggressive / custom), sensitivity, saturation, trim offset
- [ ] Axis-to-button and button-to-axis emulation
- [ ] Multi-device composition (joystick + throttle + keyboard + gamepad can coexist cleanly)
- [ ] Per-aircraft overlays (twin-engine throttles, retract-only gear binding)
- [ ] Named profile management (Keyboard default, Gamepad twin-stick, Mouse+keyboard hybrid minimum); switchable at runtime
- [ ] Device-disconnect resilience: mid-flight unplug is survivable
- [ ] Haptic feedback where supported: stall buffet, touchdown, terrain proximity, gear contact, engine stress (per-source sliders; globally disable-able for accessibility)
- [ ] Binding-conflict detection with last-wins + warning
- [ ] Reset-to-defaults per profile / per category
- [ ] **Camera bindings integrated:** view cycle, direct-view selects, look-around axes (POV-hat emulation), tween-skip hard-cut, pan modifiers -- all remappable
- [ ] Accessibility: one-handed profile, toggleable-vs-held options per input, configurable prompt size, colorblind-friendly binding UI

### Milestone 4.8: HUD and map

- [ ] Minimal HUD: altitude, airspeed, heading, fuel, wind indicator, stall warning, terrain proximity
- [ ] Hand-rolled per [DESIGN.md § Non-Goals](DESIGN.md#non-goals)
- [ ] Cel-shading-compatible palette; colorblind modes
- [ ] Hand-drawn cel-shaded map view (fog-of-war / flight-log memory)
- [ ] Logbook view summarizing flights

### Milestone 4.9: Difficulty dials and accessibility

- [ ] In-game difficulty panel, adjustable any time; takes effect next tick
- [ ] Dials: wind magnitude, gust variance, wind shear, storm freq/intensity, hydrodynamics realism, temp/humidity variance, stall severity, adverse yaw/P-factor, damage sensitivity, economy harshness, autopilot authority, fuel-consumption multiplier, mission time-pressure
- [ ] Presets: Arcade / Relaxed / Sim-lite / Simulator, with per-dial override
- [ ] Assists: auto-rudder, auto-trim, takeoff/landing guide, terrain proximity, flight-path vector
- [ ] Audio-only cues for stall, over-speed, ground prox, engine stress, gear warning
- [ ] Subtitles for radio; colorblind HUD palettes

### Milestone 4.10: Save/load

- [ ] Deterministic save format (binary, versioned)
- [ ] Save at hub arrival and at camp; load resumes at last save
- [ ] Save includes flight-log memory for map persistence
- [ ] Save file budget < 5 MB for a long playthrough

### Deliverable

A full loop is playable. Fuel, cargo, weather, damage, and reputation all matter. Template missions sequence into harder variants. Wildlife runs produce non-financial rewards. Character disembark and minimal interaction work. Difficulty dials are available. Not polished, but complete.

> **Gate:** Phase 4 is complete when a tester can: complete five missions spanning at least three templates; incur one damage event, reroute to a repair hub, and recover; trigger a weather-forced reroute; capture one wildlife-run non-financial reward; adjust a difficulty dial mid-session with no break in determinism; and return to a "home" hub with the same aircraft across sessions. All in one session without crashes or regressions against Phase 2-3 gates.

---

## Phase 5: Polish & v1

**Goal:** v1.0.0 release. Polished feel, full content pass per chosen world scope, fleet system, paint shop, wildlife rescue polish, documented limitations.

### Milestone 5.1: Content

- [ ] Final map content per chosen world scope
- [ ] Aircraft catalog: at least one entry per family (light taildragger, mid utility taildragger, STOL mod, nosewheel utility, large cargo trike, light float, utility float, big-iron float, amphib floats, hull flying boat, ski-plane variant, water bomber)
- [ ] Iconic-homage aircraft (sanitized visual homages) as unlockable cosmetic tier
- [ ] Full cargo catalog keyed to community demand and seasonal events

### Milestone 5.2: Fleet and hangar system

- [ ] Hangar storage at major / regional hubs: individual aircraft with all mods, livery, CG, fuel, cargo, maintenance state retained
- [ ] Swap active aircraft at a hub-hangar where target is stored
- [ ] Barn-find system: discover abandoned/damaged aircraft via exploration/missions; trailer to hangar for fee or time; restore
- [ ] Trailer contract tiers: fast (flat fee) vs slow (cheaper, in-game days)
- [ ] Fleet UI view (last hub, status, paint, loadout)
- [ ] "Home hub" designation with loyalty perks

### Milestone 5.3: Paint shop and livery editor

- [ ] Rotate-and-paint livery editor onto currently viewable texture surface
- [ ] Layer system: base color + pattern/stripe/spot decals + logos
- [ ] Few bitmap texture options preloaded (rivets, canvas, polished, weathered)
- [ ] Cel-shade-compatible tooling (banded base, bold accent stripes read well)
- [ ] Save named schemes; livery persists per aircraft
- [ ] Wildlife / exploration-unlocked pre-designed liveries (apply-only, not editable)

### Milestone 5.4: Audio polish

- [ ] Full Glicol patch library committed (per biome, per aircraft, per weather, per mission state, per time-of-day)
- [ ] Mix and master passes
- [ ] Radio comms (sampled voices filtered through Glicol EQ/squelch/static)
- [ ] Landing / takeoff highlight flourishes at personal-best moments
- [ ] Soundtrack stems unlocked via wildlife / exploration progression

### Milestone 5.5: Visual polish

- [ ] LUT color grading selection (unlockable via exploration / wildlife)
- [ ] Dithered bloom tuned per biome
- [ ] Distant wildlife, birds, boats, boats-with-wakes
- [ ] Morning mist / wingtip mist water-takeoff effect
- [ ] Alpenglow / golden-hour lighting beats
- [ ] Cel-shaded damage visuals finalized

### Milestone 5.6: Celestial sky (render-only; night gameplay post-v1)

- [ ] Celestial sphere model with physically-placed stars (~few thousand bright catalog), planets (Venus/Mars/Jupiter/Saturn), and moon with phases keyed to in-world date/time/position
- [ ] Cel-shaded Milky Way band with stylized galactic-bulge and dust-lane art
- [ ] Curated deep-sky object catalog rendered as cel-shaded shapes (Andromeda, Orion Nebula, Pleiades, LMC/SMC regional)
- [ ] Constellation line overlays (classical + optional stylized variant)
- [ ] Light pollution response near major hubs; dark-sky rewards at remote/high-desert locations
- [ ] Aurora shader: undulating cel-shaded color bands, triggered by high-latitude biome + season + clear weather, with rare any-biome occurrence on solar-activity spikes
- [ ] Meteor shower events tied to in-world date (Perseids / Geminids / Leonids analogues) producing rare streak occurrences
- [ ] Moonlight scene lighting (ambient shift, cooler palette, soft shadows under full moon)
- [ ] Dev overlay: hover to identify stars, constellations, planets, DSOs
- [ ] Night flight is gated off in v1; visuals ship for dusk/twilight only until night-flight post-v1 enables full nocturnal gameplay

### Milestone 5.7: Wildlife polish and recovery-by-proximity

- [ ] Biome-specific wildlife spawns finalized (bears, moose, caribou, wolves, whales, orcas, otters, seals, eagles, ospreys, flocks, reindeer, puffins, dolphins, turtles, alligators, etc. as biome-appropriate)
- [ ] Boids flocking for birds; state machines for mammals; no rigid-body sim
- [ ] Wildlife-run mission category polished and folded into the main mission list with distinctive iconography + filter
- [ ] Logbook "moments" auto-captured for first-encounters
- [ ] Recovery-by-proximity system (from FEATURES.md § Recovery by Hub Proximity):
    - Near complex hub -> professional lift, minimal secondary damage, higher cost, faster turnaround
    - Near simple hub -> pickup-truck drag-out, moderate additional wear, lower cost, slower turnaround
    - Truly remote -> self-extraction (winch / hoist, significant additional damage) OR wildlife-assist (moose / bear / whale), gated and improving with wildlife-mission reputation
    - Cost tuned to "painful but not punitive"; scenarios tuned to produce stories, never gruesome

### Milestone 5.7: Platform verification

- [ ] Linux: native binary, AppImage + deb/rpm via `cargo-packager` or similar
- [ ] Windows: native binary, installer
- [ ] macOS: native binary, notarized .app (if feasible)
- [ ] Steam Deck: verified at 90 Hz with reduced water tier

### Milestone 5.5: Documentation and release

- [ ] README finalized
- [ ] `CHANGELOG.md`
- [ ] Release build profile verified (LTO, strip, panic=abort considered)
- [ ] Binary-size and cold-start targets in [DESIGN.md](DESIGN.md#performance-budget) verified
- [ ] v1.0.0 tagged

### Deliverable

Shippable v1.0.0.

> **Gate:** Phase 5 is complete when the release build passes the frame budget on all three hardware tiers (minimum, reference, high), Steam Deck verification passes, and installation works cleanly on Linux and Windows.

---

## Future Considerations (Post-v1)

Potential additions that maintain the design principles. These are deferrals, not non-goals (see [DESIGN.md](DESIGN.md#explicit-deferrals-may-revisit-after-v1)).

- HOTAS / joystick native curated profiles and detent/curve editor. (The v1 mapping system already accepts joystick input; what's deferred is the curated profile set and polish of the editor UI.)
- **Night flight with astrophotography-grade sky.** The renderer builds the celestial model in v1 (bright star catalog, Milky Way band, planets, moon with phases, aurora shader, meteor showers) but gameplay is capped to daylight in v1. Night gameplay turns on post-v1: landing-light cones, moonlit visibility, aurora triggers in northern biomes, dark-sky hubs rewarded with the best views. See [FEATURES.md § Night Sky & Astrophotography](FEATURES.md#night-sky--astrophotography).
- Astrophotography mission template (night photo runs: eclipses, conjunctions, meteor shower peaks, aurora chasing).
- Survival layer difficulty toggle -- food / water / fatigue status, camping-anywhere restoration, non-blocking.
- Fishing minigame expansion: simple-with-timing baseline lands in v1 per Phase 4; rare-catch cosmetic hooks and message-in-a-bottle task integration ships with it. Timing/skill-check tier expansion post-v1.
- Weather regimes beyond v1 core: sandstorm, hurricane cells, ice fog, lake-effect, katabatic winds.
- Seasonal cycle upgrade: dev-toggle is available in v1; full player-facing seasonal cycle post-v1.
- **Event-log save format**: the save *is* the flight log (templated, human-readable, deterministic replay). Hooks for async cartography, ghost-plane sightings, exportable route replays, and co-op via shared seeds. See [FEATURES.md § Flight Log & Save Format](FEATURES.md#flight-log--save-format).
- Shared-seed co-op: two live players on the same seed + same world-scope + same dial state share weather and world events; input traces interleave.
- Async ghost-plane sightings from other players' flight logs on the same seed.
- Photo mode (orbit camera, pause, export). Long-exposure / star-trail mode ties into the astrophotography sky.
- Modding interface (scoped: aircraft parameters and livery assets only, no code injection).
- Additional regions / map expansions (the three regions not chosen as Phase 2 Region 4 become post-v1 packs).
- Light twins for longer missions.
- Helicopter -- handling model too different; probable non-goal unless reconsidered with justification.
- Pet / companion unlocked through fishing or wildlife-mission progression (e.g., husky joins at a trapper cabin after a wildlife-rep threshold). Must not gate flight gameplay.
- Livery sharing marketplace (cosmetic, no currency).

None of these are committed.

---

## Non-Goals (Reminder)

From [DESIGN.md](DESIGN.md#non-goals) -- reproduced here because they guard the roadmap:

- Not an engine. Not an MMO. Not photorealistic.
- Not a scripted narrative game: missions are template-driven and parameterized by world state; no authored dialog trees, no branching quests. Emergence remains preferred.
- Not a full flight sim: no CFD, no engine-management minigames. Aerodynamics are tabulated per-aircraft and faithful; systems are simplified to meaningful toggles. Sim-realism is selectively deep where it matters for feel. Difficulty dials tune parameters without swapping physics models.
- World scope is under active design review (see [FEATURES.md § World Scope & Map Layout](FEATURES.md#world-scope--map-layout)) and must be locked before Phase 4.
- Not a physics toy.
- No engine framework (no Bevy, no Fyrox). No GC runtime. No plugin loader. No cloud/telemetry/DRM.
- No TAA/DLSS/FSR. No `unsafe` outside documented FFI sites.

Requests that conflict with non-goals are closed with explanation. This is scope protection, not personal judgment.
