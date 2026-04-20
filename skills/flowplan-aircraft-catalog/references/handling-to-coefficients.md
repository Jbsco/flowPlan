# "Characteristic handling" → Coefficient Intent

The `Characteristic handling:` bullets in [AIRCRAFT.md](../../../AIRCRAFT.md) are design hints, not direct coefficient assignments. This file translates the hint vocabulary into coefficient-setting intent *for when the per-aircraft coefficient migration lands* (planned as a 2.4.x follow-up). Until then, these notes guide scenario authoring (wide tolerances, expected miss directions) rather than per-aircraft tuning.

## The current state

`AircraftSpec` does not carry moment coefficients, drag polar, Cl(α) table, prop efficiency, or flap deltas. All of those live in `crates/sim/src/flight.rs` as hardcoded Beaver values.

This means: *every* aircraft today shares Beaver aero. A Super Cub at Beaver aero will feel sluggish relative to its POH (because the Super Cub is light); a Husky will feel similarly. A Maule will under-STOL. A Carbon Cub will massively under-STOL.

Scenarios for non-Beaver aircraft will consequently mis-predict. Authors should write the scenarios anyway — they become the acceptance suite for the per-aircraft migration when it lands.

## Handling vocabulary → coefficient intent

Common `Characteristic handling` phrases and what they imply about coefficients once migration lands:

### "Docile" / "forgiving" / "trustworthy"

- Static stability: moderate (Cm_α around -0.2 to -0.3), enough to return to trim without being oppressive.
- Damping: generous (Cm_q ≤ -20, Cn_r ≤ -0.5). Rates decay before the pilot has to chase them.
- Stall entry: gentle (no sharp Cl drop, wing-drop asymmetry low).
- The Beaver is the canonical docile aircraft — current coefficients reflect this.

### "STOL capable" / "extreme STOL"

- Cl_max high (2.0 + for flaps extended, or 1.8 clean with Helio-style slats).
- Stall α extends higher (slats delay separation).
- Requires a new flight-model *effect* if the aircraft uses auto-deploying slats (Just Aircraft SuperSTOL, Zenith 701) — that is a roadmap amendment, not a catalog entry.

### "Demanding tailwheel technique"

- Small directional stability (Cn_β low-ish).
- Low-speed rudder authority sharp (Cn_δr not dominated by damping at low q).
- Real-world, most of this "demand" is ground-physics, not flight. Waits for 2.5 contact physics.

### "Light" / "crisp"

- Low moments of inertia (I_xx, I_yy, I_zz scaled down from the Beaver's `8000 / 12000 / 5000 kg·m²`). A Super Cub is ~1/3 the Beaver's mass; inertias scale similarly.
- Higher control authority per unit of q.

### "Short-field capable"

- Low mass, high Cl_max, moderate thrust-to-weight.
- Not a direct coefficient change — emergent from the combination.

### "Strong low-speed authority"

- Tailplane / rudder sizing in reality. In the model: elevated Cm_δe / Cn_δr relative to baseline damping. Do not raise these without also raising damping, or the aircraft becomes PIO-prone.

### "Pusher-prop pitch coupling"

- A *new* effect. P-factor sign is opposite (pusher vs tractor). Torque-roll may also flip. Requires flight-model amendment, not catalog data.

### "V-tail response (coupled pitch/yaw)"

- A *new* effect. Requires a flight-model amendment to model the V-tail mixing.

### "Twin-engine" / "asymmetric thrust"

- A *new* effect (two engines with independent throttles, asymmetric-thrust yaw). Not in scope for 2.4; post-2.5 at earliest.

### "Radial rumble" / "turbine whine"

- Audio-only. Not a flight-model thing. Lives in the Glicol patch per-aircraft.

## What's in scope right now

For an aircraft added *before* the per-aircraft coefficient migration lands, the only flight-feel levers are:

- Mass (`max_mass_wheels_kg` / `max_mass_floats_kg`) — directly scales F/m integration.
- Wing area (`wing_area_m2`) — directly scales q·S in lift and drag.
- Aspect ratio — enters induced drag calculation (Cd_i = Cl² / (π·AR·e)).
- Power (`engine.power_hp`) — enters thrust via `T = η·P/V`.
- Fuel burn and capacity — enters endurance.

All other handling is Beaver-flavored. A well-authored spec in this state produces:

- Correct cruise altitude achievement (lift scales with area).
- Roughly correct climb rate (thrust and mass are both right).
- Roughly correct stall speed (lift balance).
- **Wrong**: elevator sensitivity, yaw damping, roll authority, flap deltas, stall-entry character.

Scenarios for such an aircraft should:

- Assert speeds (cruise, stall) with POH tolerances.
- Assert climb rate with wider tolerances (±30%).
- **Not** assert handling-qualities (turn radius, roll rate, stall behavior specifics) — those will not match until coefficients migrate.

## What the migration will add

When per-aircraft coefficients migrate into `AircraftSpec` (sketched as `FlightCoefficients { ... }` or similar), the schema grows to carry:

- Moments of inertia (three f32s).
- Moment coefficients (9+ f32s: Cm_α, Cm_δe, Cm_q; Cl_δa, Cl_p; Cn_β, Cn_δr, Cn_r; plus coupling terms).
- Drag polar (Cd0, Oswald e).
- Cl(α) table or parameterized curve.
- Prop efficiency curve.
- Flap position deltas.

At that point, the handling notes above become direct inputs. Until that amendment lands, they are design intent only.

## Cross-reference

- The full set of current Beaver coefficients lives in [`crates/sim/src/flight.rs`](../../../crates/sim/src/flight.rs).
- The 2.4.8 reference table is in [flowplan-flight-bench/references/symptom-map.md § Current Beaver coefficients](../../flowplan-flight-bench/references/symptom-map.md#current-beaver-coefficients-248-framework-snapshot).
