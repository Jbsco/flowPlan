# Model-Gap Triage

The bench's job is to surface gaps between the simulation and the real aircraft. When a scenario reveals one, **the fix is a deliberate process, not a local coefficient nudge**. This file encodes that process so the feedback loop stays honest and changes don't cascade into unseen regressions.

## Core Rule

**A model gap discovered by the bench is a system-level event, not a scenario-level one.** Scenarios are fixed inside the scenario. Coefficients are tuned inside the model — and only after ripple-effect prediction + regression check. New physics lives on the ROADMAP. Never silently tune to hide a gap.

## Triage decision tree

When a scenario shows an anomaly (failing assertion, or passing-but-suspicious behavior on the plot):

```
1. Is the anomaly at scenario boundaries (initial transient / final transient)?
   YES → likely a seeding or duration issue → fix in the scenario.
   NO  → proceed to (2).

2. Is the anomaly in the flight path (attitude, drift, zoom) rather than the
   physical quantity being asserted?
   YES → likely a scripted-input fidelity issue (e.g. constant-pitch vs
         progressive-back-pressure) → refine inputs in the scenario.
   NO  → proceed to (3).

3. Does the symptom-map show a plausible coefficient cause?
   YES → proceed to COEFFICIENT CHANGE protocol (below).
   NO  → proceed to (4).

4. Does the anomaly require a physics effect the model lacks (e.g. slipstream
   over rudder, non-linear Cl table reshape, asymmetric-flap failure)?
   YES → this is NEW PHYSICS. Roadmap item. Do not work around in scenario or
         coefficients; document the gap, defer the fix to the appropriate
         milestone. Widen the scenario tolerance with a comment stating the
         blocker.
   NO  → (5).

5. Is the discrepancy traceable to an upstream doc (AIRCRAFT.md has a
   `(verify)` value, POH citation is ambiguous, AircraftSpec field out of date)?
   YES → fix upstream first, then re-run bench.
   NO  → escalate; anomaly root cause is unclear.
```

## Fix categories

Every bench-identified gap maps to exactly one of these. If you can't pick one cleanly, the diagnosis isn't yet right — keep digging before changing anything.

| Category | What it looks like | Fix path |
|----------|-------------------|----------|
| **Scenario** | Input shape, timing, initial state, or assertion doesn't match the intended behavior | Edit the scenario; re-run; tighten once stable |
| **Coefficient** | Physics structure is right, a numeric constant is wrong | Coefficient change protocol (below) |
| **Missing physics** | The model lacks an effect that materially changes the outcome | Roadmap item; document gap; scenario stays honest (wide tolerance + comment) |
| **Upstream** | The reference value we're asserting against is wrong or unverified | Fix AIRCRAFT.md / RON / POH citation first |

## Coefficient-change protocol

**The only justified reason to change a flight-model coefficient is "a bench-identified gap traces to this coefficient, and the change has been ripple-analyzed against the full scenario suite."**

Steps, in order:

### 1. Name the gap precisely

Not "stall speed is off." Exactly: "Our model stalls at 29 m/s; POH Vs = 23.25 m/s. Analytically, Vs² = 2W/(ρ·S·Cl_max), so POH Vs implies Cl_max = 2.95, while we use 1.65. Gap: Cl_max under-set by a factor of ~1.8."

Write this down. The gap statement *is* the diagnostic artifact.

### 2. Identify every site the coefficient affects

Not "the stall scenario." Every site in the model. For Cl_max:

- `CL_TABLE` in `flight.rs` — peak value.
- Lift calculation: `L = 0.5·ρ·V²·S·Cl` — affects lift at all α where `Cl(α)` reads near peak.
- Induced drag: `Cd_i = Cl²/(π·AR·e)` — affects drag polar at high Cl.
- Stall classification: `α_stall` is a separate constant but interacts.
- Wing-drop calculation: uses `Cl(α)` at quarter-span.
- Ground effect: induced-drag factor modifies `Cd_i`, and flap aero: reads the Cl shape.

### 3. Predict directional impact on every scenario

Make a table. One row per active scenario, one column per impact direction. No empty cells — mark "unchanged" explicitly when you believe nothing will change.

### 4. Choose the smallest change that closes the gap

- Local change (one constant) beats global change (whole table reshape). When the table structure is correct but the peak is off, scale the peak. When the shape is wrong, reshape — but reshape carefully because the entire curve feeds every other scenario.
- Never change two coefficients in one commit. If two seem needed, split: change the first, re-run, narrow tolerances, then the second, re-run.

### 5. Apply the change

### 6. Run the FULL bench suite (not just the affected scenario)

```sh
cargo test -p flowplan-sim --test flight_bench --release
```

### 7. Compare actual vs predicted

For each scenario, read the CSV and the plot. Does the change behave as you predicted in step 3?

- Match → diagnosis is sound. Proceed.
- Mismatch → coupling you didn't predict. **Stop.** Revert. Return to step 2 and re-examine every site the coefficient affects.

### 8. Check for regressions

A regression is a scenario that was passing and is now failing, OR a scenario whose margin shrank unexpectedly.

