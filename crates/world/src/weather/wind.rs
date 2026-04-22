//! Altitude-stratified prevailing wind field.
//!
//! Pure function of `(master_seed, world_xyz, time)`. Derived from the
//! [`pressure`](super::pressure) field via approximate geostrophic balance,
//! with a friction-layer modification applied below the atmospheric boundary
//! layer.
//!
//! # Model
//!
//! **Geostrophic wind** at altitude: perpendicular to the horizontal pressure
//! gradient, rotated 90° clockwise in the Northern Hemisphere (game convention
//! — no hemisphere tracking at W1). Magnitude proportional to `|∇p|`. Computed
//! from a central finite-difference on [`pressure::sea_level_pressure`] —
//! pressure drives wind, and pressure is continuous + differentiable.
//!
//! **Boundary-layer friction** below [`BOUNDARY_LAYER_HEIGHT_M`]: reduces
//! magnitude and backs (rotates toward low pressure) the wind direction. Above
//! the boundary layer the wind is pure geostrophic.
//!
//! # Return shape
//!
//! [`wind_at`] returns a 3D wind vector in world axes `(east-x, up-y, north-z)`.
//! At W1 the vertical component is zero — convective updrafts / downdrafts arrive
//! with W3 per-cell deltas in Phase 3.2.3.

use glam::Vec3;

use super::pressure::{self, SYNOPTIC_WAVELENGTH_M};

/// Finite-difference step for pressure-gradient sampling (m).
///
/// Small enough to resolve synoptic-scale pressure curvature (a few percent of
/// `SYNOPTIC_WAVELENGTH_M`), large enough to avoid amplifying float-precision
/// noise. 3.5 km sits comfortably inside both constraints.
const FD_STEP_M: f32 = SYNOPTIC_WAVELENGTH_M / 100.0;

/// Geostrophic conversion: m/s of wind per hPa/m of pressure gradient.
///
/// Real geostrophic is `V = -(1/ρf) × ∇p × k̂`. For a typical ρ≈1.2 kg/m³ and
/// mid-latitude Coriolis f≈1e-4 /s, the prefactor works out to ~8000 m/s per
/// hPa/m. We divide by 100 to convert hPa → Pa, leaving ~80 m/s per Pa/m —
/// then scale down to game-plausible magnitudes (typical ∇p ≈ 0.001 hPa/m →
/// wind ≈ 8 m/s).
///
/// Chosen so synoptic-scale pressure variation (±25 hPa over ~350 km) produces
/// realistic 5–20 m/s winds.
pub const GEOSTROPHIC_GAIN: f32 = 8_000.0;

/// Atmospheric boundary-layer height (m).
///
/// Friction from terrain + surface roughness decreases wind magnitude and
/// rotates direction below this altitude. Real ABL varies 100 m (stable) to
/// 2 km (convective); 1 km is a reasonable game-average.
pub const BOUNDARY_LAYER_HEIGHT_M: f32 = 1_000.0;

/// Surface-wind magnitude scaling factor (fraction of geostrophic).
///
/// Real surface winds run 40–70% of geostrophic over land, higher over water.
/// Game-single value: 0.55.
pub const SURFACE_FRICTION_FACTOR: f32 = 0.55;

/// Surface wind backing angle (radians).
///
/// Friction rotates surface wind toward low pressure (counterclockwise in NH).
/// Real backing is 20–40°; game-single value: 25°.
pub const SURFACE_BACKING_RAD: f32 = 0.436; // ≈ 25°

/// Sea-level (2D) wind vector at world `(x, z)` at simulation `time` seconds.
///
/// Returns `(u_east, v_north)` in m/s. Derived from the pressure gradient via
/// geostrophic balance + boundary-layer friction. The vertical component is
/// zero at W1.
#[must_use]
pub fn sea_level_wind(master_seed: u64, x_m: f32, z_m: f32, time_s: f32) -> (f32, f32) {
    let (u_geo, v_geo) = geostrophic_wind(master_seed, x_m, z_m, time_s);

    // Surface layer: magnitude scaled down + backing rotation toward low pressure.
    let cos_b = libm::cosf(SURFACE_BACKING_RAD);
    let sin_b = libm::sinf(SURFACE_BACKING_RAD);
    let u = (u_geo * cos_b - v_geo * sin_b) * SURFACE_FRICTION_FACTOR;
    let v = (u_geo * sin_b + v_geo * cos_b) * SURFACE_FRICTION_FACTOR;
    (u, v)
}

