# flowPlan Test Plan

flowPlan's test suite is the primary feedback loop that keeps development on rails. This document defines the testing strategy, tiers, and gates that all code must pass.

Agents are fast and capable but accumulate subtle bugs and technical debt across iterations without guardrails. The test suite is the guardrail, especially for a system with strict real-time invariants ([DESIGN.md](DESIGN.md#invariants)) and a frame budget ([DESIGN.md](DESIGN.md#performance-budget)).

## Contents

- [Principles](#principles)
- [Test Tiers](#test-tiers)
- [Architectural Enforcement Tests](#architectural-enforcement-tests)
- [Hot-Path Enforcement Tests](#hot-path-enforcement-tests)
- [Frame-Budget Tests](#frame-budget-tests)
- [Determinism Tests](#determinism-tests)
- [Flight Test Bench](#flight-test-bench)
- [World-Gen Visual Bench](#world-gen-visual-bench)
- [Visual Snapshot Tests](#visual-snapshot-tests)
- [Audio Callback Tests](#audio-callback-tests)
- [Test Fixture Strategy](#test-fixture-strategy)
- [Mocking Boundaries](#mocking-boundaries)
- [Quality Gate](#quality-gate)
- [Coverage](#coverage)
- [Regression Policy](#regression-policy)

---

## Principles

1. **The reference hardware is the oracle for performance.** If flowPlan hits the frame budget on the reference tier with headroom, performance is correct by construction.
2. **Determinism is the oracle for simulation.** For a given seed and input sequence, the sim produces bit-identical state.
3. **Test failure paths, not just success paths.** Empty input queues, device-lost events, asset load failures, ring-buffer overflow, water sample out of bounds: each gets a test.
4. **Every bug fix includes a regression test.** Test fails before the fix, passes after. No exceptions.
5. **Unit and integration tests run fast.** They do not require a GPU, an audio device, or a window. Frame-budget and visual tests are a separate tier.
6. **Untested code does not ship.** If a function or code path has no test, it is not done. The exceptions are `render`/`audio`/`app` paths that cannot be tested without real hardware; those are tiered separately.

---

## Test Tiers

### Tier 1: Unit tests

**Location:** inline `#[cfg(test)] mod tests` blocks, plus `crates/*/tests/unit_*.rs` for public API.

**Scope:** Individual functions, types, and small modules in isolation. Dependencies on external systems are either pure (`core`, `sim`, `world`) or mocked at the adapter boundary.

**Speed:** Fast. Whole workspace unit tier completes in under 10 seconds.

**GPU/audio required:** No.

**Coverage expectations:**

- `core`: >=90% line coverage; every public function has at least one test.
- `sim`: >=85% line coverage; property tests for physics, deterministic round-trips.
- `world`: >=85% line coverage; unit tests for terrain/water sampling, weather advancement.
- `assets`: >=70%; loader paths covered with synthetic input buffers.
- `render`, `audio`, `app`: >=50% (the portions that are testable without real hardware; rest falls to Tier 2/3).

### Tier 2: Integration tests

**Location:** `tests/` at the workspace root.

**Scope:** Cross-crate interactions without real hardware. Sim+world stepping, sim+audio ring flow, sim+input queue draining. Mocks replace `wgpu::Device`, `cpal::Stream`, `winit::Window`.

**Speed:** Under 60 seconds for the whole tier.

**GPU/audio required:** No (mocked).

**What they test:**

- Input flows from queue to sim to snapshot.
- Sim publishes to the audio SPSC ring; a mock consumer drains correctly.
- Save/load round-trips produce identical state.
- Architectural enforcement (import linting).

### Tier 3: Hardware-dependent tests

**Location:** `tests/hardware/` gated by `#[cfg(feature = "hardware-tests")]` or a build script that skips when hardware is missing.

**Scope:** Real wgpu device, real cpal stream, real window (headless where possible via `winit`'s headless mode, otherwise gated).

**Speed:** Under 5 minutes.

**GPU/audio required:** Yes.

**What they test:**

- Pipeline creation succeeds on the current GPU.
- Audio stream opens and receives callback invocations.
- Visual snapshot tests against a reference frame.
- Shader compilation (WGSL -> SPIR-V or native).
- Device-lost handling.

### Tier 4: Frame-budget benches

**Location:** `benches/*.rs` via `criterion`.

**Scope:** Realistic scenes measured against the performance budget from [DESIGN.md](DESIGN.md#performance-budget).

**Runs:** Per-phase gate; on-demand during development via `cargo bench --bench frame_budget -- --test`.

**What they test:**

- Per-pass GPU time (depth, opaque, water, sky, post) on a standardized scene.
- CPU frame time on that scene.
- Sim tick cost averaged over 10 seconds of simulated time.
- Audio callback cost (synthetic invocation of the full patch graph).

A phase gate fails if a regression breaches the budget without an explicit, time-boxed exception.

### Tier 4b: Perf-budget integration tests (PINNED 2026-04-25)

**Location:** `crates/<crate>/tests/perf_budgets.rs` (currently `crates/world/tests/perf_budgets.rs`).

**Scope:** Hot-path timing assertions. Each test runs a representative workload, measures wall-clock, asserts elapsed-time stays under a documented budget. Failures = regressions.

**Runs:** Every `cargo test` run for the cheap tests; `--ignored` opt-in for the bake-dependent ones (~30-60 s setup).

**Difference from Tier 4 criterion benches:**

- Criterion is excellent for tuning sessions but requires explicit running + result interpretation.
- Perf-budget tests catch regressions automatically on every test run — the always-on regression net.
- Use both: criterion for fine-grained tuning, perf-budget tests for "is the hot path fast enough?"

**Discipline when a budget fails:**

1. Reproduce locally on the same hardware (timings are hardware-dependent — adjust if running on much weaker / stronger hardware).
2. Profile (`cargo flamegraph` or `perf record`).
3. Find the recent change that pushed cost over budget; revert / optimize / re-architect.
4. ONLY relax the budget if the new cost is genuinely irreducible AND acceptable to the project. Document in PROGRESS.md.

**Currently shipped budgets:**

- `Terrain::height` L1 — 100 K samples in <1 s (10 µs/call)
- `WorldRefs::height_at` L1+L3-with-baked — 100 K samples in <5 s (50 µs/call). Catches `BakedAssignments`-not-threaded regression that previously left compute_delta_at re-running `assign_feature` per query.
- `cells_within_radius` — 100 calls @ 22 km radius in <10 ms/call. Catches streaming-cell-set query regressions.

**First win caught**: hub-foundation branch in `compute_delta_at` had a dead `iter().find()` over ~55 K hubs; both `map_or` arms returned `ctx.base_altitude_m` regardless. ~83 ms / hub-cell query of pure waste. Removed → 157 µs/call → 3 µs/call (52× speedup). Without the perf test this would have shipped silent into playtest.

---

## Architectural Enforcement Tests

Enforces the dependency DAG from [DESIGN.md](DESIGN.md#dependency-rules) as machine-checked rules. Without this, architectural drift is invisible.

### What's enforced

1. **Crate dependency DAG.** A test parses each crate's `Cargo.toml` and fails if:
    - `core` depends on any flowPlan crate.
    - `sim` depends on `wgpu`, `winit`, or `cpal`.
    - `audio` depends on `wgpu` or `winit`.
    - `render` depends on `audio`.
    - Any crate depends on `app`.
    - Any back-edges appear in the DAG.
2. **Forbidden-crate list.** Adding any of the following to a `[dependencies]` table fails CI:
    - `bevy*`, `fyrox*`, `specs`, `legion`, `hecs` (no ECS engines per non-goals)
    - `tokio`, `async-std` (async unnecessary for this workload)
    - `egui` except in the `render` crate under `#[cfg(feature = "debug-ui")]`
    - `imgui`, `imgui-winit-support`
3. **Version pin audit.** `Cargo.toml` entries that are unpinned (`*`) or git dependencies fail CI.

### Example test

```rust
#[test]
fn core_has_no_flow_dependencies() {
    let manifest = Manifest::load("crates/core/Cargo.toml");
    for dep in manifest.dependencies() {
        assert!(
            !FLOW_CRATES.contains(dep.name()),
            "core depends on flow crate `{}`, violating DAG",
            dep.name()
        );
    }
}
```

### Placement

Architectural tests live in a dedicated crate `tests/workspace-rules/` so they compile and run via the workspace-wide `cargo test`.

---

## Hot-Path Enforcement Tests

Real-time invariants from [CODING_RULES.md](CODING_RULES.md#real-time-discipline) are enforced by automated tests, not just code review.

### Allocation assertion

In debug builds, hot paths are wrapped in `assert_no_alloc::assert_no_alloc`:

```rust
#[test]
fn sim_tick_does_not_allocate() {
    let mut state = SimState::fresh(SEED);
    let input = InputEvents::empty();
    assert_no_alloc(|| {
        state.tick(&input, FIXED_DT);
    });
}
```

Tests exist for:

- `SimState::tick`
- Frame render submit (with a mock wgpu device)
- Audio callback (with a mock device and a synthetic Glicol graph)
- Input queue drain
- Asset handle lookup

### Lock assertion

Where feasible, test-only wrappers verify no locks are acquired in the audio callback. Strategies:

- A custom `Mutex` newtype in test builds that panics on `lock()` within an audio-context guard.
- Static analysis: a lint that flags `parking_lot::Mutex::lock` inside `#[audio_callback]`-annotated functions (custom lint; out of scope for MVP if complex, document and revisit).

### Logging ban

A test grep's the `audio` crate for forbidden patterns (`println!`, `eprintln!`, `tracing::info!`/`debug!`/`trace!` inside the callback function and any function it transitively calls).

---

## Frame-Budget Tests

See [DESIGN.md](DESIGN.md#performance-budget) for targets.

### Scenes

Three standardized scenes live in `benches/scenes/`:

- `scene_minimal.flow` -- flat terrain, sky, one plane, no particles. Baseline.
- `scene_standard.flow` -- curated archipelago segment, 2-3 planes, typical weather, water near shore.
- `scene_stress.flow` -- dense particles, full water tessellation, heavy weather, 10+ planes.

### Measurements

Per scene, criterion reports:

- Frame time total (CPU)
- Per-pass GPU time (reported via `wgpu` timestamp queries when available)
- Sim tick average and p99
- Audio callback synthetic cost (scene-independent but reported)

### Gates

Per-phase gate runs `cargo bench --bench frame_budget -- --test` which fails the build if any scene breaches the budget tier for that phase:

- Phase 0-1: `scene_minimal` under minimum-tier budget.
- Phase 2: `scene_minimal` + `scene_standard` under reference-tier budget.
- Phase 3+: all three scenes under reference-tier budget.

### Refresh-rate coverage

The bench harness exercises the full supported refresh set from [DESIGN.md § Performance Budget](DESIGN.md#performance-budget). Per hardware tier, each scene is run at its target refresh plus two neighbors to verify the interpolation path + accumulator hold under varied frame timing. Explicit test points:

| Tier | Scene | Refresh points tested | Pass criterion |
|------|-------|----------------------|----------------|
| Low / Steam Deck | scene_minimal | 30 / 60 / 90 Hz | CPU frame time <= 50% of period; no sim-tick starvation |
| Minimum | scene_minimal + scene_standard | 60 / 120 / 144 Hz | same, with all visual accents enabled |
| Reference | all scenes | 120 / 144 / 165 / 240 Hz | GPU <= 60% at 240 Hz; CPU frame time <= 50% period at every tested rate |
| High | all scenes | 240 / 500 Hz | budget holds at 500 Hz on `scene_stress` |

Additionally, a dedicated **display-rate-independence** property test re-runs the flight-test bench (`cargo test -p flowplan-sim --test flight_bench`) with simulated render-side timing at 30, 60, 120, and 240 Hz; all four runs must produce byte-identical CSV output, proving sim determinism is preserved regardless of display choice.

### Regression detection

Benchmark results are committed to `benches/baseline.json` (anonymous; no per-developer data). A 10%+ regression fails the PR gate. Intentional regressions require an amendment to the baseline in the same PR with justification.

---

## Determinism Tests

Determinism is a first-class invariant ([DESIGN.md](DESIGN.md#invariants)). Tests enforce it with property-based testing (via `proptest` or `quickcheck`).

### Core properties

```rust
proptest! {
    #[test]
    fn tick_determinism(
        seed: u64,
        inputs in input_event_sequence(1000),
        ticks in 1usize..10_000,
    ) {
        let mut a = SimState::fresh(seed);
        let mut b = SimState::fresh(seed);
        for i in 0..ticks {
            a.tick(&inputs[i], FIXED_DT);
            b.tick(&inputs[i], FIXED_DT);
        }
        prop_assert_eq!(a.state_hash(), b.state_hash());
    }
}
```

### Derivative properties

- **Save/load round-trip:** `deserialize(serialize(state)) == state`
- **Flight energy conservation:** in level flight with no wind and no throttle change, total energy stays within a tolerance (small drift acceptable from integration; test the trend).
- **Stall recovery:** triggered stall recovers within a bounded time window.
- **Water surface sampling equivalence:** CPU sample and GPU sample agree within epsilon at the same (x, z, t).

### Seed-generation uniqueness (pending — Phase 3.7 save-format / seed-creation UI)

When player-facing seed generation lands (currently seeds are hardcoded constants; `MAPGEN.md § World-bake parameters` defines `master_seed: u64` + `WeatherTuning` as the seed bundle), add a battery of tests that verify the seed space produces distinct worlds without collision or correlation:

1. **Coverage:** sample N master seeds (N=1000 or 10000), compute terrain heightfield at a fixed canonical point for each, assert N distinct output values (modulo f32 precision). Same for biome classification + weather cell-state at canonical (x, z, t). Catches accidental seed-reuse at the sub-seed derivation layer.
2. **Bit-adjacency decorrelation:** for seeds differing in 1 bit, assert outputs are statistically uncorrelated (correlation coefficient < 0.1 across a sample grid). Catches weak hash functions that preserve input structure. `splitmix64` should pass trivially — this is insurance against future refactors that introduce weaker derivations.
3. **Sub-seed independence:** for a fixed master seed, assert each derived sub-seed (terrain, weather-Voronoi, terrain-Voronoi, hub-catalog) is bit-different from every other sub-seed. Zero accidental collisions.
4. **Seed-bundle collision:** (master_seed, WeatherTuning) pairs that differ in ANY component must produce distinct worlds. Verifies the `seed_hex = master_seed XOR hash(WeatherTuning)` pattern from `MAPGEN.md § Hub persistence` is collision-free.
5. **Pairwise uniqueness on realistic seed set:** sample 100 random seeds with representative WeatherTuning values, assert all 4950 pairs produce distinct world outputs at a canonical sample point.

These tests belong alongside the existing same-seed / different-seed determinism tests (`crates/core/src/rng.rs`, `crates/world/src/terrain.rs`, `crates/sim/src/hash.rs`) but exercise the **space of seeds** rather than pairs. The save-format scaffolding (Phase 3.7) introduces the seed-UI + seed-persistence that makes this failure mode possible; tests land with that milestone.

Memory: `terrain_noise_primitives_spec` notes the octave-offset convention that prevents gradient-hash collisions at the Perlin-primitive layer; this seed-uniqueness battery generalizes that discipline to the whole seed-bundle layer.

---

## Flight Test Bench

**Scope:** deterministic per-aircraft validation of the flight model against POH figures. Shipped as Phase 2.4.8 (framework + first Beaver scenarios); expands naturally as new aircraft land by adding per-aircraft scenario arrays.

**Rationale:** the flight model covers enough effects now (translational aero, moment-based rotation, engine + prop, flaps, stall physics, secondary couplings, density altitude) that numerical validation is the only way to keep it honest as coefficients are tuned and aircraft are added. "Feel" tests alone can't catch a regression that a 15 % cruise-speed drift would cause.

### Architecture

Lives in `crates/sim/src/flight_bench.rs`. Four types:

- **`ScenarioInitial`** — starting vehicle state (position, velocity, orientation, angular velocity, throttle, flaps, fuel).
- **`ScenarioInput`** — `(tick, InputEvent)` pair; injected into the sim's `InputEvents` at the matching tick.
- **`Assertion`** — enum of validation predicates. Current variants: `AltitudeStableAtEnd`, `ClimbRateAtEnd`, `AirspeedAtEnd`, `NeverStalls`, `EventuallyStalls`, `FinalAirspeedWithin`, `MinAirspeedNearTarget`. Extend as scenarios demand.
- **`FlightScenario`** — bundles `name`, `description`, `aircraft_spec_path` (RON), `initial`, `inputs`, `duration_ticks`, `assertions`.

A scenario array per aircraft (`BEAVER_SCENARIOS`, future `CARBONCUB_SCENARIOS`, etc.) is the per-aircraft test suite. Adding an aircraft = adding POH data to its `AircraftSpec` + a scenario array. The runner handles the rest.

### Runner

Two entry points, both sharing the same `SimState::tick` production code path:

1. **Headless (`run_scenario_headless`)** — advances sim for `duration_ticks`, captures a `BenchSample` (subset of `Telemetry` + tick) each tick, returns `Vec<BenchSample>`. Used by the integration test `crates/sim/tests/flight_bench.rs`.
2. **Visual playback (`cargo run -p flowplan-app -- --scenario <name>`)** — same sim code path, but inputs come from the scenario's scripted trace instead of the keyboard. Player watches in 3D with normal renderer + HUD + widgets. Optional `--log-dir <path>` captures the telemetry CSV; `--exit-on-scenario-end` closes the window automatically (useful for batch runs).

Determinism invariant holds: `(seed, scenario)` → bit-identical log. No wall-clock inputs.

### CSV schema

`BenchSample::csv_header()` / `write_csv_row()` define the file format. One row per sim tick (120 Hz), columns: `tick, time_s, pos_x/y/z, pitch/roll/yaw_deg, vel_x/y/z, airspeed_mps, vs_mps, fpa_deg, alpha_deg, beta_deg, cl, cd, throttle, pitch/roll/yaw_in, flap_deg, engine_kw, prop_eff, fuel_gph, fuel_gal, pitch_M, roll_L, yaw_N, stall_state`. Everything a plot needs, stable schema so downstream scripts don't need to track a migration ledger.

Output location: `target/flight-bench/<scenario_name>.csv`.

### Plot pipeline

`tools/plot_telemetry.py` (matplotlib) reads one or more CSVs and renders per-scenario reference plots (altitude trace, airspeed trace, α trace, moment breakdown, engine power + fuel flow, stall-state ribbon). Committed reference plots live in `docs/plots/`.

`tools/run_flight_bench.sh` orchestrates: runs the headless bench, regenerates all CSVs, invokes the plotter, surfaces pass/fail. Single script for the full loop.

### POH assertion tolerances

Tolerances start deliberately wide and tighten as 2.4.8 coefficient tuning dials the model into POH range. A scenario that fails is a regression — either the physics changed (catch now) or the coefficient was re-tuned away from spec (intentional, update the tolerance). The diff makes the intent explicit.

### When to add a scenario

- A new aircraft lands → copy the Beaver scenario pattern, replace POH targets with the new aircraft's numbers.
- A new flight-model effect lands → add a scenario that exercises it (e.g., 2.4.7 density altitude → "cruise at 3 km, verify engine power drop within band").
- A flight-test feedback surfaces a specific numeric gap → encode it as an assertion so it can't silently re-appear.

### Scenario authoring discipline (hardened 2026-04-22 post-realism audit)

Every scenario MUST have realistic pilot inputs — unless it is explicitly an envelope-limit / no-pilot test. Four buckets:

1. **Open-loop with trim/flare inputs.** Explicit `ScenarioInput` events at specific ticks. Realistic when inputs are thoughtful (e.g., cruise trim to null moments, landing flare pitch sequence, climb-power set at t=0 and held).
2. **Closed-loop PID controllers.** `ControlLaw` entries in `scenario.controllers` that run every tick. Pilot is simulated by construction — `GlideToTouchdown`, `CoordinationHold`, `BankHold`, etc.
3. **Intentionally minimal / envelope-limit.** Sparse or zero inputs by design. Scenario tests what happens with the WRONG or NO pilot (porpoise_emerge holds `SetPitch(0.4)` deliberately, wingtip_scrape is a 600-tick envelope probe). Comment block must say so explicitly.
4. **Missing pilot inputs where realism requires them.** ❌ REJECTED. Either move the scenario into bucket 1/2/3 or revise.

**Assertion discipline.** The assertion must test the scenario's INTENT, not just "the aircraft stayed alive." A landing scenario's primary assertion is touchdown quality (vs at water contact, pitch attitude), not post-crash coast-down airspeed. A takeoff scenario's assertion tests rotation + liftoff, not just on-step acceleration. If you find yourself writing `tol_mps: 10.0` just to get the test to pass, the scenario probably lacks the right assertion — not a wider tolerance.

**Prefer controller-driven scenarios** for capabilities multiple aircraft / future NPC pilots share (landings, coordinated turns, hold-altitude cruise). Open-loop tick-timed inputs are best for scenarios whose purpose is capturing an explicit pilot-trace for inspection. Both are valid; pick per-scenario.

**Regression audits.** When a physics change causes existing scenarios to drift in measured values, the drift is a regression-test signal. Re-baseline the assertion with an inline comment explaining the physics change + documenting the NEW value. Silent tolerance-widening masks the regression's story. See `WATER_RUDDER_COEFF` / `LATERAL_HYDRO_DRAG_N_PER_MPS2` / `INSIDE_FLOAT_STICK_COEFF` tuning history in `crates/sim/src/buoyancy.rs` for the pattern.

### Future integration

Once Phase 5 flight-log / replay lands (`FEATURES.md § Flight Log & Save Format`), the bench's scenario format is the natural seed: a player's captured flight (input trace + seed) becomes a scenario, telemetry is extracted, they share it with another player who replays it bit-identically. The Phase 4 aircraft-description UI can also consume bench output: run the scenario suite at build time, generate per-aircraft performance-envelope plots, commit as cel-shaded content (hand-drawn-feel ROC-vs-airspeed curve on the Beaver's spec page, etc.).

---

## World-Gen Visual Bench

**Scope:** generalize the flight-bench pattern to world-gen subsystems (terrain, weather, biomes, hydrology, hubs, Voronoi partitions). Telemetry format shifts from CSV-per-tick to PNG/SVG-per-layer because world-gen state is inherently visual. Same deterministic-per-seed + committed-reference-artifacts + CI-gate discipline.

**Rationale:** a heightfield is a 2D scalar field; a biome mask is a 2D discrete field; a wind field is 2D vectors; Voronoi partitions are colored polygons. Numerical assertions alone cannot validate these — a human looking at the output can instantly see "this biome adjacency is wrong" or "rivers don't connect at the chunk boundary." The visual-bench produces the artifacts deterministically so the human check is reproducible.

### Architecture

Mirrors the flight-bench:

- **`WorldGenScenario`** = `{master_seed, world_scope_config, layers_under_test, assertions}`. Pinned like a `FlightScenario`.
- **Runner** produces PNG artifacts per requested layer: heightfield, biome classification, hydrology graph, hub catalog, terrain Voronoi, wind vector field, weather Voronoi, frontal-boundary graph, composite overlay.
- **`cargo test` integration** asserts determinism (byte-or-perceptual-identical across runs + platforms) + structural invariants (biome adjacency, hub-count-per-tier, hydrology continuity, altitude envelope bounds).
- **Rendering** via top-down orthographic viewport + direct scalar/vector/polygon rasterization. No game camera, no aircraft involvement.
- **Reference artifacts** committed at `docs/world-gen-plots/<scenario>/<seed>/<layer>.png` — analogous to `docs/plots/beaver/` for flight scenarios.

### `--visual` flag

```
cargo test -p flowplan-world --test world_gen_bench --features visual
```

- Default off (CI runs emit only the deterministic-output hash per scenario for byte-comparison).
- Opt-in: emits full PNG suite to `target/world-gen-plots/`.
- Accept-reference: `--update-refs` (or `cargo insta`-style workflow) promotes new artifacts to the committed `docs/world-gen-plots/`.

### Artifact catalog (canonical scenarios)

- **Heightfield** — color-ramped top-down elevation image.
- **Biome classification** — discrete color-coded regions; verifies adjacency rule.
- **Hydrology graph** — river lines + lake polygons + coastlines overlaid on heightfield; verifies flow continuity.
- **Hub catalog** — scatter plot by tier; verifies placement respects biome + altitude envelope.
- **Terrain Voronoi partition** — cells colored by sub-biome feature type.
- **Sub-biome palette-adaptation** — partition with cells rendered in adapted palette; visualizes gradient transitions.
- **W1 wind vector field** — arrow plot on sample grid; reveals prevailing flow.
- **W1 pressure / temperature / humidity fields** — color-ramped top-down.
- **Frontal-boundary graph** — line structure overlaid on pressure field.
- **W2 weather Voronoi partition** — cells colored by cell-type + storm-state; snapshots at t₀, t+10m, t+1h, t+6h reveal drift + evolution.
- **Composite overlay** — heightfield + hydrology + biome + hub catalog + both Voronoi partitions composited.

### Determinism assertions

- **Byte-exact:** biome classification (categorical output; should be identical every time).
- **Perceptual (SSIM > 0.99):** heightfield (floating-point rendering has minor platform variance).
- **Graph-structural:** hydrology + frontal-boundary graphs compared by node count + edge set equality.
- **Cross-platform:** Linux-Linux + Linux-Windows direct test; Mac + Steam Deck research-based per `hardware_access_constraints`.

### Interaction with `--visual` in-engine mode

The in-engine free-camera debug mode (see [FEATURES.md § Camera & Views](FEATURES.md#camera--views) free-cam evolution path) is the runtime complement to these static artifacts: same data rendered in 3D with interactive inspection. PNG artifacts are CI-gated + committed; free-cam is dev workflow + player photo mode (Phase 5.5).

### Future integration

- **Phase 4 mission system:** mission templates can assert their generated waypoints fall within biome + hydrology constraints (visual-bench verifies placement is reasonable before the template ships).
- **Phase 5 flight-log / replay:** sharing a flight log implicitly shares the master seed; the recipient regenerates the world from seed + visual-benches verify their world matches the log author's.
- **Per-aircraft performance envelopes:** the Phase 4 aircraft-description UI pattern generalizes — per-world analytic plots for trip-planning overlays ("fuel range ring" on the 2D hub map is effectively a runtime visual-bench).

---

## Visual Snapshot Tests

Managed via `cargo-insta`.

### Scope

- Reference frames for lighting, materials, water, sky, outline.
- HUD layouts.
- Post-process chain outputs.

### Workflow

- Developer stages an approved snapshot: `cargo insta accept`.
- Approved snapshots live in `tests/snapshots/`.
- CI fails if a snapshot mismatch is introduced without explicit acceptance.
- Snapshot diffs are reviewed in the PR alongside the code change.

### Limits

- Only run on the hardware tier (Tier 3).
- Tolerance uses perceptual diff (SSIM or similar) rather than exact pixel match to tolerate driver variance.
- Reference images are captured on the reference hardware; "close enough" is what matters.

---

## Audio Callback Tests

Audio gets special attention because real-time breakage is hard to notice until it causes glitches under stress.

### Callback contract tests

- **No allocations:** `assert_no_alloc` wrap (see [Hot-Path Enforcement](#hot-path-enforcement-tests)).
- **No panics on degenerate parameters:** fuzz the ring with random `AudioParamUpdate` sequences and assert the callback never panics.
- **Bounded execution time:** the synthetic callback test asserts a budget ceiling (e.g., 500 us on reference hardware).
- **Ring overflow handling:** when the sim produces faster than audio consumes, the oldest updates are dropped with a counter increment; test this.

### DSP output sanity

- **DC offset:** no sustained DC offset beyond tolerance.
- **Clipping:** no hard clipping under expected parameter ranges.
- **Frequency content:** key patches (engine RPM sweep, wind filter) produce spectra in expected bands.

These run on Tier 1 (synthetic stream, no cpal).

---

## Test Fixture Strategy

Tiers use different fixture sources by design:

- **Tiers 1 and 2 use synthetic fixtures only.** Tests construct heightmaps, input sequences, Glicol graphs, and asset buffers inline. They never read actual game assets from disk. This keeps tests hermetic (upstream asset changes do not break them), fast, and reproducible.
- **Tier 3 (hardware) uses real assets.** Path traversal is real. Shader compilation is real. Visual snapshots reference actual curated scenes.
- **Tier 4 (bench) uses the scene files in `benches/scenes/`.** These are committed fixtures managed with the same rigor as code.

Do not mix tiers. A unit test that reads `assets/models/plane.glb` is brittle and belongs in Tier 3.

---

## Mocking Boundaries

Mock at the right layer or tests pin themselves to implementation details.

- **Tier 1 unit -- mock nothing for pure crates (`core`, `sim`, `world`).** They are already pure; no mocks needed. For `assets`, mock file I/O with in-memory buffers.
- **Tier 2 integration -- mock at the adapter boundary.** Mock `wgpu::Device` via a `DeviceHandle` trait (or use `wgpu`'s validation-only device when feasible), mock `cpal::Stream`, mock `winit::Window`.
- **Tier 3 hardware -- mock nothing.** Real GPU, real audio, real window.

A test that finds itself patching internal implementation details is in the wrong tier or mocking at the wrong layer.

---

## Quality Gate

Every phase, every PR, and every CI run pass the same four checks plus phase-specific additions:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo bench --bench frame_budget -- --test
```

Phase 2+ adds visual snapshot diff verification on hardware-test runners. Phase 3+ runs these in CI.

**Non-negotiable bar.** Phases do not complete, PRs do not merge, CI does not pass unless all four succeed plus any phase-specific test/bench gates.

---

## Coverage

- **Target coverage:** see per-tier expectations in [Test Tiers](#test-tiers).
- **Measurement:** `cargo-tarpaulin` on Linux for CI, run on every PR.
- **Coverage exclusions** (per-line `#[cfg(not(tarpaulin_include))]` or similar) are rare and commented with justification.
- **100% coverage is not a goal.** 100% on pure logic is achievable and desirable; 100% on GPU/audio integration is not.
- **Lack of coverage on new public items is a review flag.**

---

## Regression Policy

- **Every bug fix includes a test** that reproduces the bug before the fix and passes after. The test's location depends on the tier: pure logic bugs get unit tests; performance regressions get a bench baseline entry; rendering regressions get a visual snapshot.
- **Flaky tests are bugs.** If a test fails intermittently, it is quarantined into `tests/flaky/` (ignored in CI) and a GitHub issue opened for it. Quarantined tests block no phase gates but must be reviewed at the start of the next phase.
- **Determinism regressions are P0.** A determinism test that newly fails halts all other work until resolved; determinism is a structural invariant.
- **Budget regressions are P1.** A budget regression must be fixed or explicitly amended in the phase's baseline within the same PR.

---

## Test Code Quality

Tests are code. They follow the same quality rules as production code:

- Same formatting, clippy gates (with the test-file relaxations allowed in [CODING_RULES.md](CODING_RULES.md)).
- No copy-paste test matrices: share setup via `fn`s or `proptest::strategy::Strategy` impls.
- No 500-line test files; if one grows that long, split by concern.
- No untested test helpers that do complex setup; trivial ones are fine.
- No `unwrap()`-happy test code where failure messages matter; use `expect("reason")` for better diagnostics.
