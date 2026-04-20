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

8. **A model gap discovered by the bench is a system-level event, not a scenario-level one.** When an assertion fails or a plot shows suspect behavior, *triage before touching anything*. The fix category (scenario / coefficient / missing physics / upstream) is not interchangeable. Coefficient changes require ripple-effect prediction against the full scenario suite and a regression check before commit. Never tune silently to hide a gap — the bench's value is that it catches inflated inputs and papered-over coefficients later. See [references/model-gap-triage.md](references/model-gap-triage.md) for the full protocol and current open gaps.

9. **Scenarios come in two kinds: capability tests and envelope-limit tests. Don't force-fit between them.** A *capability* test says "aircraft can do X" and asserts altitude/airspeed/NeverStalls. An *envelope-limit* test says "aircraft correctly fails when pushed past X" and asserts `EventuallyStalls` / expected descent. **Both are valid; they belong to different scenarios.** Players will push the model past its realistic envelope in normal play — the bench must verify that out-of-envelope behavior stays realistic (stalls, descents, speed bleed) and doesn't degrade into nonsense when the model is fine-tuned for within-envelope performance.

   **Force-fitting anti-patterns (never do these):**
   - **Widening a tolerance** to make a capability scenario "pass" when the aircraft actually behaves as its envelope limit predicts. If the test says "hold altitude" and the aircraft is at 2g trying to sustain a 60° bank, the failure is SIGNAL — split into two scenarios (capability at 45°, limit at 60°) rather than widening tol.
   - **Dodging failure by softening conditions** (reducing bank angle, lowering load factor, shortening duration) without preserving the original as a paired envelope-limit scenario. The original scenario's failure was load-bearing information.
   - **Deleting a scenario** because "it fails." Check first: is it a bug, or is the aircraft correctly refusing to exceed physics? If the latter: keep the scenario with envelope-limit assertions.
   - **Asserting capability on a limit scenario** (e.g., `AltitudeStableAtEnd` on `*_overbank`). Descent is the expected outcome there; asserting no-descent is self-contradictory.

   **When a scenario fails, decision tree:**
   1. Is the aircraft behaving *correctly* per real-world physics (stalling, diving, losing speed at high AOA)? → the SCENARIO is mis-classified. It's an envelope-limit test, not a capability test. Fix the assertions.
   2. Is the aircraft behaving *unrealistically* (not stalling when it should, sustaining impossible flight)? → COEFFICIENT regression or scenario input error. Triage per `model-gap-triage.md`.
   3. Did a nearby capability scenario also start failing? → likely a real regression; revert and investigate.

   Pairs where both kinds are valuable:
   - `coordinated_turn_45` (capability) + `coordinated_turn_60_overbank` (limit).
   - `cruise_sl` (capability) + future `cruise_over_vne` (limit, if sim ever models Vne behavior).
   - `stall_clean` (capability: correctly stalls at Vs) + `stall_recovery` (capability: recovers cleanly) vs. `stall_unrecovered_deep` (limit: held back-pressure, spin-like dive) in the future.

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
4b. **Audit real-world fidelity.** For every scripted input, answer: does a real pilot do this? If yes (direction + magnitude): faithful. If the shape matches but we're using a stand-in (scripted constant for trim, staged pitch for smooth back-stick): approximated, name it. If the magnitude is off from real-world pilot reports: known fidelity gap, trace to a specific model coefficient. Never inflate values to "look right"; the fidelity section records the mismatch until the model catches up.
5. **Pick initial conditions.** Altitude 500-1000 m MSL unless density altitude is the point. Airspeed at the test point. `CRUISE_ATTITUDE` unless testing a non-level attitude (takeoff rotation, steep approach, unusual-attitude recovery).
6. **Pick the pilot-hold input.** Start with 0.05 for cruise, 0.10 for Vy, 0.30 for stall induction. Adjust only if the first run overshoots/undershoots. Document the value's purpose ("hold α high as speed decays").
7. **Set `duration_ticks` conservatively.** 30 s (3600) for steady-state; 20 s (2400) for stall induction; 60 s (7200) for endurance trends; 10-15 s for takeoff/landing roll isolation.
8. **Write one assertion per scenario question.** Two assertions only if they test the same question from different angles (e.g. `EventuallyStalls` + `MinAirspeedNearTarget` both verify the stall event).
9. **Start tolerances wide.** 2-5× the POH spec. Document the narrowing target in a trailing comment (`// narrow toward ±2 m/s once Cl table locks`).
10. **Add to `BEAVER_SCENARIOS`** (or the per-aircraft array when we generalize).
11. **Run the full bench** -- your new scenario shouldn't regress others. If it does, the physics is probably wrong, not the scenario.

