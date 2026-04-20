# Scenario Behavior Taxonomy

The space of real-world behaviors a flight-test-bench scenario is meant to isolate. Scenarios are organized by what they *prove*, not by what they *contain*. A behavior that doesn't map to one of these categories either belongs in a new category (propose an addition) or doesn't belong in the bench.

Every category is gated on which flight-model features have landed. Authoring a scenario for a category whose prerequisites aren't in the sim produces noise, not signal -- the scenario either can't be expressed or asserts against a physics effect the model doesn't simulate.

## Capability vs envelope-limit scenarios

A single real-world behavior often deserves TWO scenarios:

- **Capability test** — aircraft can do X. Assertions: altitude hold, airspeed match, `NeverStalls`.
- **Envelope-limit test** — aircraft correctly fails when pushed past X. Assertions: `EventuallyStalls`, expected descent, no altitude assertion (descent is the point).

Both are valid and BOTH should exist for significant envelope corners. The paired scenario `beaver_coordinated_turn_45` (capability: steep turn sustained) + `beaver_coordinated_turn_60_overbank` (limit: accelerated stall when over-banked at cruise) demonstrates this pattern. The limit test is especially valuable as a **regression guard**: if a future coefficient change silently extends the capability envelope, the limit test stops failing as expected — catching the over-tune.

Limit scenarios require care with assertions: do NOT assert what the capability test asserts. `coordinated_turn_60_overbank` asserts `EventuallyStalls` and deliberately omits `AltitudeStableAtEnd` / `AirspeedAtEnd` because altitude loss and airspeed bleed are the EXPECTED outcome. Asserting altitude-hold there would conflict with the scenario's purpose.

When adding a capability scenario that tests a limit corner, ask: "is there a natural paired envelope-limit case?" If yes, add both.

Status legend:

- **[shipped]** — scenario(s) of this type are in `BEAVER_SCENARIOS` now.
- **[ready]** — all prerequisites in the sim; scenario can be authored today.
- **[blocked: X]** — waiting on flight-model feature X.

## 1. Cruise / steady-state flight

Behaviors that validate steady-state force balance, not transients.

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| Level cruise at SL, published power | **[shipped]** `beaver_cruise_sl` | — | Baseline altitude-hold. |
| Level cruise at altitude (~3 km) | **[ready]** | 2.4.7 density altitude (shipped) | Validates ρ-derate on both aero and engine; different mean pitch than SL. |
| Best-range cruise speed | **[ready]** | 2.4.3 engine + 2.4.7 ρ | L/D_max × prop efficiency; asserts fuel flow + airspeed band. |
| Cruise at Vne with throttle reduction | **[ready]** | — | Verify aircraft doesn't overspeed in level flight at 70-80% power. |
| High-α level flight (slow flight) | **[ready]** | — | Just above Vs, full power, asserts α sits in approach band without stalling. |

## 2. Climb

Behaviors that validate excess-thrust → vertical-speed.

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| Vy full-throttle climb, SL | **[shipped]** `beaver_vy_climb` | — | POH ~1000 fpm at MTOW. |
| Vx full-throttle climb | **[ready]** | — | Steepest angle, not steepest rate. Lower ROC than Vy; higher γ. |
| Vy climb to service ceiling | **[ready]** | 2.4.7 ρ | Long scenario (~5 min sim time); asserts ROC → 0.5 m/s at POH ceiling. |
| Climb at reduced power (normal climb) | **[ready]** | — | 75% power at Vy+10; routine climb profile. |

## 3. Descent / glide

Behaviors that validate drag polar and engine-idle thrust.

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| L/D_max glide, idle throttle | **[ready]** | — | Hold 70 mph best-glide; assert sink rate + glide ratio ≈ 10. |
| Dive to near-Vne (throttle managed) | **[ready]** | — | Nose-down pitch, throttle reduced, assert airspeed lands in [65, 72] m/s. |
| Power-off 180° to touchdown | **[blocked: 2.5 contact]** | Touchdown needs friction + normal force | Classic engine-out exercise. |

## 4. Stalls

