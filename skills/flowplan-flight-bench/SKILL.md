---
name: flowplan-flight-bench
description: Authoring, running, and tuning flight-test-bench scenarios in flowPlan. Use when adding a new FlightScenario, tightening assertion tolerances, interpreting telemetry plots, or iterating on flight-model coefficients against POH targets. Covers trim seeding, assertion-to-target matching, the coefficient-tuning loop, and the headless/visual execution paths.
---

# flowPlan Flight Test Bench

The bench is a deterministic scenario runner for validating the flight model against per-aircraft POH figures. This skill covers **how to use and extend it**. The architecture is in [TEST_PLAN.md § Flight Test Bench](../../TEST_PLAN.md#flight-test-bench); the runner itself is [`crates/sim/src/flight_bench.rs`](../../crates/sim/src/flight_bench.rs).

If you are picking flight coefficients out of thin air or eye-balling altitude traces in the live game, you are working wrong. The bench is the feedback loop.

## Core Rules

1. **One scenario = one real-world behavior, in isolation.** A scenario names a specific real-world maneuver or envelope corner -- "level cruise holds altitude," "Vy climb achieves 1000 fpm," "short-field landing with full flaps on a paved runway, no crosswind." Every *other* variable is held constant so a failure points at one thing. Don't fold two behaviors into one scenario; split them. If you're asserting two different physics effects, you have two scenarios pretending to be one.

   Corollary: **"What this does NOT test"** is as important as what it does. The cruise scenario does not test gust response; the stall scenario does not test spin recovery; the Vy scenario does not test service ceiling. Naming non-goals keeps the assertions honest and prevents scope creep when tolerances tighten.
2. **Seed near trim, not at α=0.** An aircraft released at nose-level produces zero lift, drops, and biases every assertion. Use `CRUISE_ATTITUDE` (3° nose-up) plus a small pilot-hold pitch input to start in a steady state. See [references/trim-seeding.md](references/trim-seeding.md).
3. **Pilot-hold inputs stand in for trim.** Until a trim system ships (Phase 3+), scenarios must inject a `SetPitch(x)` at tick 0 to hold α. 0.05 for cruise, 0.10 for Vy climb, 0.30 to drive a stall. Document *why* in the scenario description.
4. **Tolerances start wide; commits tighten them.** Every assertion starts with a tolerance that the current coefficients can satisfy. You only tighten after a coefficient change demonstrably improves the telemetry. A tolerance that never shrinks is a test that never validates.
5. **The plot is the primary output, not the pass/fail.** Pass/fail gates CI. The CSV + plot tells you *why* it failed and *what* to change. Always read the plot before editing coefficients.
6. **Never script a workaround around a bug.** If a scenario needs unrealistic inputs to pass, the flight model is wrong, not the scenario. Fix the physics.
7. **Headless for determinism, visual for feel.** Headless (`cargo test -p flowplan-sim --test flight_bench`) is the CI gate. Visual (`cargo run -- --scenario <name>`) is for watching a scenario play out through the HUD/gizmos when plots alone aren't enough.

## Scenario Anatomy

A `FlightScenario` has five parts. Each has a job:

| Field | Job |
|-------|-----|
| `name` | CSV filename. snake_case, aircraft-prefixed (`beaver_cruise_sl`). |
| `description` | One-line intent. Read in the plot title. |
| `aircraft_spec_path` | Workspace-relative RON. Loaded fresh each run. |
| `initial: ScenarioInitial` | Position, velocity, orientation (usually `CRUISE_ATTITUDE`), angular vel (usually zero), throttle, flaps, fuel. |
| `inputs: &[ScenarioInput]` | Scripted events at specific ticks. Almost always a `SetPitch` at tick 0 to hold attitude. |
| `duration_ticks` | At 120 Hz: 3600 = 30 s. Stall scenarios run shorter (~20 s) so the log doesn't explode post-stall. |
| `assertions` | See assertion table below. |

## Assertion → Target Matching

Match the assertion to the thing the scenario actually validates.

| Scenario intent | Assertion | Tolerance starts at |
|-----------------|-----------|---------------------|
| Cruise holds altitude | `AltitudeStableAtEnd { last_secs: 5, tol_m: 100 }` | 100 m, tighten toward 20 m |
| Cruise speed reached | `AirspeedAtEnd { target_mps, tol_mps: 5 }` | ±5 m/s, tighten toward ±2 |
| Vy achieves ROC | `ClimbRateAtEnd { target_mps: 5, tol_mps: 5 }` | ±5 m/s, tighten toward ±1 |
| No stall on benign scenario | `NeverStalls` | binary |
| Stall-inducing scenario must stall | `EventuallyStalls` | binary |
| Stall speed matches Vs | `MinAirspeedNearTarget { target_mps: Vs, tol_mps: 8 }` | ±8 m/s, tighten toward ±2 |
| Speed lands in a band (Vne test) | `FinalAirspeedWithin { min_mps, max_mps }` | wide |

See [references/assertion-matrix.md](references/assertion-matrix.md) for per-scenario rationale.

## The Coefficient-Tuning Loop

This is the loop. Do not skip steps:

1. **Run the bench** (`tools/run_flight_bench.sh` or `cargo test -p flowplan-sim --test flight_bench --release`). Writes CSVs + PNGs to `target/flight-bench/`.
2. **Read the plot for the scenario that's furthest off target.** Altitude trending one direction? Airspeed oscillating? FPA not converging? Identify the *symptom* before the fix.
3. **Map symptom to coefficient.** See [references/symptom-map.md](references/symptom-map.md). Examples:
   - Altitude drifts down in cruise → Cm_α or throttle-to-thrust curve too weak.
   - Dutch-roll oscillation → CN_R (yaw damping) too small.
   - Elevator feels glassy → CM_DELTA_E too small relative to |CM_ALPHA|.
   - Stall triggers in benign flight → ALPHA_STALL_RAD too low or CL table peaks too early.
4. **Change one coefficient.** Not two. Not "one, and while I'm here, this other one." One.
5. **Re-run the bench.** Compare plots before/after.
6. **If improved and the assertion now has margin, tighten the tolerance** in the same commit. If unchanged or worse, revert.
7. **Commit.** Message: `Phase 2.4.8: narrow cruise altitude tolerance to 50m (CM_ALPHA -0.25 stable)`. Cite the coefficient change + the new tolerance.

A single commit may only touch one scenario's tolerance(s) and at most one flight coefficient. If you need two coefficient changes, that's two commits.

## Visual Playback

When a plot can't explain it -- usually transient attitude oscillations or gizmo-visible coupling -- run the scenario visually:

```sh
cargo run -- --scenario beaver_cruise_sl --log-dir /tmp/flowplan --exit-on-scenario-end
```

The scripted inputs drive the sim; keyboard input is ignored during playback. The HUD, dynamics pane, glide gizmo, and orientation widget all work as in free flight, so you can *see* which axis is misbehaving. CSV is written on exit to `--log-dir`.

Use visual playback only when headless + plot aren't enough. Don't bake visual reproduction into the CI loop.

## Adding a New Scenario

Checklist:

1. **Name the real-world behavior.** Write the one-line description as a specific maneuver: "Short-field landing, paved runway, no wind, full flaps, MTOW." Not "landing test." If you can't name the maneuver precisely, the scenario is underdefined.
2. **Cite the POH figure.** Every assertion must map to a published number (POH ground roll, POH stall speed, POH ROC). "Feels right" is not a target. See [references/poh-targets-beaver.md](references/poh-targets-beaver.md) for the Beaver's table.
3. **List the isolation variables.** What is deliberately held constant so the assertion points at one thing? Typical: sea-level altitude, zero wind, MTOW, flaps up, no flap transition during the window. Document these in the scenario's comment block (template below).
4. **State what this scenario does NOT test.** Explicit non-goals prevent tolerance creep. Example: a cruise scenario does not test phugoid damping even though it would manifest in the altitude trace.
5. **Pick initial conditions.** Altitude 500-1000 m MSL unless density altitude is the point. Airspeed at the test point. `CRUISE_ATTITUDE` unless testing a non-level attitude (takeoff rotation, steep approach, unusual-attitude recovery).
6. **Pick the pilot-hold input.** Start with 0.05 for cruise, 0.10 for Vy, 0.30 for stall induction. Adjust only if the first run overshoots/undershoots. Document the value's purpose ("hold α high as speed decays").
7. **Set `duration_ticks` conservatively.** 30 s (3600) for steady-state; 20 s (2400) for stall induction; 60 s (7200) for endurance trends; 10-15 s for takeoff/landing roll isolation.
8. **Write one assertion per scenario question.** Two assertions only if they test the same question from different angles (e.g. `EventuallyStalls` + `MinAirspeedNearTarget` both verify the stall event).
9. **Start tolerances wide.** 2-5× the POH spec. Document the narrowing target in a trailing comment (`// narrow toward ±2 m/s once Cl table locks`).
10. **Add to `BEAVER_SCENARIOS`** (or the per-aircraft array when we generalize).
11. **Run the full bench** -- your new scenario shouldn't regress others. If it does, the physics is probably wrong, not the scenario.

## In-Code Documentation Template

Every `FlightScenario` in the source gets a structured comment block above the `FlightScenario { ... }` literal. The `description:` field is a one-line summary; the comment block is the full rationale. This is where the scope, isolation, and non-goals live so a reader three phases from now can re-derive why tolerances were set where they were.

```rust
// beaver_short_field_landing_nominal
//
// Real-world maneuver:
//   Short-field approach and landing. Full flaps (LANDING), 1.3 × Vso approach,
//   idle throttle at threshold, flare to touchdown.
//
// POH target:
//   Ground roll ~530 ft = 162 m after touchdown at MTOW, no wind, paved runway,
//   SL, ISA. (DHC-2 POH § 5 Performance, Short Field Landing chart.)
//
// Isolation (held constant):
//   - Sea level ISA; no density-altitude effect.
//   - Zero wind -- no headwind credit, no crosswind.
//   - MTOW, clean airframe (no damage), full fuel.
//   - Paved runway surface (once contact physics lands; until then, the passive
//     min_altitude clamp stands in).
//   - No pilot modulation after touchdown (brakes only, no differential).
//
// Does NOT test:
//   - Crosswind technique (separate scenario: *_crosswind_15kt).
//   - Soft-field / rough-surface rolling friction (separate: *_soft_field).
//   - Tailwind landing (separate: *_tailwind_10kt).
//   - Float water landing (separate: *_water_landing_calm).
//
// Assertion rationale:
//   GroundRollDistanceFromTouchdown asserts the horizontal distance from
//   first-ground-contact tick to airspeed-below-taxi-threshold tick. Starting
//   tolerance ±40 m (pending contact-physics tuning); target ±10 m.
FlightScenario {
    name: "beaver_short_field_landing_nominal",
    description: "Short-field landing, paved, no wind, MTOW, full flaps. POH ~162 m ground roll.",
    // ... initial, inputs, assertions ...
},
```

Keep the blocks concise -- 10-20 lines, not a design document. The point is that anyone reading the file can answer "why does this exist, what does it prove, and what would I break if I changed the tolerance?" without cross-referencing a wiki.

## Anti-Patterns (Do Not)

1. **Authoring a scenario that passes only at a specific RNG seed.** The bench seeds `SimState` with `0xBEEF` (see `run_scenario_headless`). If your scenario relies on noise in that stream to pass, you're testing the wrong thing.
2. **Widening a tolerance to make a failing test pass.** Tolerances only narrow. If a previously-passing assertion fails, the coefficient change regressed the flight model; revert.
3. **Stacking multiple pilot inputs to "correct" a physics bug.** If cruise needs 0.15 pitch input to hold altitude, the Cm_δe/|Cm_α| stability ratio is probably off, not the scenario.
4. **Running visual playback in CI.** It's an authoring/debug tool, not a gate.
5. **Adding a scenario without a POH citation.** Every scenario's target must trace to a published figure. "Feels right" is not a target.
6. **Committing plot PNGs into `target/`.** That directory is git-ignored. Reference plots get copied to `docs/plots/` when they're *the* reference for a POH figure.

## Quick Commands

```sh
# Headless run + plots (primary loop)
tools/run_flight_bench.sh

# Headless only, no plots
cargo test -p flowplan-sim --test flight_bench --release

# Visual playback of a single scenario
cargo run -- --scenario beaver_cruise_sl --log-dir /tmp/flowplan --exit-on-scenario-end

# Generate plots from existing CSVs
python tools/plot_telemetry.py target/flight-bench/*.csv --outdir target/flight-bench/
```

## References

- [references/behavior-taxonomy.md](references/behavior-taxonomy.md) -- the full space of real-world behaviors scenarios are meant to cover, categorized. Start here when scoping a new scenario.
- [references/trim-seeding.md](references/trim-seeding.md) -- why CRUISE_ATTITUDE is 3° nose-up, and how to pick it for other aircraft.
- [references/assertion-matrix.md](references/assertion-matrix.md) -- per-scenario assertion rationale + starting tolerances for the Beaver.
- [references/symptom-map.md](references/symptom-map.md) -- plot symptoms mapped to likely coefficient culprits.
- [references/poh-targets-beaver.md](references/poh-targets-beaver.md) -- the DHC-2 Beaver POH figures the bench validates against.
- [TEST_PLAN.md § Flight Test Bench](../../TEST_PLAN.md#flight-test-bench) -- architectural intent.
- [crates/sim/src/flight_bench.rs](../../crates/sim/src/flight_bench.rs) -- the runner, `BEAVER_SCENARIOS`, `BenchSample` schema.
- [crates/sim/src/flight.rs](../../crates/sim/src/flight.rs) -- the coefficients you'll tune.
- [tools/plot_telemetry.py](../../tools/plot_telemetry.py) -- CSV → 2×3 panel plot.
- [tools/run_flight_bench.sh](../../tools/run_flight_bench.sh) -- orchestrator.