## In-Code Documentation Template

Every `FlightScenario` in the source gets a structured comment block above the `FlightScenario { ... }` literal. The `description:` field is a one-line summary; the comment block is the full rationale. This is where the scope, isolation, non-goals, **and real-world fidelity** live so a reader three phases from now can re-derive why tolerances were set where they were.

The block has six required sections, in order:

```rust
// <scenario_name>
//
// Real-world maneuver:
//   <One-paragraph description of the real-world flight procedure or maneuver
//    this scenario represents. What a pilot would do, and under what conditions.>
//
// POH target:
//   <The specific published figure the assertion anchors against, with unit
//    conversions and POH section reference. If no single number applies, name the
//    qualitative behavior being validated.>
//
// Isolation (held constant):
//   - <List every variable deliberately held fixed so the assertion points at
//      one thing. Altitude, wind, weight, config, trim choices, throttle profile.>
//
// Real-world fidelity:
//   <THIS SECTION IS LOAD-BEARING.> Document which of the scripted inputs match
//   real pilot technique directly, which are trim-wheel stand-ins, which are
//   approximations, and — for any approximation — WHY and WHAT consequence.
//   Use three sub-labels so reviewers can scan:
//     Faithful: <what in the scenario is a direct match to real-world>.
//     Approximated: <what we've simplified, e.g. constant inputs standing in
//       for trim, or staged inputs standing in for continuous pilot control>.
//     Magnitudes: <analytical justification for numeric values, OR a KNOWN
//       FIDELITY GAP note explaining where values don't match real-world and why
//       that's traceable to a model limitation (not a scenario fudge)>.
//   When a magnitude looks off from real-world pilot reports, NAME the suspected
//   model coefficient gap and link to symptom-map.md / coefficient-tuning items.
//   Never inflate a value "to look right" -- scenarios reflect what the model
//   needs; model gaps stay visible in the fidelity note until coefficients tune.
//
// Does NOT test:
//   - <Explicit non-goals. What this scenario appears to touch but deliberately
//      does not validate. Prevents tolerance creep and scope drift.>
//
// Assertion rationale:
//   <Why these specific assertions, in this order, with these tolerances. Include
//    the tolerance-narrowing trajectory (starting value → target after which
//    tune lands).>
FlightScenario {
    name: "<scenario_name>",
    description: "<one-line summary used in plot titles>",
    // ... initial, inputs, assertions ...
},
```

Why the fidelity section matters: without it, a future reader sees `SetPitch(0.20)` and can't tell whether that's "what a pilot would do" or "what our model needs to make the assertion pass." The fidelity block forces us to name the difference every time, keeping the feedback loop honest. When we fudge (because no trim system exists yet), we say so. When a model coefficient produces a value that doesn't match real pilot experience, we say so and point at the coefficient. Never silently.

Keep the blocks concise -- 20-30 lines is typical, not a design document. The point is that anyone reading the file can answer "why does this exist, what does it prove, what differs from real pilot technique, and what would I break if I changed the tolerance?" without cross-referencing a wiki.

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
- [references/model-gap-triage.md](references/model-gap-triage.md) -- the protocol for diagnosing bench anomalies, coefficient-change ripple analysis, regression checks. Read when a scenario fails or a plot shows suspect behavior.
- [TEST_PLAN.md § Flight Test Bench](../../TEST_PLAN.md#flight-test-bench) -- architectural intent.
- [crates/sim/src/flight_bench.rs](../../crates/sim/src/flight_bench.rs) -- the runner, `BEAVER_SCENARIOS`, `BenchSample` schema.
- [crates/sim/src/flight.rs](../../crates/sim/src/flight.rs) -- the coefficients you'll tune.
- [tools/plot_telemetry.py](../../tools/plot_telemetry.py) -- CSV → 2×4 panel plot (altitude, airspeed+VS, attitude, α/β; ground-track, moments, controls, engine+fuel).
- [tools/run_flight_bench.sh](../../tools/run_flight_bench.sh) -- orchestrator.
