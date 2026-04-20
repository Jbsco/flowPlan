---
name: flowplan-telemetry
description: Extending or consuming flowPlan's telemetry pipeline. Use when adding a new readout to a display surface (HUD panel, widget, cockpit gauge, bench CSV column), plumbing a new derived quantity through SimSnapshot + Telemetry, or debugging why a HUD value disagrees with reality. Covers the sim-state → snapshot → derived-Telemetry → consumer flow, the `BenchSample` counterpart for offline CSV, and the Pod/padding gotchas when extending snapshot structs.
---

# flowPlan Telemetry

Every value on every display surface in flowPlan comes from a single derivation. This skill covers **how the derivation flows**, **where a new field belongs**, and **the Pod/padding trap that will bite you** if you extend the snapshot structs carelessly.

Authority: [DESIGN.md § Telemetry derivation (render side)](../../DESIGN.md#telemetry-derivation-render-side); [FEATURES.md § Telemetry & Instrumentation Data Flow](../../FEATURES.md#telemetry--instrumentation-data-flow); source of truth is [`crates/render/src/telemetry.rs`](../../crates/render/src/telemetry.rs).

## Core Rules

1. **One `Telemetry` struct, many consumers.** Every display surface — debug HUD, attitude indicator, orientation gizmo, future cockpit gauges, autopilot readouts — reads from the *same* `Telemetry` struct. No consumer reaches into `SimState`. Adding a second derivation of the same quantity is the anti-pattern this layer exists to prevent.
2. **Pure function, no hidden state.** `Telemetry::derive(&SimSnapshot) -> Telemetry`. No accumulators, no RC, no globals. If a reading needs history (peak-hold, rolling average), the history lives in the *consumer*, not in telemetry.
3. **Render-rate, not sim-rate.** Telemetry is computed once per render frame, in the `render` crate (or `app`). Sim doesn't know it exists. Computing it sim-side would bloat the snapshot with derived values.
4. **SI internally, conversions at the format edge.** `airspeed_total` is m/s. The HUD that shows knots multiplies by `1.94384` when formatting; it does not store knots.
5. **Determinism-safe.** Presentation smoothing (needle lag, averaging) is fine in telemetry or its consumers; it *never* belongs in sim. The sim's bit-identical invariant doesn't care about smoothed display values.
6. **Snapshot fields are POD.** `SimSnapshot`, `VehicleSnapshot`, `ControlsSnapshot`, `CameraSnapshot` use `#[repr(C)]` + `bytemuck::Pod + Zeroable`. Extending them requires padding discipline (see below).
7. **`BenchSample` is telemetry's CSV twin.** When a new field lands in `Telemetry`, decide whether it belongs in `BenchSample` too (if a plot or POH assertion needs it). Failing to update `BenchSample` is a silent miss; failing to update its CSV header is a corruption bug.

## The Flow

```
SimState (live, sim-owned, mutable)
    |
    | at tick end: SimSnapshot::from_state(&s)
    v
SimSnapshot (POD, cross-thread-safe, read-only)
    |  - vehicle: VehicleSnapshot (pose, velocity, controls, fuel, ang_vel)
    |  - camera: CameraSnapshot (pose, fov, view_kind)
    |  - flight: FlightOutputs (forces, moments, α, β, cl, cd, stall_state, engine)
    |  - tick, time_of_day
    v
Telemetry (derived per-frame, render-owned, read-only)
    |  - pose (from vehicle)
    |  - attitude (yaw/pitch/bank rad, from quat decompose)
    |  - aerodynamics (α, β, Cl, Cd, forces, moments)
    |  - derived kinematics (FPA, glide ratio, vertical speed)
    |  - engine + fuel (endurance, density ratio)
    |  - flap position, stall state, tick
    v
Consumers (all read Telemetry):
    - debug_hud.rs       (top-left, top-right, bottom-left panels)
    - debug_gizmo.rs     (bottom-right orientation widget, bottom-left AI)
    - (future) hand-rolled HUD, cockpit gauges, autopilot readouts
```

Parallel path for the bench:

```
SimState (live, mid-tick or post-tick)
    |
    | BenchSample::capture(tick, &sim, &flight)
    v
BenchSample (per-tick POD, 31 fields)
    |
    | log_to_csv(&log)
    v
CSV row string -> target/flight-bench/<scenario>.csv
    |
    | tools/plot_telemetry.py reads
    v
PNG plots, human review
```

`BenchSample` is a subset of the information Telemetry carries, shaped for offline plotting and POH assertions. It lives in `sim` (not `render`) so headless tests don't pull the render crate.

## Where Does a New Field Belong?

Decide with these questions in order:

1. **Is it a first-class sim quantity or a derived one?**
   - First-class (state that the tick itself updates): goes in `VehicleState` (or `FlightOutputs` if it's a per-tick aero output). Then snapshot it into `VehicleSnapshot` or stay in `FlightOutputs` (which is already in `SimSnapshot.flight`).
   - Derived (computable from snapshot fields): goes in `Telemetry` only.

2. **Do multiple display surfaces need it?**
   - Yes: add it to `Telemetry` so they share one derivation.
   - Just one: still add it to `Telemetry` — the one-struct-many-consumers invariant is cheap to preserve and expensive to walk back from.

3. **Does a bench plot or POH assertion need it?**
   - Yes: add to `BenchSample` too. Update the CSV header string and `write_csv_row` formatting. Run `cargo test -p flowplan-sim --test flight_bench --release` to regenerate CSVs and verify headers align.
   - No: skip `BenchSample`; telemetry-only suffices.

4. **Is it unit-loaded (m/s, rad, kg/m³, N·m)?**
   - Units live in the field *name* (`airspeed_mps`, `alpha_rad`, `thrust_nm`). Not in a separate tag. Consumers that convert for display do it at the format call.

## The Pod / Padding Trap

`VehicleSnapshot` and `ControlsSnapshot` are `#[repr(C)]` with `bytemuck::Pod + Zeroable` derives. These derives statically enforce that the struct has no uninitialized bytes — any padding must be explicit fields.

This means:

- Adding a scalar to a snapshot usually requires adding or adjusting `padN: [f32; K]` fields around it so alignment is honored.
- Discriminant types (enums) must be cast to `u32` and unpacked at the consumer. See `ControlsSnapshot::flap_position: u32` + `flap_position_to_u32` / `flap_position_from_u32`.
- `bool` fields are awkward — prefer `u32` with documented `0` / `1` values.

**What breaks if you get it wrong:** the derive macro either fails to compile (best case) or the struct silently contains undefined padding bytes that fail the GPU uniform upload at a later stage (worst case). Work through the layout on paper before editing.

Worked example — how `ControlsSnapshot` packs:

```rust
#[repr(C)]
pub struct ControlsSnapshot {
    pub throttle: f32,      // 4 bytes, offset 0
    pub pitch: f32,         // 4 bytes, offset 4
    pub yaw: f32,           // 4 bytes, offset 8
    pub roll: f32,          // 4 bytes, offset 12
    pub flap_position: u32, // 4 bytes, offset 16
    pad0: [f32; 3],         // 12 bytes, offset 20  (padding to 32 bytes)
}
```

The pad brings the struct to 32 bytes, a 16-byte-aligned size suitable for GPU upload. Adding a new `u32 gear_position` would require reducing pad0 to `[f32; 2]`. Adding a `Vec3` would require a `pad_after_vec3: f32` because `Vec3` is naturally 12-byte — the remainder-to-16 padding rule applies.

Best practice: whenever you edit these structs, compile-test the bytemuck derives *and* run the snapshot tests (`cargo test -p flowplan-sim snapshot`). The Pod derive gives a loud error on misalignment; trust it.

## Extending `BenchSample`

When a new telemetry field should also land in CSV:

1. Add a field to the `BenchSample` struct in [`crates/sim/src/flight_bench.rs`](../../crates/sim/src/flight_bench.rs). Keep primitive types (`f32`, `u32`) — CSV is text-formatted.
2. Populate it in `BenchSample::capture(tick, sim, flight)`.
3. Add the column name to `BenchSample::csv_header()` — in the exact order it will be written.
4. Add the field to the `writeln!` format string in `write_csv_row`, matching the header order.
5. If a plot panel needs it, add the panel logic to `tools/plot_telemetry.py`. Pandas reads the CSV; matplotlib renders.

The header and row-write are paired. Mismatch produces columns that don't align — a subtle but corrupting bug. Do both in the same edit.

Run the bench after any change:

```sh
cargo test -p flowplan-sim --test flight_bench --release
```

and eyeball one of the output CSVs (`target/flight-bench/beaver_cruise_sl.csv`) to verify the new column appears and is populated.

## Anti-Patterns

1. **Second derivation of an existing quantity.** If `glide_ratio` already exists in `Telemetry`, the autopilot readout does *not* re-compute `horiz / |vert|`. It reads `telemetry.glide_ratio`.
2. **Storing in `SimState` what is derivable from `SimState`.** α, β, Cl, Cd, pitch/roll/yaw moments — all these *are* outputs of `compute_forces`, so they live in `FlightOutputs`, not in `VehicleState`. State is what the tick updates each step; derivations are what the tick *produces*.
3. **Non-SI storage in the telemetry struct.** If you see `airspeed_knots: f32` in a PR diff, it's wrong. Store SI, convert at display.
4. **Mutable telemetry.** `Telemetry` is derived fresh per frame. No accumulator fields. No `&mut Telemetry` anywhere in the codebase.
5. **Consumer reaching into `SimState` or `SimSnapshot.vehicle` directly.** The telemetry struct exists to prevent this. If a consumer needs data not in `Telemetry`, add it to `Telemetry` — don't bypass.
6. **Render mutating sim state from a telemetry-adjacent path.** The whole derivation is read-only. If a consumer feels it needs to "reset" a value, the consumer owns its own state.
7. **Adding a `BenchSample` column without updating both header and row-write.** Silent column misalignment. Always the same edit.
8. **Smoothing added sim-side to make telemetry "look better".** Smoothing is presentation. If the underlying sim value is jittery, either the sim is wrong or the jitter is real — either way, smoothing lives in the consumer.

## Quick Reference

| Need | Put it in | Why |
|------|-----------|-----|
| New tick-updated vehicle field (position-adjacent) | `VehicleState` → `VehicleSnapshot` | Ground truth lives in state |
| New force / moment / aero output | `FlightOutputs` (already in snapshot) | Outputs of `compute_forces` |
| Derived quantity multiple consumers need | `Telemetry` | One-struct-many-consumers |
| CSV column for offline analysis | `BenchSample` + header + row-write | Bench replay artifact |
| Scalar with GPU upload downstream | `#[repr(C)]` + Pod, mind the padding | Layout stability |

## Quick Commands

```sh
# Snapshot tests (fastest round-trip for Pod struct edits)
cargo test -p flowplan-sim snapshot

# Telemetry derivation tests (after adding a Telemetry field)
cargo test -p flowplan-render telemetry

# Full bench run (after extending BenchSample)
cargo test -p flowplan-sim --test flight_bench --release

# Sanity-check CSV header + row count match
head -1 target/flight-bench/beaver_cruise_sl.csv
wc -l target/flight-bench/beaver_cruise_sl.csv
```

## References

- [references/data-flow-diagram.md](references/data-flow-diagram.md) — the full sim→snapshot→telemetry→consumer diagram with a worked "add a field" example end-to-end.
- [references/bench-sample-extension.md](references/bench-sample-extension.md) — the exact edit list for extending `BenchSample` + CSV + plot.
- [DESIGN.md § Telemetry derivation (render side)](../../DESIGN.md#telemetry-derivation-render-side) — architectural invariants.
- [FEATURES.md § Telemetry & Instrumentation Data Flow](../../FEATURES.md#telemetry--instrumentation-data-flow) — consumer progression by phase, field list by phase.
- [`crates/render/src/telemetry.rs`](../../crates/render/src/telemetry.rs) — the `Telemetry` struct + `derive`.
- [`crates/sim/src/snapshot.rs`](../../crates/sim/src/snapshot.rs) — `SimSnapshot` / `VehicleSnapshot` / `ControlsSnapshot` Pod structs.
- [`crates/sim/src/flight.rs`](../../crates/sim/src/flight.rs) — `FlightOutputs` struct, the aero-output carrier.
- [`crates/sim/src/flight_bench.rs`](../../crates/sim/src/flight_bench.rs) — `BenchSample` + CSV header / row-write.
- [flowplan-flight-bench/SKILL.md](../flowplan-flight-bench/SKILL.md) — how the bench consumes the telemetry flow.
