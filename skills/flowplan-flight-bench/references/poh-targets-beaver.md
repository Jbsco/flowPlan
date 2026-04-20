# POH Targets — DHC-2 Beaver (Northlander)

The assertion targets in `BEAVER_SCENARIOS` come from the de Havilland Canada DHC-2 Beaver POH. This file is the reference card: figures in SI, conversion notes, and citation discipline for each target. When we add a scenario, it asserts against a number on this page (not against "what feels right").

## Primary source

**de Havilland Canada DHC-2 Beaver Aircraft Operating Manual** — piston variant, Pratt & Whitney R-985 Wasp Junior, 450 hp. Commonly available as publicly shared PDFs; specific figures cross-referenced with Viking Air's post-production support documentation and NACA TN-1945 for the 64A-series airfoil.

Figures in `data/aircraft/northlander.ron` are the authoritative SI conversions; this file documents *why* those numbers look the way they do and where the conversions come from.

## Speeds (from POH, converted to SI)

| V-speed | POH | SI (ron file) | Notes |
|---------|-----|---------------|-------|
| Vne | 160 mph | 71.53 m/s | Never-exceed; structural |
| Cruise | 125 kt | 64.31 m/s | Best recognized figure; flown at 65-70% power |
| Vs (clean) | 52 mph | 23.25 m/s | Stall, flaps UP |
| Vso | 47 mph | 21.01 m/s | Stall, flaps full + floats config |
| Vx | 65 mph | 29.06 m/s | Best-angle climb |
| Vy | 85 mph | 37.99 m/s | Best-rate climb |

Conversion constants: 1 mph = 0.44704 m/s, 1 kt = 0.51444 m/s, 1 ft/min = 0.00508 m/s, 1 USgal = 3.785 L, 1 lb = 0.4536 kg, 1 hp = 0.7457 kW, 1 sq ft = 0.0929 m².

## Performance figures (used by the bench)

### Cruise (`beaver_cruise_sl`)

- POH: 125 kt at ~65% MCP, SL, MTOW. Cruise is 64.31 m/s target.
- Scenario uses throttle 0.65 (matches "65% power") and airspeed 55 m/s initial (slower than POH cruise — the assertion is altitude-hold, not speed-match, so 55 m/s with a light back-stick is stable without pushing Vne margins).
- Target: altitude drift ≤ 100 m over 30 s window. POH has no "steady cruise altitude drift" figure because real cruise uses trim; our assertion is a bench stand-in.

### Climb (`beaver_vy_climb`)

- POH: best rate of climb ≈ 1000 ft/min at SL, MTOW, Vy = 85 mph. That's 5.08 m/s.
- Scenario uses throttle 1.0 (full power), airspeed 38 m/s initial (~Vy), pitch input 0.10.
- Target: mean climb rate 5.0 ± 5.0 m/s in last 5 s. Tolerance very wide until coefficient tuning.
- Narrowing roadmap: after tuning, target ±1 m/s.

### Stall clean (`beaver_stall_clean`)

- POH: Vs clean = 52 mph = 23.25 m/s at MTOW.
- Scenario: decel from cruise to stall, idle throttle, 0.30 pitch input to keep α high.
- Target: minimum airspeed in log within ±8 m/s of 23.25. Very wide initial tolerance.
- Narrowing roadmap: after α-stall model locks, target ±2 m/s.

## Future scenario targets (pending implementation)

### Vne dive (`beaver_vne_dive`)

- POH: Vne = 160 mph = 71.53 m/s. Do not exceed.
- Scenario: cruise, push over into a shallow dive at full throttle, verify terminal airspeed lands between 65-72 m/s (below Vne).
- Target: `FinalAirspeedWithin { 65, 72 }`. If aircraft overshoots Vne, drag polar is too weak.

### Takeoff distance

- POH: ~530 ft ground roll at MTOW on wheels, SL, no wind.
- Scenario: spawn on ground, full throttle, track horizontal distance until airborne (vehicle.y climbs above an AGL threshold).
- Target: assertion not yet in `Assertion` enum; likely `GroundRollDistanceAtRotation { max_m }`. Add alongside 2.5 contact physics when rolling-resistance lands.

### Range at cruise

- POH: 455 nm at 65% power, SL, still air. 455 nm = 842.66 km.
- Scenario: cruise until fuel exhaustion, integrate distance.
- Target: distance traveled ≥ 800 km (wide tolerance until density-altitude + fuel-flow tuning settles).

### Service ceiling

- POH: 18,000 ft absolute, typically 16,000 ft service. 16,000 ft = 4877 m.
- Scenario: full-throttle climb at Vy; assert climb rate drops to 0.5 m/s (~100 fpm, the service-ceiling definition) at ~4800 m altitude.
- Target: `ClimbRateAtEnd { last_secs: 10, target_mps: 0.5, tol_mps: 0.5 }` with scenario starting at 4000 m MSL and climbing for several minutes.

### Turn radius at 45° bank, cruise

- POH: standard rate turn + bank-turn formulas give ~900 ft radius at 60 kt / 45°.
- Scenario: enter a coordinated 45° bank at cruise, hold for 360°, measure max lateral excursion.
- Target: requires a new assertion variant (track position extent). Defer until the basics are locked.

### Stall with landing flaps (`beaver_landing_flaps`)

- POH: Vso = 47 mph = 21.01 m/s.
- Scenario: same as `beaver_stall_clean` but start with `flaps: FlapPosition::Landing`.
- Target: `MinAirspeedNearTarget { 21.01, tol_mps: 3 }`.

### L/D_max glide

- POH: best glide speed ≈ 70 mph = 31.3 m/s, approx 10:1 L/D ratio.
- Scenario: idle throttle from cruise, hold pitch to maintain 31 m/s, measure sink rate.
- Target: mean sink rate 3.1 m/s ± 1.5 m/s. Also verify `glide_ratio` (horizontal / |vertical|) ≈ 10.

## Citation discipline

When adding a scenario:

1. **Cite the POH figure in the scenario's `description`.** Not "matches Beaver" — actually quote the number: "POH Vs=52 mph (23.25 m/s); verify min airspeed in decel-to-stall."
2. **Convert on authoring, not at runtime.** All scenario targets are SI literals. No multiplication by 0.44704 inside the scenario.
3. **Mark `(verify)` on anything taken from AIRCRAFT.md where uncertainty is higher.** Some Beaver figures in AIRCRAFT.md carry `(verify)` tags — if we base a scenario on one, the tolerance starts wider.
4. **Never source a target from another flight sim.** X-Plane / MSFS Beaver numbers are other people's tuning, not the real aircraft.

## Related references

- [AIRCRAFT.md § MVP Reference](../../../AIRCRAFT.md#mvp-reference) — the catalog entry with POH citations
- [`data/aircraft/northlander.ron`](../../../data/aircraft/northlander.ron) — the authoritative SI conversions
- [FEATURES.md § MVP Reference Aircraft](../../../FEATURES.md#mvp-reference-aircraft-dhc-2-beaver-analogue) — flight-model target parameters
