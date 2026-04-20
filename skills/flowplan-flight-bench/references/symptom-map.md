# Plot-Symptom → Coefficient Map

A plot tells you the *what*; this map tells you *where to look first*. All coefficients live in [`crates/sim/src/flight.rs`](../../../crates/sim/src/flight.rs). Current Beaver values are at the bottom.

## How to read the 2×4 panel plot

`tools/plot_telemetry.py` produces an 8-panel PNG:

| Panel | Trace | What it tells you |
|-------|-------|-------------------|
| (0,0) altitude | pos_y vs time, dotted line at initial altitude | Long-term trend: climb / cruise / sink / phugoid |
| (0,1) airspeed + VS | airspeed (left y) + vs (right y) | Energy state + climb/descent rate |
| (0,2) attitude | pitch / roll / yaw / γ (FPA) | Full attitude story on one chart |
| (0,3) α / β + thresholds | α, β, 16° stall, 12.8° approach | Where we are in the lift curve |
| (1,0) ground-track | pos_x vs -pos_z, equal aspect, start/end markers | Ground-truth course drift (invisible in time-series panels) |
| (1,1) moments | pitch_M, roll_L, yaw_N | Which axis has residual moment; transients from staged inputs |
| (1,2) controls | throttle, pitch/roll/yaw input (optionally flap overlay) | What the scripted scenario commanded |
| (1,3) engine + fuel | engine kW (left y) + fuel gph (right y) | Power chain state; density-altitude derate visible |

Read (0,0) altitude first for the one-number summary. If altitude is doing something unexpected, jump to (1,1) moments and (0,3) α — they almost always explain the attitude story. Ground-track (1,0) is for catching course drift (torque/P-factor residuals) that never shows up in time-series.

## Symptom → coefficient

### Altitude

**Altitude drifts down in cruise.**
- α panel shows constant α below stall — aircraft isn't producing enough lift for weight at the commanded attitude. Candidates (in order): pilot-hold pitch input too small; `CM_ALPHA` too strong (pitches nose down harder than the held elevator can counter); throttle/thrust curve under-powered.
- First cheap check: raise the scenario's `SetPitch` from 0.05 to 0.08 and re-run. If it now holds altitude, the stability/authority ratio is the issue, not the thrust. If it still sinks, look at thrust.

**Altitude drifts up in cruise.**
- Inverse of above. Usually thrust curve too strong or `CM_ALPHA` too weak. Rare since we started with conservative curves; more likely the pilot-hold is too large.

**Altitude oscillates (phugoid, 20-40 s period, sinusoidal).**
- Phugoid is a constant-α speed/altitude trade — normal at low damping. If amplitude is growing, `CM_Q` (pitch damping) is too weak or positive in sign. Check sign first: `CM_Q` should be *negative*.
- If amplitude is shrinking, phugoid is damped — this is correct behavior. Don't chase it.

### Airspeed

**Never reaches target cruise speed at throttle 0.65.**
- Prop efficiency curve peaks away from cruise airspeed, or `CD0` / drag polar eating too much thrust. Check bot-right panel: prop_eff should be ~0.82 at cruise (55-70 m/s).
- Raise throttle to 1.0; if speed still plateaus low, it's drag; if speed jumps, it's thrust curve.

**Oscillates around target (short-period, 2-5 s).**
- Short-period mode under-damped. Usually a damping issue: `CM_Q`, or in fast cruise, the airspeed-ramp gating on coupling terms letting P-factor/torque kick in unevenly. Check coupling column in dynamics HUD / pitch_M column in CSV.

**Decays to stall during "cruise" scenario.**
- Either scenario has wrong initial conditions (too slow for the target attitude), or the aircraft is dragging itself down because α crept above the point where Cd rises faster than thrust recovers. Check α panel — if α is climbing, the aircraft is decelerating and pitch-up is reinforcing decay.

### Attitude / rotation

**Dutch roll (roll + yaw oscillation, visible on bank + β panels).**
- Yaw damping `CN_R` too weak. Should be ≤ -0.6.
- Adverse yaw `CN_DELTA_A_ADVERSE` too strong amplifies it; check roll input correlation.

**Wing drop past stall too aggressive.**
- `CM_STALL_PITCH_SLOPE` or the wing-drop roll moment too strong. Both are in `flight.rs` near the stall physics. 2.4.8 reduced pitch slope from -2.0 to -0.75; if further reduction is needed, do it one step at a time.

**Continuous left roll in cruise (torque reaction).**
- This is correct behavior. The Beaver has a right-rotating prop, so torque on the airframe is nose-left / left-roll. Pilot compensates with right aileron trim. The bench reveals it because scenarios don't have trim.
- Only a bug if bank angle accelerates unboundedly without aileron input. Steady ~14° bank over 30 s is expected given the coupling ramp.

**Rotational runaway near V=0.**
- The airspeed-ramp gating on coupling terms (`COUPLING_AIRSPEED_RAMP_MIN=5`, `MAX=20`) failed to prevent it, or residual 0.3/s angular decay is insufficient. Don't fix at physics level; fix by bounds. This is a known stand-in until 2.5 contact physics supplies normal-force constraints.

### Angle of attack / stall

**Stall triggers in benign cruise.**
- `ALPHA_STALL_RAD` (0.2793 rad ≈ 16°) too low for the actual α at cruise. Check if α at cruise is really below 0.1 rad (≈5.7°); if it's sitting at 0.2 rad, the wing is near-stall at cruise, which means either lift table is flat (Cl_max too low) or mass is high / airspeed is low.

**Never stalls in `beaver_stall_clean`.**
- Pilot-hold pitch input (0.30) not enough to hold α above stall as speed decays. Either raise the input, extend duration, or — more likely — elevator authority is too weak. Check dynamics HUD: control column should dominate stability column at high back-stick.

