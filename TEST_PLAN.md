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
