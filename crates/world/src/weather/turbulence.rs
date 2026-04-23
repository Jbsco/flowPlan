//! Terrain-induced turbulence baseline.
//!
//! Pure function of `(master_seed, terrain, world_xyz, time)`. Models the W1
//! baseline intensity — the component driven by topographic gradients and
//! wind. Does not model storm-cell turbulence; that arrives with W3 per-cell
//! deltas in Phase 3.2.3.
//!
//! # Model
//!
//! Three contributions combine into a non-dimensional intensity scalar:
//!
//! 1. **Terrain-slope × wind magnitude.** Steep terrain under strong wind
//!    produces mechanical turbulence regardless of relative orientation. The
//!    slope is computed via central finite-difference on the terrain height
//!    field at `(x, z)` with step [`TERRAIN_FD_STEP_M`].
//!
//! 2. **Lee-side amplification.** When the wind blows *up* a slope the
//!    upwind-side flow is smooth; when it blows *down* a slope the downwind
//!    side generates rotors. The sign of `wind · ∇h` distinguishes the two
//!    cases — positive = upwind climb = reduced, negative = lee descent =
//!    amplified.
//!
//! 3. **Altitude decay.** Terrain-induced turbulence is strongest near the
//!    surface and decays exponentially with altitude. Scale height
//!    [`TERRAIN_TURB_SCALE_HEIGHT_M`] matches real ABL-top turbulence decay.
//!
//! # Output scale
//!
//! Roughly non-dimensional. Values ≈ 0 are calm air; ≈ 1 is moderately
//! turbulent; ≥ 3 is severe. Clamped to [`MAX_TURBULENCE_INTENSITY`] to keep
//! physics + audio consumers from blowing up on extreme terrain/wind.

use crate::terrain::HeightSampler;

use super::wind;

/// Finite-difference step for terrain-slope sampling (m).
///
/// Smaller than the coarsest terrain octave wavelength (~1 km) so gradients
/// resolve detail. 50 m is fine enough to catch ridge-scale features while
/// avoiding aliasing on the finest octaves.
pub const TERRAIN_FD_STEP_M: f32 = 50.0;

/// Gain applied to `slope × wind` product (inverse seconds).
///
/// Tuned so a typical 10% slope in a 10 m/s wind produces ≈ 1 unit of
/// turbulence (= moderate). Arrived at from: slope 0.1 × wind 10 m/s = 1.0
/// m/s raw product × gain 1.0 → 1.0 intensity.
pub const TERRAIN_TURB_GAIN: f32 = 1.0;

/// Lee-side amplification factor.
///
/// Downwind (wind-down-slope) turbulence is boosted by this multiplier; upwind
/// is unaffected. Real-world lee rotors are 2–4× the upwind turbulence;
/// conservative value chosen for baseline realism.
pub const LEE_FACTOR: f32 = 2.5;

/// Exponential scale height for terrain-turbulence altitude decay (m).
///
/// Terrain influence on turbulence is essentially gone above ~3× this height.
/// Matches real-world boundary-layer turbulence decay.
pub const TERRAIN_TURB_SCALE_HEIGHT_M: f32 = 800.0;

/// Upper bound on returned intensity.
///
/// Severe turbulence tops out here. Keeps downstream consumers (physics
/// integrators, audio gain staging) from blowing up on pathological
/// terrain/wind products.
pub const MAX_TURBULENCE_INTENSITY: f32 = 5.0;