- If a regression appears, one of:
  - The change is wrong (diagnosis missed an interaction) → revert.
  - The change is right but another coefficient also needs a nudge → propose; do NOT bundle; commit and iterate.
  - The assertion was too-permissive before and is now exposed — OK, update the assertion with a comment explaining.

### 9. Tighten tolerances on scenarios that benefited

Only after regressions are clean. A tolerance narrows when it has sustained margin after a model change, never to manufacture a pass.

### 10. Commit

One commit per coefficient change. Subject: `Phase 2.4.8: tighten Cl_max to 2.0 (narrows stall Vs toward POH)`. Body: cite the gap statement, the affected scenarios, predicted vs actual impacts.

## Ripple-effect worked example

Gap: `stall_clean` scenario stalls at 29 m/s; POH Vs = 23.25 m/s. Analytic cause: `Cl_max = 1.65` implies Vs = 31 m/s by pure `Vs² = 2W/(ρSCl_max)`.

**Candidate fix**: raise `Cl_max` from 1.65 → 2.0 (narrows gap, does not close it).

**Sites affected:**

1. `CL_TABLE` in `flight.rs` — peak entry
2. Lift at high α (direct)
3. Induced drag at high α (Cl² term)
4. Stall trigger via α-based state transitions (α threshold is separate, but Cl peaks later)
5. Wing-drop roll moment (reads `Cl(α)` at quarter-span)
6. Flap aero composition (`Cl_max` + flap `ΔCl`)
7. Ground-effect factor (Cl-dependent induced drag)

**Predicted impact table:**

| Scenario | Direction | Why |
|----------|-----------|-----|
| cruise_sl | unchanged | cruise α ~3°, well below Cl_max region |
| cruise_climb | unchanged | cruise α ~3° same as above |
| stall_clean | min-V drops ~3 m/s | higher Cl_max → lower stall V |
| (future) Vne_dive | unchanged | never hits high α |
| (future) turn_60deg | margin improves slightly | accelerated stall at 2g |
| (future) landing_flaps | Vso drops ~2 m/s | flap aero uses base Cl_max |

**Regression risks:**

- If the `CL_TABLE` shape is re-scaled uniformly, cruise Cl may shift upward even at α=3°, producing trim drift in cruise scenario. Check cruise_sl trim inputs remain effective post-change.
- If only the peak entry scales but neighboring points stay, the curve shape sharpens near peak — affects post-stall behavior (Cl drop past stall) differently than a smooth reshape. Compare stall-recovery trace pre/post.

**Decision rule:** if post-change, any scenario that was passing now has <50% of its previous margin, investigate before commit. Revert if the cause isn't a predicted ripple.

## Missing-physics worked example

Gap: `cruise_climb` uses 5.5% right rudder trim where a real Beaver pilot uses 20-40%. Scaling our trim up to 30% would "fix" the apparent magnitude but changes nothing physical in the model.

Triage says: this is either a coefficient gap (P_FACTOR_ARM_M too small) OR a missing-physics gap (slipstream over rudder: prop slipstream increases local air velocity over the rudder at high power, multiplying rudder effectiveness — a 20% slipstream boost explains most of the 4-7× pilot-input difference).

**Correct response:**

1. Attempt the coefficient fix first (simpler, bounded). Raise `P_FACTOR_ARM_M` toward the value that would demand 30% rudder for counter. Ripple-analyze: does this over-correct at cruise? Likely yes.
2. If coefficient fix over-corrects at cruise, **it is a missing-physics gap** (the real effect scales with thrust, not linearly with α — slipstream is the variable we don't model).
3. Document in PROGRESS as "missing-physics gap: slipstream over rudder; scenario trim values under-represent real pilot input until this lands; not fixing with coefficient inflation."
4. Widen scenario tolerance if needed. Never inflate trim values to "look right."

The bench's value is specifically that it cannot be fooled by inflated inputs — tolerances narrow over time, and any inflation shows up as a regression later.

## What not to do

1. **Don't change a coefficient without writing the ripple-impact table first.** Every time this is skipped, a regression shows up two weeks later.
2. **Don't widen a tolerance to make a failing scenario pass.** Tolerance narrows with model tuning; widening reflects either new understanding (document why) or an authoring mistake (revert).
3. **Don't inflate scripted inputs beyond what the model needs.** Real-pilot-experience values are a *target* for model tuning, not a scenario knob. The fidelity block in the scenario documents the gap; the coefficient eventually closes it.
4. **Don't diagnose from a single scenario.** A gap visible in one scenario should be cross-checked against at least one other scenario that exercises the same physics. If the diagnosis only fits scenario A and doesn't predict behavior in scenario B, the diagnosis is probably wrong.
5. **Don't bundle coefficient changes.** One per commit. Chained changes hide which one caused a regression.

## Related

- [symptom-map.md](symptom-map.md) — current coefficient values + plot-symptom diagnosis.
- [assertion-matrix.md](assertion-matrix.md) — assertion types + tolerance-narrowing discipline.
- [PROGRESS.md § Bench-identified gaps](../../../PROGRESS.md) — standing log of open gaps surfaced by the bench.
- [ROADMAP.md § Milestone 2.4.8](../../../ROADMAP.md#milestone-24-flight-model-beaver-tuned-reference) — phase-level status of the tuning pass.
