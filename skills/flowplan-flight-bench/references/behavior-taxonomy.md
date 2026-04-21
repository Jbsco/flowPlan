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

## 6b. Takeoff / landing execution-quality variations

Cross-cutting section: each takeoff and landing scenario in §5–6 above is the **nominal** case. Real pilots execute maneuvers across a spectrum — from a "buttered" perfect touchdown to a harsh arrival that tests structural limits — and players will want that range. This table enumerates the quality / mishap variants that should be authored once the 2.5b contact physics + Phase 4 damage model are in place.

Why call these out explicitly:
- Each variant exercises a different coupling of the physics — "buttered" validates the low-sink-rate damping regime, "harsh" validates the high-compression regime, "porpoise" validates attitude oscillation under mis-input, "tail-strike" validates contact-point detection at non-gear locations.
- Players who practice bush flying (MVP audience) push aircraft into regimes where mishaps are expected outcomes. A sim that only validates nominal behavior leaves every off-nominal flight silently untested.
- Feel tuning converges faster against a quality spectrum than against a single target: "this touchdown should feel like a greaser" is more actionable than "landing must work."

| Variant | Applies to | Status | Prereqs | What it validates |
|---------|-----------|--------|---------|-------------------|
| **Buttered touchdown** (minimal sink) | Landing (land + water) | **[blocked: 2.5b / 2.5a]** | Contact damping tuning | Touchdown sink < 0.3 m/s; gear compression smooth; no bounce; centerline hold. Success = the aircraft is "on the ground" by tick N+K with no visible impact transient. |
| **Firm / planted** (nominal) | Landing | **[blocked: 2.5b / 2.5a]** | Normal-force + damping | Positive touchdown with controlled sink (~0.5-1 m/s); gear compresses and stays; no bounce. This is the reference variant. |
| **Harsh / hard landing** (high sink) | Landing | **[blocked: 2.5b damage — 2.5b.5 detection hooks]** | Sink-rate event + Phase 4 structural load tracking | Sink > 2 m/s at contact; compression maxes out; raises hard-landing event. Detection ships with 2.5b.5; damage consequences are Phase 4. |
| **Bounced landing** | Landing | **[blocked: 2.5b.2]** | Gear spring-damper; elevator authority post-touchdown | Aircraft touches, lifts back off, re-contacts. Validates the spring's restitution behavior + the pilot's ability to either re-flare or go-around. Secondary contact expected within 2-3 s. |
| **Porpoise on rollout** (ground) | Landing rollout | **[blocked: 2.5b.5]** | Wetted-fraction × pitch coupling (same pattern as water porpoise in 2.5a.4) | Oscillating pitch on rollout when pilot over-corrects; validates that ground-contact pitch coupling behaves analogously to float porpoise. |
| **Tail-strike on rotation** | Takeoff | **[blocked: 2.5b.5]** | Tail contact-point detection | Over-rotation on takeoff: elevator held full-aft, pitch exceeds rotation angle before Vs, tail strikes ground. Event raised; damage Phase 4. |
| **Tail-strike on flare** | Landing | **[blocked: 2.5b.5]** | Tail contact-point detection | Excessive flare attitude at touchdown (> ~12° pitch for taildragger Beaver); tail contacts before gear. |
| **Wingtip strike** | Landing (crosswind / crab-to-kick) | **[blocked: 2.5b.5 + 3.2]** | Wingtip contact-point detection + wind field | Landing with residual bank angle; wingtip contacts surface. Detection ships with 2.5b.5. |
| **Wheel-barrow takeoff** | Takeoff | **[blocked: 2.5b.2 + 2.5b.3]** | Per-gear normal-force distribution | Pilot doesn't hold sufficient back-pressure; weight-on-nose dominant; aircraft rolls on mains without rotation; takeoff distance long / fails to rotate by end-of-runway. |
| **Ground loop** | Takeoff or landing (taildragger) | **[blocked: 2.5b.3]** | Lateral friction + tailwheel steering + mis-input | Asymmetric lateral friction → heading departure → uncontrolled yaw. Distinguishes proper tailwheel technique from simple taxi. |
| **Long flare / floating** | Landing | **[blocked: 2.5b]** | Ground effect + rolling-resistance | Pilot holds aircraft off with excess approach speed; aircraft floats hundreds of feet before touching. Validates that ground-effect lift delays touchdown proportionally. |
| **Short approach / undershoot** | Landing | **[blocked: 2.5b + playable-setting]** | Target-aim-point in scenario schema | Landed short of aim point; validates flare-late failure mode. |
| **Hot approach / runway overrun** | Landing | **[blocked: 2.5b.4 brakes]** | Brake authority + rolling resistance | Landed long with excess energy; validates whether braking decel matches POH. |
| **Go-around from short final** | Landing (abort) | **[blocked: 2.5b scenarios + throttle transient]** | Throttle transient + pitch trim change | Power applied late in approach; aircraft arrests descent and climbs. Validates power-on-pitch coupling + trim transient. |
| **Crosswind landing — crab to kick** | Landing | **[blocked: 3.2 + 2.5b]** | Wind + rudder on final | Crab angle held on final, de-crabbed with rudder just before touchdown. Validates that sideslip → straight touchdown is achievable. |
| **Crosswind landing — wing-low sideslip** | Landing | **[blocked: 3.2 + 2.5b]** | Wind + aileron-into-wind | Aileron-into-wind + opposite rudder; aircraft tracks straight with bank angle. Validates coordination. |
| **Glassy-water landing** (floats) | Landing | **[blocked: 2.5a.7 chop hook + scenario schema]** | Chop-intensity override | Power-on approach to glassy water (no wave reference for depth perception) — per-float contact at minimum sink rate. The single scenario where "buttered" technique is **required**, not optional. |
| **Rough-water landing** (floats) | Landing | **[blocked: 2.5a + 3.2]** | Wave-crest sampling per-float + chop intensity | Land into-swell at low ground speed; wave crest shock validates per-float contact dynamics vs a flat-mean-water assumption. |
| **Aborted takeoff** (RTO analog) | Takeoff | **[blocked: 2.5b.4 brakes + throttle cut]** | Brakes + simulated engine failure | Full throttle → failure event → throttle cut + brakes; aircraft stops before runway end. Validates emergency deceleration. |
| **Engine failure after rotation** | Takeoff (abort path) | **[blocked: Phase 3 engine failures]** | Engine failure event | Power loss just after lift-off; validates gliding response with flaps and nose-down pitch attitude recovery. |
| **Single-pontoon asymmetric landing** | Landing (water) | **[shipped: 2.5a.5]** | 2.5a multi-point buoyancy | Already shipped as `beaver_water_landing_one_pontoon_{right,left}` — included here for reference so the taxonomy stays complete. |

