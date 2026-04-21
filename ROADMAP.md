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
- [x] **2.4.4d Real stall + α-based stall alert**: nose-down pitching moment past α_stall (self-recovery behavior); wing-drop via `Cl(α)` sampled at each wing's quarter-span with roll-rate-induced α perturbation (non-linear differential past stall reinforces roll rate → wing-drop / spin-entry); α-based HUD stall alert replacing the Vs heuristic. Accelerated stall works correctly since the test is directly on α. Spin dynamics proper (autorotation, PARE recovery procedure) remain post-2.4.
- [x] **2.4.5 Flaps**: 4 slotted positions (UP/CLIMB/TAKEOFF/LANDING). `Δ Cl`, `Δ Cd0`, `Δ Cm` per detent; Landing flap drops stall `23 m/s → 20.8 m/s` (matches `Vso` in Beaver POH) and ~doubles `Cd0`. Keyboard: `F` extend, `C` retract. Asymmetric-flap failure is post-v1.
- [x] **2.4.6 Secondary effects**: ground effect (Phillips factor, ~49% induced-drag reduction at 1 m AGL for 14.6 m span), P-factor (thrust × sin(α) nose-left yaw), adverse yaw (aileron → opposite yaw), torque roll (engine-power-scaled left-roll reaction). All four sit in the new `MomentBreakdown.coupling` slot.
- [x] **2.4.7 Density altitude**: exponential ISA approximation `ρ(h) = ρ₀·exp(-h/8500)` drives both q-based aero scaling and a linear engine power derate. 3 km ρ/ρ_0 ≈ 0.71; 5 km ≈ 0.56. Temperature / non-ISA atmosphere is post-MVP.
- [~] **2.4.8 Flight test bench + feel-tuning pass**. See [TEST_PLAN.md § Flight Test Bench](TEST_PLAN.md#flight-test-bench) for design; coefficient-change discipline is encoded in [skills/flowplan-flight-bench/references/model-gap-triage.md](skills/flowplan-flight-bench/references/model-gap-triage.md) — no silent tuning, ripple-analyze before commit.

    **Shipped:**
    - [x] Bench framework (`crates/sim/src/flight_bench.rs`) — `FlightScenario`, `ScenarioInput`, `Assertion`, `BenchSample`, `run_scenario_headless`, `check_assertions`.
    - [x] `--scenario <name>` CLI playback for visual replay through normal renderer/HUD/widgets.
    - [x] First 3 Beaver scenarios with structured in-code fidelity docs: `beaver_cruise_sl`, `beaver_cruise_climb`, `beaver_stall_clean`.
    - [x] Plot pipeline: `tools/plot_telemetry.py` (2×4 panels including ground-track and moment breakdown) + `tools/run_flight_bench.sh` orchestrator (progress indicators, `--pause`, summary).
    - [x] Model-gap triage process (decision tree, ripple-impact discipline, regression protocol) encoded as `skills/flowplan-flight-bench/references/model-gap-triage.md`.
    - [x] Standing bench-identified gaps log in [PROGRESS.md § Bench-identified model gaps](PROGRESS.md#bench-identified-model-gaps-open-from-phase-248-audit) — three gaps open (Cd0 light, P-factor/slipstream, Cl_max vs Vs).
    - [x] Initial tolerance narrowing based on measured margins.

    **Remaining, in order (pre-Phase-2.5 completion path):**

    *Step 1 — Expand scenario suite (no new physics required; adds constraints for coefficient tuning):*
    - [x] `beaver_ldmax_glide` — idle-throttle glide at L/D_max. Asserts airspeed + sink rate at best-glide equilibrium. **SHIPPED.** Model best-glide V ≈ 45 m/s vs POH 31 m/s (Gap 1 consequence).
    - [x] `beaver_vne_dive` — pushover dive, staged pulse-release pitch input. Asserts airspeed lands in [60, 95] band. **SHIPPED.** Wide band until Cd0 tunes + ramped-input type lands.
    - [x] `beaver_stall_flaps_landing` — stall with LANDING flaps. Asserts min airspeed near Vso = 21.01 m/s. **SHIPPED.** Min ~28 m/s (same Gap 3 as clean stall, offset by flap Δ Cl).
    - [x] `beaver_endurance_cruise` — extended cruise validating fuel burn + long-term trim stability. Asserts altitude + airspeed + `FinalFuelWithin`. **SHIPPED.** Measured burn rate 22.8 gph vs POH 27 (Gap 1 consequence).
    - [x] Added `FinalFuelWithin { min_gal, max_gal }` assertion type for endurance scenarios. **SHIPPED.**
    - [deferred post-2.4.8] `beaver_coordinated_turn_30` / `_turn_60` — attempted during 2.4.8 audit; deferred with clear rationale inline in `flight_bench.rs`. Reason: flight model lacks Cl_β (dihedral roll stability), so a banked aircraft has no restoring roll moment. With only scripted constant inputs and no trim system, bank angle spirals instead of locking at a target. Unblocking options: (a) add Cl_β coefficient (cleanest), (b) ship trim system (Phase 3+), (c) add ramped scripted-input type. Not blocking 2.5 — the 4 new scenarios above (plus 3 original) give 7 constraints for coefficient tuning, which is enough. Revisit when feel-tuning returns to rotational or trim system ships.

    *Step 2 — Coefficient tuning pass (follow [model-gap-triage.md](skills/flowplan-flight-bench/references/model-gap-triage.md); one coefficient per commit; full ripple analysis and regression check each):*
    - [ ] Close Gap 3 (Cl_max) — raise `CL_TABLE` peak from 1.65 toward ~2.0. Analytic target `Vs² = 2W/(ρSCl_max)` → ~2.95 for POH Vs; choose conservative first step. Ripples: stall scenarios, flap-stall, wing-drop, ground-effect, induced drag at high α.
    - [ ] Close Gap 1 (Cd0 light) — raise `Cd0` from 0.045 toward ~0.055-0.060. Ripples: cruise equilibrium throttle, cruise-climb ROC, L/D_max glide ratio.
    - [ ] Attempt coefficient-only fix for Gap 2 (P-factor). If over-corrects cruise rudder, mark as missing-physics (slipstream over rudder) and defer to post-2.4 roadmap item. This is an expected outcome — document it cleanly rather than forcing a fit.

    *Step 3 — Lock and commit references:*
    - [ ] Narrow tolerances on all 9 scenarios based on post-tuning measured margins (2-3× actual margin per triage discipline).
    - [ ] Commit reference plots to `docs/plots/beaver/<scenario>.png` (versioned artifacts, reviewed in PRs).
    - [ ] Update [PROGRESS.md § Bench-identified gaps](PROGRESS.md#bench-identified-model-gaps-open-from-phase-248-audit): move closed gaps to "Resolved" subsection with final values; open-phase deferrals (slipstream physics) documented with target phase.
    - [ ] Close out 2.4.8 with a section in PROGRESS summarizing final coefficient set, final scenario suite, and what Phase 2.5 inherits.

    **Deferred (not required before 2.5):**
    - Per-aircraft coefficient migration from `flight.rs` constants to `AircraftSpec`. Explicitly deferred to [Phase 5.1 Content milestone](#milestone-51-content) when the 2nd aircraft lands — migrating before multiple aircraft means migrating and then waiting. Beaver values in `flight.rs` are authoritative until then.
    - Difficulty-dial arcade-vs-sim-lite default decision: deferred to Phase 4.9 when settings UI lands. The bench is dial-agnostic; whichever default we pick has to pass the bench. Determinism invariant preserved under any dial setting: dials scale parameters before the tick, same `(seed, scenario, dials) → bit-identical` log.
    - Takeoff / landing / crosswind / headwind / tailwind / water-takeoff scenarios: blocked on 2.5 contact physics or 3.2 weather. Enumerated in [flowplan-flight-bench/references/behavior-taxonomy.md](skills/flowplan-flight-bench/references/behavior-taxonomy.md) with blocker tags; they become actionable as their prerequisites land.
    - Service ceiling scenario: waits for a proper long-climb scenario + stable density-altitude behavior; low priority.

    **Pre-Phase-2.5 gate (what must be true before starting 2.5 contact physics):**
    1. All 9 scenarios above pass with narrowed tolerances.
    2. Gaps 1 and 3 resolved or consciously-deferred with target phase; Gap 2 resolved OR documented as missing-physics with clear decision.
    3. Reference plots committed to `docs/plots/beaver/`.
    4. Determinism invariant verified: rerun bench twice, byte-identical CSVs.
    5. No open flight-model regressions; fmt/clippy/test gate green across workspace.
    6. Final coefficient set documented in PROGRESS so 2.5 contact physics work starts from a known-stable flight model.

    Completing these six items means the flight model is *locked for 2.5* — contact physics development won't see coefficient drift mid-milestone.
- [ ] Property tests (close at end of 2.4):
    - Level flight at cruise power holds altitude within tolerance under zero wind
    - Published cruise speed achievable at published power setting within tolerance
    - Stall speeds (clean / flaps / floats) match POH figures within tolerance
    - Stall recovery within documented time at configured input sequence
    - Determinism: same seed + same input trace + same dial state -> bit-identical state
- [ ] Validate against DHC-2 POH performance charts (takeoff distance, climb rate, cruise fuel burn) before milestone close
- [ ] **Scratch-branch prototype of the flight-model feel belongs in Phase 1**; this milestone is where feel locks. If feel has not clicked by 4-6 weeks into this milestone, trim scope per MVP re-planning in [skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md).

### Milestone 2.4.9: Flight-model refinement (optional-before-2.5, required-before-v1)

Closes the two missing-physics items identified by the 2.4.8 bench audit. Optional-before-2.5 (Phase 2.5 contact physics doesn't depend on either item), required-before-v1 (both materially improve feel at high power / in turns, and unblock scenario-taxonomy entries currently tagged `[deferred]`). Can run between 2.4.8 and 2.5, after 2.5, or during Phase 3 — scheduled at the implementer's discretion.

Both items discovered via the triage protocol encoded in [skills/flowplan-flight-bench/references/model-gap-triage.md](skills/flowplan-flight-bench/references/model-gap-triage.md). Each required an explicit coefficient experiment to confirm the fix is *not* a coefficient change — which is exactly how the protocol is supposed to work.

**2.4.9a Slipstream-over-rudder physics.**
- Context: Gap 2 in [PROGRESS.md § Bench-identified gaps](PROGRESS.md#bench-identified-model-gaps-open-from-phase-248-audit). Real Beaver pilots at full power use 20-40% right rudder; our model only needs 5.5%. Confirmed via coefficient experiment (raising `P_FACTOR_ARM_M` over-corrects cruise while barely helping climb) that the issue is *rudder effectiveness scaling with thrust*, not P-factor magnitude.
- Implementation sketch: add slipstream term to yaw-moment calc. `q_local_at_rudder = q_freestream + slipstream_boost(thrust)`, where `slipstream_boost` is a function of prop-disc velocity increment (derived from thrust, prop area, freestream velocity). Rudder moment uses `q_local_at_rudder` instead of `q_freestream`. Similar boost applies to elevator (if behind prop) but smaller due to geometry.
- [ ] Add `slipstream_boost(thrust, airspeed)` helper in `flight.rs`.
- [ ] Modify yaw-moment calculation to use slipstream-augmented q for rudder term.
- [ ] Optionally modify pitch-moment for elevator (if geometry supports).
- [ ] Ripple-analyze + regression check per [triage protocol](skills/flowplan-flight-bench/references/model-gap-triage.md).
- [ ] Re-tune cruise / climb scenarios if rudder-trim values shift (expected: cruise_climb rudder rises toward 0.25-0.30; cruise_sl slightly rises too).
- [ ] Update Gap 2 in PROGRESS to "Resolved" with final slipstream parameters + scenario rudder-trim updates.

**2.4.9b Cl_β dihedral roll stability.**
- Context: discovered during 2.4.8 Step 1 when attempting turn scenarios. Without Cl_β (rolling moment from sideslip), a banked aircraft has no restoring roll moment — bank spirals rather than locking. Coordinated turn scenarios blocked.
- Implementation sketch: add `Cl_β` coefficient to flight.rs. Roll moment gains `q_dyn·S·b·Cl_β·β` term. Typical value for a Beaver-class GA single: Cl_β ≈ -0.07 to -0.10 per rad (negative = right-wing-down rolls induce nose-right yaw which induces nose-right β which induces right-wing-up rolling moment = restoring).
- [ ] Add `CL_BETA` constant in `flight.rs`.
- [ ] Add dihedral-roll term to roll moment calculation.
- [ ] Ripple-analyze: unlikely to affect straight-line scenarios; may slightly dampen roll transients.
- [ ] Add `beaver_coordinated_turn_30` scenario (previously deferred inline in `flight_bench.rs`). Validates turn holds at 30° bank without spiral.
- [ ] Add `beaver_coordinated_turn_60` scenario. Validates accelerated-stall α calculation under 2 g load.
- [ ] Update PROGRESS "Bench-identified gaps" section: remove turn-scenarios-deferred note.

### Deliverable (2.4.9)

- `beaver_cruise_climb` and hypothetical `beaver_climb_vy` scenarios use realistic rudder-trim values matching real Beaver pilot reports.
- Turn scenarios (30° / 60°) pass with tight tolerance — bank angle pins at target, altitude holds, airspeed holds.
- Gap 2 resolved; turn scenarios unblocked.
- Nine scenarios total (seven existing + two turns), ~20 assertions, all passing.

> **Gate (2.4.9):** All 7 existing scenarios still pass; new slipstream-physics tuning produces cruise-climb rudder trim in 0.20-0.35 range; turn scenarios bank-lock at target within ±2° over assertion window.

### Milestone 2.5: Ground + water contact physics

Replaces the pre-2.5 passive altitude clamp (which keeps the CG above
`max(terrain, water_level) + float_offset` but does not resolve rotation, apply
friction, or drag the plane in water) with proper contact physics. Both the floatplane
(buoyancy + hydrodynamics) and wheeled (runway / soft-field contact) configurations
are in scope -- the Beaver is flown on both.

**2.5a Water contact (floats):** Dependency-ordered sub-milestones; each gated on its own bench scenarios landing. All share the invariant that contact forces are a deterministic function of `(state, water sample, dt)` — no wall-clock in the contact path. Per-pontoon physics up front means asymmetric behavior (one-float-dipped, one-pontoon takeoff) emerges for free rather than requiring special-case code later.

**2.5a.1 Per-pontoon buoyancy + deterministic wave clock. — SHIPPED**
- [x] Add `SimState.wave_time: f32`, advanced `dt` per tick, folded into `state_hash` so determinism covers wave phase
- [x] Sim-side consumption of `world::water_height(x, z, t)` (API shipped 2.2); render continues using `SceneUniform.time.x` (wall-clock), both read the same `WAVES` constants so visuals stay aligned with physics. Architectural note: `sim → world` dependency now documented in DESIGN.md § Dependency Rules (flipped from the previously-anticipated `world → sim` direction so primitives live at the natural layer).
- [x] Per-pontoon water-height sample at each float's attachment point (Edo 4930 analogue geometry — port / starboard body offsets ±1.25 m lateral, -0.5 m vertical)
- [x] Buoyancy force per pontoon = `ρ_water · V_submerged · g`, clamped by pontoon displacement volume (2.236 m³ per float). Box-approximation wetted volume sums to per-pontoon force; body-frame `r × F` produces moment that sums into `angular_accel` alongside aero.
- [x] Symmetric case validated analytically: `equilibrium_draft_produces_weight_matching_force` unit test asserts buoyancy equals weight at CG_y = 0.66 m (analytic equilibrium for Beaver MTOW on Edo 4930 floats). Full tick-loop settle test deferred to 2.5a.2 — needs vertical water drag to damp the buoyancy-gravity oscillation (without it, undamped explicit Euler across the displacement cap is numerically unstable; this is the physics 2.5a.2 introduces).

**2.5a.2 Per-pontoon horizontal water drag + vertical damping. — SHIPPED**
- [x] Wetted-area approximation per pontoon, scaled by immersion depth
- [x] Horizontal drag force per pontoon, scaled by pontoon velocity (quadratic), tuned for realistic plowing (`HORIZONTAL_DRAG_N_PER_MPS2 = 75.0`)
- [x] Vertical damping to prevent bobbing oscillation at rest (linear, for numerical stability near zero velocity; `VERTICAL_DAMPING_N_PER_MPS = 30_000.0`, ~0.56 of critical at equilibrium wetted fraction)
- [x] Orientation-aware float-bottom calculation (rotate body-Y by orientation to get world-frame lowest point) — avoids wetted-area error when aircraft pitches from drag moment
- [x] Per-pontoon velocity includes angular contribution (`v_pontoon = v_cg + ω_world × r_world`) so roll/yaw rates couple into drag → angular damping emerges naturally
- [x] Integration test: float dropped from 1.5 m settles near analytic equilibrium draft (~0.66 m CG mean over last 10 s of a 30 s run, ±0.15 m tolerance)
- [x] App `min_altitude` clamp drops the `WATER_LEVEL` floor — buoyancy now manages water contact in-game; terrain remains the only `min_altitude` source until 2.5b lands
- [ ] Water-taxi scenario migrated to 2.5a.3 — single-point float sampling can't produce the fore/aft buoyancy restoring moment that real floats use to limit pitch-down under drag; 2.5a.3 multi-point sampling supplies it naturally. 2.5a.2 drag-in-isolation is validated by unit tests (vertical damping opposes motion, horizontal drag opposes motion, drag scales with wetted fraction).

**2.5a.3c On-step retune (drag + planing-lift forward offset). — SHIPPED**
- [x] Reduced `HORIZONTAL_DRAG_N_PER_MPS2` 75 → 40 (drag pitch-down moment was 4.5× too strong post mesh-aligned floats)
- [x] Raised `PLANING_LIFT_N_PER_MPS2` 50 → 75 (stronger planing supports rise to step)
- [x] Added `PLANING_LIFT_CENTROID_Z_FWD_M = -1.5` — planing lift acts at a forward offset (real planing-hull bow-wave CoP location), giving the natural pitch-up balancing moment that lets aircraft sit stably on step
- [x] Bench: `beaver_water_takeoff` reaches stable on-step skim at 18-20 m/s with bounded pitch (±5°) and roll (±10°). Aircraft accelerates from rest through hump speed to on-step plateau over 60 s.

**2.5a.4 Porpoising + bench scenario expansion. — SHIPPED**
- [x] Porpoise mode emerges naturally in `beaver_water_takeoff` (visible as ±5° pitch oscillation in the on-step phase) — demonstrates the under-damped pitch mode characteristic of real planing-hull dynamics. Buoyancy fore/aft asymmetry + vertical damping bound it; not destabilizing.
- [x] `beaver_water_takeoff_rotation` — pilot back-pressure (0.15) added at t=10s. Documents current state where buoyancy pitch dynamics override elevator at low Q (aircraft does NOT yet rotate to liftoff, settles on-step at 18-20 m/s). "Document the gap" scenario per model-gap-triage discipline; tightens toward POH liftoff (V > 25 m/s off the water) when Cm_δe vs buoyancy pitch balance retunes.
- [x] `beaver_water_landing_approach` — idle-power descent from 100 m at 30 m/s with landing flaps. Validates float-contact deceleration (V drops from 26 m/s touchdown to 5.5 m/s coast over 30 s).
- [x] `beaver_water_porpoise_emerge` — strong sustained back-pitch (0.4) during takeoff. Envelope-limit scenario per Core Rule #9 pairing discipline. Currently shows model handles the stress without crashing; explicit porpoise-amplitude assertion deferred (would need new `PitchAmplitudeBounded` assertion type).

**2.5a.3 Step transition (plow → on-step). — SHIPPED**
- [x] Multi-point float sampling (fore / fore-mid / mid / aft-mid / aft per float, 5 samples × 2 floats = 10 force points) so longitudinal wetted-area asymmetry produces the natural pitch-restoring moment under drag — decouples pitch dynamics from the single-point-centroid approximation. Buoyancy, vertical damping, horizontal drag, and planing lift all distributed per-sample with `per_section_scale = 1/N` normalization.
- [x] Hydrodynamic planing lift: `F_lift = PLANING_LIFT_N_PER_MPS2 × v_forward_body² × wetted_fraction` per section (world +Y direction, quadratic in forward body velocity, only positive for forward motion). Physical mechanism by which wetted pontoon area produces upward lift at speed. `PLANING_LIFT_N_PER_MPS2 = 50.0` tunes toward plausible plow-to-hump transition; full liftoff requires tuning pass documented as future 2.5a.3b follow-up if needed.
- [x] Step transition emerges naturally from wetted-fraction-per-section scaling: as aircraft rises, aft sections stay wet while fore sections rise out of water, so effective drag + planing lift drop from the per-section wetted terms. No separate step geometry needed in v1.
- [x] Pitch attitude reduction as aircraft rises onto planing surface (tests: `nose_down_pitch_produces_pitch_up_buoyancy_moment`, `nose_up_pitch_produces_pitch_down_buoyancy_moment` — would have caught the 2.5a.2 flip bug).
- [x] Bench: `beaver_water_taxi` reaches steady 3.4 m/s at idle thrust (not runaway, not stationary — validates plow-drag balance at equilibrium draft; ±1.5 m/s tolerance per 2-3× discipline).
- [x] Bench: `beaver_water_takeoff` reaches 12.7 m/s hump-speed region at full throttle in 30 s, rises from 0.66 m to 0.88 m CG (plow → partial planing), pitch stays bounded at -0.8° (vs unbounded flip pre-2.5a.3). Full rotation-off-water requires further planing-lift tuning; documented as v1 limitation.
- [x] `symmetric_rest_produces_no_roll_moment` test refactored to `deeply_submerged_symmetric_produces_no_roll_or_yaw` — old assertion relied on single-point sampling luckily hitting the displacement cap at y=0.3; multi-point correctly exposes wave-phase asymmetry at partial immersion, so test now uses deep submersion (y=-1.0) where every section saturates and wave variation doesn't affect buoyancy.

**2.5a.4 Porpoising.**
- [ ] Pitch oscillation when elevator input mismatches step phase (classic pilot-induced oscillation)
- [ ] Emerges from pitch-up / pitch-down coupling between wetted area and pitch trim
- [ ] Bench: porpoise-setup scenario shows oscillation; recovery scenario shows technique resolves it

**2.5a.5 Asymmetric / one-pontoon maneuvers (validation).**
- [ ] Bench scenarios exercise the asymmetric physics already shipped in 2.5a.1 (no new physics here — validation)
- [ ] One-pontoon-on-water takeoff scenario: bank reduces wetted area, takeoff distance shortens measurably
- [ ] Left-pontoon-dipped vs right-pontoon-dipped asymmetry pair (per `skills/flowplan-flight-bench/SKILL.md` Core Rule #10 asymmetric-pair discipline — right-rotating prop means left/right are not mirror-symmetric)

**2.5a.6 Water rudder + weather-vaning.**
- [ ] Water rudder for low-speed directional control (on/off input event)
- [ ] Weather-vaning when water rudder retracted (nose tracks wind at low speed — interacts with Phase 3.2 wind field when that lands; until then, zero-wind is the tested case)
- [ ] Bench: low-speed-taxi scenario with/without water rudder; heading response validated

**2.5a.7 Chop-intensity hook (Phase 3.2 integration point).**
- [ ] `chop_intensity: f32` parameter (default 1.0) scales wave amplitude in `water_height(x, z, t)` sampling
- [ ] Part of determinism state (hashed). Phase 3.2 later wires `ChopState` to drive it; until then, constant 1.0 is current behavior
- [ ] Keeps 2.5a usable under future weather integration without breaking scenario reproducibility

**2.5a.X Visual polish (before 2.5a close; not sub-milestone-gated).**
- [ ] Wake spawn: particle + decal trail behind moving floats
- [ ] Spray effects at high speed or in chop, obstructing forward view
- [ ] Touchdown moment feels good in playtest (subjective, required before 2.5a gate)

**Non-global water heights (elevation lakes, rivers, sandbars for bush-pilot parking) are Phase 3+** per [FEATURES.md § Terrain & Static Landscape](FEATURES.md#terrain--static-landscape). The `world::water_height(x, z, t)` API is position-conditioned already, so the eventual hydrology rebuild plugs in without changing 2.5a's consumer contract. Sandbar parking specifically bridges 2.5a (water sampling near the bar) + 2.5b (soft-surface ground contact on the bar) + Phase 3+ shore classification.

**2.5b Ground contact (wheels / taildragger):**
- [ ] Per-contact-point normal force distributed across wheels + tailwheel (or ski pair in winter biome), resolving roll / pitch the CG clamp currently ignores
- [ ] Rolling resistance + per-surface friction (paved / grass / gravel / sand / tundra), feeding both longitudinal drag and lateral grip for taxi steering
- [ ] Brake input (`B` held when on ground → symmetric braking; asymmetric toe-brake support arrives with remap UI in Phase 4)
- [ ] Tailwheel steering: locked above a threshold ground speed, castoring at taxi speeds (Beaver's actual behavior)
- [ ] Pitch / roll attitude constrained when on surface: plane can rotate into nose-high (stall-taildown) or wingtip-scrape attitudes but the contact resolution pushes back with the appropriate torque
- [ ] Prop strike, tail strike, wingtip strike detection -- damage consequences land in Phase 4+, but the detection events are raised from this milestone so the hooks exist

**2.5c Invariants + tests:**
- [ ] Determinism preserved: contact forces are a deterministic function of `(state, terrain/water sample, dt)`; no wall-clock in the contact path.
- [ ] Property test: a stationary plane on level ground remains stationary (no drift, no chatter) for 10 s of sim time.
- [ ] Property test: a plane taxiing straight on paved surface at low throttle reaches a steady taxi speed determined by rolling resistance and idle thrust (not runaway).
- [ ] Property test: a float-configured plane at rest on water settles to equilibrium height matching displacement of the fully-loaded vehicle.

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

### Milestone 3.5b: FFT-based ocean water (Tessendorf method)

Replaces the Phase 2.2 sum-of-sinusoids Gerstner water with a spectrum-based FFT
ocean simulation. Triggered by Phase 3.2 weather landing — FFT ocean naturally
parameterizes by wind speed + direction (Phillips or JONSWAP spectrum → IFFT →
height field), so wind-driven sea state becomes physically motivated rather than
a separate layer on top of fixed-parameter sinusoids.

**Motivation (from 2.5a iteration):**

During Phase 2.5a water contact work, the sum-of-sinusoids water was iterated
heavily. Techniques explored and documented:

- **Multi-scale wave hierarchy** (2 swells + 3 chop + 2 fine chop = 7 waves)
- **Per-wave amplitude modulation** (each wave's effective amp varies with two
  combined sine modulators + spatial noise; "wave-of-waves" approach)
- **Noise displacement** on top of Gerstner sum (3 octaves of value_noise2 at
  rotated sampling grids, scales chosen to not overlap wave wavelengths)
- **Per-wave domain warping** (each wave's phase position perturbed by unique
  noise offset so crest lines curve rather than running straight)
- **Camera-follow mesh** (water moves with aircraft, stays tessellated where
  the aircraft is)
- **High-tessellation small-extent mesh** (512² vertices over 2 km, ~4 m per
  vertex — enables 5 m chop to be geometrically visible)
- **Per-component color tinting** (swell-trough green hue + chop-crest blue lift)
- **Slope-driven foam** (foam follows wave faces, not just crest peaks)
- **Subsurface-scattering proxy** (translucent warm glow at back-lit crests)

**Fundamental limitation discovered:** sum-of-sinusoids water produces a visible
regular interference lattice at 30-40 m spacing (classic "oval lattice" from
crossing wave crests). Domain warping, noise displacement, and amplitude
modulation mitigate but don't eliminate the pattern because the sinusoidal wave
crests themselves are structural. No amount of stylized post-effect stacking on
top of Gerstner gets past this without replacing the underlying wave model.

The iteration produced the `water_topdown` + `water_topdown_wide` tests (see
`crates/render/tests/visual_snapshot.rs`) for rapid visual inspection — those
remain useful for tuning the FFT implementation too.

**FFT ocean scope (Tessendorf 2001 / "Simulating Ocean Water"):**

- [ ] Add compute pipeline support to the renderer (currently vertex+fragment only)
- [ ] Storage buffer / texture setup for the spectrum + height field
- [ ] Spectrum generator (compute shader, Phillips or JONSWAP driven by wind
      direction + speed from Phase 3.2 `WorldState`)
- [ ] IFFT pass (Cooley-Tukey in-place, 256×256 Rgba16Float recommended starting
      point; 512² for higher tiers)
- [ ] Second overlaid FFT at different tile size to break horizon repetition
      (irrational tile size ratio, e.g., 150 m and 250 m)
- [ ] Vertex shader samples the resulting height texture instead of computing
      waves — mesh becomes "just geometry", all dynamics in the FFT pass
- [ ] Sim-side height sampler mirrors the FFT (or samples a CPU-readback of the
      height texture each tick) so buoyancy stays consistent
- [ ] Chop-intensity hook (2.5a.7) becomes "wind speed input to spectrum"
- [ ] Port the 2.5a visual effects (color tinting, slope foam, SSS glow) to the
      FFT height field — they're orthogonal to the wave-generation method

**Dependencies / scope amendments:**

- **DESIGN.md amendment:** compute-pipeline support in `wgpu`-facing code. Not
  currently in the architecture; needs a narrow boundary for the FFT.
- **Dependency review:** vendoring an FFT compute shader is viable without a
  new dep (Cooley-Tukey is ~200-400 lines of WGSL); alternative is a crate like
  `wgfft` which would require DESIGN.md tech-stack amendment.
- **No runtime FFT on CPU.** Sim-side `water_height` should sample the GPU
  texture (via a readback path) OR maintain a parallel CPU FFT — pick during
  implementation based on buoyancy's sample density needs.

**Performance budget (3060 reference tier, 240 Hz target = 4.17 ms budget):**

| Component | Estimated cost |
|-----------|----------------|
| Main FFT pass (256² Rgba16Float) | 0.1-0.15 ms (2.5-3.5% of budget) |
| Overlaid second FFT at different scale | 0.1 ms (2.5% of budget) |
| Vertex shader texture sampling (replaces current math) | ~0.01 ms (negligible; actually FASTER than current 7-wave vertex shader) |
| Spectrum generation (pre-FFT) | 0.02 ms (re-run only on wind change) |
| **Total** | **~0.2-0.3 ms (~5-7% of frame budget)** |

- Memory: ~0.5-2 MB for height textures + scratch buffers.
- Scales down cleanly for lower-tier hardware: 128² FFT on Steam Deck (~2-3%).
- Scales up for high-tier: 1024² FFT on RTX 4070+ (~10-15%) for photorealistic
  ocean scenes if that direction is ever pursued.

**Why this performance is acceptable:**

- Current Gerstner water vertex shader is ~0.2-0.5 ms (7 waves × domain-warp ×
  modulation × per-vertex). FFT version is similar total cost, with much better
  visual quality.
- 5-7% of frame budget is within our 60% GPU utilization cap by a wide margin.
- Budget headroom exists: frame-budget tests (`benches/frame_budget.rs`)
  confirm reference-tier scenes hit the target with headroom.

**What FFT water gives us that Gerstner can't:**

- **No visible interference lattice.** Spectrum contains continuous frequencies;
  patterns emerge stochastically.
- **Physically-correct wind-driven sea state.** Beaufort scale → wave height
  statistics flow directly through the spectrum.
- **Automatic chop/swell mixing.** The spectrum handles both scales naturally.
- **Matches AAA reference** (Sea of Thieves, Far Cry, commercial flight sims).

**Caveats:**

- Heavier code (roughly 500-1000 lines across compute shaders + setup).
- Less directly-tunable artistically (spectrum parameters vs. tweaking N sine
  waves) — wind direction + speed + "peakiness" replace per-wave params.
- Loses some stylized cel-shading character — FFT ocean LOOKS like realistic
  ocean. For flowPlan's cel-shaded art direction, the foam + color tinting
  passes stay the same; only the underlying wave geometry changes.

**Gate (3.5b):**

All of the following, verified on reference hardware:
1. FFT ocean renders without visible interference lattice (compare
   `water_topdown_wide` output against the 2.5a snapshots — foam should be
   scattered, not in a diagonal grid).
2. Wind speed changes in `WorldState` measurably change sea state (wave
   heights rise with wind).
3. Frame-budget bench under `scene_standard` still hits 240 Hz on reference
   hardware with ≤ 60% GPU utilization.
4. Buoyancy physics unchanged — the sim-side height sampler produces a match
   to the GPU FFT output (byte-identical would be ideal; within 1-2 cm RMS
   would be acceptable given potential float-precision differences).
5. All Phase 2.5a bench scenarios (water_taxi, water_takeoff,
   water_takeoff_rotation, water_landing_approach, water_porpoise_emerge)
   still pass with the new water model.

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
- [ ] Autopilot tiers (upgrade-gated): hold altitude, hold vertical speed, hold roll, hold turn, hold heading, hold all, full-auto to waypoint + circle. **Control laws already shipped in `crates/sim/src/control.rs` as the `ControlLaw` enum (shared with the scenario bench — see Milestone 2.4.9c).** Phase 4.7 work is UI (mode select, engage/disengage, readouts), session state (which laws active), and authority limits (turbulence disengage). Bench scenarios validate law behavior by construction.
- [ ] State for active AP laws lives in `SimState` so determinism hash covers them — AP behavior is replay-identical.
- [ ] AP authority respects turbulence; graceful disengage with audio cue
- [ ] Waypoint-nav law (full-auto) — new `ControlLaw` variant needed here.

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

### Milestone 4.8.5: Graphics & display settings

- [ ] In-game graphics panel with all settings enumerated in [FEATURES.md § Display & Graphics Settings](FEATURES.md#display--graphics-settings)
- [ ] Display-mode handling: exclusive fullscreen / borderless windowed / windowed; monitor selection on multi-monitor systems
- [ ] Resolution + refresh-rate detection; user-selectable from detected list (30 / 60 / 90 / 120 / 144 / 165 / 240 Hz + any custom modes the display advertises)
- [ ] Present-mode selector: VRR / Mailbox (default), VSync / FIFO, No-VSync / Immediate; explained with tradeoffs inline
- [ ] Frame-rate cap (independent of present mode): unlimited / 30 / 60 / 90 / 120 / 144 / 165 / 240 / custom
- [ ] Render-scale slider 50% - 125% (bilinear / nearest upsample only; TAA/DLSS/FSR explicitly excluded per non-goals)
- [ ] Quality presets: Low / Medium / High / Ultra / Custom; per-subsystem overrides (water tessellation, MSAA 0/2/4/8x, view distance, foliage density, particle density, cel outline, rim light, bloom)
- [ ] First-launch GPU tier detection with confirmation dialog; no silent override
- [ ] FOV per view (cockpit / chase / orbit)
- [ ] HUD / UI scale
- [ ] Config persistence to `config.toml` at platform-appropriate location (XDG / %APPDATA% / `~/Library/Application Support/flowPlan/`); human-readable
- [ ] Reset-to-default per section and global
- [ ] Steam Deck profile auto-detected on launch (Low preset, 60 Hz handheld / 90 Hz docked, 75% render scale)
- [ ] Integrated-GPU detection defaults to Low preset with non-blocking warning
- [ ] Invariant: every setting preserves sim determinism (see [DESIGN.md § Display-refresh independence](DESIGN.md#display-refresh-independence)); changing any graphics setting does not alter physics, weather advancement, or scenario reproducibility

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
- [ ] Full refresh-rate matrix verified across hardware tiers (per [DESIGN.md § Display-refresh independence](DESIGN.md#display-refresh-independence)):
    - Low-end / Steam Deck: 60 Hz handheld, 90 Hz docked, Low preset, reduced water tier
    - Minimum tier: 120 Hz at High preset, all visual accents enabled
    - Reference tier: 144 / 165 / 240 Hz at High preset
    - High tier: 240 Hz+ at Ultra, uncapped Mailbox
    - Edge: 30 Hz display with all tiers (accumulator cap must hold physics determinism)
- [ ] Present-mode coverage verified: VRR / Mailbox, VSync / FIFO, No-VSync / Immediate each produce playable output on representative hardware; tearing / input-latency characteristics match documented expectations.
- [ ] Graphics-settings panel exercised on every platform; config persistence round-trips cleanly across OSes.
- [ ] Scenario bench re-run at 30 / 60 / 240 Hz confirms byte-identical CSVs at every display rate (proves physics-to-display independence in shipping build).

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

## Phase 6 (Stretch Goal, Post-v1): Multiplayer

**Goal:** 2-4 player lockstep co-op over LAN and wide network, with single-player progression carrying into multiplayer sessions. A natural extension of flowPlan's design methodology: the determinism invariant, fixed-timestep sim, snapshot architecture, and deterministic scenario replay — all baked in since Phase 0 — make this tractable without rewriting subsystems.

**This phase is a stretch goal, not a v1 commitment.** Entry criteria are strict: v1 has shipped, playtesting confirms a complete single-player experience, and community signal supports the investment. If any of those don't hold, the phase stays deferred. See [DESIGN.md § Non-Goals](DESIGN.md#non-goals) — "Not an MMO, not multiplayer-first" remains true; this is 2-4 player friends-list co-op, not massively multiplayer.

### Why this is a methodology dividend, not a retrofit

Every invariant and architectural choice in flowPlan's specs was written with *single-player determinism* as the primary motivator — reproducible replays, deterministic testing, frame-budget discipline. It turns out those same invariants are exactly what lockstep multiplayer needs. Specifically:

| flowPlan invariant | Multiplayer payoff |
|--------------------|---------------------|
| Fixed 120 Hz sim tick, accumulator-driven (Invariant #1) | Clear sync boundary every peer agrees on |
| Deterministic sim: `(seed, inputs, dials, dt, tick) → bit-identical state` (Invariant #8) | The core lockstep requirement, already proven by the flight bench |
| Bounded input queue drained per tick (Invariant #7) | Network sync is just "broadcast this queue to peers" |
| Read-only POD snapshots (Invariant #3) | Cross-thread architecture already matches cross-peer boundary |
| No wall-clock in sim (Invariant #6, codified in rules) | Sim advances at its own rate regardless of network jitter |
| Lock-free audio SPSC ring (Invariant #4) | Each peer renders its own audio locally; no network coupling |
| No retained scene graph (§ State Discipline) | Render just redraws from snapshot; extends naturally to N aircraft |
| Seeded RNG with stream separation (§ Simulation Architecture) | Weather, wildlife, missions deterministic per seed |

Said another way: the work to make flowPlan testable, deterministic, and single-player-replayable *is* most of the work to make it networkable. The design-forward methodology pays here in a way that most games — which bolt multiplayer on late — can't replicate.

### Entry criteria

Phase 6 is not opened unless all of:

- [ ] v1.0.0 has shipped and stabilized for at least one update cycle.
- [ ] Playtesting confirms the single-player experience is complete, polished, and well-received.
- [ ] Community / audience signal warrants the investment (not pursued speculatively).
- [ ] Resources (time, engineering focus) are available for a 12-18 month dedicated effort.

If these aren't met, Phase 6 stays closed. Shipping a complete v1 is the forcing function; multiplayer without v1 is an anti-pattern flowPlan is explicitly built to avoid.

### Milestone 6.1: Determinism hardening (cross-platform)

The single biggest risk to lockstep is floating-point divergence across OSes / CPU microarchitectures. This milestone closes that risk *before* networking work begins.

- [ ] Pin compiler flags for strict IEEE float behavior (`-ffp-contract=off`, no fast-math).
- [ ] Replace `std` trig (`sin`, `cos`, `atan2`) with `libm` crate (pure Rust, platform-independent) in sim hot paths.
- [ ] Audit every `f32` operation in `flight.rs`, `state.rs`, `world::weather`, and `world::water` for platform-dependent patterns.
- [ ] Cross-platform determinism test: run the flight-test bench on Linux + Windows + macOS + Steam Deck; CSVs must be byte-identical across all targets. This extends the existing bench at `crates/sim/tests/flight_bench.rs`.
- [ ] CI addition: every PR runs the bench on all three major-OS runners; byte-divergence fails the gate.

### Milestone 6.2: Async ghost-plane sightings

The smallest multiplayer feature — practically free given the scenario-replay machinery. Can ship as a stretch feature even before committing to live co-op.

- [ ] Player exports a flight log (seed, input trace, scenario config) as a shareable file.
- [ ] Other players import it into their session.
- [ ] The imported trace plays back as an NPC plane in the world, deterministically at every tick.
- [ ] Multiple ghosts per session, each independently replayed.
- [ ] Ghosts use the same visual pipeline as remote players will (aircraft rendered from snapshot, no direct sim coupling).
- [ ] Flight log format matches the eventual save format (see FEATURES.md § Flight Log & Save Format) — forward-compatible.

### Milestone 6.3: Sim-vehicle refactor (N vehicles)

- [ ] `SimState.vehicle: VehicleState` → `SimState.vehicles: ArrayVec<VehicleState, MAX_PLAYERS>`. Fixed capacity so no hot-path allocation.
- [ ] Camera becomes a render-side concern (one per peer, not in `SimState`).
- [ ] Input events gain a `player_id`; tick dispatch routes each input to the correct vehicle.
- [ ] Aircraft-aircraft collision detection (naïve AABB is fine for 4 players).
- [ ] Per-vehicle fuel, damage, livery, aircraft-type state.
- [ ] Every existing single-player scenario still passes the bench post-refactor (regression guard).

### Milestone 6.4: Multiplayer sim-rate policy

Wide-area lockstep at 120 Hz sim would need a 2-12 tick input buffer depending on RTT — perceptible but playable. Relaxed sim rate is an option for multiplayer specifically.

- [ ] Decide: keep 120 Hz sim with hybrid 60 Hz network-tick (inputs exchanged every 2 sim ticks), OR drop sim to 60 Hz in multiplayer sessions (requires coefficient re-tune or explicit acceptance of coarser physics).
- [ ] If 60 Hz: a config flag switches sim rate for multiplayer sessions; single-player remains 120 Hz. Scenario compatibility: single-player scenarios run at 120 Hz and cannot be directly replayed in a 60 Hz MP session — they're flagged as "single-player only" in the save format.
- [ ] Whichever choice: preserve determinism within the chosen rate. The determinism invariant scales to N peers regardless of rate.

### Milestone 6.5: Progression carry-over (SP → MP)

Single-player progress carries into multiplayer as a reward for completing the base experience. Keeps MP from being a parallel grind.

- [ ] Bring into MP: owned fleet (aircraft + liveries + hangar customizations), pilot character, accumulated wildlife/exploration unlocks, paint-shop creations.
- [ ] Do NOT bring: money, mission progress, hub discoveries (MP world is fresh / seeded separately — discoveries are per-world).
- [ ] MP-earned rewards optional: cosmetic unlocks earned in MP can be applied to SP fleet (celebrating shared experiences). Credits and reputation do NOT carry back (avoids MP-as-grind-tool-for-SP).
- [ ] Host selects world seed + dial state; joining players confirm the session config before joining (prevents silent downgrades).
- [ ] Aircraft selection at session start: each player picks from their owned fleet.
- [ ] Save format versioning handles the SP ↔ MP boundary cleanly.

### Milestone 6.6: Lockstep networking layer

- [ ] UDP transport via `renet` or `laminar` (both Rust-native, approved-stack candidates — require DESIGN.md amendment).
- [ ] Session discovery: LAN broadcast (zero-config friends-over-a-wifi) + lobby codes (wide-network friends).
- [ ] Handshake: protocol version, scenario-config hash, seed, dial state must all match before session starts.
- [ ] Input buffering: 2-6 tick sliding window, configurable per-session based on measured RTT.
- [ ] Tick-sync protocol: all peers confirm inputs for tick N collected before any peer executes tick N.
- [ ] Periodic state-hash exchange (every 60 ticks ≈ 1 s): all peers compare `state_hash()`. Mismatch = desync = session ends with clear reason.
- [ ] Disconnect handling: graceful if a peer drops (others continue); host-migration if the host drops.
- [ ] No anti-cheat beyond state-hash desync detection. Sessions are friends-list trust-based.

### Milestone 6.7: Multiplayer UI

- [ ] Session browser: LAN discovery list + "enter lobby code" for wide-network.
- [ ] Pre-flight lobby: player list, each player's selected aircraft, session config, ready state.
- [ ] In-session HUD additions: other players' callsigns overlaid on their aircraft at distance, player list with pings, chat box (text).
- [ ] Post-session summary: per-player stats, shared moments captured (landings, wildlife sightings, discoveries).
- [ ] Voice chat NOT in scope for stretch-goal phase; third-party (Discord, TeamSpeak) is the assumed channel.

### Milestone 6.8: Playtest + platform certification

- [ ] 2, 3, and 4 player sessions all stable over LAN.
- [ ] Wide-network stress test: cross-region sessions with 50-150 ms RTT; gameplay remains playable.
- [ ] NAT traversal / edge-case network conditions documented.
- [ ] Steam Deck MP viable (hosting and joining).
- [ ] Documentation: how to host, how to join, what progression carries over, what's shared.

### Deliverable

2-4 player flowPlan co-op sessions over LAN and wide network. Each player brings their single-player fleet + unlocks. Shared world, per-player progression. Physics identical across all peers by determinism invariant. Desyncs caught early via state-hash exchange. Friends-list trust model.

Spiritually: what single-player flowPlan feels like, shared with three friends.

> **Gate:** Phase 6 is complete when a 4-player wide-network session can run a full delivery mission from spawn to completion without desync, with each player bringing a saved single-player fleet, and the post-session state persists (with appropriate restrictions) back to their save files. Playtested by 10+ independent groups before release.

### What Phase 6 does NOT include

Stays explicitly out of scope even in the stretch-goal phase:

- Matchmaking service (no central servers; peer-to-peer only).
- Leaderboards, ranks, seasonal events.
- In-game voice chat.
- More than 4 players.
- PvP / competitive modes.
- Dedicated server support.
- Cheat detection beyond desync hashes.
- Anti-piracy / DRM.

All of the above would require additional scoping and are not promised.

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
- **Async ghost-plane sightings + live 2-4 player co-op**: now formalized as [Phase 6 (Stretch Goal): Multiplayer](#phase-6-stretch-goal-post-v1-multiplayer) above. Elevated from an open deferral to a concrete stretch-goal phase with entry criteria, because the determinism invariant makes it a natural (not retrofit) extension of flowPlan's design.
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
