# Data Flow — Full Path, Worked Example

This is the SimState → SimSnapshot → Telemetry → consumer path, expanded, with a worked example of adding a single new field end-to-end.

## The five layers

### 1. SimState (live, mutable, sim-owned)

Lives in `crates/sim/src/state.rs`. Owns `VehicleState`, `Camera`, `time_of_day`, aircraft spec, last `FlightOutputs`. Mutable only inside `SimState::tick(inputs, difficulty, dt, min_altitude)`. No one outside `sim` holds a `&mut SimState`.

State updates per tick:
- `vehicle.controls` integrates input events (set / delta / ramp).
- `compute_forces(state, spec, dt, altitude_agl_m) -> FlightOutputs` returns per-tick forces, moments, α, β, Cl, Cd, engine state, stall state.
- `tick_dynamics` integrates linear and angular accelerations; updates position, velocity, orientation, angular velocity.
- `last_flight = outputs` caches the most recent FlightOutputs for snapshot publication.
- `vehicle.fuel_gal` decrements by `fuel_flow_gph × dt / 3600`.
- `time_of_day` advances by `dt / DAY_LENGTH_SECONDS`, wrapped into `[0, 1)`.

### 2. SimSnapshot (POD, cross-thread-safe, read-only)

Lives in `crates/sim/src/snapshot.rs`. Built at tick end via `SimSnapshot::from_state(&s)`. Triple-buffered so render reads while sim writes.

Contents (exact layout as of 2.4.8):

- `tick: u64`
- `vehicle: VehicleSnapshot` — `position`, `orientation`, `velocity`, `angular_velocity`, `controls: ControlsSnapshot`, `fuel_gal`, `pad*` fields for `#[repr(C)]` alignment.
- `camera: CameraSnapshot` — `position`, `orientation`, `fov_y_radians`, `view_kind: u32`.
- `time_of_day: f32`
- `flight: FlightOutputs` — the whole struct, including force vectors, α, β, Cl, Cd, per-axis `MomentBreakdown`, engine kW, prop efficiency, fuel flow gph, stall state.

