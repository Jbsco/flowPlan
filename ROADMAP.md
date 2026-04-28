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
cargo test --workspace                       # includes Doc-tests blocks — scan output for them
cargo bench --bench frame_budget -- --test   # runs the budget check in CI mode
```

`cargo test --workspace` runs both unit tests and doctests, but the doctest summary lives in separate `Doc-tests <crate>` blocks at the end of the output. **Scan for those blocks specifically** — a unit-test pass does not imply a doctest pass, and CI fails on doctest regressions exactly like unit-test regressions. For f32 round-trip math at world scale, default doctest tolerance to ≥1e-2 absolute (or relative-aware bounds) — tighter tolerances exceed f32 precision at magnitude ~5000.

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

- [x] Accumulator in `app` drives `sim.tick(dt_fixed)` at 120 Hz
- [x] `SimState` triple-buffered; render reads front while sim writes back
- [x] Interpolation alpha passed to render each frame
- [x] Property test: deterministic state for fixed input sequence and seed
- [x] Property test: swap correctness under concurrent tick/render

### Milestone 1.2: Input

- [x] winit events -> `InputEvent` enum in main thread
- [x] Bounded `crossbeam::queue::ArrayQueue<InputEvent>` drained at tick start
- [x] Gamepad via `gilrs` (approved dep added in this phase if not in DESIGN.md's stack yet; amend DESIGN.md first)
- [x] Test: simulated input event sequence produces expected sim state

### Milestone 1.3: Camera

- [x] `Camera` in `sim` (position, orientation, fov, view-kind tag)
- [x] View kinds implemented: first-person cockpit (placeholder rig until a plane model lands in Phase 2), third-person chase, exterior orbit. Tail / wingtip / cowl views are Phase 2+ once the plane model provides reference points.
- [x] Free switching between view kinds at any time, bound to keys / gamepad face buttons
- [x] Short 60-120 ms camera transition tween with hold-to-hard-cut option
- [x] Chase-camera controller that tracks a target entity with configurable stiffness/damping
- [x] Camera interpolation uses the same alpha as scene interpolation
- [x] Right-stick look-around inside a view (POV-hat emulation) with center-snap-on-release; mouse-look on modifier
- [x] Per-view FOV setting and inertia tuning

### Milestone 1.4: Terrain

- [x] Heightmap loaded from a 16-bit PNG / RAW in `assets`
- [x] Terrain mesh generated on CPU into a static vertex/index buffer
- [x] Flat-shaded render pass (no cel shading yet; just verify geometry)
- [x] Terrain sample function in `world` exposed to `sim` (for future buoyancy)

### Milestone 1.5: Test / bench

- [x] Unit tests for `sim` tick determinism, input queue bounds, camera damping
- [x] Frame-budget bench on a flat terrain + cube scene: must hit 240 Hz on reference hardware

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
- [x] Wheels + straight floats variants modeled at minimum; amphib floats in the same milestone if feasible, otherwise Milestone 2.5 *(current model bundles both as the STL source; per-part swap comes later)*
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
    - [x] Close Gap 3 (Cl_max) — raise `CL_TABLE` peak from 1.65 toward ~2.0. Analytic target `Vs² = 2W/(ρSCl_max)` → ~2.95 for POH Vs; choose conservative first step. Ripples: stall scenarios, flap-stall, wing-drop, ground-effect, induced drag at high α.
    - [x] Close Gap 1 (Cd0 light) — raise `Cd0` from 0.045 toward ~0.055-0.060. Ripples: cruise equilibrium throttle, cruise-climb ROC, L/D_max glide ratio.
    - [x] Attempt coefficient-only fix for Gap 2 (P-factor). If over-corrects cruise rudder, mark as missing-physics (slipstream over rudder) and defer to post-2.4 roadmap item. This is an expected outcome — document it cleanly rather than forcing a fit.

    *Step 3 — Lock and commit references:*
    - [x] Narrow tolerances on all 9 scenarios based on post-tuning measured margins (2-3× actual margin per triage discipline).
    - [x] Commit reference plots to `docs/plots/beaver/<scenario>.png` (versioned artifacts, reviewed in PRs).
    - [x] Update [PROGRESS.md § Bench-identified gaps](PROGRESS.md#bench-identified-model-gaps-open-from-phase-248-audit): move closed gaps to "Resolved" subsection with final values; open-phase deferrals (slipstream physics) documented with target phase.
    - [x] Close out 2.4.8 with a section in PROGRESS summarizing final coefficient set, final scenario suite, and what Phase 2.5 inherits.

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
- [x] Property tests (close at end of 2.4):
    - Level flight at cruise power holds altitude within tolerance under zero wind
    - Published cruise speed achievable at published power setting within tolerance
    - Stall speeds (clean / flaps / floats) match POH figures within tolerance
    - Stall recovery within documented time at configured input sequence
    - Determinism: same seed + same input trace + same dial state -> bit-identical state
- [x] Validate against DHC-2 POH performance charts (takeoff distance, climb rate, cruise fuel burn) before milestone close
- [x] **Scratch-branch prototype of the flight-model feel belongs in Phase 1**; this milestone is where feel locks. If feel has not clicked by 4-6 weeks into this milestone, trim scope per MVP re-planning in [skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md).

### Milestone 2.4.9: Flight-model refinement (optional-before-2.5, required-before-v1)

Closes the two missing-physics items identified by the 2.4.8 bench audit. Optional-before-2.5 (Phase 2.5 contact physics doesn't depend on either item), required-before-v1 (both materially improve feel at high power / in turns, and unblock scenario-taxonomy entries currently tagged `[deferred]`). Can run between 2.4.8 and 2.5, after 2.5, or during Phase 3 — scheduled at the implementer's discretion.

Both items discovered via the triage protocol encoded in [skills/flowplan-flight-bench/references/model-gap-triage.md](skills/flowplan-flight-bench/references/model-gap-triage.md). Each required an explicit coefficient experiment to confirm the fix is *not* a coefficient change — which is exactly how the protocol is supposed to work.

**2.4.9a Slipstream-over-rudder physics.**
- Context: Gap 2 in [PROGRESS.md § Bench-identified gaps](PROGRESS.md#bench-identified-model-gaps-open-from-phase-248-audit). Real Beaver pilots at full power use 20-40% right rudder; our model only needs 5.5%. Confirmed via coefficient experiment (raising `P_FACTOR_ARM_M` over-corrects cruise while barely helping climb) that the issue is *rudder effectiveness scaling with thrust*, not P-factor magnitude.
- Implementation sketch: add slipstream term to yaw-moment calc. `q_local_at_rudder = q_freestream + slipstream_boost(thrust)`, where `slipstream_boost` is a function of prop-disc velocity increment (derived from thrust, prop area, freestream velocity). Rudder moment uses `q_local_at_rudder` instead of `q_freestream`. Similar boost applies to elevator (if behind prop) but smaller due to geometry.
- [x] Add `slipstream_boost(thrust, airspeed)` helper in `flight.rs`.
- [x] Modify yaw-moment calculation to use slipstream-augmented q for rudder term.
- [x] Optionally modify pitch-moment for elevator (if geometry supports).
- [x] Ripple-analyze + regression check per [triage protocol](skills/flowplan-flight-bench/references/model-gap-triage.md).
- [x] Re-tune cruise / climb scenarios if rudder-trim values shift (expected: cruise_climb rudder rises toward 0.25-0.30; cruise_sl slightly rises too).
- [x] Update Gap 2 in PROGRESS to "Resolved" with final slipstream parameters + scenario rudder-trim updates.

**2.4.9b Cl_β dihedral roll stability.**
- Context: discovered during 2.4.8 Step 1 when attempting turn scenarios. Without Cl_β (rolling moment from sideslip), a banked aircraft has no restoring roll moment — bank spirals rather than locking. Coordinated turn scenarios blocked.
- Implementation sketch: add `Cl_β` coefficient to flight.rs. Roll moment gains `q_dyn·S·b·Cl_β·β` term. Typical value for a Beaver-class GA single: Cl_β ≈ -0.07 to -0.10 per rad (negative = right-wing-down rolls induce nose-right yaw which induces nose-right β which induces right-wing-up rolling moment = restoring).
- [x] Add `CL_BETA` constant in `flight.rs`.
- [x] Add dihedral-roll term to roll moment calculation.
- [x] Ripple-analyze: unlikely to affect straight-line scenarios; may slightly dampen roll transients.
- [x] Add `beaver_coordinated_turn_30` scenario (previously deferred inline in `flight_bench.rs`). Validates turn holds at 30° bank without spiral.
- [x] Add `beaver_coordinated_turn_60` scenario. Validates accelerated-stall α calculation under 2 g load.
- [x] Update PROGRESS "Bench-identified gaps" section: remove turn-scenarios-deferred note.

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
- [x] Water-taxi scenario migrated to 2.5a.3 — single-point float sampling can't produce the fore/aft buoyancy restoring moment that real floats use to limit pitch-down under drag; 2.5a.3 multi-point sampling supplies it naturally. 2.5a.2 drag-in-isolation is validated by unit tests (vertical damping opposes motion, horizontal drag opposes motion, drag scales with wetted fraction).

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

**2.5a.4 Porpoising.** (SHIPPED — co-landed with 2.5a.3c on-step retune; pitch oscillation emerges from wetted-area × pitch-trim coupling without a separate scenario file.)
- [x] Pitch oscillation when elevator input mismatches step phase (classic pilot-induced oscillation)
- [x] Emerges from pitch-up / pitch-down coupling between wetted area and pitch trim — observed in `beaver_water_porpoise_emerge` and emergent during on-step phase of `beaver_water_takeoff`
- [x] Bench: `beaver_water_porpoise_emerge` captures the oscillation + damp-out

**2.5a.5 Asymmetric / one-pontoon maneuvers (validation).** (SHIPPED)
- [x] Bench scenarios exercise the asymmetric physics already shipped in 2.5a.1 (no new physics here — validation)
- [x] One-pontoon-on-water takeoff scenario: `beaver_water_takeoff_one_pontoon_{right,left}` — 5° bank reduces wetted area on the raised side, takeoff rollout shortens vs two-pontoon baseline
- [x] Left/right-banked pair for both takeoff AND landing (`beaver_water_landing_one_pontoon_{right,left}` with bank→level transition) — per `skills/flowplan-flight-bench/SKILL.md` Core Rule #10 asymmetric-pair discipline (right-rotating prop breaks mirror symmetry)

**2.5a.6 Water rudder + weather-vaning.** (SHIPPED)
- [x] Water rudder on/off input event (`InputEvent::SetWaterRudder(bool)`, `Controls.water_rudder`); deployed state adds an aft side-force scaling with `v_forward² × avg_wetted_fraction × yaw_input` at `WATER_RUDDER_ARM_M = 3.2 m` aft of CG
- [x] Weathervaning via existing aero `CN_BETA` when retracted — tested indirectly by `beaver_water_taxi_rudder_retracted` tracking heading under the aerodynamic stability that remains. Wind-field interaction deferred to Phase 3.2 as planned.
- [x] Bench: `beaver_water_taxi_rudder_deployed` + `beaver_water_taxi_rudder_retracted` validate heading response delta between the two states

**2.5a.7 Chop-intensity hook (Phase 3.2 integration point).** (SHIPPED)
- [x] `SimState.chop_intensity: f32` (default 1.0) scales the `water_height(x, z, t)` sample in `buoyancy::compute_buoyancy` — 0.0 = glassy water, 1.0 = nominal waves, 2.0+ = heavy chop. Scalar multiplication preserves phase and determinism.
- [x] Folded into `state_hash`; unit tests `chop_intensity_zero_freezes_wave_phase_effect` + `chop_intensity_amplifies_wave_phase_swing` verify the two invariants
- [x] Phase 3.2 `ChopState` wiring + visual `SceneParams.chop_intensity` are the remaining integration points; v1 ships with constant 1.0 = current behavior, preserving scenario reproducibility

**2.5a.X Visual polish (before 2.5a close; not sub-milestone-gated).**
- [ ] Wake spawn: particle + decal trail behind moving floats
- [ ] Spray effects at high speed or in chop, obstructing forward view
- [ ] Touchdown moment feels good in playtest (subjective, required before 2.5a gate)

**Non-global water heights (elevation lakes, rivers, sandbars for bush-pilot parking) are Phase 3+** per [FEATURES.md § Terrain & Static Landscape](FEATURES.md#terrain--static-landscape). The `world::water_height(x, z, t)` API is position-conditioned already, so the eventual hydrology rebuild plugs in without changing 2.5a's consumer contract. Sandbar parking specifically bridges 2.5a (water sampling near the bar) + 2.5b (soft-surface ground contact on the bar) + Phase 3+ shore classification.

**2.5b Ground contact (wheels / taildragger):**

Decomposed into 7 dependency-ordered sub-milestones mirroring 2.5a's structure. `VehicleConfig::Wheels` activates the entire stack; `Floats` short-circuits it. Geometry uses placeholder `BEAVER_GEAR` (3 contact points) anchored to DHC-2 real-world dimensions; mesh-alignment to `northlander_land.glb` bbox is a cross-cutting follow-up analogous to 2.5a.3's float realignment.

**2.5b.1 Per-gear normal force (spring-damper) + config enum.** (SHIPPED)
- [x] `VehicleConfig::{Floats, Wheels}` enum (default Floats preserves pre-2.5b behavior); `config: VehicleConfig` on `VehicleState`; folded into `state_hash`
- [x] `ground_contact::compute_ground_contact(v, spec, ground_y) -> GroundContactOutputs` — 3-point gear set, per-gear spring (200 kN/m) + damper (10 kN·s/m) compression model, world +Y normal force, body-frame moments from `r × F`
- [x] Short-circuits: no spec / `config != Wheels` / all gear above ground
- [x] Buoyancy still runs for Floats config only (mutual exclusion by config, not by runtime branching in the caller)
- [x] 8 unit tests: no-spec / Floats-config / airborne / at-rest weight-matching / symmetric no-roll-no-yaw / sinking adds damping / lifting never pulls / one-wheel-in-pit rolls restoring

**2.5b.2 Rolling resistance + longitudinal drag.** (SHIPPED)
- [x] Per-gear rolling friction at paved baseline `µ_r = 0.02`. Horizontal drag force `F_roll = -µ_r · N · v̂_ground` applied at each compressed gear, isotropic (forward and lateral modeled the same — cornering + lateral slip lands in 2.5b.3). Friction short-circuits at near-zero speed (1e-4 m/s threshold) so a stopped aircraft doesn't oscillate through sign flips.
- [x] `Terrain::flat_land()` sibling to `Terrain::flat()` — base_height = 0 so wheels-variant scenario visuals rest on solid ground instead of the submerged-terrain/water-surface combo used by floats scenarios.
- [x] App-side plumbing: `game_app.rs` resolves the aircraft selection first, then picks `Terrain::flat()` vs `Terrain::flat_land()` based on the selected variant's config; `Renderer::set_water_visible(false)` suppresses the water mesh for wheels scenarios so the y=0 water surface doesn't occlude the y=0 terrain.
- [x] Bench: `beaver_land_taxi` — wheels-only scenario at throttle 0.05, validates bounded steady-state taxi at ~5.2 m/s (matches real Beaver low-throttle paved taxi ~10 kt). Five new unit tests cover the rolling-friction behavior.

**2.5b.3 Lateral friction + tailwheel steering.** (SHIPPED)
- [x] Per-gear wheel-frame velocity decomposition: horizontal velocity at each gear split into (forward, lateral) components in the wheel's reference frame. Rolling friction (`µ_r = 0.02`) acts along -forward; lateral friction (`µ_lat = 0.6`, 30× rolling) acts along -lateral. Anisotropy IS what makes wheeled vehicles track straight. Both smoothly ramp through zero via `V_EPS = 0.1` m/s to prevent stopped-aircraft jitter.
- [x] Tailwheel steering: commanded heading = body-forward rotated by `yaw_input · TAILWHEEL_MAX_STEER_RAD (±30°) · lock_factor`. `lock_factor` smoothsteps from 1.0 (locked) below `V_STEER_FULL = 7.7 m/s` (~15 kt) to 0.0 (castoring) above `V_CASTOR_FULL = 10.0 m/s` (~20 kt) — matches Beaver's real behavior (locked at taxi, castors above rudder-takes-over speed). Lateral friction coefficient at the tailwheel also scales with `lock_factor` so a castoring tailwheel free-pivots with negligible lateral grip.
- [x] `Assertion::HeadingChangeAtEnd { last_secs, target_deg, tol_deg }` — new assertion type; short-arc wrap-safe. Primary use: taxi-turn / coordinated-turn scenarios.
- [x] Bench: `beaver_land_taxi_turn_{right, left}` asymmetric-pair scenarios — 0.5 yaw input at throttle 0.05 produces ±8.6°–8.8° heading change over 30 s at 3-point rest attitude. Tolerance ±3° covers coefficient-tuning drift.
- [x] 4 new unit tests on top of existing ground_contact coverage: lateral drag > forward drag at same slip, yaw-input-right produces nose-right moment, yaw-left mirror, castoring reduces tailwheel yaw authority at speed.

**2.5b.4 Brake input + elevator slipstream + player-extracurricular benches.** (SHIPPED)
- [x] `Controls.brakes: bool` + `InputEvent::SetBrakes(bool)` + snapshot bit + state.rs apply_inputs wire. Keyboard binding: momentary 'B' (press → on, release → off) in `input_map.rs`.
- [x] `GearPoint.brake_factor` (mains 1.0, tail 0.0) + `BRAKE_FRICTION_COEFF = 0.5` in `ground_contact.rs`. When `controls.brakes` and the gear is both brakable and compressed, an additional forward-axis drag `F_brake = -µ_brake · N · brake_factor` acts along -wheel_forward, smoothed through `V_EPS` like rolling.
- [x] **Elevator slipstream q-boost** (physics addition to `flight.rs`): the pitch-control moment now uses `q_local_at_elevator = q_dyn + slipstream_q_boost`, ungated by airspeed so the elevator retains authority at V=0. Basis for the tail-up-balance maneuver. Rudder remains airspeed-gated (`q_local_at_rudder`) because real Beaver rudder is ineffective at taxi speed — directional control comes from the tailwheel (2.5b.3 physics).
- [x] Gear damping raised 10k → 25k N·s/m (damping ratio ~0.75) to reduce the post-brake pitch-transient bounce; preserves all 2.5b.1-3 tests.
- [x] Bench: `beaver_land_braked_stop` — 5 m/s taxi + brakes held → decelerates to ≤4 m/s over 15 s (gear-geometry calibration limitation keeps it from reaching 0; assertion uses `FinalAirspeedWithin` as the v1 gate).
- [x] Bench: `beaver_land_tail_up_balance` — bush-pilot classic; brakes locked, 25% power, full forward stick. Aspirational scenario that captures current physics and serves as a visible regression target for the gear-calibration follow-up in 2.5b.5.
- [x] 3 new unit tests: brakes-off vs brakes-on decel ratio, brakes at v=0 produce no force.
- [x] Free side-effect: elevator slipstream fixes the previously-triaged `beaver_coordinated_turn_60_overbank` scenario. All 42 bench runs now pass.

**2.5b.5 Contact-physics calibration.** (SHIPPED)

Audited the 2.5b.4 `beaver_land_tail_up_balance` scenario and found three coupled issues plus **one root-cause bug**. The bug turned out to be a missing config check, not a calibration deficit — buoyancy was running on wheels-configured aircraft, applying phantom pontoon forces (+19 m/s² upward) that drove the at-rest drift. The other three issues (equilibrium, rebound damping, spring rate) were still real but masked by the buoyancy bug. See [PROGRESS.md § Milestone 2.5b.5](../PROGRESS.md) for the full diagnostic trace.

- [x] **Buoyancy config short-circuit.** Added the symmetric check to `compute_buoyancy`: `if !v.config.uses_buoyancy() { return default; }`. Pre-fix, `VehicleConfig::Wheels` + `BuoyancyOutputs` ran phantom-pontoon physics on wheeled aircraft, producing +19 m/s² when CG sat above the water-mean surface. This was the primary driver of the at-rest drift the 2.5b.4 audit chased. Caught by the new invariant test.
- [x] **Gear geometry — stable 3-point equilibrium.** Replaced equal-spring, equal-compression geometry (which was NOT a moment equilibrium) with equal-compression + softer tail spring. Tail's `spring_factor = 0.428` gives correct 82/18 main/tail weight distribution at the geometry-constrained pitch θ = atan2(0.6, 5.3) ≈ 6.46°. Both gears lose contact simultaneously on upward perturbation (stable, not metastable). Real Beaver tailwheels ARE softer than main oleos — the softer spring is physically grounded, not a fudge factor.
- [x] **Asymmetric damping — rebound ≫ compression.** Split the single `GEAR_DAMPING_N_PER_MPS` into `GEAR_DAMPING_COMPRESSION = 25,000` and `GEAR_DAMPING_REBOUND = 75,000` (3:1). Compression side absorbs impact energy moderately; rebound bleeds it out aggressively to prevent trampoline. Real oleo struts behave this way via a one-way hydraulic orifice.
- [x] **Invariant test: `wheels_at_rest_has_zero_drift`.** Wheels aircraft at declared rest with zero inputs + zero fuel (isolates gear/buoyancy loop from idle thrust creep) drifts no more than 0.5° in pitch and 5 cm in position over 2 s. This test guards the calibration against regression.
- [x] **`wheels_three_point_rest` helper updated** to the stable equilibrium (θ=6.46°, CG=1.155 m) — used by the steering-related unit tests.
- [x] **Bench scenarios retuned** for the new physics state: `beaver_land_taxi_turn_{right,left}` shortened to 10 s (aero yaw damping is zero at V=0, so turn rates compound over 30 s under sustained rudder — 10 s captures early-turn behavior without degenerating into a continuous pivot). `beaver_land_braked_stop` + `beaver_land_tail_up_balance` pass cleanly at their existing `FinalAirspeedWithin` gates.

**Deferred to a later pass** (not blocking 2.5b.6/7/8): progressive spring rate (`F = k·c + k_hard·max(0, c-c_soft)²`) and velocity-dependent damping (`c(v) = c_0 + c_2·v²`). The asymmetric damping + corrected equilibrium + buoyancy fix cleared the observable symptoms; the remaining compliance work is feel-polish that stays shippable-even-without-it.

**Tail-up maneuver on the Beaver — confirmed not achievable with this airframe:** after calibration, the `beaver_land_tail_up_balance` scenario shows the aircraft rock-solidly planted (pitch 6.46° → settles to 6.30°, airspeed ≈ 0, zero position drift). Elevator authority at 25% throttle produces ~2,700 N·m nose-down; the gear restoring moment if the tail were to lift is ~21,000 N·m. The Beaver's mass + moderate power-to-weight ratio make tail-up physically infeasible at practical throttle settings in this model. The scenario stays in the bench as a regression target: when Super Cub / Carbon Cub (250 W/kg) airframes land in Phase 5.1, their lighter mass + relatively higher power will drive the maneuver to completion, validating the physics with the right airframe. The Beaver-specific caveat documented in [PROGRESS.md § 2.5b.4 follow-up audit](../PROGRESS.md) is now empirically confirmed.

**2.5b.6 Attitude / strike detection.** (SHIPPED)

- [x] Pitch / roll attitude emerges from multi-point gear. Guard test `wheels_pitch_converges_to_rest_from_perturbation`: spawn at +10° pitch, zero velocity, zero inputs, zero fuel. Aircraft rotates back to 6.46° rest within 5 s with no overshoot beyond 5°. Validates that the per-gear `r × F` moment shaped by the 2.5b.5 calibration produces stable attitude behavior, not just a stable rest.
- [x] **Strike detection** (detection only — damage is Phase 4+). `StrikeFlags { prop, tail, wingtip_left, wingtip_right }` on `GroundContactOutputs`, set each tick when the corresponding `BEAVER_STRIKES` body-frame point's world-y drops below `ground_y`. Four points: prop (3.0 m forward, 1.0 m below), tail (5.5 m aft, 0.3 m below — NOT the tailwheel; the rear-fuselage underside sits above the tailwheel at rest), wingtips (±7.3 m lateral, 1.0 m below). Strike detection is wheels-only (short-circuits on floats config, mirroring the buoyancy symmetry).
- [x] `SimState.last_strikes` carries the latest tick's strike flags through to bench CSV + debug HUD + future Phase 4 damage system.
- [x] Four new `BenchSample` CSV columns: `strike_prop`, `strike_tail`, `strike_wingtip_left`, `strike_wingtip_right` (u8: 0/1). Per-tick scenario strike history available for offline analysis.
- [x] New bench assertions: `TailStrikeOccurs`, `PropStrikeOccurs`, `WingtipStrikeOccurs` (any wingtip) — pass if the scenario log recorded at least one tick with the corresponding strike.
- [x] 7 new unit tests: no strikes at rest; floats-config produces no strikes; over-rotation → tail strike; nose-down → prop strike; left-bank → left wingtip; right-bank → right wingtip; airborne aircraft produces no strikes regardless of attitude.
- [x] Bench: `beaver_land_stall_taildown` (tail-first landing simulated by spawning the aircraft at 15° nose-up close enough to the ground that the tail-strike point is immediately in contact) and `beaver_land_wingtip_scrape` (aircraft in 15° combined pitch+bank attitude with wingtip contacting ground). Both pass the corresponding assertions.
- [x] **Empirical finding**: the Beaver's gear restoring moment at rest is ~33,000 N·m per degree past 6.46°; full-power elevator (+10,000 N·m nose-up) cannot over-rotate the aircraft through back-stick alone. Tail-strike in this model happens from airborne attitudes descending into contact, not from elevator-driven rotation on-ground. Matches the Beaver-specific caveat already documented for tail-up-balance.

**2.5b.7 Per-surface friction table.** (Was 2.5b.6. Deferred — blocks on terrain surface-classification work that doesn't exist yet.)

The current ground-contact model uses a single paved-baseline coefficient set (`ROLLING_FRICTION_COEFF = 0.02`, `LATERAL_FRICTION_COEFF = 0.6`, `BRAKE_FRICTION_COEFF = 0.5`). Per-surface variation wants a `world::surface_at(x, z) -> SurfaceClass` sampler that doesn't exist yet — terrain is still a single procedural heightfield with no surface class, lakes, rivers, or shore classification (per [FEATURES.md § Terrain & Static Landscape](FEATURES.md#terrain--static-landscape) Phase 3+ hydrology rebuild). Keeping 2.5b.7 on the ROADMAP as a slot, but ordering it AFTER 2.5b.8 (app integration) since 2.5b.8 is unblocked and 2.5b.7 is blocked:

- [ ] `SurfaceClass` enum (`Paved | Grass | Gravel | Sand | Tundra | Water`), world-side sampler at `(x, z)`.
- [ ] Per-surface coefficient lookup: `µ_r` rolling, `µ_lat` lateral, `µ_brake`, compliance softness. Tundra should feel genuinely soft (mains sink, planing effect during takeoff); sand should feel draggy; paved tight.
- [ ] Bench: paved-vs-grass taxi comparison at identical throttle → grass decelerates aggressively. Soft-field takeoff scenario where the aircraft rides on the step briefly.

**2.5b.8 App integration: config toggle + land-mesh selection.** (SHIPPED)

Most of the app-integration plumbing landed alongside the earlier catalog work — the `--aircraft` / `--variant` CLI flags, `FLOWPLAN_AIRCRAFT` env var, `AircraftCatalog` auto-scan, and variant-aware mesh + terrain selection all shipped during the catalog implementation. 2.5b.8 closes the remaining gap: **free-flight mode's `min_altitude` computation**, which up to now was config-agnostic and assumed a float-offset.

- [x] App CLI + env-var selector (`--aircraft <id>` / `--variant <id>` / `--list-aircraft`; env `FLOWPLAN_AIRCRAFT=<id>[:<variant>]`) — shipped with the catalog.
- [x] Variant-aware mesh selection: catalog entry's `variants[].mesh_path` drives the `.glb` loaded at startup; `sim.vehicle.config` is set to the variant's `config` field.
- [x] Variant-aware terrain + water suppression: scenario-playback mode picks `Terrain::flat_land()` (wheels) vs `Terrain::flat()` (floats); renderer's `set_water_visible(false)` suppresses water for wheels scenarios so the y=0 terrain isn't occluded.
- [x] **Per-config `min_altitude` in free-flight mode (new in 2.5b.8):** wheels config uses `terrain_here + SURFACE_MARGIN` (gear physics handles the rest); floats config keeps `terrain_here + vehicle_float_offset + SURFACE_MARGIN` (pontoon clearance). Pre-fix, the wheels variant in free-flight had its ground physics seeing an "elevated phantom ground" ~1.69 m above the rendered terrain, so gear never reached contact on descent.
- [x] Smoke-tested: `flowplan --aircraft northlander --variant wheels` boots cleanly in free-flight mode. `flowplan --scenario beaver_land_taxi` auto-resolves to wheels variant and works. Pre-existing water scenarios (`beaver_water_takeoff`) still route to floats config correctly.

**Deferred to playtest pass:** full "can taxi, takeoff, land, park the wheeled Beaver — feel matches expectations for a taildragger" — the free-flight spawn still starts airborne at 25 m AGL cruising at 55 m/s, which exercises ground-contact only on landing. A dedicated "start on ground, ready to taxi" spawn-mode toggle could land with settings UI in Phase 4.

**2.5c Invariants + tests.** (SHIPPED)

Mostly consolidation — the four invariants were ALREADY covered by tests that grew organically during 2.5a / 2.5b implementation. 2.5c formalizes the coverage and adds the missing contact-path determinism proptests.

- [x] **Determinism preserved** — contact forces are pure functions of `(state, ground_y, dt)`. Three proptests in `hash.rs`:
  - `prop_same_seed_same_trace_same_hash` (Phase 2.4, general) — 32 random cases per run.
  - `prop_wheels_on_ground_same_seed_same_trace_same_hash` (new in 2.5c) — aircraft spawned at 3-point rest with wheels config, gear physics fires every tick; 32 random cases.
  - `prop_floats_on_water_same_seed_same_trace_same_hash` (new in 2.5c) — floats at water-rest equilibrium, buoyancy + wave-sampling fires every tick; 32 random cases.
- [x] **Stationary on level ground for 10 s** — `wheels_at_rest_has_zero_drift` extended from 2 s (2.5b.5 guard) to 10 s (1200 ticks at 120 Hz). Asserts pitch drift < 0.5° AND position drift < 10 cm. Fuel=0 so idle-thrust creep doesn't conflate.
- [x] **Taxi reaches steady speed on paved** — `beaver_land_taxi` bench scenario: idle-plus throttle at 3-point rest, 30-s run, `AirspeedAtEnd target_mps: 5.2, tol: 3.0`. Not runaway, not stationary — equilibrium between thrust and rolling friction. Matches real Beaver low-throttle paved taxi (~10 kt).
- [x] **Float plane settles to equilibrium** — `state::tests::float_at_rest_settles_to_equilibrium` (from 2.5a): Beaver on floats dropped at y=4.0 (above analytic equilibrium 2.40 m) settles under damped buoyancy-gravity to the correct height, matching displacement.
- [x] **"No wall-clock in contact path"** — confirmed by code review: `compute_ground_contact` takes `(state, ground_y)`; `compute_buoyancy` takes `(state, wave_time, chop_intensity)` where `wave_time` is a deterministic scalar advanced by `dt` each tick. No `Instant`, no `SystemTime`, no hidden RNG. The proptests above would detect any wall-clock leak (parallel same-seed runs would diverge).

Phase 2.5 (ground + water contact physics) is now complete. 2.5a water physics + 2.5b.1-2.5b.6 + 2.5b.8 ground physics + 2.5c invariants. 2.5b.7 (per-surface friction) deferred pending terrain surface classification (Phase 3+ terrain work).

### Milestone 2.6: Sky (daytime foundation) -- COMPLETE

- [x] Procedural sky: analytic zenith-to-horizon gradient + warm sun band + sun disc / halo in `sky.wgsl`. Full Preetham / Hosek-Wilkie deferred; current stylized model fits cel-shading aesthetic and ships cheap. Pre-scene pass.
- [x] Sun direction tied to sim time-of-day: `SimState.time_of_day: f32` in `[0, 1)` advances per tick at `DAY_LENGTH_SECONDS = 1200` (20 min symmetric cycle); `world::sun_direction(t)` derives the unit vector; app writes into `SceneParams.sun_dir` each frame.
- [x] Exponential height fog with sun tint in the scene fragment shader (distance-based `1 - exp(-density * dist)`, biased toward `sun_color` along the view-to-sun axis).
- [x] Celestial-sphere hook placed (reserved slot `SceneUniform.ambient.a`); populated in Phase 5 Milestone 5.6.

### Deliverable

A plane can be flown, landed on water, taxied, taken off again. Cel-shaded look is locked. Water is the centerpiece. Frame budget holds at 240 Hz on the reference hardware.

> **Gate:** Phase 2 is complete only when the quality gate passes, visual snapshot tests match approved references, the frame-budget bench holds, and a playtest confirms the core feel. Flight model arcade vs. sim-lite decision recorded as an amendment in [DESIGN.md](DESIGN.md#open-design-questions).

---

## Pre-Phase-3 prerequisites

A small set of design / prototype tasks that land **between Phase 2.5 close-out and Phase 3 opening**. None of these individually are phase-sized, but they resolve decisions that would otherwise block later work if deferred. They can run in parallel while Phase 3 implementation work starts — they're prerequisites for *later* Phase 3 / 3.6 milestones, not for Phase 3's start.

### World-scope prototype (unblocks Phase 3.6 + Phase 4)

- [ ] **Scratch-branch prototype of Candidate C (fresh-per-session, no persistence) and Candidate D (hybrid persistent hubs + procedural surroundings)** per [FEATURES.md § World Scope & Map Layout](FEATURES.md#world-scope--map-layout). Two short iterations, not a polished implementation.
- [ ] **Primary architectural hypothesis to validate:** the **Voronoi-chunk + sub-biome-feature + fuzz+resolve-hub** architecture pinned in MAPGEN.md. Specifically:
    - Decoupled streaming disk (10 km outer at Medium preset, slider-exposed to Low/Medium/High/Ultra/Custom per [FEATURES.md § Display & Graphics Settings](FEATURES.md#display--graphics-settings)) vs Voronoi chunk shape.
    - Blue-noise seed distribution at ~2 km average spacing; cell size varies by sub-biome feature type.
    - Sub-biome feature overlap into neighboring biomes with palette adaptation.
    - World-bake hub catalog (~50–80K hubs for a 1000 × 1000 km world) with approximate-coord fuzz + streaming-time resolve to nearest compatible Voronoi cell.
    - If the prototype surfaces a performance wall or aesthetic problem, the architecture bends or falls back to hex/square lattice — but Voronoi is the hypothesis to test first.
- [ ] Measure: world-bake time (hydrology graph + biome mask + Voronoi partition + hub catalog), persistence footprint per play-hour (Candidate D deltas only), frame-budget impact at the 10-km streaming outer radius, subjective replay appeal after 2–3 fresh sessions on each candidate.
- [ ] **Deterministic / telemetric validation** via the **visual-bench pattern** (see [TEST_PLAN.md § World-Gen Visual Bench](TEST_PLAN.md#world-gen-visual-bench)): emits PNG artifacts per layer (heightfield / biome / hydrology / hub catalog / terrain Voronoi / wind field / weather Voronoi / composite) committed to `docs/world-gen-plots/<scenario>/<seed>/`. Determinism assertions: byte-exact biome classification, SSIM > 0.99 heightfield cross-run, graph-structural equality for hydrology + frontal-boundary graphs. Cross-platform scope limited to Linux + Windows 10 (research-based Mac/Deck per `hardware_access_constraints`).
- [ ] **Free-camera debug mode (dev-only, `#[cfg(debug_assertions)]`)**: detach camera from aircraft, WASD / mouse-look, toggleable overlay for Voronoi cell boundaries + hub catalog + hydrology + weather cells. In-engine complement to the static PNG artifacts. Same camera primitive evolves into [Phase 5.5 photo mode](#milestone-55-visual-polish) and [Phase 6 stretch-goal multiplayer spectator](#phase-6-stretch-goal-post-v1-multiplayer).
- [ ] Output: amendment PR to FEATURES.md § World Scope + MAPGEN.md § Chunk model + MAPGEN.md § Hub late-binding. Decision locked. If a completely different architecture wins, the amendment documents that instead.

