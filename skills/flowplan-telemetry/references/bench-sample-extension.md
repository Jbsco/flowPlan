# Extending `BenchSample` (CSV Column + Plot)

The flight-test bench writes a CSV row per tick (120 Hz). Adding a column is mechanical but touches four places that must stay synchronized. Miss one and you get silently-misaligned columns — which corrupts every plot downstream.

## The four synchronized edits

All four live in [`crates/sim/src/flight_bench.rs`](../../../crates/sim/src/flight_bench.rs) except the plot, which is in `tools/plot_telemetry.py`.

### 1. Struct field

```rust
#[derive(Clone, Copy, Debug, Default)]
pub struct BenchSample {
    // ... existing fields ...
    pub my_new_field_unit: f32,  // add near kindred fields, document units in the name
}
```

Note: `BenchSample: Default` — the derive requires every field implement `Default`. Primitive types are fine. If you need a custom default (non-zero), implement `Default` manually.

### 2. `capture` population

```rust
impl BenchSample {
    pub fn capture(tick: u64, sim: &SimState, flight: &FlightOutputs) -> Self {
        Self {
            // ... existing fields ...
            my_new_field_unit: /* compute from sim or flight */,
        }
    }
}
```

The value must be derivable from `&SimState + &FlightOutputs` alone. If it's not, the precondition has moved — the bench runs headless and captures the state available at tick boundary.

### 3. CSV header

```rust
pub fn csv_header() -> &'static str {
    "tick,time_s,pos_x,pos_y,pos_z,...,\
     my_new_field_unit"   // add to the exact same position as the row-write
}
```

Header order must match row-write order exactly.

### 4. CSV row-write

```rust
pub fn write_csv_row(&self, out: &mut String) {
    let _ = writeln!(
        out,
        "{},{:.5},{:.3},...,\
         {:.3}",                // precision format for your new field
        self.tick,
        self.time_s,
        self.position_x,
        // ...
        self.my_new_field_unit, // in the same positional slot as the header
    );
}
```

Precision discipline: match neighbors. Speeds at `{:.3}` (mm precision), angles at `{:.3}` (millidegrees), coefficients at `{:.4}`, power at `{:.2}`, fuel at `{:.2}`, integers plain `{}`.

### 5. Plot script (optional)

`tools/plot_telemetry.py` has eight panels in a 2×4 grid (altitude, airspeed+VS, attitude, α/β; ground-track, moments, controls, engine+fuel). If the new field belongs in one of those panels, add the pandas column read + matplotlib plot call. If it deserves its own panel, extend the grid (or rework — the script is small, don't over-engineer).

## Verification

Always verify alignment after editing:

```sh
# Regenerate CSVs
cargo test -p flowplan-sim --test flight_bench --release

# Eyeball one of the outputs. Path is <aircraft>/<variant>/<scenario>.csv.
head -2 target/flight-bench/northlander/floats/beaver_cruise_sl.csv

# Verify column count matches field count
head -1 target/flight-bench/northlander/floats/beaver_cruise_sl.csv | tr ',' '\n' | wc -l
```

Header column count and row column count must match. A mismatch shows up as:

```
tick,time_s,...,my_new_field
0,0.00000,...,1.234,5.678   <- extra value — row has more columns than header
```

Pandas will either drop the extra or mis-assign, depending on mood. Both are bugs.

## What not to do

- **Don't refactor the CSV format.** Keep fields appended at the end. Downstream plot scripts and any committed reference CSVs in `docs/plots/` track positional columns. A rearrangement breaks them.
- **Don't change precision of an existing column.** Plot scripts assume stable formatting. If a column needs more precision, add a second column (rare; flag during review).
- **Don't add fields only for "it might be useful".** Every CSV column is compute at 120 Hz × scenario-duration — cheap but not free. Earn each column with a scenario or plot panel that needs it.

## Reference: the current 31-column schema

```
tick, time_s,
pos_x, pos_y, pos_z,
pitch_deg, roll_deg, yaw_deg,
vel_x, vel_y, vel_z, airspeed_mps, vs_mps, fpa_deg,
alpha_deg, beta_deg, cl, cd,
throttle, pitch_in, roll_in, yaw_in, flap_deg,
engine_kw, prop_eff, fuel_gph, fuel_gal,
pitch_M, roll_L, yaw_N, stall_state
```

Maintenance: if this list and the `csv_header()` string drift, the source is `flight_bench.rs`, not this file. Grep the source for authority.
