# Assertion Matrix

Per-assertion semantics, intended use, and starting tolerances for the Beaver. The matrix expands per-aircraft as the catalog grows -- keep the same rows, adjust targets.

## AltitudeStableAtEnd

**Checks:** mean altitude over `last_secs` vs. initial altitude.

**Use for:** steady cruise, hold-altitude-after-transient tests.

**Do not use for:** climb scenarios (use `ClimbRateAtEnd`), stall scenarios (altitude won't be stable).

**Starting tolerance:** 100 m on a 30 s run. Tighten to 20 m once Cm_α + thrust curves lock. Below 20 m and you're testing sensor noise, not the flight model.

**Beaver cruise SL target:** 500 m initial, `tol_m: 100`, `last_secs: 5`.

## ClimbRateAtEnd

**Checks:** mean vertical_speed over `last_secs`.

**Use for:** Vy climb, Vx climb, terrain-clearance scenarios.

**Gotcha:** the window must be after the aircraft has settled into a steady climb -- if `last_secs` extends back into the initial transient, the mean will be biased toward the starting sink. 30 s run with `last_secs: 5` is safe.

**Beaver Vy target:** `target_mps: 5.0, tol_mps: 5.0` (wide). POH spec is ~5 m/s (1000 fpm) ROC at MTOW; narrow toward ±1 m/s as coefficients settle.

## AirspeedAtEnd

**Checks:** mean airspeed over `last_secs`.

**Use for:** cruise speed, Vne-at-full-throttle-dive.

**Do not use for:** any scenario where airspeed is deliberately decelerating.

**Beaver cruise target:** ~55-64 m/s depending on throttle. Loose tolerance (±5 m/s) until prop-efficiency curve is tuned.

## NeverStalls

**Checks:** no tick in the log reports `StallState::Stall`.

**Use for:** every benign scenario as a background sanity check. Free -- log is already captured.

**Gotcha:** `Approach` state is allowed. Only a hard `Stall` fails.

## EventuallyStalls

**Checks:** at least one tick reports `StallState::Stall`.

**Use for:** stall-induction scenarios. Companion to `MinAirspeedNearTarget`.

**Gotcha:** if this fails, the scenario either didn't drive α high enough (pilot-hold too small) or the stall angle is too high. Check the plot's α trace.

## FinalAirspeedWithin { min, max }

**Checks:** last sample's airspeed is in the band.

**Use for:** Vne tests (must not exceed Vne even at full-throttle dive), range-at-cruise endpoints.

**Prefer `AirspeedAtEnd` when possible** -- single-sample assertions are brittle. Use band-based only when the question is "did it stay out of an envelope corner", not "where did it settle".

## MinAirspeedNearTarget

**Checks:** minimum airspeed across the whole log vs target.

**Use for:** Vs validation. Scenario drives the plane into a stall; this asserts the speed at which it stalled.

**Beaver Vs target:** 23.25 m/s (52 mph POH Vs). Starting tolerance `tol_mps: 8` is very wide; narrow to ±2 once stall-α and Cl-table are tuned.

## Tolerance-narrowing discipline

Rule: a tolerance only narrows in a commit that *also* changes a flight coefficient. If you want to narrow just because "we got lucky", don't -- the margin is protection against tuning regressions you haven't found yet.

When two PRs both try to narrow the same tolerance, the second one must re-run the bench on the first one's tip, not on `main`, to confirm the margin is still there.

## Per-scenario assertion sets (Beaver, 2.4.8 framework snapshot — shipped only)

| Scenario | Category | Assertions |
|----------|----------|------------|
| `beaver_cruise_sl` | Cruise §1 | `AltitudeStableAtEnd { 5, 100 }`, `NeverStalls` |
| `beaver_vy_climb` | Climb §2 | `ClimbRateAtEnd { 5, 5.0, 5.0 }` |
| `beaver_stall_clean` | Stalls §4 | `EventuallyStalls`, `MinAirspeedNearTarget { 23.25, 8.0 }` |

For the full space of intended scenarios — including which are ready-to-author today and which are blocked on 2.5 contact physics / 3.2 weather / Phase 4 damage — see [behavior-taxonomy.md](behavior-taxonomy.md). The taxonomy is the authoritative list of what the bench *should* cover; this matrix tracks what it *does* cover.

## Isolation discipline (one assertion = one question)

When authoring a new scenario, the assertion set must answer exactly what the scenario is testing. Common mistakes:

- **Testing cruise stability but asserting on stall state.** `NeverStalls` on a cruise scenario is a cheap sanity check (free, since the log is captured anyway) but does not validate altitude stability. Both are still needed; neither is redundant.
- **Asserting climb rate in a scenario whose inputs change mid-window.** If the scripted input changes during `last_secs`, the assertion is measuring a transient, not a steady climb. Either extend duration so the transient decays before the window, or use a different assertion.
- **Single assertion doing double duty.** `MinAirspeedNearTarget` catches the stall speed *if* the scenario actually stalls. Pair with `EventuallyStalls` so a scenario that decelerates to the stall band but never crosses it also fails.

## Tolerance trajectory discipline

Every tolerance should be accompanied by a trailing comment stating the narrowing target. Examples:

```rust
Assertion::AltitudeStableAtEnd {
    last_secs: 5.0,
    tol_m: 100.0, // start-wide; narrow toward 20 m after 2.4.8 Cm_α tune
},
Assertion::MinAirspeedNearTarget {
    target_mps: 23.25,
    tol_mps: 8.0, // start-wide; narrow to ±2 once α-based stall + Cl table lock
},
```

This makes the tolerance narrowing PR diffable: reviewer sees the old wide value + comment, the new narrow value, and can judge whether the coefficient change justifies the narrowing without reading the plot.
