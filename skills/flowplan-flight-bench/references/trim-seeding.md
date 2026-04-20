# Trim Seeding

## Why seeding at α = 0 is wrong

The flight model's lift coefficient table starts at zero α and rises with α. An aircraft released nose-level at cruise speed has α ≈ 0, Cl ≈ 0.2, and *less lift than weight*. It sinks. The bench captures that sink as "altitude drift", the assertion fails, and you spend an afternoon chasing a coefficient bug that isn't there.

The POH cruise attitude is closer to 3° nose-up because the wing is mounted at a small incidence and the airframe flies slightly nose-high at steady-state cruise α. Seeding scenarios there starts us in trim.

## CRUISE_ATTITUDE

```rust
// 3° nose-up around body +X.
const CRUISE_ATTITUDE: Quat = Quat::from_xyzw(0.026_177_19, 0.0, 0.0, 0.999_657_4);
```

Quaternion from `Quat::from_axis_angle(Vec3::X, 3.0_f32.to_radians())`, constant-folded to avoid a runtime call in static-data context.

## Picking a trim angle for a different aircraft

Rule of thumb: use the POH cruise α, or 3° if unknown. Most GA fixed-wing flies cruise α in 2-4°. Use the lower end for slippery/high-AR wings (gliders), higher for draggy/STOL airframes.

For a float/amphibious variant, add ~1° (floats raise induced drag; α at cruise is slightly higher).

## The pilot-hold input

`CRUISE_ATTITUDE` alone isn't enough. α-stability (Cm_α < 0) will pitch the nose down from the trim attitude unless the pilot holds a small back-stick. In the bench, we inject a `SetPitch(x)` input at tick 0:

| Scenario | Typical pitch input | Why |
|----------|---------------------|-----|
| Steady cruise | 0.05 | Counters Cm_α, holds attitude. |
| Vy climb | 0.10 | Holds higher α for best-rate climb. |
| Stall induction | 0.30 | Commands sustained high α until the wing stalls. |
| Approach on flaps | 0.05-0.08 | Compensates for flap pitch-down ΔCm. |

If 0.05 doesn't hold cruise, the flight model (not the scenario) is probably wrong -- Cm_δe/|Cm_α| should be in [0.3, 0.5]. See the symptom map.

## Seeding velocity

Velocity vector is parallel to world +Z negative (flowplan convention: nose points -Z). Magnitude is the target airspeed in m/s. For climb scenarios, *do not* tilt the velocity up -- the aircraft should climb from the attitude + lift, not from an initial upward velocity component. Starting with `velocity.y > 0` conceals whether lift is actually being produced.

## Seeding angular velocity

Always `Vec3::ZERO`. Initial pitch/roll/yaw rates that aren't zero make the scenario untraceable: the first few ticks are coupled-mode transients that the flight model has to damp out before the assertion window. If you think you need non-zero initial ω, you're probably trying to test a transient response; that's a different scenario with an explicitly ramped input, not a seeded rotation.

## Fuel

Start at a round non-zero value (80 gal for the Beaver) unless the scenario tests fuel exhaustion. Full-tank (95 gal) scenarios add weight; a mid-tank value is more representative of "typical flight" without introducing a mass-variation variable into feel-tuning runs.