### Cross-platform determinism smoke test (alongside 3.4 CI)

Phase 2.5 bench scenarios have run Linux-only. Before Phase 3.6 world-bake work ships (hydrology graph + Voronoi partition + hub catalog all demand bit-identical cross-platform output), validate that the existing flight bench produces byte-identical CSVs across available hardware.

- [ ] Run `cargo test -p flowplan-sim --test flight_bench --release` on **multiple Linux machines** (different distros / CPU generations / Intel + AMD where available). Byte-diff CSVs. Goal: catch same-ISA different-microarch drift.
- [ ] Run same bench on **Windows 10** target. Byte-diff against Linux output. Catches MSVC-CRT vs glibc transcendental drift + FMA contraction defaults.
- [ ] **Pin build config as insurance**: adopt `libm` crate in sim hot paths (pure Rust, platform-independent transcendentals), set `-Ctarget-feature=-fma` or equivalent, verify `codegen-units=1` + `lto="thin"` already pinned. Pull forward from Phase 6.1 determinism hardening.
- [ ] **Research-based Mac / Steam Deck coverage:** since those platforms aren't directly testable at this time (see user-memory `hardware_access_constraints`), survey public game-dev + scientific-computing determinism reports for Apple Silicon (ARM / NEON) and AMD APU drivers. Document findings + risks. Full verification of those platforms deferred to [Phase 6.1 determinism hardening](#milestone-61-determinism-hardening-cross-platform) when hardware access lands.
- [ ] Output: cross-platform bench report committed to `docs/determinism-report.md`; any fixes from the Linux/Windows smoke test (likely minimal with libm + fp-contract pinned); documented risk register for Mac/Deck pending direct test.