**Authoring discipline for variants:**
- Each variant gets its own `FlightScenario` with the quality level in the name: `beaver_land_landing_buttered`, `beaver_land_landing_harsh`, etc. Avoids one scenario with a quality-level parameter.
- Assertion per variant targets the *distinguishing* physical quantity: buttered landing asserts touchdown sink < threshold, harsh asserts sink > threshold, bounced asserts tick-range where vertical velocity goes positive after first contact.
- Use `variants: BEAVER_WHEELS_ONLY_VARIANTS` or `BEAVER_FLOATS_ONLY_VARIANTS` as appropriate — the quality variants are not cross-variant-meaningful (a buttered landing on floats is a different physical event than a buttered landing on wheels).
- Scenarios that depend on pilot quality (buttered, harsh) should use a `ControlLaw`-driven closed-loop controller that targets the specific sink rate at touchdown, not a scripted-event open-loop trim. The controller's job is to make the quality variant reproducible.
- Target POH / operator-handbook figures where they exist (maximum sink rate for gear design, published touchdown speeds per configuration); where they don't, document the feel target explicitly and cross-reference the playtest that set it.

**Phase mapping** (so the roadmap carries the dependency):
- 2.5b.2 — spring-damper physics enables buttered / firm / bounced variations
- 2.5b.3 — lateral friction + steering enables wheel-barrow, ground-loop, crosswind basics
- 2.5b.4 — brakes enable RTO, overrun scenarios
- 2.5b.5 — strike-detection hooks (tail, wing, prop) enable tail-strike / wingtip-strike variants
- Phase 3.2 — weather enables crosswind + rough-water + gust variants
- Phase 4 — damage model turns detection events into gameplay consequences (hard-landing gear fatigue, prop strike engine damage)
- The scenario authoring itself is a dedicated post-physics pass; a single sub-milestone in Phase 4 or a scenario-family follow-up alongside the damage model.

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

