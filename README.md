# flowPlan

A small, lightweight, high-FPS 3D floatplane / bushplane game built around the *feel* of low-altitude flight over quiet water, remote strips, and weather that matters. Deliveries, exploration, wildlife, and the aircraft itself carry the game; no engine framework, no photorealistic PBR, no scripted narrative -- emergent systems do the work.

"flowPlan" is a working title, a pun on *float plane*.

---

## Status

**Phase 2 — Feel Core, in progress.** Phases 0 (Skeleton) and 1 (Core Loop) complete. Currently deep into the flight model (Milestone 2.4): airfoil lift/drag, moment-based rotational dynamics, engine + prop + fuel, flaps, secondary effects (adverse yaw / P-factor / torque roll / ground effect), density altitude, and a flight-test bench for scripted POH validation. Next: Phase 2.5 contact physics (buoyancy + ground friction). See [PROGRESS.md](PROGRESS.md) for the authoritative snapshot — it's kept current tick-by-tick and survives session compaction.

This follows the front-loaded design discipline used in the author's prior projects ([hadlink](https://github.com/Jbsco/hadlink), admt) where specifications are the contract and code traces back to them. See [skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md) for the approach.

---

## Common commands

All commands run from the workspace root. Release builds are recommended for running the app; debug builds work but are noticeably slower.

### Running the game

```bash
# Open the window with full keyboard flight controls.
cargo run -p flowplan-app --release
```

In-window controls:

| Key | Action |
|---|---|
| W / S | throttle up / down |
| ↑ / ↓ | stick forward (dive) / back (climb) |
| ← / → | roll left / right |
| Q / E | yaw left / right |
| F / C | flaps extend / retract (4 detents: UP → CLIMB → TAKEOFF → LANDING) |
| 1 / 2 / 3 | view: first-person / chase / exterior orbit |
| V | cycle view |
| L | reset POV look |
| R | reset vehicle |
| Esc | quit |

Screen layout: top-left `debug` panel (tick, view, vehicle pos/vel/attitude, controls, flaps), top-right `dynamics` panel (airspeeds, α/β/Cl/Cd, stall alert, forces, moments-with-coupling, engine, aircraft spec), bottom-left `glide` panel (FPA / α / β / VS / L/D) above the attitude-indicator widget, bottom-right cel-shaded orientation gizmo (body axes + velocity + force vectors).

### Flight test bench (Phase 2.4.8)