/// Turbulence intensity at world `(x, y_altitude, z)` at `time` seconds.
///
/// Pure function of inputs. Uses the passed-in [`HeightSampler`] for terrain
/// gradient and [`wind::wind_at`] for the ambient wind vector. Returns a
/// non-dimensional intensity scalar in `[0, MAX_TURBULENCE_INTENSITY]`.
#[must_use]
pub fn turbulence_at<S: HeightSampler>(
    master_seed: u64,
    terrain: &S,
    x_m: f32,
    y_altitude_m: f32,
    z_m: f32,
    time_s: f32,
) -> f32 {
    let altitude = y_altitude_m.max(0.0);

    // Central finite-difference for the terrain gradient.
    let h_xp = terrain.height(x_m + TERRAIN_FD_STEP_M, z_m);
    let h_xm = terrain.height(x_m - TERRAIN_FD_STEP_M, z_m);
    let h_zp = terrain.height(x_m, z_m + TERRAIN_FD_STEP_M);
    let h_zm = terrain.height(x_m, z_m - TERRAIN_FD_STEP_M);
    let dh_dx = (h_xp - h_xm) / (2.0 * TERRAIN_FD_STEP_M);
    let dh_dz = (h_zp - h_zm) / (2.0 * TERRAIN_FD_STEP_M);
    let slope_mag = libm::sqrtf(dh_dx * dh_dx + dh_dz * dh_dz);

    // Ambient wind at this point (sampled at the query altitude so boundary-layer
    // friction reductions propagate).
    let w = wind::wind_at(master_seed, x_m, altitude, z_m, time_s);
    let wind_mag = libm::sqrtf(w.x * w.x + w.z * w.z);

    // Determine upwind-vs-lee orientation. Positive = wind climbs the slope
    // (upwind side), negative = wind descends (lee side). Unit-normalize to
    // avoid coupling magnitude into the factor.
    let lee_factor = if wind_mag > 0.01 && slope_mag > 1e-6 {
        let dot = (w.x * dh_dx + w.z * dh_dz) / (wind_mag * slope_mag);
        if dot < 0.0 {
            LEE_FACTOR
        } else {
            1.0
        }
    } else {
        1.0
    };

    // Altitude decay — terrain turbulence dies off above ABL.
    let altitude_decay = libm::expf(-altitude / TERRAIN_TURB_SCALE_HEIGHT_M);

    let intensity = slope_mag * wind_mag * lee_factor * altitude_decay * TERRAIN_TURB_GAIN;
    intensity.clamp(0.0, MAX_TURBULENCE_INTENSITY)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::terrain::Terrain;

    #[test]
    fn flat_terrain_produces_zero_turbulence() {
        let terrain = Terrain::flat_land();
        let t = turbulence_at(42, &terrain, 1_000.0, 100.0, -500.0, 0.0);
        assert_eq!(t.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn terrain_slope_produces_nonzero_turbulence() {
        let terrain = Terrain::from_seed(42);
        // Sample a 100 km × 100 km grid; at least one point should produce
        // measurable terrain-induced turbulence. Range widened post 3.6.0
        // terrain rewrite — Perlin-fBm has gentler local slopes than the
        // old sum-of-sines, so broader area needed to guarantee a
        // turbulence-triggering gradient.
        let mut any_nonzero = false;
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 5_000.0;
                let z = (iz as f32 - 10.0) * 5_000.0;
                let t = turbulence_at(42, &terrain, x, 200.0, z, 0.0);
                if t > 0.001 {
                    any_nonzero = true;
                    break;
                }
            }
            if any_nonzero {
                break;
            }
        }
        assert!(
            any_nonzero,
            "expected at least one turbulent point over seeded terrain"
        );
    }

    #[test]
    fn turbulence_decreases_with_altitude() {
        let terrain = Terrain::from_seed(42);
        // Find a point with nonzero slope.
        let (mut qx, mut qz) = (0.0, 0.0);
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 500.0;
                let z = (iz as f32 - 10.0) * 500.0;
                let t = turbulence_at(42, &terrain, x, 100.0, z, 0.0);
                if t > 0.05 {
                    qx = x;
                    qz = z;
                    break;
                }
            }
            if qx != 0.0 || qz != 0.0 {
                break;
            }
        }
        let t_sfc = turbulence_at(42, &terrain, qx, 100.0, qz, 0.0);
        let t_high = turbulence_at(42, &terrain, qx, 5_000.0, qz, 0.0);
        assert!(
            t_high < t_sfc,
            "turbulence at 5 km ({t_high}) should be below surface ({t_sfc}) at ({qx}, {qz})"
        );
    }

    #[test]
    fn turbulence_deterministic() {
        let terrain = Terrain::from_seed(123);
        let t1 = turbulence_at(42, &terrain, 500.0, 200.0, -300.0, 10.0);
        let t2 = turbulence_at(42, &terrain, 500.0, 200.0, -300.0, 10.0);
        assert_eq!(t1.to_bits(), t2.to_bits());
    }

    #[test]
    fn turbulence_bounded_by_max() {
        let terrain = Terrain::from_seed(42);
        for ix in 0..30 {
            for iz in 0..30 {
                let x = (ix as f32 - 15.0) * 2_000.0;
                let z = (iz as f32 - 15.0) * 2_000.0;
                let t = turbulence_at(42, &terrain, x, 0.0, z, 0.0);
                assert!(
                    (0.0..=MAX_TURBULENCE_INTENSITY).contains(&t),
                    "turbulence {t} at ({x}, {z}) outside [0, {MAX_TURBULENCE_INTENSITY}]"
                );
            }
        }
    }

    #[test]
    fn altitude_clamp_negative_to_zero() {
        let terrain = Terrain::from_seed(42);
        let t_neg = turbulence_at(42, &terrain, 500.0, -100.0, -300.0, 0.0);
        let t_zero = turbulence_at(42, &terrain, 500.0, 0.0, -300.0, 0.0);
        assert_eq!(t_neg.to_bits(), t_zero.to_bits());
    }
}