/// 3D wind vector at world `(x, y_altitude, z)` at simulation `time` seconds.
///
/// Blends between surface-layer (frictional) wind at `y=0` and free-atmosphere
/// (geostrophic) wind at `y ≥ BOUNDARY_LAYER_HEIGHT_M`. Vertical component is
/// zero — convective vertical motion arrives with W3 cell deltas in Phase
/// 3.2.3.
#[must_use]
pub fn wind_at(master_seed: u64, x_m: f32, y_altitude_m: f32, z_m: f32, time_s: f32) -> Vec3 {
    let (u_geo, v_geo) = geostrophic_wind(master_seed, x_m, z_m, time_s);
    let altitude = y_altitude_m.max(0.0);

    // Altitude blend: t=0 at surface (full friction), t=1 at top of boundary layer.
    let t = (altitude / BOUNDARY_LAYER_HEIGHT_M).clamp(0.0, 1.0);

    // Friction magnitude lerps from SURFACE_FRICTION_FACTOR → 1.0 across the ABL.
    let friction_factor = SURFACE_FRICTION_FACTOR + (1.0 - SURFACE_FRICTION_FACTOR) * t;

    // Backing angle lerps from SURFACE_BACKING_RAD → 0 across the ABL.
    let backing = SURFACE_BACKING_RAD * (1.0 - t);
    let cos_b = libm::cosf(backing);
    let sin_b = libm::sinf(backing);

    let u = (u_geo * cos_b - v_geo * sin_b) * friction_factor;
    let v = (u_geo * sin_b + v_geo * cos_b) * friction_factor;

    Vec3::new(u, 0.0, v)
}