Behaviors that validate α-based stall physics and stall-speed predictions.

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| Clean decel-to-stall | **[shipped]** `beaver_stall_clean` | — | Idle, hold back-stick, min airspeed near Vs. |
| Landing-flap stall (Vso) | **[ready]** | 2.4.5 flaps (shipped) | Same but flaps LANDING; asserts min near Vso = 21.01 m/s. |
| Accelerated stall in banked turn | **[ready]** | 2.4.4d α-based stall (shipped) | Tight turn at cruise; airspeed-only heuristic would miss this -- that's the point. |
| Stall recovery within time band | **[ready]** | 2.4.4d stall-pitching-moment (shipped) | Release back-stick; assert stall exit within N seconds. |
| Power-on stall | **[ready]** | — | Full throttle + back-stick; stall at higher α because thrust adds vertical component. |
| Departure stall (climb, high α) | **[ready]** | — | Takeoff configuration; validates flap-deployed stall character. |

## 5. Takeoff

Behaviors that validate ground-roll dynamics + rotation + initial climb.

**All blocked on Milestone 2.5 ground contact physics.** Until then, the passive altitude clamp doesn't produce a realistic ground roll (no rolling resistance, no tailwheel steering, no brake, no prop-wash over elevator, no weight-on-wheels).

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| Nominal takeoff, paved, no wind, MTOW | **[blocked: 2.5b wheels]** | Rolling resistance + normal force | POH ~650 ft ground roll. |
| Takeoff, soft field | **[blocked: 2.5b]** | Per-surface friction | Higher rolling resistance; longer roll. |
| Takeoff, crosswind 10 kt | **[blocked: 3.2 weather + 2.5b]** | Wind + yaw moment from gear side-load | Slip-into-wind technique; assert minimal drift. |
| Takeoff, headwind 15 kt | **[blocked: 3.2 weather + 2.5b]** | Wind field in sim | Shorter roll; validates wind credit. |
| Takeoff, tailwind 5 kt | **[blocked: 3.2 weather + 2.5b]** | — | Longer roll; catch if model incorrectly gives tailwind credit. |
| Takeoff, high density altitude | **[blocked: 2.5b]** | 2.4.7 ρ (shipped) | Substantially longer roll at 3 km ISA; validates ρ-derate effect. |
| Takeoff, off-nominal (aborted above V1 analog) | **[blocked: 2.5b + throttle-cut]** | — | Bonus; late phase. |
| Water takeoff, calm | **[blocked: 2.5a water]** | Float hydrodynamics | Plow → on-step → fly. Porpoise risk modeled. |
| Water takeoff, chop | **[blocked: 2.5a + 3.2]** | Water chop state | Harder to achieve step; longer roll. |

## 6. Landing

Symmetric to takeoff; same 2.5 prerequisite.

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| Nominal landing, paved, no wind, MTOW | **[blocked: 2.5b]** | Rolling resistance + touchdown detection | POH ~530 ft ground roll. |
| Short-field landing, full flaps | **[blocked: 2.5b]** | — | Approach at 1.3×Vso; assertive flare. |
| Soft-field landing | **[blocked: 2.5b]** | Per-surface friction | Extra-slow approach, hold nose up in rollout. |
| Crosswind landing | **[blocked: 3.2 + 2.5b]** | Wind + directional control | Crab-into-wind or wing-low; assert centerline hold. |
| Downwind landing | **[blocked: 3.2 + 2.5b]** | — | Longer roll; validates wind-direction sign. |
| Water landing, calm (glassy) | **[blocked: 2.5a]** | Float buoyancy | Power-on approach; validate nose-up attitude at touchdown. |
| Water landing, chop | **[blocked: 2.5a + 3.2]** | Water chop state | Spray obstructs forward view; validate airframe doesn't take chop impact as stall spike. |

## 7. Turns / maneuvering

Behaviors that validate coordinated flight and load-factor response.

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| Coordinated 30° banked turn at cruise | **[ready]** | — | Assert airspeed hold + altitude hold + load factor ≈ 1.15. |
| Coordinated 60° banked turn at cruise | **[ready]** | — | Load factor 2.0; stall margin halves; may need power addition. |
| Steep turn to specified heading change | **[ready]** | — | Assert final heading within tolerance of target. |
| Uncoordinated skidding turn (stall setup) | **[ready]** | — | Demonstrates β-induced early stall; precursor to spin scenarios. |

## 8. Emergency / failure states