## Real-world effect coverage matrix

Per-effect coverage of the discrete physical phenomena the model implements. Distinct from the scenario taxonomy above (which groups by maneuver). This matrix tracks **whether a given physics effect is exercised by at least one scenario AND visible in plot output** — the "do we know it works and stays working?" check.

Column meanings:
- **Modeled**: implementation exists in `crates/sim/src/{flight,buoyancy}.rs`.
- **Scenario**: at least one bench scenario produces non-trivial readings of this effect.
- **Plot column**: column in the bench CSV / 2×4 panel plot where the effect is visible.
- **Notes**: how to read the effect, current calibration status, known gaps.

### Aerodynamic forces
| Effect | Modeled | Scenario | Plot column | Notes |
|--------|---------|----------|-------------|-------|
| Lift (Cl(α) lookup) | yes | every aerial scenario | `cl`, `vs_mps` | Cl_max=2.0; cruise α≈3°→Cl≈0.4 |
| Induced drag (Cl²) | yes | every aerial | `cd` | Oswald e=0.75 |
| Parasite drag (Cd0) | yes | every aerial | `cd` | Cd0=0.055 |
| Stall (Cl drop past α_stall) | yes | `*_stall_*` | `alpha_deg` past 16°, `stall_state` | Hard stall at 16° |
| Post-stall pitching moment | yes | `*_stall_*` | `pitch_M` | -0.75 slope past stall |
| Wing-drop differential | yes | `*_stall_*` | `roll_L` | Quarter-span Cl sampling |
| Ground effect | yes | none yet (needs low-AGL scenario) | n/a | Phillips factor; tested via unit test |
| Density altitude | yes | aerial scenarios show no climb degradation w/in altitude budget | `engine_kw` over altitude | exp(-h/8500) |

### Aero couplings (cross-axis)
| Effect | Modeled | Scenario | Plot column | Notes |
|--------|---------|----------|-------------|-------|
| α-stability (Cm_α restoring) | yes | every cruise | `pitch_M` near 0 at trim | Cm_α=-0.25 |
| Pitch damping (Cm_q) | yes | implicit in steady cruise | `pitch_M` damping when q changes | Cm_q=-25 |
| Aileron→roll (Cl_δa) | yes | turn scenarios | `roll_L` during entry | Cl_δa=0.075 |
| Roll damping (Cl_p) | yes | turn scenarios stable bank | `roll_L` damping | Cl_p=-0.5 |
| Dihedral (Cl_β) | yes | turn scenarios self-stabilize | `roll_L` from β | Cl_β=-0.05 (2.4.9a) |
| Rudder→yaw (Cn_δr) | yes | every scenario w/ trim | `yaw_N` from yaw input | Cn_δr=0.04 |
| Weathervane (Cn_β) | yes | `vne_dive`, turn entries | `yaw_N` from β | Cn_β=0.08 |
| Yaw damping (Cn_r) | yes | turns | `yaw_N` damping | Cn_r=-0.6 |
| Adverse yaw (Cn_δa) | yes | turn entries | `yaw_N_coupling` | -0.012 |
| P-factor | yes | `cruise_climb` (high α + thrust) | `yaw_N_coupling` | arm=10m, gated below V=5 |
| Spiral slipstream | yes | `cruise_climb`, takeoff scenarios | `yaw_N_coupling` | arm=2m, V_ref=20m/s (2.4.9b) |
| Slipstream q-boost on rudder | yes | takeoff scenarios | rudder authority visible in `yaw_input` vs `yaw_N` | K=0.15 (2.5a follow-up) |
| Torque roll | yes | every full-power scenario | `roll_L_coupling` | 1.5 N·m/kW; throttle ✓ density ✓ feathering ✗ (no prop control yet); airspeed-gated 5-20 m/s as numerical-stability stand-in until 2.5b ground contact |