/// Raw geostrophic wind at world `(x, z)`, time `t`.
///
/// Returns `(u_east, v_north)` in m/s. Finite-difference on the sea-level
/// pressure field; 90° clockwise rotation gives the geostrophic direction
/// (Northern-Hemisphere convention).
#[must_use]
fn geostrophic_wind(master_seed: u64, x_m: f32, z_m: f32, time_s: f32) -> (f32, f32) {
    let p_xp = pressure::sea_level_pressure(master_seed, x_m + FD_STEP_M, z_m, time_s);
    let p_xm = pressure::sea_level_pressure(master_seed, x_m - FD_STEP_M, z_m, time_s);
    let p_zp = pressure::sea_level_pressure(master_seed, x_m, z_m + FD_STEP_M, time_s);
    let p_zm = pressure::sea_level_pressure(master_seed, x_m, z_m - FD_STEP_M, time_s);

    let dp_dx = (p_xp - p_xm) / (2.0 * FD_STEP_M);
    let dp_dz = (p_zp - p_zm) / (2.0 * FD_STEP_M);

    // 90° clockwise rotation of ∇p (Northern-Hemisphere geostrophic convention):
    // wind blows with high pressure on the right.
    //   u_geo = -∂p/∂z × gain
    //   v_geo =  ∂p/∂x × gain
    (-dp_dz * GEOSTROPHIC_GAIN, dp_dx * GEOSTROPHIC_GAIN)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn sea_level_wind_in_plausible_range() {
        // Sample a 400 km grid; wind magnitudes should lie in a realistic
        // synoptic range (a few m/s to ~25 m/s).
        let seed = 42;
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 20_000.0;
                let z = (iz as f32 - 10.0) * 20_000.0;
                let (u, v) = sea_level_wind(seed, x, z, 0.0);
                let mag = libm::sqrtf(u * u + v * v);
                assert!(
                    mag < 35.0,
                    "surface wind {mag} m/s at ({x}, {z}) exceeds plausible bound"
                );
            }
        }
    }

    #[test]
    fn wind_magnitude_increases_with_altitude() {
        // Above the boundary layer, geostrophic wind should exceed surface wind.
        let seed = 42;
        // Pick a point guaranteed to have nonzero gradient (away from pressure extrema).
        let (u_sfc, v_sfc) = sea_level_wind(seed, 50_000.0, 80_000.0, 0.0);
        let sfc_mag = libm::sqrtf(u_sfc * u_sfc + v_sfc * v_sfc);

        let aloft = wind_at(seed, 50_000.0, 3_000.0, 80_000.0, 0.0);
        let aloft_mag = libm::sqrtf(aloft.x * aloft.x + aloft.z * aloft.z);

        // Allow some points near gradient null to be exempt; check a second point
        // if the first trips.
        if sfc_mag < 0.5 {
            let (u2, v2) = sea_level_wind(seed, 200_000.0, 100_000.0, 0.0);
            let m2 = libm::sqrtf(u2 * u2 + v2 * v2);
            let a2 = wind_at(seed, 200_000.0, 3_000.0, 100_000.0, 0.0);
            let a2m = libm::sqrtf(a2.x * a2.x + a2.z * a2.z);
            assert!(a2m > m2, "aloft {a2m} should exceed surface {m2}");
        } else {
            assert!(
                aloft_mag > sfc_mag,
                "aloft {aloft_mag} should exceed surface {sfc_mag}"
            );
        }
    }

    #[test]
    fn wind_aloft_matches_geostrophic_direction() {
        // At top of boundary layer and above, backing angle → 0 so the
        // returned vector matches raw geostrophic direction.
        let seed = 42;
        let (u_geo, v_geo) = geostrophic_wind(seed, 75_000.0, 60_000.0, 0.0);
        let aloft = wind_at(seed, 75_000.0, BOUNDARY_LAYER_HEIGHT_M, 60_000.0, 0.0);
        assert_eq!(aloft.x.to_bits(), u_geo.to_bits());
        assert_eq!(aloft.z.to_bits(), v_geo.to_bits());
    }

    #[test]
    fn wind_deterministic() {
        let seed = 12_345;
        let w1 = wind_at(seed, 1_000.0, 500.0, 2_000.0, 100.0);
        let w2 = wind_at(seed, 1_000.0, 500.0, 2_000.0, 100.0);
        assert_eq!(w1.x.to_bits(), w2.x.to_bits());
        assert_eq!(w1.y.to_bits(), w2.y.to_bits());
        assert_eq!(w1.z.to_bits(), w2.z.to_bits());
    }

    #[test]
    fn different_seeds_produce_different_winds() {
        let w1 = wind_at(1, 50_000.0, 100.0, 50_000.0, 0.0);
        let w2 = wind_at(2, 50_000.0, 100.0, 50_000.0, 0.0);
        // At least one component differs.
        assert!(
            w1.x.to_bits() != w2.x.to_bits() || w1.z.to_bits() != w2.z.to_bits(),
            "seeds 1 vs 2 produced identical wind vectors"
        );
    }

    #[test]
    fn vertical_component_zero_at_w1() {
        let w = wind_at(42, 10_000.0, 500.0, -5_000.0, 500.0);
        assert_eq!(w.y.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn altitude_clamp_negative_to_zero() {
        let seed = 42;
        let w_neg = wind_at(seed, 0.0, -100.0, 0.0, 0.0);
        let w_zero = wind_at(seed, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(w_neg.x.to_bits(), w_zero.x.to_bits());
        assert_eq!(w_neg.z.to_bits(), w_zero.z.to_bits());
    }

    #[test]
    fn surface_wind_backs_relative_to_geostrophic() {
        // Surface wind is rotated CCW from geostrophic by the backing angle.
        // Given surface = R(+α) · geostrophic for α > 0, the 2D cross
        // (surface × geostrophic) = -|geo|² · sin(α) is negative.
        let seed = 42;
        let (u_g, v_g) = geostrophic_wind(seed, 120_000.0, 90_000.0, 0.0);
        let (u_s, v_s) = sea_level_wind(seed, 120_000.0, 90_000.0, 0.0);
        if libm::sqrtf(u_g * u_g + v_g * v_g) > 1.0 {
            let cross = u_s * v_g - v_s * u_g;
            assert!(
                cross < 0.0,
                "surface wind should back (CCW) from geostrophic; expected cross < 0, got {cross}"
            );
        }
    }
}