The bench replays deterministic `FlightScenario`s through the production `SimState::tick` code path, capturing per-tick telemetry CSVs that feed the plotting pipeline. See [TEST_PLAN.md § Flight Test Bench](TEST_PLAN.md#flight-test-bench) for the design.

```bash
# Headless: runs all scenarios in crates/sim/src/flight_bench.rs::BEAVER_SCENARIOS,
# writes target/flight-bench/<scenario>.csv, fails on assertion violation.
cargo test -p flowplan-sim --test flight_bench

# Visual playback of a single scenario (same sim code, scripted inputs, normal
# renderer + HUD + widgets). Auto-exits when the scenario's duration elapses
# so shell-driven runs don't need manual intervention.
cargo run -p flowplan-app --release -- --scenario beaver_cruise_sl
cargo run -p flowplan-app --release -- --scenario beaver_vy_climb \
    --log-dir target/flight-bench --exit-on-scenario-end

# Full orchestrator: headless suite → CSVs → PNG plots (requires matplotlib +
# pandas). Add --visual to additionally watch each scenario in sequence
# (game window opens, plays scripted inputs, auto-exits per scenario).
# Add --pause to wait for Enter between visual runs.
tools/run_flight_bench.sh
tools/run_flight_bench.sh --visual
tools/run_flight_bench.sh --visual --pause

# Ad-hoc plot generation from captured CSVs.
python3 tools/plot_telemetry.py target/flight-bench/*.csv

# List all known scenarios.
cargo run -p flowplan-app --release -- --help
```

Scenario names currently in `BEAVER_SCENARIOS`: `beaver_cruise_sl` (level cruise at SL), `beaver_cruise_climb` (full-throttle climb at cruise airspeed), `beaver_stall_clean` (clean stall validation). Expand by appending a `FlightScenario` const to the array; no other wiring needed.

### Quality gate

Every phase ends with the same gate. Not negotiable — nothing moves forward with red bars.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo bench --bench frame_budget -- --test
```

### Smoke run

```bash
# 3-second boot + exit, for CI-style sanity checks.
timeout 3 env RUST_LOG=info cargo run -p flowplan-app --release
```

### Optional environment

```bash
# Silence noisy info logs (most commonly wgpu_hal). The default filter already
# clamps wgpu_core / wgpu_hal / naga / calloop to warn/error; override with
# RUST_LOG if you want something different.
RUST_LOG=info,wgpu=warn cargo run -p flowplan-app --release

# Shader hot-reload (dev-only convenience; feature-gated so release builds don't
# pull the file-watcher in).
cargo run -p flowplan-app --release --features shader-hot-reload
```

---

## What it is

A single-player, cel-shaded, lightweight 3D flight game. The plane is a floatplane or a bushplane (and eventually an amphibian). You fly deliveries, routes, supply runs, and survey missions between hubs of varying capability in regions shaped after real places -- coastal Alaska, boreal lake country, Mediterranean archipelagos, high-desert plateaus -- without ever being authored as a specific real place.

- **Feel over fidelity.** Frame time and response latency are the budget. The game targets 240Hz+ on mid-range hardware with GPU headroom still remaining. Cel-shading is not just the look; it is part of the budget strategy.
- **Emergent, not scripted.** Weather, wind, cargo, damage, fuel, daylight, community demand, and wildlife interact to produce situations. Missions are template-driven and parameterized by world state; no branching quest trees.
- **Sim-realism where it matters.** Aerodynamics are tabulated per-aircraft and faithful (airfoil lift curves, stall character, density altitude, ground effect). Systems are simplified to meaningful toggles (flaps, trim, mixture, feather, carb heat). Difficulty dials tune parameters in-game without swapping physics models.
- **Relaxing, rewarding both ways.** Pick-up-and-play accessible; rewarding for pilots who know what they're looking at.

Full creative pitch in [PLAN.md](PLAN.md).

---

## Design pillars

- Fixed-timestep simulation, interpolated rendering, lock-free audio. Determinism is a first-class invariant.
- Narrow, frozen boundaries between subsystems. `core -> sim / progression -> world -> render / audio -> app`, strict DAG.
- Cel-shaded base + sparing shader accents (Gerstner water, procedural sky, aurora post-v1, astrophotography-grade night sky post-v1).
- One or a few hand-authored or seeded-procedural regions; no open-world content treadmill. Scope locked before Phase 4.
- Non-financial progression protected: wildlife, exploration, and astrophotography unlock cosmetic and navigational rewards that cannot be bought. This guards the "relaxing" pillar against economy-first pressure.

Full treatment in [DESIGN.md](DESIGN.md).

---

## Tech stack (planned)

| Layer       | Choice             | Rationale                                                                |
| ----------- | ------------------ | ------------------------------------------------------------------------ |
| Language    | Rust (stable)      | No GC, predictable frame time, matches Glicol.                           |
| Graphics    | `wgpu`             | Modern GPU access, Vulkan / Metal / DX12 cross-platform.                 |
| Windowing   | `winit`            | Standard pairing with wgpu.                                              |
| Math        | `glam`             | SIMD-friendly gamedev math.                                              |
| Audio DSP   | `glicol_synth`     | Sound-design-oriented, native Rust, graph-based.                         |
| Audio I/O   | `cpal`             | Cross-platform device abstraction; what Glicol targets.                  |
| Asset import| `gltf`             | Standard 3D format; Blender pipeline.                                    |

Deliberately **not** using Bevy / Fyrox / Unity / Unreal. Engines hide the frame; the appeal here is feeling the machine. Full list and version policy in [DESIGN.md § Tech Stack](DESIGN.md#tech-stack).

---

## Repository layout

```
flowPlan/
  CLAUDE.md                              # Entry point for AI agents (read order)
  README.md                              # This file
  PLAN.md                                # Creative vision -- the "why"
  DESIGN.md                              # Technical architecture -- the "what"
  CODING_RULES.md                        # Rust + real-time discipline -- the "how"
  ROADMAP.md                             # Phased execution plan -- the "order"
  TEST_PLAN.md                           # Verification strategy -- the "verify"
  FEATURES.md                            # Feature backlog -- every captured detail
  AIRCRAFT.md                            # Living aircraft catalog with design-model specs
  skills/flowplan-methodology/SKILL.md   # Design-first discipline, applied to game dev
  crates/          # One per subsystem (DAG per DESIGN.md); populated in Phase 0
    README.md      # Layer map
    {core, assets, sim, progression, world, render, audio, app}/
  data/            # Content-only .ron files (aircraft, hubs, cargo, missions, wildlife)
    README.md      # Authoring conventions
  assets/          # Source assets (shaders, models, textures, audio) baked at build time
    README.md
  benches/         # Criterion frame-budget benches; per-phase perf gate
    README.md
  tests/           # Cross-crate integration tests
    README.md
  tools/           # Build-time tools (shader baker, data validators, etc.)
    README.md
```

Subdirectories under `crates/`, `data/`, and `assets/` currently hold only `.gitkeep` files; they populate as ROADMAP phases advance. See [DESIGN.md § Directory Structure](DESIGN.md#directory-structure) for the layout rationale.

---

## How to read this repo

### Humans

Start with **[PLAN.md](PLAN.md)** for the feel of the thing. Then **[DESIGN.md](DESIGN.md)** if the architecture interests you. Then **[ROADMAP.md](ROADMAP.md)** for where the project currently stands and how it will advance. **[FEATURES.md](FEATURES.md)** is the backlog of captured ideas and open questions.

### AI agents

Read the specs *in the order declared in [CLAUDE.md](CLAUDE.md)* before any work in the repo. Skim is not read. Every line of code must trace to a spec section. Ambiguity is resolved by proposing an amendment, not by guessing. See [skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md).

---

## Core principle

> **The simulation holds the invariants. Rendering interprets state at display rate. Audio traces state lock-free. The boundaries are narrow and frozen.**

Every design and implementation decision honors this one-liner.

---

## What flowPlan is not

- **Not an engine.** flowPlan uses `wgpu` directly. No reusable engine will emerge from v1.
- **Not an MMO.** Single-player in v1. Async / co-op hooks considered for post-v1 via shared seeds and deterministic replay.
- **Not photorealistic.** Cel shading is the art direction.
- **Not a full flight sim.** No CFD, no engine-management minigames. Aerodynamics are tabulated and faithful; systems are simplified to meaningful toggles.
- **Not a scripted narrative game.** No authored dialog trees, no branching quests. Missions are template-driven and parameterized by world state.
- **Not content-heavy in the traditional sense.** World scope is bounded; exact shape (single map / regional maps / procedural) is under active review, locked before Phase 4.
- **Not a physics toy.** No rigid-body destruction stacks, no cloth or softbody, no ragdolls.

Full list and rationale in [DESIGN.md § Non-Goals](DESIGN.md#non-goals).

---

## Status badges (planned)

CI / lint / test / frame-budget badges will land when Phase 3 adds the CI workflow. Until then, the local gate is the only gate -- see [ROADMAP.md § Per-Phase Quality Gate](ROADMAP.md#per-phase-quality-gate).

---

## Contributing

This is a personal project. If collaboration opens up, contributions will follow the rules in [CODING_RULES.md](CODING_RULES.md) and the design-first discipline in [skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md):

- Read the specs first. Every line traces to a spec section.
- Propose design amendments *before* writing code that doesn't already trace.
- Phase gates are non-negotiable: `cargo fmt`, `cargo clippy -D warnings`, `cargo test`, and the phase's frame-budget bench all green before any merge.
- One logical change per commit. Imperative subject, capital letter. Explain *why*, not *what*.
- No speculative features, no engine-ification, no allocations or locks in hot paths.

---

## License

TBD. Placeholder pending decision. Likely permissive (MIT / Apache-2.0) for code; assets may carry a separate license once content exists. Decision captured during Phase 0 skeleton work.

---

## Acknowledgements

Methodology borrowed from two prior projects by the author:

- **hadlink** -- formally verified URL shortener. Skeleton-first commit, contract-first design, invariants as structural guarantees, non-goals as scope protection, phased roadmap with testable gates.
- **admt** -- CLI tool built through multi-agent design convergence. Four foundational spec docs as the single source of truth; code that does not trace is rejected.

The front-loaded design discipline these projects demonstrated is the reason flowPlan's specs exist before its code.