Behaviors that validate system-failure aerodynamic response.

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| Engine-out glide | **[ready]** | — | Idle throttle == engine-out for current model (no damage yet). |
| Partial flap asymmetry | **[blocked: post-v1]** | Asymmetric flap failure is post-v1 | Out of 2.4 scope. |
| Control-surface damage | **[blocked: Phase 4 damage]** | Damage → lift/drag/authority modifiers | Per-zone damage state needed. |

## 9. Weather interactions

Behaviors that validate wind, turbulence, density altitude response.

**Most blocked on Phase 3.2 weather state.**

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| High density altitude cruise | **[ready]** | 2.4.7 ρ (shipped) | No wind needed; scenario starts at 3+ km. |
| Cruise in steady headwind | **[blocked: 3.2]** | Wind field | Ground speed vs airspeed divergence. |
| Cruise in crosswind, crab angle hold | **[blocked: 3.2]** | — | Validate ground track vs heading. |
| Wind shear encounter (descent through a layer) | **[blocked: 3.2]** | Wind-shear events | Transient airspeed change; validate response. |
| Turbulence response | **[blocked: 3.2]** | Gust variance | Statistical assertion: attitude RMS within band. |
| Density-altitude takeoff | see Takeoff §5 | | |

## 10. Contact physics (ground / water)

Behaviors that validate 2.5 contact physics in isolation, not during a full takeoff/landing.

**All blocked on Milestone 2.5.**

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| Stationary on level ground remains stationary | **[blocked: 2.5c]** | Normal force + friction | No drift, no chatter, 10 s. |
| Taxi straight at idle thrust → steady speed | **[blocked: 2.5b]** | Rolling resistance | Not runaway; reaches equilibrium. |
| Float at rest settles to equilibrium draft | **[blocked: 2.5a]** | Buoyancy | Matches vehicle displacement. |
| Ground impact at shallow angle | **[blocked: 2.5b]** | Per-part normal force | Validates contact distribution, not crash. |
| Water impact, glassy | **[blocked: 2.5a]** | Float + water interface | Hard touchdown produces expected spray + deceleration. |
| Prop strike detection | **[blocked: 2.5b]** | Per-point contact | Event raised; damage consequences are Phase 4. |
| Wingtip strike detection | **[blocked: 2.5b]** | — | Event raised; damage consequences Phase 4. |
| Wave-crest clip while floating | **[blocked: 2.5a]** | Wave-height sampling per-float | Avoids the "flat mean water" simplification. |

## 11. Endurance / long-duration

Behaviors that validate fuel burn and engine model over time.

| Behavior | Status | Prereqs | Notes |
|----------|--------|---------|-------|
| Endurance at cruise, fuel to flameout | **[ready]** | 2.4.3 fuel (shipped) | Long scenario; assert endurance matches cruise_fuel_gph × capacity. |
| Range at cruise | **[ready]** | — | Distance integration over endurance window. |
| Fuel-exhaustion flameout transition | **[ready]** | — | Power drop at fuel=0; asserts clean engine cut, no spike. |

## 12. Determinism meta-scenarios

Not about the flight model -- about the bench itself.

| Behavior | Status | Notes |
|----------|--------|-------|
| Same scenario, two runs, identical logs | — | Asserted implicitly by integration test seed. |
| Dial change mid-scenario produces deterministic fork | **[ready]** | Requires `DialChange` scripted input; validates dial-state is part of determinism hash. |

## How to propose a new category

If a real-world behavior doesn't fit any row above:

1. Check whether it's actually a missing *physics effect* rather than a missing scenario. If so, it's a flight-model roadmap item, not a bench item.
2. If it's genuinely a new category of test, propose an addition to this taxonomy in the same PR that adds the scenario. The taxonomy is load-bearing -- ungrouped scenarios accumulate.
3. Keep categories broad. We don't want "takeoff in fog" as a separate category from "takeoff in rain"; those are both weather-interaction variants of takeoff.

## Related

- [assertion-matrix.md](assertion-matrix.md) -- maps assertion types to scenario intents with starting tolerances.
- [poh-targets-beaver.md](poh-targets-beaver.md) -- the Beaver POH figures each scenario anchors against.
- [ROADMAP.md Milestone 2.5](../../../ROADMAP.md#milestone-25-ground--water-contact-physics) -- the blocker for most takeoff / landing / contact scenarios.
- [ROADMAP.md Phase 3.2](../../../ROADMAP.md#milestone-32-weather-state) -- the blocker for wind / turbulence / shear scenarios.