Pod derives on `VehicleSnapshot` / `ControlsSnapshot` / `CameraSnapshot`. `SimSnapshot` itself is `Copy, Debug` but not Pod (the `FlightOutputs` field contains nested structs that aren't all Pod yet).

### 3. Telemetry (derived per-frame, render-owned, read-only)

Lives in `crates/render/src/telemetry.rs`. Built once per render frame via `Telemetry::derive(&snapshot)`. Pure function. No hidden state.

Contents (grouped for legibility):

- **Pose:** `position`, `orientation`
- **Linear motion:** `velocity`, `airspeed_total`, `airspeed_horizontal_fwd`, `airspeed_vertical`
- **Attitude:** `heading_rad`, `pitch_rad`, `bank_rad` — derived from quat `to_euler(YXZ)`
- **Aerodynamics:** `alpha_rad`, `beta_rad`, `cl`, `cd`
- **Forces:** `thrust`, `lift`, `drag`, `gravity`, `linear_accel` (sum)
- **Angular:** `actual_angular_vel`, `angular_accel`, `pitch_moment` / `roll_moment` / `yaw_moment` (each a `MomentBreakdown { control, stability, damping, coupling }`)
- **Altitude / VSI:** `altitude_msl`, `vertical_speed_mps`, `flight_path_angle_rad`, `glide_ratio`
- **Controls:** `throttle`, `pitch_input`, `yaw_input`, `roll_input`
- **Engine + fuel:** `engine_power_kw`, `prop_efficiency`, `fuel_flow_gph`, `fuel_gal`, `endurance_seconds`
- **State:** `stall_state`, `flap_position`
- **Atmosphere:** `air_density_kg_m3`, `density_ratio`
- **Bookkeeping:** `tick`

### 4. Consumers (read Telemetry, never SimState)

Currently implemented:

- **`debug_hud.rs`** — three egui panels. Top-left `debug` (tick, view, pos/vel/attitude, controls, flaps); top-right `dynamics` (airspeeds, α/β/Cl/Cd, stall alert, forces, moments with coupling column, engine); bottom-left `glide` (FPA, α, β, VS, L/D).
- **`debug_gizmo.rs`** — two widget builders. Bottom-right orientation gizmo (body axes + velocity + forces as unit vectors); bottom-left attitude indicator (horizon, pitch ladder, FPV crosshair).
- **`renderer.rs` widget path** — the `TriangleList` pipeline + widget-local ortho projection; renders the two widgets in per-slot VBOs (see the 2.4.2c + 2.4.4e descriptions in PROGRESS.md — per-slot VBOs were added to stop queue-write clobbering between widget slots).

Planned:

- Phase 4 hand-rolled HUD (airspeed tape, altitude tape, artificial horizon, heading, VSI, turn coordinator, AoA indicator, engine cluster, fuel cluster, flap/gear/trim state).
- Phase 4+ 3D cockpit instruments (needles driven by the same fields).
- Phase 4+ autopilot readout (target HDG bug, ALT capture, NAV deviation).
- Phase 5+ flight log / replay visualization.

### 5. Bench parallel (BenchSample)

Lives in `crates/sim/src/flight_bench.rs`. Capture method `BenchSample::capture(tick, sim, flight)` reads the same `SimState + FlightOutputs` the snapshot would see, but assembles a CSV-friendly subset. Written to `target/flight-bench/<scenario>.csv` by the integration test.

Why it's separate from `Telemetry`: headless tests don't pull the render crate, so Telemetry isn't reachable. `BenchSample` is telemetry-shaped data *in sim* for offline use.

## Worked example: adding `load_factor` to Telemetry + HUD

Scenario: a Phase 2.4.4d+ TODO in FEATURES.md calls for G-load display. Walkthrough:

### Step 1 — Decide the layer

Load factor `n = |lift| / (mass · g)` is derivable from `FlightOutputs.lift` and `AircraftSpec.max_mass_wheels_kg`. Pure derivation. Not state. Belongs in `Telemetry`, not `VehicleState` and not `FlightOutputs`.

### Step 2 — Add the field to `Telemetry`

```rust
// crates/render/src/telemetry.rs
pub struct Telemetry {
    // ... existing fields ...
    /// Load factor (g-load). `|lift| / (mass · g)`. 1.0 in level unaccelerated flight,
    /// >1 in a pull-up or banked turn, 0 in freefall.
    pub load_factor: f32,
}
```

### Step 3 — Populate in `derive`

```rust
// In Telemetry::derive, add after computing forces:
let mass_kg = /* from spec, or snapshot if mass is added there */;
let g = 9.81;
load_factor: f.lift.length() / (mass_kg * g),
```

**Problem:** `SimSnapshot` does not currently carry the aircraft's mass. Options:

- (a) Add `mass_kg` to `VehicleSnapshot` (new scalar, adjust Pod padding).
- (b) Read the spec directly at derive time (requires passing the spec into `derive`).
- (c) Add `load_factor` to `FlightOutputs` instead — `compute_forces` has the spec in scope.

Choice (c) is usually best: load factor is already a byproduct of the force computation, and the right layer is "derived-per-tick aero output". This is a case where the layer-choice question in SKILL.md pushes toward `FlightOutputs`.

### Step 4 — Add to `FlightOutputs` + `compute_forces`

```rust
// crates/sim/src/flight.rs
pub struct FlightOutputs {
    // ...
    pub load_factor: f32,
}

// inside compute_forces, after lift is computed:
let load_factor = lift.length() / (mass_kg * 9.81);
// return with load_factor,
```

Now it flows `FlightOutputs.load_factor` → `SimSnapshot.flight.load_factor` → `Telemetry::derive` populates `telemetry.load_factor = snapshot.flight.load_factor`.

### Step 5 — Consume in the HUD

```rust
// crates/app/src/debug_hud.rs — add to dynamics panel
ui.label(format!("G-load: {:.2}", telemetry.load_factor));
```

### Step 6 — Extend `BenchSample` for plotting (if wanted)

If a future `accelerated_stall` scenario wants to plot load factor during a pull-up, add the column:

```rust
// crates/sim/src/flight_bench.rs
pub struct BenchSample {
    // ...
    pub load_factor: f32,
}
// in capture:
load_factor: flight.load_factor,
// in csv_header:
",load_factor"
// in write_csv_row:
",{:.3}"
```

### Step 7 — Test

```sh
# Sim-side tests (FlightOutputs + snapshot changes)
cargo test -p flowplan-sim

# Telemetry derivation test
cargo test -p flowplan-render telemetry

# Bench regeneration (if BenchSample was updated)
cargo test -p flowplan-sim --test flight_bench --release

# Sanity-check CSV. Path is <aircraft>/<variant>/<scenario>.csv.
head -1 target/flight-bench/northlander/floats/beaver_cruise_sl.csv
```

Every added field touches 2-5 files, in a predictable pattern. The path is boilerplate-heavy on purpose — the alternative (each consumer computing its own load factor) leads to display values that disagree.

## Cross-reference

- [flowplan-flight-bench/references/symptom-map.md](../../flowplan-flight-bench/references/symptom-map.md) — what plots reveal when the derivation is correct.
- [bench-sample-extension.md](bench-sample-extension.md) — the CSV-column-extension details.