### Engine + fuel
| Effect | Modeled | Scenario | Plot column | Notes |
|--------|---------|----------|-------------|-------|
| Power curve (linear in throttle) | yes | every scenario | `engine_kw` | 20-335 kW |
| Prop efficiency curve | yes | varies w/ V | `prop_eff` | 0.30 static→0.82 cruise |
| Fuel burn (BSFC) | yes | `endurance_cruise` | `fuel_gph`, `fuel_gal` | 0.124 gph/kW |
| Density-altitude derate | yes | high-altitude scenarios | `engine_kw` falls w/ h | exp(-h/8500) |
| Flameout at zero fuel | yes | unit test | `engine_kw → 0` when `fuel_gal == 0` | Tested in flight.rs |
| V_min thrust clamp | yes | water scenarios at low V | `engine_kw` finite at V=0 | V_MIN_FOR_THRUST=12 |

### Water contact (Phase 2.5a)
| Effect | Modeled | Scenario | Plot column | Notes |
|--------|---------|----------|-------------|-------|
| Per-pontoon buoyancy | yes (2.5a.1) | `water_*` scenarios | `pos_y` settled at ~2.40 m | Multi-point sampling (5/pontoon) |
| Vertical water damping | yes (2.5a.2) | water settle behavior | `vy` at low speeds bounded | Linear, distributed |
| Horizontal water drag | yes (2.5a.2) | `water_taxi` | drag visible in `airspeed` plateauing | Quadratic, distributed |
| Multi-point pitch restoring | yes (2.5a.3) | `water_takeoff` pitch bounded | `pitch_deg` ±0.8° on takeoff | Buoyancy fore/aft asymmetry |
| Planing lift | yes (2.5a.3) | `water_takeoff` rises onto step | `pos_y` rises 2.40→2.55 | Quadratic in V_forward |
| Planing pitch-up moment | yes (2.5a.3c) | bounded pitch on step | `pitch_deg` settles small | -1.5m fwd offset of CoP |
| Wave clock determinism | yes (2.5a.1) | repeat runs byte-identical | n/a (state hash) | `wave_time` folded into state |

### Controls + autopilot
| Effect | Modeled | Scenario | Plot column | Notes |
|--------|---------|----------|-------------|-------|
| Slew-rate keyboard ramp | yes (2.4.4a) | n/a (bench uses scripted SetX) | n/a | 1.5/s, keyboard only |
| Closed-loop bank hold | yes (2.4.9c) | turn scenarios | `roll_deg` tracks target | PID, also Phase 4.7 AP |
| Closed-loop altitude hold | yes (2.4.9c) | turn scenarios | `pos_y` bounded | PID |
| Closed-loop airspeed hold | yes (2.4.9c) | turn scenarios | `airspeed_mps` bounded | PID |
| Closed-loop coordination | yes (2.4.9c) | turns + takeoff | `yaw_deg` drift killed | PID |
| Vertical speed hold | yes (2.4.9c) | climb scenarios available | `vs_mps` | PID |
| Heading hold | yes (2.4.9c) | n/a yet | n/a | PID |

### Known unmodeled / partially-modeled effects
| Effect | Status | Blocker |
|--------|--------|---------|
| Wind / gusts / shear | not modeled | Phase 3.2 weather |
| Crosswind on landing | not modeled | Phase 3.2 weather |
| Ground rolling friction | not modeled | Phase 2.5b ground |
| Brake force | not modeled | Phase 2.5b |
| Tailwheel steering | not modeled | Phase 2.5b |
| Per-aircraft coefficients | hardcoded Beaver only | Phase 5.1 spec migration |
| Asymmetric flap failure | not modeled | post-v1 |
| Spin dynamics (autorotation, PARE) | not modeled | post-2.4 |
| Carb ice | not modeled | post-MVP |
| Pitch-dependent Cl_planing | not modeled | 2.5a.3b deferred (rotation tuning) |
| Multi-point step geometry | simple wetted-fraction approx | 2.5a.3 limit; works for v1 |
| Wake spawn (visual particles) | not modeled | 2.5a polish |
| Spray effects | not modeled | 2.5a polish |

When a new physics effect lands or a scenario starts exercising a new combination, **update this matrix** so coverage stays explicit. A scenario without a corresponding effect entry doesn't earn a coverage claim.

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