### Engine / fuel

**Engine power drops off with altitude.**
- Correct: density ratio derate is `air_density / ρ₀`. At 3 km ρ/ρ₀ ≈ 0.71, so 335 kW × 0.71 ≈ 238 kW. If the drop is steeper, check `ATMOSPHERE_SCALE_HEIGHT_M` (8500 m is standard).

**Fuel runs out absurdly fast.**
- `BSFC_GPH_PER_KW` (0.124) tuned from 27 gph at cruise. If a scenario burns faster than expected, it's likely running at higher power than intended (check engine_kw in plot). Not a coefficient bug.

## Current Beaver coefficients (2.4.9 close snapshot)

Copy-paste from `crates/sim/src/flight.rs` as of 2.4.9 close (Cl_max tune, Cd0 tune, dihedral, slipstream).

| Constant | Value | Meaning | Tuned |
|----------|-------|---------|-------|
| `CM_ALPHA` | -0.25 | α-stability (nose-down restoring moment per rad of α) | 2.4.4 |
| `CM_DELTA_E` | 0.24 | Elevator authority (pitch moment per unit of pitch input) | 2.4.4 |
| `CM_Q` | -25 | Pitch damping | 2.4.4 |
| `CL_DELTA_A` | 0.075 | Aileron authority | 2.4.4 |
| `CL_P` | -0.5 | Roll damping | 2.4.4 |
| **`CL_BETA`** | **-0.05** | **Dihedral roll stability (NEW in 2.4.9a)** | **2.4.9a** |
| `CN_BETA` | 0.08 | Weathervane (yaw-into-wind) | 2.4.4 |
| `CN_DELTA_R` | 0.04 | Rudder authority | 2.4.4 |
| `CN_R` | -0.6 | Yaw damping | 2.4.4 |
| `CN_DELTA_A_ADVERSE` | -0.012 | Adverse yaw from aileron | 2.4.6 |
| `P_FACTOR_ARM_M` | 10.0 | P-factor moment arm | 2.4.6 |
| `TORQUE_ROLL_NM_PER_KW` | 1.5 | Torque roll scale | 2.4.6 |
| **`SPIRAL_SLIPSTREAM_ARM_M`** | **2.0** | **Spiral slipstream moment arm (NEW in 2.4.9b)** | **2.4.9b** |
| **`SPIRAL_SLIPSTREAM_V_REF`** | **20.0 m/s** | **Slipstream decay reference airspeed** | **2.4.9b** |
| `COUPLING_AIRSPEED_RAMP_MIN/MAX` | 5 / 20 m/s | Airspeed ramp on coupling terms | 2.4.6 |
| `I_XX / I_YY / I_ZZ` | 8000 / 12000 / 5000 kg·m² | Moments of inertia (pitch / yaw / roll) | 2.4.4 |
| `MEAN_CHORD_M / WING_SPAN_M` | 1.52 / 14.63 | Reference lengths | 2.3 |
| **`CL_TABLE` peak** | **2.0 at 16°** | **Cl_max (raised from 1.65)** | **2.4.8 Gap 3** |
| **`CD0 / OSWALD_E`** | **0.055 / 0.75** | **Parasite drag + induced-drag efficiency (Cd0 raised from 0.045)** | **2.4.8 Gap 1** |
| `ALPHA_STALL_RAD / ALPHA_APPROACH_RAD` | 0.2793 / 0.2234 | Stall thresholds (≈16° / 12.8°) | 2.4.4d |
| `CM_STALL_PITCH_SLOPE` | -0.75 | Post-stall nose-down moment per rad past stall | 2.4.4d |
| `ENGINE_POWER_MAX_KW / IDLE_KW` | 335 / 20 | Power curve (linear in throttle) | 2.4.3 |
| `V_MIN_FOR_THRUST` | 12 m/s | Airspeed floor for T = ηP/V to keep static thrust finite | 2.4.3 |
| `BSFC_GPH_PER_KW` | 0.124 | Fuel burn per kW | 2.4.3 |
| `ATMOSPHERE_SCALE_HEIGHT_M` | 8500 | ISA exponential ρ(h) scale height | 2.4.7 |

**All tuning happens in `flight.rs`. Never in scenarios. Never in tolerances.**

## New symptoms unlocked by 2.4.9

**Dutch-roll oscillation in turn entry.**
- Cl_β too strong relative to yaw damping. Symptom: aircraft oscillates bank ±25° every ~20-30 s in a banked scenario.
- Fix: lower `CL_BETA` magnitude, OR increase `CN_R` yaw damping.

**Spiral divergence (bank grows without command).**
- Cl_β too weak or missing. Without dihedral, banked aircraft has no roll restoring.
- Fix: raise `CL_BETA` magnitude (current -0.05 works).

**Unrealistically low rudder at full power.**
- Spiral slipstream or P-factor too weak.
- Check: at full power V=55, cruise-climb rudder trim should be 0.15-0.25. If it's < 0.10, slipstream effect is too small; raise `SPIRAL_SLIPSTREAM_ARM_M`.

**Aircraft won't hold bank in closed-loop turn scenario.**
- PID gains wrong, NOT a physics issue (probably). Tune `BankHold::kp` / `ki` in the scenario. If bank oscillates: lower `kp` or add `kd`. If bank drifts: raise `ki`.
- Remember: the `ControlLaw` module is in `control.rs`; PID gains are per-scenario, not global.

**Closed-loop scenario won't converge.**
- Could be PID oscillation (see above) OR genuine envelope limit. Decision: does the aircraft correctly fail (accelerated stall, descent)? If yes, it's an envelope-limit scenario, not a capability test. See SKILL.md Core Rule #9.