### Weather implementation reference (pre-3.2x → SHIPPED 2026-04-25)

[**`docs/WEATHER.md`**](../docs/WEATHER.md) — implementation reference companion to [MAPGEN.md § Three-layer weather composition](../MAPGEN.md#three-layer-weather-composition-second-application-of-the-pattern). Originally scoped as `docs/CLOUDS.md` (a focused cloud-rendering research memo) but expanded to cover the surrounding weather subsystem context — particle system architecture, fog + visibility coupling, sim cloud-fly-through hookups, atmospheric optics deferral — because cloud-rendering decisions can't be made in isolation from those neighbors. Pattern matches MAPGEN.md → WORLD_GEN.md (design doc + implementation reference).

**Critical requirements pinned (also captured in WEATHER.md):**

- **Fly-through semantics:** players will invariably fly through low clouds. Billboard-based approaches fundamentally cannot render camera-inside-cloud — rejected. Raymarched volumetric is required. See `cloud_flythrough_requirement` memory.
- **CPU-samplable density field:** the sim must query cloud density each tick for turbulence (scales with density-gradient magnitude), visibility reduction (hundreds of meters inside dense cloud), and icing probability (cold + moist cloud interior). Implies **CPU-GPU shared noise field** — same Perlin-Worley formula runs on both sides, bit-identical by construction. Pattern matches `world::water_height(x, z, t)` from Phase 2.2.

- [x] Survey references: Schneider & Vos "The Real-Time Volumetric Cloudscapes of Horizon: Zero Dawn" (SIGGRAPH 2015); Andrew Schneider GDC follow-ups at Guerrilla; Frostbite "Physically Based Sky, Atmosphere and Cloud Rendering" (SIGGRAPH 2016); Sebastian Lague's accessible YouTube walk-through. *(References pinned in WEATHER.md; survey work happens during 3.6.3c.6 / 3.2x implementation as needed.)*
- [x] Pick v1 scope: Perlin-Worley noise field (2-3 octaves), raymarch 32-48 steps near / 16-24 far, banded-alpha via quantized density sampling for cel-shading compatibility, simple sun-lit vs shadow-side shading (no multi-scatter). *(Locked in WEATHER.md § Cloud rendering technique.)*
- [x] Pin CPU-GPU shared noise architecture: shared formula, shared parameters, deterministic from master seed; `world::cloud_density(x, y, z, t)` signature for sim consumption. *(Already shipped in `crates/world/src/weather/deltas.rs` per Phase 3.2.3; CPU side complete, GPU port lands in 3.6.3c.6.)*
- [x] Defer beyond v1: cloud shadows on terrain, volumetric light scattering through gaps, multi-layer cloud decks. *(Plus atmospheric optics — halos, sun dogs, rainbows, crepuscular rays — deferred to Phase 5.5 polish per WEATHER.md.)*
- [x] Output: `docs/WEATHER.md` design + implementation reference committed; 3.6.3c.6 + 3.2x implementation use it as the spec.

### Design-doc closures (pre-Phase-4 commitments)

Tracked under [Pre-Phase-4 design-doc commitments](#pre-phase-4-design-doc-commitments) at the end of Phase 3.6. Flagged here because they can start before Phase 3 if the author has bandwidth — the world-scope prototype is the only hard pre-Phase-3 gate; the other design docs land opportunistically.

### Gate

> **Gate (Pre-Phase-3):** Phase 3 implementation work can start without these complete — the only hard block is that **Phase 3.6 cannot open until the world-scope prototype lands and its amendment PR merges.** Cross-platform determinism smoke + cloud research memo land alongside early Phase 3 work (3.4 CI + 3.2x kickoff). This gives Phase 3 audio / weather / thread / CI work a runway while the prototype happens on scratch branches.

---

## Phase 3: Systems

**Goal:** audio spine, weather state, dedicated sim thread, CI pipeline. The game becomes a living system.

### Execution order

Milestones below are listed in numeric order for presentation; **implementation order is not numeric** because of cross-milestone dependencies + parallel-development opportunities. Recommended sequence:

1. **3.4 CI** — full fmt / clippy / test / frame-budget gate on every push, on top of the Pre-Phase-3 cross-platform determinism smoke workflow. Unblocks all downstream work with automated regression detection.
2. **3.2.1 Layer W1 atmospheric base** — pressure / temperature / humidity / altitude-stratified wind / frontal-boundary graph / terrain-induced turbulence. Ships playable alone per three-layer-composition pattern. Exposes `world::wind_at` + `world::turbulence_at` + related samplers.
3. **World-scope prototype** (scratch branch, parallel with 3.4 / 3.2.1). Validates the pinned Voronoi + three-layer + hub-catalog architecture before Phase 3.6 needs to start. Non-blocking on Phase 3 proper.
4. **3.1 Audio spine + 3.3 Sim worker thread (co-landed)** — audio SPSC ring from sim-thread is the forcing function for 3.3's refactor. Doing them together means the sim-thread design is driven by a concrete consumer (audio callback reading `AudioContext` snapshots that sim pre-computes per tick from Voronoi / weather / engine / position state). Separating 3.3 from 3.1 would force audio to consume shared-state-with-careful-locking on main thread — strictly worse architecture. `AudioContext` struct (engine params + cell IDs + proximity + event flags) is designed here.
5. **3.2.2 Layer W2 Weather Voronoi partition** — blue-noise seeds, per-cell state, adjacency-bias evolution, temporal drift. Consumes W1. Cells feed audio snapshots via the 3.1 SPSC ring (weather cell ID + storm state → audio) → richer soundtrack layers unlock automatically.
6. **3.2.3 Layer W3 deltas + weather-affects-flight + 3.2b chop wiring** — per-cell wind/turbulence/precip deltas; wires weather into flight model (turbulence, visibility, icing); closes the 2.5a.7 chop hook. Exposes `world::cloud_density` + `world::precipitation_at`.
7. **3.5b FFT ocean** — builds the compute-pipeline abstraction that 3.2x clouds + 3.6.2 hydrology bake + 5.6 aurora all later consume. Design the abstraction with those downstream consumers in mind (see the 3.5b infrastructure-dividend note).
8. **3.2x Weather visuals + particle system + volumetric clouds** — consumes 3.2 W3 density fields + 3.5b compute abstraction + the pre-3.2x CLOUDS.md design memo.
9. **3.5 Soundtrack** — Glicol-graph library layered on top of 3.1 audio spine. Per-biome motifs, storm-cell proximity layers, hub-approach radio, altitude-band pads, wildlife-zone pan — all driven by the `AudioContext` snapshots produced by sim thread.
10. **Phase 3 gate closes** once all above are green against the quality gate + CI smoke. Cross-platform determinism report committed (within-platform verified; cross-platform deferred to 6.1 per Phase 3 Step 1 findings).
11. **World-scope prototype results lock** the world-scope decision + chunk-shape decision. Amendment PR merges.
12. **Phase 3.6 opens** with Voronoi + three-layer + hub-catalog architecture validated.

Parallel-development opportunities:
- **3.4 CI / 3.2.1 W1 / world-scope prototype** can all run simultaneously — independent work surfaces.
- **3.1 audio + 3.3 sim-thread** must co-land per above; can't be safely separated.
- **3.5b compute infrastructure** must complete before 3.2x starts (unless 3.2x develops the compute abstraction standalone, in which case 3.5b inherits it — symmetrical option).
- **3.5 soundtrack** is downstream of 3.1 audio; but its Glicol patch authoring (per-biome motifs, etc.) can start whenever the `AudioContext` schema lands in 3.1.

**Why 3.3 co-lands with 3.1 rather than as its own step:**
3.3's stated benefits (determinism under threading, latency, Phase 6 readiness) are **future-proofing**, not solving any current pain — sim tick is ~0.5 ms of the frame budget, below the threshold where threading is forced. The audio SPSC ring in 3.1 is the first consumer that genuinely requires sim-thread separation (lock-free audio callback reading from a thread that owns `SimState`). Doing 3.3 as a standalone step means inventing an abstraction without a concrete consumer; doing 3.3 together with 3.1 means the audio schema informs the sim-thread boundary. The co-land is the cheaper + less speculative path.

The order is **non-binding** where milestones are genuinely independent. The hard dependencies are: (a) 3.4 CI before Phase 3 gate closes; (b) 3.5b or 3.2x builds compute abstraction before the other consumes it; (c) Pre-Phase-3 world-scope prototype lands before Phase 3.6 opens; (d) 3.1 audio and 3.3 sim-thread co-land.

### Milestone 3.1: Audio spine

- [ ] `cpal` stream in `audio` crate
- [ ] `glicol_synth` graph construction at startup
- [ ] `rtrb` SPSC ring between sim and audio
- [ ] Engine sound: Glicol patch modulated by RPM, load, airspeed. **Reference aircraft (MVP) is the DHC-2 Beaver's Pratt & Whitney R-985 radial** -- 9-cylinder pattern, distinctive low-RPM rumble, flagship audio example for the project. Other engine types (flat-six, turbine) become patch variants once the radial signature is locked.
- [ ] Wind noise: filtered noise patch keyed to airspeed
- [ ] Water lapping: procedural patch near shoreline
- [ ] Callback invariants verified by hot-path allocation test and logging-free lint

### Milestone 3.2: Weather state

Weather is architected as a three-layer composable subsystem paralleling the terrain three-layer (see [MAPGEN.md § Three-layer weather composition](MAPGEN.md#three-layer-weather-composition-second-application-of-the-pattern)). Split into three sub-milestones matching W1/W2/W3 — each ships playable on its own, enabling incremental validation.

### Milestone 3.2.1: Layer W1 — Atmospheric base field

**Ships W1 alone. Playable world with real atmospheric state — wind affects flight, density altitude varies with weather, no clouds or weather systems yet.**

- [ ] **Pressure field** (latitude + seasonal + regional patterns) driving prevailing-wind derivation via geostrophic balance.
- [ ] **Temperature + humidity fields** (latitude + elevation + season + diurnal).
- [ ] **Altitude-stratified prevailing-wind field** — low-altitude wind differs from wind aloft; will drive the W2 temporal drift offset in 3.2.2 (clouds at different altitudes drift at different rates).
- [ ] **Frontal-boundary graph** pre-baked as a line-structure graph analogous to the L1 hydrology river graph. Cells along a frontal line will bias toward frontal-passage cell types in W2 (3.2.2). Immutable across W2/W3.
- [ ] **Terrain-induced turbulence map** derived from L1 topographic gradients (lee-side rotors, valley channeling, ridge thermals). Static per master seed after terrain bakes.
- [ ] **Consumer API sketch** (see [MAPGEN.md § Consumer API surface](MAPGEN.md#consumer-api-surface)): expose `world::pressure_at(x, y, z)`, `world::temperature_at(x, y, z, t)`, `world::humidity_at(x, y, z, t)`, `world::wind_at(x, y, z, t)` (W1 baseline, pre-W3-delta), `world::turbulence_at(x, y, z)` (W1 baseline).
- [ ] **Visual-bench artifacts** (per [TEST_PLAN.md § World-Gen Visual Bench](TEST_PLAN.md#world-gen-visual-bench)): pressure / temperature / humidity / wind-vector / frontal-boundary-graph / terrain-turbulence PNG plots committed to `docs/world-gen-plots/`.
- [ ] **Determinism + cross-platform:** byte-identical W1 fields across Linux + Windows given same master seed.

### Milestone 3.2.2: Layer W2 — Weather Voronoi partition

**Adds W2 semantics on top of W1. Playable world: weather cells exist as data + debug-overlay, but no cloud rendering or physics delta yet.**

- [ ] **Weather Voronoi bake** (see [MAPGEN.md § Weather Voronoi layer](MAPGEN.md#weather-voronoi-layer-second-partition)): ~20–50 km average seed spacing, ~11K cells world-wide, blue-noise + Fortune's sweep at world-bake. Independent seed stream from terrain Voronoi.
- [ ] **Per-cell state**: `{cell_type, base_altitude, top_altitude, density, column_height, precip_type, storm_state, evolution_phase}`. Placement constraints check against W1 (pressure range, humidity threshold, temperature range, wind-shear threshold); bad placements re-roll.
- [ ] **Temporal drift** via altitude-dependent W1 wind offset — weather moves past stationary observers at prevailing wind speed, layer-stratified.
- [ ] **Adjacency-biased evolution** on slow cadence (once per sim-second; folds into `state_hash` for determinism); clear-air cells first-class (`no-weather is weather`). Frontal-boundary-graph lines bias cell evolution along fronts.
- [ ] **Debug overlay** via the free-camera debug mode (`#[cfg(debug_assertions)]`) — cell boundaries + cell-type color coding + storm-state labels. Doubles as visual-bench artifact.
- [ ] **Visual-bench artifacts:** weather Voronoi partition snapshots at t₀ / t+10min / t+1h / t+6h; verifies drift + evolution + adjacency bias.
- [ ] **Determinism + cross-platform:** byte-identical partition + cell state trajectory across platforms given same master seed + tick count.

### Milestone 3.2.2b: Orographic + rain-shadow humidity coupling (near-term addition)

**Small realism win before wiring weather into flight. Windward slopes see boosted humidity → lower cloud base, more convective cells; lee sides desiccate → high LCL, clear-air-biased. Source of every high-desert-next-to-snowy-range biome.**

- [ ] `terrain_moisture_modifier(terrain: &T, wind: Vec3, x, z) -> f32` returning humidity bias in ~`[0.5, 1.5]`. Pattern mirrors `turbulence_at` lee-side factor: terrain-gradient dotted with wind direction; scaled by terrain elevation.
- [ ] New `humidity_at_with_terrain` variant (keep old API for tests); cell-state classification consumes the terrain-aware humidity.
- [ ] Visual-bench artifact: humidity-with-terrain heatmap alongside existing humidity-sea-level.
- [ ] Unit tests: flat terrain → no bias; windward of high peak → boost; lee of high peak → reduce.
- [ ] Determinism: bit-identical modifier given same `(master_seed, terrain, wind, x, z)`.

Memory: `weather_orographic_rain_shadow` captures the rationale + implementation template.

### Milestone 3.2.3: Layer W3 — Cell atmospheric deltas + weather-affects-flight

**Adds W3 deltas + wires weather into flight physics. Weather is now functionally present in the sim — turbulence, visibility, icing all respond to cloud density + storm state.**

- [ ] **Per-cell delta functions** for wind / turbulence / visibility / precipitation (see MAPGEN.md § W3 table). Storm cells add local updrafts/downdrafts/gusts; clear-air zero. Frontal cells add horizontal wind shift across frontal line.
- [ ] **Adjacent-cell edge-interpolation** for organic transitions — density / wind / turbulence lerp across Voronoi edges.
- [ ] **Consumer API extension:** `world::cloud_density(x, y, z, t)`, `world::wind_at(x, y, z, t)` (W1 + W3 composed), `world::turbulence_at(x, y, z, t)` (W1 + W3 composed), `world::precipitation_at(x, y, z, t)`. Shared CPU-GPU interface for 3.2x cloud rendering + 2.5 turbulence hooks + 4.5 mission capability.
- [ ] **Frontal-boundary-graph-immutable invariant enforced:** W3 deltas cannot place weather inconsistent with W1 pressure/frontal structure. Verified at bake-time by rejecting incompatible cell types at W2.
- [ ] **Precipitation regime** (drizzle → heavy_rain → squall → light_snow → blizzard → hail) per cell type + temperature. Rain / snow particle rendering ships in 3.2x; 3.2.3 just exposes the rate.
- [ ] **Fog / mist state ladder** (ground_mist → wingtip_mist → layer_fog → dense_fog); derives from W1 humidity + temperature + W3 cloud-base altitude. Visual render in 3.2x.
- [ ] **Debris regime** (dust in dry biomes, leaves in autumn, blowing snow, ash/smoke during fire events) — per-cell parameter feeding 3.2x wind-debris particles.
- [ ] **Season overlay** (spring / summer / autumn / winter — static-per-region acceptable for Phase 3; cycling is an open question).
- [ ] **Time-of-day advances with sim clock;** dawn/dusk palette shifts land in Phase 5 polish.
- [ ] **Weather-affects-flight wiring:**
    - **Wind** on ground track + gust attitude perturbation.
    - **Density altitude** effect on performance (consumes W1 temperature + humidity).
    - **Turbulence** scales with density-gradient magnitude (W3 cloud density) + baseline (W1 terrain turbulence).
    - **Visibility** reduction inside dense cloud — drops from fog-limited 6-8 km to hundreds of meters.
    - **Icing** accumulation at cold + moist cloud interior; affects lift + drag + stall speed.
- [ ] **Weather forecast at hubs** with accuracy decay by range and time. Consumes W1 + W2 state; projects forward on the slow-evolution cadence.
- [ ] **Visual-bench artifacts:** W3 delta composite overlays (wind + turbulence + precipitation) on selected scenarios.

### Milestone 3.2b: Chop wiring + water scenario family

Consumes the 2.5a.7 chop hook (`SimState.chop_intensity`) and the 3.2 wind state. Produces the water scenario variants blocked on 3.2 weather landing.

- [ ] Wire `WorldState.ChopState` (per water body, driven by wind fetch + duration from 3.2) → `SimState.chop_intensity` scalar each tick. Deterministic by design; `chop_intensity` is already in `state_hash`.
- [ ] Wire `SceneParams.chop_intensity` → water shader so visual chop tracks physics chop (prevents dial-driven desync when weather varies chop away from 1.0).
- [ ] Author glassy-water takeoff scenario (`beaver_water_takeoff_glassy`, `chop_intensity = 0.0`) — harder than nominal because no wave pattern breaks float suction.
- [ ] Author rough-water takeoff scenario (`beaver_water_takeoff_rough`, `chop_intensity = 2.0+`) — envelope-limit; floats pound on crests; documents model behavior under heavy chop.
- [ ] Author crosswind / headwind / tailwind takeoff + landing pairs once 3.2 wind state is plumbed into the aero surfaces — the wind deferrals enumerated in 2.5a follow-ups close here.
- [ ] Re-baseline `beaver_preview` / `scene_extended` visual snapshots post-chop-wiring (pending `cargo insta accept`).

### Milestone 3.2x: Weather visuals & particle system -- LARGELY SHIPPED 2026-04-27

Parallel to 3.2 (state) and 3.2b (chop). Weather **state** is one problem; weather **visuals** are another, and flowPlan has no particle system yet. This milestone builds the visual/particle layer that the rest of Phase 3+ will reuse.

- [x] **Particle system architecture:** preallocated pool, arena-backed per-frame instances, LOD + distance culling, lifecycle (spawn / update / despawn) on the render side. *(Shipped Phase E.4 as `RainParticleSystem` — fixed pool of 4096, world-anchored, recycled by lifetime + camera-distance + altitude. Pattern reusable for wingtip mist / wake / debris.)*
- [x] **Volumetric clouds** — target volumetric rendering from the start. 2D billboards rejected (fundamentally cannot render camera-inside-cloud, and players will invariably fly through clouds — see `cloud_flythrough_requirement` memory). Design spec comes from the [pre-3.2x cloud rendering research memo](#cloud-rendering-research-memo-pre-32x) (`docs/CLOUDS.md`). Implementation:
    - [x] **Raymarched volumetric** with CPU-GPU shared noise field (Perlin-Worley blend). Camera-inside-cloud handled natively. *(Shipped B1-B3c + CM-1..10 + post-CM-10 phases A-E.)*
    - [x] **Weather-Voronoi-cell lookup** per raymarch sample. *(Shipped 3-cell softmax in Phase 3.6.3c.6 / 3.2x C2 + V-cut fix Post-Phase-B.)*
    - [x] **CPU-samplable density:** `world::cloud_density(x, y, z, t)`. *(Shipped Phase 3.2.3 + extended in Phase E.6 with `magnus_dewpoint_k`.)*
    - [x] Banded alpha + hard-edge thresholding for cel-shading compatibility. *(Shipped CM-10 with low-end smoothstep.)*
    - [x] **Per-pass budget**: 3.24 ms at 1080p (target ≤ 4.7 ms gate; ≤ 0.6 ms reference target deferred behind GPU 3D noise textures cost-recovery option).
    - Deferred beyond v1: cloud shadows on terrain, volumetric light scattering through gaps, multi-layer cloud decks.
- [x] **Rain / snow particles** — world-space streaks (far) + screen-space on-windshield overlay (near-camera). *(Shipped E.4 world-space particle pool + E.3 rain/snow temperature derivation; cloud-bottom darkening + rain shafts visible from afar shipped E.8.)*
- [x] **Layer fog render** — horizontal fog plane at altitude. *(Shipped Phase E.6 as `CellType::Mist` — volumetric ground-hugging fog via the cloud raymarcher; conditions: T-dewpoint < 2 K + RH ≥ 0.80 + calm wind. Per-pixel terrain-following deferred.)*
- [ ] **Wingtip mist effect** — the signature mood beat from PLAN.md. Thin-column particle emitter at wingtip positions under humidity + low-altitude + water-takeoff conditions. *(Particle pool architecture from E.4 reusable; needs aircraft-wing-position attach + W3 humidity sample at wingtip + spawn gating.)*
- [ ] **Wake + spray particles (water)** — closes the 2.5a.X visual polish deferrals (wake spawn behind floats, spray at high-speed water operations, touchdown moment). *(Likely depends on 3.5b FFT ocean for proper waterline interaction.)*
- [ ] **Wind debris particles** — leaves in autumn, dust in dry biomes, blowing snow in winter. Deterministic per-chunk spawn from weather-RNG stream so replay works. *(Depends on 3.6.4 biome-specific environments for asset/region rules.)*
- [ ] **Flag flutter at hubs** — small geometry-driven (not particle) but grouped here because it's the same wind-state consumer. *(Depends on hub system arrival in 3.6.3c.3+.)*

Storm visuals, atmospheric optics (halos, sun-dogs, rainbows, crepuscular rays), and lightning flashes slip to Phase 5.5 polish unless a compelling bench scenario pulls them forward.

**Net 3.2x state**: core volumetric weather system shipped end-to-end. Remaining items are gameplay/biome-specific particle effects that depend on later milestones (3.5b water, 3.6.3c hubs, 3.6.4 biome environments). 3.2x is **functionally complete for the weather-visual aspect** and unblocks downstream Phase 3 milestones.

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

**Infrastructure dividend (critical architectural awareness):** the compute-pipeline support added here (storage buffers, compute-shader bind groups, dispatch lifecycle, readback path) is **reusable by multiple downstream milestones**. The FFT ocean is the first compute consumer and therefore carries the burden of designing the abstraction; shape decisions made here ripple into every consumer. **Design with these downstream consumers in mind from day one**, even if the FFT needs of each differ:

- **[3.2x Volumetric clouds]** — raymarch in compute (more efficient for high-step counts) vs fragment shader with compute-precomputed 3D noise texture. Either way needs storage-buffer / 3D-texture bind-group patterns.
- **[3.6.2 Hydrology-graph bake]** — one-shot compute at session load (coarse heightfield → D8 drainage → river + lake + coastline graph). Compute-pipeline lifecycle (dispatch-and-readback) differs from the per-frame FFT use; abstraction should handle both patterns.
- **[Phase 5.6 Aurora shader]** — continuous compute for dynamic aurora bands; similar dispatch pattern to FFT but at lower cadence.
- **[Post-v1 erosion simulation]** — if pursued, multi-pass compute on heightfield; same abstraction.

Specifically, the FFT-ocean abstraction design should anticipate:

- **Lifecycle variation:** one-shot bake (hydrology) vs per-frame (FFT ocean, clouds, aurora).
- **Readback path:** GPU-only (clouds, aurora) vs CPU-readback (hydrology graph, some bench-visual-artifact rendering).
- **Storage buffer vs 3D texture vs atomic buffer:** each downstream case varies; abstraction should support each without forcing rework.
- **Bind-group reuse patterns:** standard layouts so downstream consumers don't re-author each time.

The abstraction shape is **emergent** — it finalizes as the FFT implementation lands and the first downstream consumer pulls on it. **Awareness that these consumers exist + will depend on the abstraction's shape is encoded here** so design decisions made in isolation don't accidentally foreclose on downstream needs. A short design memo (candidate: `docs/COMPUTE_PIPELINE.md` stub) captured during 3.5b scoping is cheap insurance.

**Visual target (from 2.5a iteration):** water is the star of flowPlan's visual identity. Near/small scale (floats in chop, spray, wake, touchdown) AND large scale (horizon-far sea state, wind-swept waves, swell patterns) must read well. Slope-based shading tricks along the lines of Sea of Thieves — normal-map variation driven by wave slope, not just height — are a good reference. FFT ocean supports this naturally since the spectrum yields both height AND slope per frame.

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

### Milestone 3.7: Save-format scaffolding (late-Phase-3)

**Data-model deliverable for the pilots × seeds progression matrix locked 2026-04-21. First concrete consumer is weather event logs (3.2.3) + hub catalog persistence (3.6.1).**

- [ ] **Three orthogonal persistence entities** per [MAPGEN.md § Determinism & save-format implications](MAPGEN.md#determinism--save-format-implications):
  - `pilots/<uuid>.save` — global pilot data (identity, fleet, currency, career stats)
  - `pilots/<uuid>/<seed_hex>.progress` — per-(pilot, seed): fog-of-war, hubs, missions, resume-point
  - `worlds/<seed_hex>.world` — per-seed world event log (shared across pilots)
- [ ] `<seed_hex>` = hex of `master_seed XOR hash(WeatherTuning)` so tuning changes distinguish worlds.
- [ ] Versioning: `(game_version, pilot_schema, world_schema, progress_schema)` — each entity versions independently.
- [ ] **Append-only event log format** for world-level events (weather storms initiated, hydrology changes, etc.). Replay reconstructs world state exactly.
- [ ] **Fog-of-war representation:** 1 km × 1 km discovery bitmap for a 5000 × 5000 km world = 125 KB uncompressed per (pilot, seed) pair; RLE compression to 5–20 KB typical.
- [ ] **Load path:** session start reads `pilot.save` + `pilot/<this-seed>.progress` + `worlds/<this-seed>.world`. Sub-second cold read, <10 MB peak.
- [ ] **Save path:** append deltas during play, compact on clean session close.
- [ ] Determinism: save + reload → byte-identical world state continuation.
- [ ] **Seed-uniqueness test battery** per [TEST_PLAN.md § Seed-generation uniqueness](TEST_PLAN.md#seed-generation-uniqueness-pending--phase-37-save-format--seed-creation-ui): coverage (N distinct seeds → N distinct worlds), bit-adjacency decorrelation, sub-seed independence, seed-bundle collision check, pairwise uniqueness across representative seeds. First milestone where seed-generation happens at scale, so the test battery lands here.

### Deliverable

The plane has a voice (engine, wind, water, splash). Weather actively shapes flight. Sim runs on its own thread. CI runs on every push. Save format scaffolded with pilots × seeds matrix.

> **Gate:** Phase 3 is complete when the quality gate passes in CI, the determinism test passes with the sim worker thread active, the audio callback passes the no-alloc/no-log verification, save/load round-trips deterministically, and the Phase 3 scene benches hold within the frame budget on reference hardware.

---

## Phase 3.6: World Foundations

**Goal:** replace the Phase 2.2 sum-of-sines heightfield with a proper procedural terrain + hydrology + biome system. This is where the world becomes a **world** rather than a placeholder — coherent rivers, biome-appropriate landscape, shore classification, flora, hub placement infrastructure. Stands as its own phase rather than a Phase 3 milestone because its scope, dependencies, and gate all deserve phase-tier treatment.

**Scope drivers:**
- Closes the Phase 2 known limitations (no real hydrology, no shore classification, no independent amplitude/gradient control) enumerated in [PROGRESS.md § Phase 2 known limitations](PROGRESS.md#known-remaining-issues-in-phase-2-deferred-to-25-or-later).
- **Unblocks [Milestone 2.5b.7 per-surface friction](#milestone-25-ground--water-contact-physics)** by shipping the `world::surface_at(x, z) -> SurfaceClass` sampler.
- **Unblocks Phase 4 Delivery Loop** — hubs + routes + cargo + missions all need real terrain + biomes + hydrology underneath them.
- Reuses the compute-pipeline infrastructure introduced in [Milestone 3.5b FFT ocean](#milestone-35b-fft-based-ocean-water-tessendorf-method). That's part of why 3.6 follows Phase 3: the infra boundary is in place.

**Detailed design:** [MAPGEN.md](MAPGEN.md).

**Prerequisite (gated):** the **world-scope prototype** (pre-Phase-3 prerequisite, see [Pre-Phase-3 prerequisites](#pre-phase-3-prerequisites)) must be complete before Phase 3.6 starts. The prototype's outputs — world-scope decision + chunk-shape decision + persistence footprint measurement — drive this phase's implementation path.

Each milestone below corresponds to **one layer** of the three-layer composition (see [MAPGEN.md § Layered generation](MAPGEN.md#layered-generation--three-layer-composition-pinned)): Layer 1 = Topographic base; Layer 2 = Voronoi partition; Layer 3 = Feature-delta heightfield modifications + flora. Each layer ships playable on its own.

### Milestone 3.6.0: Toroidal wrap + sampler quantization (pre-Layer-1 foundation)

**Lands the Option A toroidal world-topology contract across every sampler, as a foundation for the Layer-1/2/3 milestones that follow. Short milestone; wide touch surface.**

- [ ] `WORLD_EXTENT_M` constant added to `flowplan_core` (strawman 5000 km, adjustable). Threaded through all world-gen samplers + Voronoi + hydrology bake.
- [ ] **Terrain octave frequencies quantized** to `k / WORLD_EXTENT_M` for integer `k` so sum-of-sines wraps cleanly. Verify quantization error < 0.1% at finest octaves (visually indistinguishable from current).
- [ ] **Weather harmonic wavelengths quantized** to integer-cycle-per-world. Adjust pressure-synoptic 350 km → 357.14 km (14 cycles at 5000 km); temperature-synoptic + humidity-synoptic likewise. Temperature-climate 500 km (10 cycles) and humidity-regional 200 km (25 cycles) already fit.
- [ ] **Voronoi lookup wraps** via mod-N indexing: `cell_at(x, z)` and `cell_at_drifted` both resolve neighbors through `((ix + N) % N, (iz + N) % N)` where `N = WORLD_EXTENT_M / GRID_SPACING_M` is exact integer. Weather W2 spacing adjusted to 20 km or 25 km so N divides evenly (N=250 or N=200 at 5000 km).
- [ ] **Hydrology graph** (3.6.1 scope) bakes wrap-respecting: rivers flowing east return from west; catchment basins may span the seam.
- [ ] **Compass rules locked to world-north** regardless of wrap (no edge-flip). Implementation: compass reads heading directly from state quaternion, no coordinate transformation at edges.
- [ ] **Day/night cycle accelerated + east-to-west.** `sim_time_of_day_at(x)` = `sim_time_of_day - (x / WORLD_EXTENT_M) × DAY_LENGTH_S`. Placeholder acceleration ratio 24:1 (tunable).
- [ ] **Visual-bench wrap-continuity tests:** emit each sampler's PNG at a crop straddling the seam; assert pixels at `x=WORLD_EXTENT_M - ε` match pixels at `x=ε`.
- [ ] **Determinism-bench wrap tests:** `sample(x)` == `sample(x + WORLD_EXTENT_M)` bit-for-bit across every sampler. Adds to the existing determinism CI battery.

This milestone slots between Phase 3 close and Phase 3.6.1 so Layer-1 hydrology is wrap-aware from birth. Can also absorb the orographic + rain-shadow (3.2.2b) in parallel if resources allow.

### Milestone 3.6.1: Layer 1 — Topographic base + hydrology

**Ships Layer 1 in full. Playable world with real biome-coherent terrain and hydrology after this milestone, even with no Voronoi partition or features yet.**

- [x] New `world::terrain` module replacing sum-of-sines. Ridged multifractal + domain warping with per-biome amplitude / frequency / gradient profiles. Independent amplitude and gradient tuning (closes the Phase 2 limitation). **SHIPPED in 3.6.1a + 3.6.1b + 3.6.1c.**
- [x] **Hydrology sub-system:** world-bake pass at session load — coarse global heightfield → D8 drainage network → river edges + lake footprints + coastlines. Deterministic from master seed. River-continuity invariant + rivers-grow-into-lakes. Lake / sea / small-water-body classification + per-water-body surface parameters. **SHIPPED in 3.6.1d** (priority-flood + wetness equilibrium + channel classification + delta fans + Marching Squares coastline at 2.5 km bake resolution). Cross-platform bit-identity deferred to Phase 6.1.
- [x] **Biome climate fields** (temperature / humidity / elevation / latitude proxy) → biome classification. Biome catalog (4 pinned: HighDesert, RockyAlpine, CoastalBoreal, MediterraneanArchipelago) per MAPGEN.md altitude-envelope table. **SHIPPED in 3.6.1a**. Adjacency-rule enforcement structural via threshold design; explicit rejection pass deferred. Expanded biome set (PNW, subarctic, tropical, Smoky) deferred post-3.6.2.
- [x] **Magnetic variation field** (biome-dependent at world-bake time, deterministic). **SHIPPED in 3.6.1e** via `magnetic_variation(seed, x, z) -> f32°` — latitude-proxy bias + Perlin-fBm, amplitude ±15°.
- [x] **Hub catalog bake** (~55 K entries for our 5000 km world) per [MAPGEN.md § Hub late-binding (fuzz + resolve)](MAPGEN.md#hub-late-binding-fuzz--resolve). Each entry: `{HubId, approx_world_pos, tier, parent_biome, region_id, operational_tags, runway_naming_style, display_name}`. **SHIPPED in 3.6.1e**. Placement: dart-throwing with region-density weighting (big-region dominance per real-geography pattern). Tiers: Major / Regional / Local / Remote / Isolated. Per-biome procedural display names. Voronoi late-binding resolution deferred to 3.6.2 when cells exist.
- [ ] `Terrain::flat()` / `Terrain::flat_land()` remain for bench scenarios; procedural path gated by world-scope + scenario flags. **All existing Phase 2 / 2.5 flight-bench + water-bench scenarios must continue to pass unchanged.**
- [ ] **Deterministic / telemetric validation:** extend the bench harness with a "world-gen" scenario class — assert hydrology-graph byte-identical output across platforms; assert flow-continuity at chunk boundaries; assert lake-volume / watershed-area bounds; assert hub catalog byte-identical across platforms.

### Milestone 3.6.2: Layer 2 — Voronoi partition + sub-biome feature assignment

**Ships Layer 2 semantics only — Voronoi partition + feature-type tags. Heightfield unchanged from Layer 1; features are semantic, not yet visual.** Playable state: real Layer 1 terrain with semantic feature tags on Voronoi cells; useful for telemetry, debug visualization, and validating the partition + placement logic without risking terrain correctness.

- [x] Voronoi partition at world-bake time: Bridson 2007 blue-noise Poisson-disk at 2 km average spacing; spatial grid hash for O(1) nearest-seed queries. **SHIPPED in 3.6.2** (`voronoi_cells.rs`). 3.9 M cells baked in ~20 s release.
- [x] **Sub-biome feature libraries** per parent biome, RON-authored per user direction. 4 land biomes (HighDesert including user-flagged `book_cliffs` / RockyAlpine / CoastalBoreal / Mediterranean) + Oceanic library for sea cells. 38+ feature types with placement constraints (slope/roughness/altitude/water-adjacency). **SHIPPED in 3.6.2** (`data/world/sub_biome_features.ron` + `sub_biome.rs`).
- [x] RNG feature-type assignment per Voronoi cell with placement-constraint rejection + `plain` fallback. **SHIPPED in 3.6.2**. Fallback rate ~13 % on canonical seed (acceptable per MAPGEN design).
- [ ] **Sub-biome palette adaptation (rendering)**: cells carry `BiomeSource::BleedOver { host, source }` info ready for consumer; shader-side home/host palette blending deferred to 3.6.3+.
- [ ] **Hub streaming-time resolution**: `HubEntry.approx_world_pos` snaps to cell-center positions using the adjacency graph shipped in 3.6.2; actual resolve step deferred to 3.6.3 (wants heightfield deltas for runway-end / dock-point snapping).
- [ ] **Variable Voronoi cell size** for sub-biome layer: partial — power-diagram pattern established for weather + hub regions; sub-biome cells currently uniform-weight. Adding per-seed biome-coherence weights is a ~50 LOC extension.
- [x] **Biome edge bleed-over** per [MAPGEN.md § Biome edge blending](MAPGEN.md#biome-edge-blending--tri-biome-rare-experiences-pinned): cells with primary biome weight < 0.8 are in transition zone; selection can draw from adjacent biome's library (restricted to `is_bleed_over_candidate: true` entries). **SHIPPED in 3.6.2**. Observed rate ~18.5 % of cells. Palette adaptation (rendering) deferred to 3.6.3+.
- [ ] **Tri-biome intersection features** — a small authored set of features that spawn ONLY at tri-biome cell neighborhoods (topologically rare: 3-way biome meetings require 3 regions converging within bleed radius). Examples: crystal lake in desert/alpine/boreal intersection; lone-tree-on-mesa at boreal/alpine/mediterranean. Gates rare cosmetics / photo-mode-worthy moments.
- [ ] **Fine-octave climate tuning** (alongside biome terrain in 3.6.1b): add fine octaves to `temperature_at` / `humidity_at` climate fBm so microclimate pockets exist at ~15-30 km scale. Enables tri-biome intersections at the 2 km feature-cell scale (without this, biomes only meet at very coarse regional boundaries).
- [ ] Preallocated chunk pool + background generation thread (honors DESIGN.md no-alloc hot-path rule). Streaming disk budget + tier-adjusted defaults per MAPGEN.md § Streaming disk.
- [ ] Debug visualization: Voronoi-cell overlay in debug HUD (cell edges drawn, feature-type labels at centroids). Reuses the existing `DebugLinesRenderer` pipeline.
- [ ] **Deterministic / telemetric validation:** assert Voronoi partition byte-identical across platforms; assert feature-type assignment reproducible given same master seed; assert biome-adjacency graph rejects forbidden pairs; assert hub resolution outcome stable given same streaming approach.

### Milestone 3.6.3: Layer 3 — Feature-delta heightfield + shore classification

**Ships Layer 3 — feature deltas become visible in the heightfield + shore classification + landmark placement. This is where features go from labels to geometry.**

Split (pinned 2026-04-23 per user direction, detail in [MAPGEN.md § Layer 3 phase split](MAPGEN.md#layer-3-phase-split-pinned-2026-04-23) + agent-memory `feature_delta_expansion_3_6_3`):

**3.6.3a — per-cell delta engine + natural features + hub foundation + shore classification (code): SHIPPED 2026-04-23.**

- [x] **`FeatureDelta` trait + registry** mapping feature_id → delta implementation. Additive on top of Layer 1: `final_height = base + Σ feature_deltas`.
- [x] **Boundary falloff invariant:** each delta smoothly to zero at Voronoi cell edges (bump-function / cosine falloff). Continuous terrain across Voronoi edges by construction.
- [x] **Hydrology continuity invariant** enforced at query time: deltas that would elevate a river channel / fill a lake basin / dam a drainage network are suppressed.
- [x] **~10 natural signature feature deltas**: `lake_cluster`, `alpine_tarn`, `cirque`, `mesa_top`, `book_cliffs`, `dune_field`, `bush_strip_clearing`, `sandy_beach`, `cliff_coast`, `protected_cove`. Covers the major delta-pattern classes (bowl / plateau / cliff / flatten / ramp / dune) so 3.6.3c extensions can match the pattern.
- [x] **Hub foundation delta** (tier-adaptive): Major = full flatten + 50 m skirt + cut-and-fill into positive elevation; Regional = flatten + 30 m skirt; Local = partial flatten + natural edges; Remote = minimal smoothing; Isolated = no delta. Cells hosting hubs get this delta instead of their natural feature delta (priority: `hub_foundation` > natural feature > `plain`).
- [x] **Shore classification sampler** (`world::surface_at(x, z) -> SurfaceClass`: Sand, Gravel, Rock, Cliff, Mudflat, Delta, IceEdge, Grass, Paved, Dirt, Snow, PowderSnow, DryLake, Water) from final heightfield + hydrology graph + biome + feature tag. **Unblocks 2.5b.7 per-surface friction.**
- [x] L3 integration into `Terrain::height_at_scale` (opt-in param).
- [x] Visual bench: L1-vs-L3 heightfield comparison, shore classification map, hub foundation effect visualization (5 artifact families × 3 crops = 15 PNGs).
- [x] **L1 coastline tuning pass** (mid-3.6.3a): `COASTAL_BOREAL.mean_altitude_m` -50 → +50 + `shore_attenuation` function (proximity-to-shore fine-octave attenuation, 400 m band, 0.20 floor) — fixes "waterworld + speckled coastline" symptom. Both load-bearing; regression risk if reverted. Memory: `coastline_tuning_pass`.

**3.6.3b — design-authoring sprint (no code): SHIPPED 2026-04-23.** All pinned design directions (palette adaptation, simple primitives) in MAPGEN + memory; 18 features authored in RON with palettes + categories; full-spec placement rules in MAPGEN + FEATURES per-feature table; palette-adaptation mechanism revised mid-phase to biome-receives direction.

- Expand `data/world/sub_biome_features.ron` + MAPGEN.md + FEATURES.md with cross-biome feature categories:
  - **Developments** (5): `farmland_parcel` ✅ (HighDesert home; bleeds ONLY to CB + Med — not arable in RA), `dwelling_cluster` ✅, `remote_compound` ✅, `ranch` ✅ (HighDesert home, bleed-over enables "winter ranch"), `homestead_clearing` ✅. **Complete.** Placement-rule evaluation deferred to 3.6.3c.
  - **Infrastructure** (4): `road_segment` ⏳, `dirt_road_segment` ⏳, `highway_intersection` ⏳, `rail_corridor_segment` ⏳ — all blocked on 3.6.3c `FeatureGroup` engine. Design spec authored in FEATURES.md § placement rules table.
  - **Resupply / interactables** (6): `gas_station` ✅ (Phase 4.5 refuel economy anchor; `selection_weight: 0.15` until 3.6.3c road-adjacency gate activates), `bait_shop` ✅, `general_store` ✅, `fishing_camp` ✅, `tackle_shop` ✅, `ferry_dock` ✅. **Complete.**
  - **Scenery cues** (7) (render-tag-only, no heightfield delta): `natural_meadow` ✅, `alpine_meadow` ✅, `creek_valley` ✅, `scenic_overlook` ✅, `river_bend_viewpoint` ✅, `hot_springs` ✅, `abandoned_homestead` ✅. **Complete.**
- [x] **Palette-adaptation mechanism (PINNED)** authored in MAPGEN § Layer 3 + memory `palette_adaptation_mechanism`. RON authoring surface: `home_palette: Option<Palette>` + `adaptation_per_host: HashMap<String, Palette>` shipped on `SubBiomeFeature`.
- [x] **Simple-primitives render direction (PINNED)** authored in MAPGEN § Layer 3 + memory `simple_primitives_render_direction`. Per-feature primitive composition recorded in RON comments.
- [x] **Placement rule design** (Task #215): 7-category schema (hub_proximity, hub_exclusion, min_self_spacing_km, requires_road_adjacency, prominence_attractor, hydrology_coupling, climate_window) + evaluation order + per-feature rules for all 21 cross-biome features authored as **design spec** in [MAPGEN.md § Placement rule vocabulary (PINNED)](MAPGEN.md#placement-rule-vocabulary-pinned) + [FEATURES.md § Sub-biome feature placement rules (PINNED)](FEATURES.md#sub-biome-feature-placement-rules-pinned). `PlacementRules` RON struct + evaluation implementation land in 3.6.3c.
- [ ] User review / iteration on the full catalog before 3.6.3c implementation.

**3.6.3c — multi-cell group engine + development/scenery deltas (code): IN PROGRESS.** Split into 5 sub-milestones for incremental ship.

**3.6.3c.1 — `PlacementRules` struct + bake-time evaluation: SHIPPED 2026-04-23.**

- [x] **`PlacementRules` nested schema** with 7 rule-category sub-structs (`HubRules`, `SelfSpacingRules`, `RoadAdjacencyRules`, `ProminenceRules`, `HydrologyRules`, `ClimateRules`, `SpilloverRules`). All `#[serde(default)]`.
- [x] **`evaluate_placement_rules` function**: 5 rule categories evaluated — hub_proximity (weighted boost + required bool), hub_exclusion, prominence (median-of-9 elevation proxy + slope gate), hydrology_coupling (direct enum match + synthesized RiverBend/LakeEdge), climate_window (temp + humidity via existing samplers at `time_s=0.0`).
- [x] **Wired into `assign_feature`** via optional `PlacementRulesContext` bundle. Existing baseline callers unchanged; new `feature_at_cell_with_rules` / `sub_biome_at_with_rules` catalog methods thread rules through.
- [x] **18 features authored with `placement_rules` in RON** per FEATURES.md table.
- [x] **Diagnostic** (`emit_placement_rule_rejection_stats`): 10 K-cell sample reports reassignment rate + per-feature deltas. Canonical-seed run shows 2.84 % reassignment — rules active, some too-strict features flagged for 3.6.3c.2 tuning (`abandoned_homestead`, `scenic_overlook`).
- Stubs parsed but not evaluated: `self_spacing` (3.6.3c.1c), `requires_road_adjacency` (3.6.3c.3), `spillover` (3.6.3c.3+).
- [x] **Hub emergent upgrade via feature spillover** — design PINNED in MAPGEN + memory `hub_emergent_upgrade_via_spillover`; stub `SpilloverRules` struct shipped for future anchor-feature authoring. Per-trigger `max_count` budget + optional region-cap safety net.

**3.6.3c.1c — Eager `BakedAssignments` bake + rayon parallelization: SHIPPED 2026-04-23.**

- [x] **`BakedAssignments` struct** with u16-interned feature IDs (~20 MB total, lean).
- [x] **`SubBiomeCellCatalog::bake_assignments(...)`** entry — single-call eager bake.
- [x] **Self-spacing rule enforcement** via per-feature spatial hash (`SelfSpacingState`), inserted at bake-time in master-seed-keyed permuted order.
- [x] **Master-seed-keyed permutation** (`splitmix64(master_seed, cell_id)`) for cell scan order — removes Bridson's depth-first spatial-correlation bias while preserving full bit-identical determinism.
- [x] **Per-cell hub-radius cache** eliminates redundant `hub_catalog.hubs_in_radius` queries across candidate evaluations (~5× cost reduction inside per-cell work).
- [x] **Attractor `weight_mult` consumption** in `sample_feature_with_context` — features in attractor zones (hub_proximity / prominence) get multiplied selection weight.
- [x] **Rayon parallel bake** (Phase A par_iter + Phase B serial merge): **275 s → 33 s = 8.3× speedup** at canonical seed, bit-identical output. `rayon` added to [DESIGN.md tech-stack](DESIGN.md#tech-stack) + workspace deps.
- [x] **Diagnostic** (`emit_baked_assignments_calibration` visual-bench test): full-bake stats including per-feature placement counts, fallback rate, bleed-over rate, self-spacing rejection count, wall-clock.

**3.6.3c.2 — Per-cell deltas for 11 dev/resupply features + tuning: SHIPPED 2026-04-25 (Task #272).**

- [x] **Development + resupply deltas** wired via `flatten_delta` in `feature_delta_impls.rs::natural_delta`. 11 features (5 development + 6 resupply) with strengths interleaved with natural flattens by surface character (plowed 0.85 > paved 0.8 > shop-pad 0.65 > yard 0.6 > clearing 0.5 > rough-camp 0.4). Magnitudes bounded < 5 m by the proxy ceiling, well under hub_foundation envelope.
- [x] **Scenery cues** = render-tag-only at heightfield level (already shipped in 3.6.3a — `creek_valley` channel, `scenic_overlook` prominence, `hot_springs` bowl, etc.; the remaining four scenery cues use natural-feature deltas).
- [x] **`scenic_overlook` placement-rule tuning** — `prominence: (min_m: 100→25, min_slope_deg: 20→0)`. Slope gate dropped (cell-seed slope is locally smooth even on alpine ridges); per-feature `PROMINENCE_SAMPLE_RADIUS_M` tuning queued for 3.6.3c.4+. Post-tune calibration: 37 → 12 placements at canonical seed.
- [x] **`STREAM_RADIUS_M` cushion** (Task #261, .12k closure) — 20 → 22 km; covers cells whose Voronoi polygon extends inward into the visible terrain disc.
- [ ] **Infrastructure deltas** (`road_segment`, `dirt_road_segment`, `highway_intersection`, `rail_corridor_segment`) — blocked on 3.6.3c.3 `FeatureGroup`; need group-aware continuous-polyline deltas, not per-cell.

**3.6.3c.3 — `FeatureGroup` multi-cell engine: SHIPPED 2026-04-25 (Task #274 + #275).**

Shipped in three sub-milestones:

**3.6.3c.3a — `FeatureGroup` engine + hub-trunk roads (Task #274 part 1)**

- [x] **`FeatureGroup` concept** in new `crates/world/src/feature_groups.rs`: cells belong to a group via `(GroupId, seg_idx)` membership entries. Polylines stored in literal world coords, wrap-aware via single-frame alignment.
- [x] **Hub-trunk road network**: `FeatureGroupBundle::build_road_network` connects each Major/Regional/Local hub to its k-nearest neighbors via straight polylines (Steiner-tree-lite). Wrap-aware polyline construction handles toroidal-seam-crossing.
- [x] **Group-aware deltas**: `compute_group_delta_at` reads per-cell `(GroupId, seg_idx)` membership, evaluates perpendicular distance to ≤3 nearby polyline segments. Smoothstep-falloff depression to 0 at half-width band edge.
- [x] Visual-bench (`crates/world/tests/feature_groups_visual_bench.rs`): continental + spawn-50km artifacts, color-by-kind.

**3.6.3c.3b — 4-then-5-tier road kinds + `requires_road_adjacency` activation (Task #275)**

- [x] **Tiered `FeatureGroupKind`**: `RoughDirt < Dirt < Gravel < Paved < Highway` (`Ord`-comparable via `road_tier()`); `Rail` separate ladder. Per-kind `half_width_m` / `depth_m` / `max_length_m`. Rename `Road→Paved`, `DirtRoad→Dirt` propagated.
- [x] **Per-kind connection rules** in `build_road_network`: Highway connects Major (k=2, 250 km cap), Paved Major+Regional (k=2, 120 km), Dirt Local (k=2, 60 km), Rail Major (k=1, 200 km).
- [x] **`requires_road_adjacency` activated** at bake time. `PlacementRulesContext` gains `road_adjacent_cells: Option<&HashSet<CellId>>` + `sub_biome: Option<&SubBiomeCellCatalog>`; bake passes both, populated from `FeatureGroupBundle::road_adjacent_cells()`. `gas_station` etc. now snap to the trunk network.

**3.6.3c.3c — Feature-emit roads with A* pathfinding + 5-tier ladder (Task #274 part 2)**

- [x] **`SubBiomeFeature.road_emit: Option<RoadEmitKind>`** RON-authored on 12 development features (RoughDirt for `homestead_clearing`/`abandoned_homestead`/`fishing_camp`; Dirt for `ranch`/`mining_camp`/`ghost_town`/`watchtower`; Gravel for `farmland_parcel`/`mountain_lodge`/`dwelling_cluster`/`remote_compound`; Paved for `adobe_village`/`coastal_village`/`olive_grove`).
- [x] **Pass-2 builder**: `FeatureGroupBundle::extend_with_feature_emit_roads` runs after `BakedAssignments`. Per-tier multi-source Dijkstra over Voronoi-cell graph (priority queue, length-capped, slope penalty 5×@>25° / 2×@>15°, water = unreachable). Per-feature path reconstruction walks successor chain in O(path_length).
- [x] **Polyline-distance perf fix (3.6.3c.3c.1)**: drop 9-shift wrap loop + per-cell `seg_idx` hint. ~300× speedup on the per-vertex L3-road-delta evaluator.
- [x] **`BakedAssignments`-aware `compute_delta_at` (3.6.3c.3c.3)**: thread `Option<&BakedAssignments>` through `WorldRefs/WorldRefsArc`. O(1) `feature_at_cell` lookup instead of recomputing `assign_feature` per query — closes the 140 s rebuild regression.
- [x] **Network growth**: 809 hub-trunk segments → 30 536 total (Pass 1 + 2). 54K → 813K km total length. ~9 % of hubs participate (was 1.2 %).
- [x] Spillover deferred to **3.6.3c.3d** (future).

**3.6.3c.4 — Voronoi-cell chunked terrain streaming: SHIPPED 2026-04-25 (Task #279 + #280).**

Architectural pivot. Replaces the v1 monolithic `StreamingTerrain` (one big 20 km disc-mesh, rebuild = re-sample 1.4 M verts as a unit) with **per-Voronoi-cell mesh chunks**. Universal pattern — water (3.6.3c.5), volumetric weather (3.6.3c.6) follow the same shape. Pinned in memory `chunked_voronoi_layers_universal`.

- [x] **Per-cell async build** via 4-worker pool, shared priority queue (nearest-first by camera distance), `mpsc` result channel. Cell-enter → enqueue; cell-exit → drop GPU mesh. Per-camera-step cost = O(entered cells), not O(whole mesh).
- [x] **`CpuMesh::terrain_voronoi_chunk`** (`crates/assets/src/mesh.rs`): centroid-owned grid algorithm — each grid quad goes to whichever Voronoi cell contains its centroid. Owning chunk includes all 4 corners (even if some are nearest-to-neighbor). World-aligned grid origin at world (0,0) → adjacent chunks share exact vertex positions, no z-fight, no T-junctions, no gaps. Vertex compression (only included verts in the VBO).
- [x] **`ChunkedTerrain`** manager (`crates/app/src/chunked_terrain.rs`): worker pool + queue + result drain + per-frame `update(active_cells, ...)` integration with `StreamingCellSet`.
- [x] **Renderer integration**: `terrain_chunks: HashMap<u32, GpuMesh>` field, `update_terrain_chunk` / `evict_terrain_chunk` API, render-pass branch (chunked iteration when present, single-mesh fallback for scenarios / fast-iteration).
- [x] **3.6.3c.4.1 fixes**: centroid-ownership (closes water-line cell-boundary gap bug); detached worker shutdown (no `join()` so ESC closes promptly even mid-build).
- [x] **Bake cache** (3.6.3c.3c.2 / 3.7-precursor): `BakedAssignments` + `FeatureGroupBundle` serialized to `target/world-bake-cache/<seed>-vN.bin` via bincode. Subsequent launches load in seconds instead of re-running the 6-min bake.

**3.6.3c.5 — Chunked water mesh streaming: SHIPPED 2026-04-25 (Task #282).**

Per-Voronoi-cell water chunks following the universal chunked pattern. Each cell = its own flat-quad mesh; FFT displacement per Phase 3.5b plugs in via shader-side per-cell uniform buffers without re-architecting.

- [x] **`CpuMesh::water_voronoi_chunk`** — per-cell builder. World-aligned 60 m grid + centroid-owned ownership + vertex compression. No height sample (Y=0); uniform `+Y` normal; uniform color.
- [x] **`ChunkedWater` manager** — 2-worker pool, priority queue, condvar wake. `WATER_STREAM_RADIUS_M = 15 km` (smaller than terrain's 22 km — water visibility is horizon-limited). No `WorldRefsArc` dep.
- [x] **Renderer integration** — `water_chunks` HashMap, `update_water_chunk` / `evict_water_chunk` API, render-pass branch. Legacy single-mesh path preserved for scenarios.
- [x] **Game-app wiring** — `ChunkedWater` constructed alongside `ChunkedTerrain`; same `scenario.is_none()` gating.

**3.6.3c.6 — Volumetric weather chunks: B1-B3c + C1 + C2 + C2-polish shipped 2026-04-26, future polish pending.**

Volumetric-cloud sampling per W2 weather Voronoi cell (~20-50 km cells). Shader-side cloud-density texture. Builds on the chunked-Voronoi pattern.

Shipped in 2026-04-26 iteration:

- [x] **`docs/WEATHER.md`** — implementation reference companion to MAPGEN's three-layer weather composition design. Locks technique decisions (Perlin-Worley blend, ray-cylinder early-out, banded alpha cel-shading, atmospheric character per cell type, etc.) per LOCKED / RECOMMENDED markers. Cited from CLAUDE.md docs stack.
- [x] **B1 — Worley noise primitive (CPU + WGSL)**: u32 PCG hash chain (bit-identity-capable), `worley_2d` / `worley_3d` with multi-feature F1+F2 smooth-blend, `cloud_perlin_3d`, `features_per_cell_at` for regional density variation. Octave-offset slot `[240, 280)` reserved.
- [x] **B2 — cloud shader skeleton**: `volumetric.wgsl` empty raymarch, render pipeline + pass slot. Pass ordering locked: `sky → scene → outline → cloud → widgets`.
- [x] **B3 — density sampling**: hardcoded test Cumulus at world origin, altitude profile + footprint falloff + Worley sub-cell detail. Visible cumulus puff in-game.
- [x] **B3 perf-polish**: ray-vertical-cylinder early-out, adaptive step count (4-32 depending on band length), structural perf-budget regression test (`RAYMARCH_STEPS_MAX ≤ 64`, `CLOUD_FEATURES_PER_CELL ≤ 2`, etc.).
- [x] **B3b — depth termination**: cloud raymarch clips at scene depth (terrain, water, aircraft, instances). Camera-relative-rendering caveat resolved (do not subtract `cam_pos` from already-camera-relative unprojected coord).
- [x] **B3 lighting stand-in**: altitude-gradient sun-lit/shadow shading until B3c+ shadow march lands.
- [x] **B3c — Perlin-Worley blend**: 0.7 worley_struct + 0.3 perlin_norm per WEATHER.md Cumulus mapping. Visible internal puff variation.
- [x] **GPU wall-clock perf-gate**: `crates/render/tests/cloud_pass_perf.rs`. **Locked at 1080p / 144 Hz target / 3 ms budget** per user direction 2026-04-26. Hardware-gated via `Features::TIMESTAMP_QUERY`. Run via `./flowplan test-cloud-perf`. Current cost: 1.0 ms median (989 fps cloud + minimal scene on RX 7900 GRE).

Shipped 2026-04-26 follow-ups:

- [x] **B4 — CPU-GPU agreement gate** — hardware-gated compute-shader test, max abs diff 3.3e-4 vs 5e-3 tolerance.
- [x] **C1 — ChunkedWeather streaming manager** — universal-pattern async-worker streaming, 50 km radius, 25-50 active cells, pop-time staleness check.
- [x] **C2 — multi-cell rendering integration** — `GpuWeatherCell` storage buffer (64-slot capacity), `Renderer::update_weather_cells` API, render-pass binds group(3) cells.
- [x] **C2-polish (priority ladder #1-3)** — periodic 5 sim-second cell refresh; per-cell-type Perlin-Worley blend weights; **tessellation correction** (replaced circular footprints with nearest-cell lookup per WEATHER.md "no-weather is a type of weather"). Tessellation is THE locked architectural correction — same-type adjacent cells now appear as one continuous body of weather.

Pending — see [`docs/WEATHER.md` § Implementation priority ladder](../docs/WEATHER.md#implementation-priority-ladder-post-c2-polish-2026-04-26) for the 17-item ranked list. Highlights:

- [ ] **#4 per-cell-type Worley spacing** — Stratus smooth, Squall textured, Cumulus puffy.
- [ ] **#5 Beer-Lambert shadow march** (E1) — proper sun-lit / shadow-side shading via secondary ray toward sun.
- [ ] **#6 boundary blending** — smoothstep across cell edges using `distance_to_edge`. Fixes visible boundaries at type transitions.
- [ ] **#7 AtmosphericMood per cell type** (3.2x) — fog tint / sun_color / sky desaturation per cell type.
- [ ] **#8 fog + visibility coupling** (3.2x) — sim's `visibility_at` → render fog density.
- [ ] **#9-11 particle system + rain/snow + wingtip mist + wake spray + wind debris** (3.2x).
- [ ] **#12 sim cloud-fly-through effects** (3.2x) — turbulence amplification, visibility reduction, icing.
- [ ] **#13 adjacency-biased cell evolution** (3.2.2c) — sprawling fronts.
- [ ] **#14 cloud shadows on terrain**.
- [ ] **#16 atmospheric optics** (5.5) — sun halo, sun dog, rainbow, crepuscular rays, lightning.
- [ ] **#17 multiple weather layers at different altitudes** — late-milestone, needs design memo first.

**3.6.3c.7 — Palette-adaptation render integration: pending.** *(Was 3.6.3c.4 in earlier roadmap; renumbered when chunked terrain claimed the 4 slot.)*

- [ ] `composed::WorldRefs` computes `source_weight` + `blend_factor = smoothstep(0.1, 0.9, source_weight)`; shader-side palette lerp. Visual-bench `palette_bleed_over_{crop}.png`. Makes the winter-ranch / desert-fjord effects actually visible.

**3.6.3c.5b — Render streaming (Task #226): SHIPPED 2026-04-23.**

- [x] **Layer 1 — uniform terrain mesh, recenter on camera**: `StreamingTerrain` with `maybe_rebuild` checking 5 km distance threshold. 20 km half-extent / 30 m cells / ~1 M verts default. Performance slider exposed in Phase 4.8 settings UI.
- [x] **Layer 2 — active cell set**: `Vec<CellId>` of Voronoi cells within `STREAM_RADIUS_M` (20 km) of active camera, refreshed per frame via `SubBiomeCellCatalog::cells_within_radius`. `entered`/`exited` deltas drive 5c content streaming.
- [x] **`app::ActiveCamera`** abstraction — single source of truth; returns sim camera in normal play, debug camera in debug mode.
- [x] **Async mesh build** — rayon-parallel vertex sampling inside the worker thread pattern (`std::thread` + `std::sync::mpsc`). Main thread sees only ~6 ms GPU upload at finalize.
- [x] **Disc-clipped mesh** (not rectangular) — consistent horizon distance in all directions (square gave 14 km axial vs 20 km diagonal).
- [x] **Toroidal wrap handling** via `wrap_coord`.
- [x] Full design spec: [MAPGEN.md § Render streaming](MAPGEN.md#render-streaming--uniform-terrain-mesh--voronoi-cell-active-set-pinned-2026-04-23) + memory `streaming_render_architecture`.

**3.6.3c.5a — Debug camera (Task #223): SHIPPED 2026-04-23.**

- [x] Slew-mode free 6-DoF camera — WASD horizontal, Left Shift / Left Ctrl vertical, mouse Δ for yaw + pitch, Tab cycles speed (**50 / 100 / 500 / 1000 / 5000 m/s** — 1 km/s + 5 km/s tiers added 2026-04-23 for world-scale sweeps), backtick (`) toggles in/out, ESC releases mouse capture.
- [x] `app::ActiveCamera` integration.
- [x] Minimal debug overlay: position (X/Y/Z world coords), heading, speed-tier, FPS + frametime, active-cell-count, **instance count** (added 5c.1), streaming mesh center + rebuild status.
- [x] Sim-carried-with-camera: `vehicle.position = debug_camera.position` + `sim.camera.follow(dt=10.0)` every frame in debug mode. Prevents "crashed while away" physics problem AND keeps chase-cam converged so debug-exit doesn't cascade streaming rebuilds.
- [x] Position stored as `DVec3` (f64) to preserve sub-meter integration at large world coords — f32 at 5 Mm ULP ~0.5 m rounded the 0.83 m/frame at 50 m/s to zero.
- [ ] Screenshot (F12 → PNG to `target/screenshots/`) — deferred; user can capture with external tools meanwhile.

**3.6.3c.5c.1 — Feature-instance streaming MVP (Task #227): SHIPPED 2026-04-23.**

- [x] `crates/world/src/feature_instances.rs` — `PrimitiveKind::Box`, `FeatureInstance` struct, `compose_instances_for_feature` with hardcoded `dwelling_cluster` composition (5-8 box houses per cell, terrain-Y-anchored).
- [x] `BakedAssignments` Phase C — sparse `HashMap<CellId, Vec<FeatureInstance>>` built via rayon `filter_map` gated by `AUTHORED_FEATURE_IDS`.
- [x] `crates/render/src/instances.{rs,wgsl}` — storage-buffer instanced draw pipeline. `cull_mode: None` because `CpuMesh::cube` has mixed winding (5c.2+ revisits once authored meshes land).
- [x] `game_app` wires active-cell-set refresh → instance upload. Tracing log emits active/entered/exited/populated_in_active/instances_uploaded per upload.
- [x] **`FLOWPLAN_FEATURE_INSTANCES=1`** dev toggle gates the ~30-60 s cold bake. Off by default to keep dev-iteration fast. Serialized preload is a 3.7 save-format target.
- [x] **`FLOWPLAN_SPAWN_NEAR_INSTANCE=1`** dev toggle — teleports debug camera to populated cell nearest origin (toroidal distance), auto-activates debug mode, pre-centers streaming mesh at that position. Prints nearest-8 populated cells with `(x, y, z) ~N km from spawn` for navigation. Eliminates "fly 200 km to find something" dev cost.
- [x] Hardware-gated `instance_preview` test writes reference PNG to `target/instance_preview/` (no insta gate, iterative debug tool). Pair with `instance_preview_far` at 2.5 Mm offset — both visually identical post-5d.

**3.6.3c.5d — Camera-relative rendering (Task #234): SHIPPED 2026-04-23.**

- [x] `perspective_view_proj` returns `proj * rotation_only_view` (no translation column). Documented in [DESIGN § Rendering Architecture → Camera-relative rendering](DESIGN.md#camera-relative-rendering-phase-363c5d-invariant).
- [x] Vertex shaders (`shader.wgsl` / `water.wgsl` / `instances.wgsl`) subtract `camera.view_pos` from world positions before the matmul. GPU-side precision preserved at 5 Mm world coords.
- [x] Regression tests: `view_proj_has_no_translation_column` (unit) + hardware-gated `instance_preview_far` (visual).
- [x] **Sync-rebuild gate** (followup): `StreamingTerrain::maybe_rebuild` tests jump distance > `TERRAIN_HALF_EXTENT_M`; if so, drops any in-flight worker and sync-rebakes on the main thread. Normal play (<5 km/frame) never triggers this — only debug-cam teleport scenarios. Prevents "empty scene for ~1 s" artifact on debug ↔ sim toggle.

**Known limitation**: sim-side f32 precision at extreme world coords still jitters dynamic objects (chase-cam smoothing + aircraft physics integration). Invisible during normal play (aircraft takes hours to reach >1 Mm via flight). Surface-only via debug-cam teleport. Three fix options documented in memory `render_precision_at_world_scale`: (A) ship-it + document — **current**; (B) debug-mode-only render-origin rebasing (~1 h); (C) full end-to-end CPU f64 camera-relative refactor (~1 day, may include sim-side `DVec3` position upgrade).

**3.6.3c.5c.2 — Primitive vocabulary expansion + RON-authored composition: pending.**

- [ ] Extend `PrimitiveKind` enum: `Cone`, `Cylinder`, `Sphere`, `HalfCylinder`, `FlatQuad`, `StickTree` (composite).
- [ ] `CpuMesh::primitive_*` builders for each (unit-sized, consistent winding → can re-enable `cull_mode: Back` on the instance pipeline).
- [ ] RON schema extension: each `SubBiomeFeature` declares its primitive composition (list of `(PrimitiveKind, count, size_range_m, color_hint)` tuples). Replaces the hardcoded `dwelling_cluster`-only branch in `compose_instances_for_feature`.
- [ ] Covers all 18 3.6.3b features (currently 17 return empty instance lists).

### Developer toggles

Env-var dev tools documented in PROGRESS.md iteration-11 + memory `render_precision_at_world_scale`:

- `FLOWPLAN_AIRCRAFT=<id>[:<variant>]` — catalog-slot selector (Phase 2.5).
- `FLOWPLAN_FEATURE_INSTANCES=1` — enable 3.9 M-cell bake for instance rendering at startup (Phase 3.6.3c.5c.1; ~30-60 s cold cost).
- `FLOWPLAN_SPAWN_NEAR_INSTANCE=1` — teleport debug camera to nearest populated cell, auto-activate debug mode, pre-center streaming mesh (Phase 3.6.3c.5c.1). Requires `FLOWPLAN_FEATURE_INSTANCES=1` to also be set.

### Milestone 3.6.4: Flora, decoration, seasonal overlay

- [ ] Flora representation (billboards / instanced meshes); per-biome density + species tables. **Consider splitting:** if 3.6.1-3.6.3 grow, 3.6.4 can slip to Phase 5 polish without blocking Phase 4.
- [ ] Rocks, boulders, snags, driftwood — placed procedurally.
- [ ] Snow overlay per season (altitude-gated; driven by 3.2 weather / season state).

### Deliverable (Phase 3.6)

The world is properly generated: coherent hydrology (rivers flow downhill, lakes form at minima, coastlines trace sea level), biomes are climate-gradient-coherent with real-world-inspired aesthetics, shore classification is available for 2.5b.7 and hub placement, flora populates each biome. `Terrain::flat*` regression tests still pass. All existing Phase 2 / 2.5 bench scenarios still pass unchanged.

> **Gate (Phase 3.6):** the quality gate passes in CI. World-bake hydrology graph produces byte-identical output across Linux / Windows / Steam Deck given the same master seed (proves cross-platform determinism for world-gen, not just sim). All Phase 2 / 2.5 bench scenarios pass unchanged. New procedural world passes visual snapshot at a canonical frame. Frame budget holds with the new terrain + flora at reference tier. `world::surface_at` sampler available, unblocking 2.5b.7.

---

### Pre-Phase-4 design-doc commitments

Phase 4 implementation work requires several design decisions that can be frozen earlier. These are **design doc deliverables**, not code — authored in Phase 3 / 3.6 (or earlier) so Phase 4 milestones have a concrete spec to build from.

- [x] **[MAPGEN.md](MAPGEN.md)** — map generation / chunk model / hydrology / biomes. **DRAFT SHIPPED** (initial version authored during Phase 2.5 close-out stock-take).
- [ ] **UI_ARCHITECTURE.md** — UI framework decision (egui for settings/menus vs hand-rolled vs hybrid), navigation model, prompt system, toggle-able debug overlay. Target: vector/math/shader-drawn UI with minimal bitmap assets (see [FEATURES.md § UI Direction](FEATURES.md#ui-direction)). Unblocks Milestones 4.8 / 4.8.5 / 4.9.
- [ ] **LIVERY.md** (or expanded FEATURES.md § Livery & Customization) — paint-editor representation choice (UV-space / decal projection / hybrid), brush types, layer model, undo-history scope, save format. Unblocks save-format lock in 4.10 and Paint Shop in 5.3.

---

## Phase 4: Delivery Loop

**Goal:** a complete playable loop. Hub -> destination -> return, with cargo affecting flight, damage persisting, economy and reputation progressing, and discovery unlocking content. No polish yet.

**Prerequisite (world-scope decision):** resolved during the [Pre-Phase-3 world-scope prototype](#world-scope-prototype-unblocks-phase-36--phase-4) and then implemented in [Phase 3.6 World Foundations](#phase-36-world-foundations). By the time Phase 4 opens, the world-scope decision is locked, hydrology + biomes are generated coherently, and `world::surface_at` is available.

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

### Milestone 4.3b: Takeoff / landing scenario buildout + execution-quality variants

Depends on 2.5b physics + 4.3 damage detection hooks. Authors the full comprehensive takeoff/landing scenario set — including variants that exercise off-nominal technique.

- [ ] Nominal takeoff + landing for each `(aircraft × variant)` combo: paved/grass/gravel/sand for wheels, calm/chop/glassy for floats. Fills the entries currently marked `[blocked: 2.5b]` / `[blocked: 2.5a]` in [`skills/flowplan-flight-bench/references/behavior-taxonomy.md`](../skills/flowplan-flight-bench/references/behavior-taxonomy.md) §5–6.
- [ ] **Execution-quality variants** per `behavior-taxonomy.md` §6b — buttered / firm / harsh / bounced touchdowns, porpoise-on-rollout, tail-strike (rotation + flare), wingtip-strike, wheel-barrow, ground-loop, long-flare/float, short-approach, runway-overrun, go-around, crosswind (crab + wing-low), glassy-water, rough-water, aborted-takeoff, engine-failure-after-rotation.
- [ ] Each variant gets its own `FlightScenario`; assertions target the *distinguishing* quantity (buttered: sink < threshold; harsh: sink > threshold; bounced: tick-range where vertical velocity re-goes-positive; tail-strike: event raised at expected pitch angle).
- [ ] Use `ControlLaw` closed-loop controllers (not open-loop scripted trims) for pilot-quality variants — controller targets the specific quality metric so variants are reproducible across coefficient tuning.
- [ ] Damage consequences wire through 4.3: hard-landing → gear fatigue event; tail-strike → tail structural damage; prop-strike → engine damage + prop bend. Scenarios validate that the detection → damage → flight-model-modifier chain closes correctly.
- [ ] Per-variant CSVs feed the (aircraft × variant) matrix harness (shipped 2.5b.2 follow-up), so every combo gets its quality spectrum captured in the bench gate. Expected scenario count ~3-4× current (25 → ~80-100), matching real pilot training-maneuver coverage.
- [ ] Cross-links: [`behavior-taxonomy.md` §6b](../skills/flowplan-flight-bench/references/behavior-taxonomy.md) is the authoritative catalog; this milestone is the "author them against shipped physics" slot.

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
- [ ] **Capability-requirements record per mission** (see [FEATURES.md § Mission capability requirements](FEATURES.md#mission-capability-requirements)): `service_ceiling_m`, `range_nm_required`, `required_gear`, `cargo_capacity_kg`, `electronics_min`, `aircraft_class_hint`. Requirements computed at mission-generation time from source + destination hubs + heightfield maxima + cargo specs.
- [ ] **Capability check against player's current aircraft + loadout** (five-category capability vector per [FEATURES.md § Aircraft Configuration & Upgrades](FEATURES.md#aircraft-configuration--upgrades)). Result: green / yellow / grayed status per mission.
- [ ] Sequential progression: completing a template unlocks a harder variant (load, distance, multi-stop, time pressure, adverse weather)
- [ ] Mission board at hub (and radio-board for remote hubs). **Grayed-mission display** with specific failure reason + fleet-swap hint + upgrade hint per [FEATURES.md § Grayed-mission capability display](FEATURES.md#grayed-mission-capability-display).
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
- [ ] Autopilot tiers (upgrade-gated): hold altitude, hold vertical speed, hold roll, hold turn, hold heading, hold all, full-auto to waypoint + circle, **approach + land** (uses `GlideToTouchdown` ControlLaw). **Control laws already shipped in `crates/sim/src/control.rs` as the `ControlLaw` enum — `BankHold`, `AltitudeHold`, `AirspeedHold`, `VerticalSpeedHold`, `CoordinationHold`, `HeadingHold`, `GlideToTouchdown` — shared with the scenario bench (Milestones 2.4.9c + 3.2.3c-audit).** Phase 4.7 work is UI (mode select, engage/disengage, readouts), session state (which laws active), and authority limits (turbulence disengage). Bench scenarios validate law behavior by construction.
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
- [ ] **2D hub map** (hotkey-toggled, typical `M`) per [FEATURES.md § 2D hub map](FEATURES.md#2d-hub-map-hotkey-accessible-navigation-overlay). Reads from the world-bake hub catalog + flight-log-memory discovery flags + cartographer unlocks — works world-wide, not limited to streaming disk. Cel-shaded hand-drawn aesthetic; **vector-drawn with zero bitmap map-tile assets** per [FEATURES.md § UI Direction](FEATURES.md#ui-direction).
    - Tier-differentiated hub iconography; undiscovered hubs hidden or hinted per cartographer progression.
    - Current aircraft position + heading overlay; mission-destination route line; fuel-range ring; pinned waypoints; pan/zoom; snap-to-aircraft hotkey.
    - Biome shading + hydrology-graph overlay (rivers / lakes / coastlines) at low zoom; terrain contour hints at high zoom — all drawn from pre-baked world data.
    - **Mission capability overlay:** when a mission is selected on the map, destination hubs render green / yellow / grayed per the capability check — matches the mission-board display (Milestone 4.5).
    - Frame-cost target: ≤ 1 ms at reference tier when map open.
- [ ] Logbook view summarizing flights; cross-linked to the 2D hub map (hover a discovered hub → that hub's logbook entries).

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

### Milestone 5.8: NPC pilot AI + ambient world traffic (post-v1 candidate; may slip)

**Goal:** the world feels alive with other pilots, not empty. NPC-piloted aircraft conduct safe pattern work around the player, with light player-facing interaction and theatrical fallback behaviors. Entirely consumes the `ControlLaw` infrastructure shipped in Phase 2.4.9c + 3.2.3c-audit.

**Design anchor:** memory `npc_pilots_emergent_experiences`. Core direction:
- NPCs conduct **safe landings + takeoffs** around the player using `GlideToTouchdown` + `CoordinationHold` + `AirspeedHold` stacked controllers.
- **Radio announcements** (text-on-radio-band, eventually synthesized voice) — "flowPlan 42, 3-mile final" etc.
- **Lightweight player interaction** via d-pad / hotkey: yes/no/silence. Silence = defer/go-ahead. Low-cognitive-load design.
- **Hub-capacity awareness.** NPC checks hub runway/dock occupancy before committing; if player + NPC would exceed capacity, NPC performs **single-pontoon skim touch-and-go** as theatrical fallback and diverts to an alternative hub.
- **Per-NPC skill variation** (ControlLaw gains) + personality (chatty vs laconic radio).
- Deterministic per `(master_seed, sim_time)` so NPC traffic is replayable + composes with ghost-plane MP (Phase 6).

**Scope drivers:**
- Forest-service tankers looping fire sites
- Medevac hub-to-hub runs
- Cargo haulers
- Ambient pattern traffic at tier-1/2 hubs
- Barn-find solo pilots in remote areas
- NPC weather-reactions (diverting around storms, waiting for weather to clear)

**Prerequisites:**
- Hub-catalog occupancy field (runtime state on top of Phase 3.6 hub catalog) — tracks which aircraft are approaching / parked / taxiing.
- Per-NPC persistent state (ControlLawState + aircraft spec + mission intent + skill/personality config).
- Streaming infrastructure for NPCs (they exist + simulate in a radius around the player, not world-wide).
- Audio system (Phase 3.1+) for radio-call events + distant engine sounds.
- Controller composition + state-machine for maneuver chaining (GlideToTouchdown → SingleFloatSkim → TakeoffClimb for the theatrical skim).

**Status:** design direction captured; implementation falls outside Phase 5 release but the foundation (ControlLaw architecture, deterministic bench scenarios) is shipped. Whether this lands in v1 or slips post-v1 depends on Phase 5 schedule pressure.

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
