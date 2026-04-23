//! Magnetic variation field (Phase 3.6.1e).
//!
//! Deterministic per-world spatial field giving the local magnetic
//! variation (compass deviation from true north) in degrees.
//!
//! Consumed by runway-numbering authoring (`display_number =
//! round((heading - variation) / 10)`) and by avionics instruments
//! (compass shows magnetic; GPS / HSI show true; the difference IS
//! this field).
//!
//! # Model
//!
//! flowPlan's world is a flat torus — there's no magnetic pole, so
//! variation is a purely artistic choice. We match real-Earth
//! expectations via two components:
//!
//! 1. **Latitude-proxy bias**: the same `cos(2π·z/W)` latitude proxy
//!    used by the temperature field. "Northern" latitudes (z near ±W/2)
//!    get more variation — matches real Earth where high-latitude
//!    regions show the biggest compass deviations (Alaska ~20°E, Norway
//!    ~5°E, equator ~0°, etc.).
//! 2. **Perlin-fBm spatial variation**: ~800 km wavelength, 3 octaves.
//!    Gives organic field structure on top of the latitude bias so
//!    local variation differs meaningfully on ~500 km flights.
//!
//! Total amplitude ±`MAG_VAR_AMPLITUDE_DEG` (= 15°). Realistic for
//! continental-US scale; not Alaska-brutal. Tune the constant if
//! playtest reveals the wrong difficulty.
//!
//! # Determinism
//!
//! Pure function of `(master_seed, x, z)`. Bit-identical on every
//! machine. Toroidal-wrap-clean via the standard octave-offset + fBm
//! pattern — `magnetic_variation(x, z) == magnetic_variation(x + W, z)`.

use core::f32::consts::TAU;

use flowplan_core::WORLD_EXTENT_M;

use crate::noise::fbm_2d;

/// Magnetic variation amplitude (degrees, peak-to-peak is ±this).
///
/// 15° chosen to match continental-US variation range (0-20° real-world,
/// rounded to 15° as a representative scale). Large enough that pilots
/// notice the runway-number-vs-true-heading gap; small enough that new
/// players aren't brutalized by navigation calculus. Push to 30° for
/// Alaska-scale brutality; drop to 5° to soften.
pub const MAG_VAR_AMPLITUDE_DEG: f32 = 15.0;

/// Spatial fBm wavelength (m).
///
/// 800 km — roughly the scale of real-Earth regional variation (e.g.
/// the US mainland shows maybe 5 major contour bands across 5000 km).
/// Quantized to a clean divisor of `WORLD_EXTENT_M` for wrap.
pub const MAG_VAR_SPATIAL_WAVELENGTH_M: f32 = flowplan_core::WORLD_EXTENT_M / 6.0;

/// Latitude-proxy contribution weight (0..1). 0.6 = latitude dominates
/// the field, fBm provides texture. Gives the "high-latitude regions
/// have more variation" character while letting spatial variation
/// provide distinct-pocket feel.
const LATITUDE_WEIGHT: f32 = 0.6;

/// Spatial-fBm contribution weight (0..1). `1 - LATITUDE_WEIGHT` by
/// construction so total amplitude stays ≤ `MAG_VAR_AMPLITUDE_DEG`.
const SPATIAL_WEIGHT: f32 = 1.0 - LATITUDE_WEIGHT;

/// Octave-offset base for the spatial fBm — reserved in the project's
/// octave-offset convention alongside other weather / terrain / biome
/// offsets. Range `[700..720)` claimed by magnetic variation so it
/// doesn't collide with biome primitive offsets `[500..700)` or
/// reserve space for future layers.
const MAG_VAR_OCTAVE_OFFSET: u32 = 700;

const MAG_VAR_FIELD_TAG: u64 = u64::from_le_bytes(*b"magvaria");

/// Deterministic splitmix64-inspired seed mixer. Mirrors the pattern
/// in `weather::mod::mix64` (keeping this module independent of the
/// weather subsystem's visibility).
const fn mix_seed(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Magnetic variation at world coordinate `(x, z)` in degrees.
///
/// Positive = east variation (compass reads less than true); negative
/// = west variation (compass reads more than true). Magnitude bounded
/// by [`MAG_VAR_AMPLITUDE_DEG`].
///
/// Usage: `true_heading = magnetic_heading + magnetic_variation(x, z)`.
/// Equivalently, runway-number authoring computes
/// `runway_display_heading = round((true_heading - variation) / 10) × 10`.
#[must_use]
pub fn magnetic_variation(master_seed: u64, x_m: f32, z_m: f32) -> f32 {
    let seed = mix_seed(master_seed, MAG_VAR_FIELD_TAG);

    // Latitude proxy — same `cos(2π·z/W)` as temperature. +1 at equator
    // (z=0), -1 at poles (z=±W/2). We treat "northern" (z < 0) and
    // "southern" (z > 0) asymmetrically: northern = positive variation
    // (matches real Earth northern hemisphere east-variation bias for
    // the dominant continental US analog).
    let lat_phase = TAU * z_m / WORLD_EXTENT_M;
    // sin(lat_phase) gives +1 at z=W/4 ("northern peak"), 0 at equator,
    // -1 at z=-W/4 ("southern peak"). Asymmetric: matches real-world
    // N/S variation asymmetry.
    let latitude_component = libm::sinf(lat_phase);

    // Spatial fBm at ~800 km wavelength, 3 octaves. Output ~[-1, 1].
    let spatial_component = fbm_2d(
        seed,
        MAG_VAR_OCTAVE_OFFSET,
        x_m,
        z_m,
        MAG_VAR_SPATIAL_WAVELENGTH_M,
        3,
    );

    let normalized = LATITUDE_WEIGHT * latitude_component + SPATIAL_WEIGHT * spatial_component;
    MAG_VAR_AMPLITUDE_DEG * normalized
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a = magnetic_variation(42, 100_000.0, 200_000.0);
        let b = magnetic_variation(42, 100_000.0, 200_000.0);
        assert_eq!(a.to_bits(), b.to_bits());
    }

    #[test]
    fn different_seeds_differ() {
        let a = magnetic_variation(1, 100_000.0, 200_000.0);
        let b = magnetic_variation(2, 100_000.0, 200_000.0);
        assert_ne!(a.to_bits(), b.to_bits());
    }

    #[test]
    fn amplitude_bounded() {
        // Over a wide sample grid, no value should exceed the amplitude
        // (plus a tiny fBm peak allowance of ~1.4×).
        let seed = 42;
        let bound = MAG_VAR_AMPLITUDE_DEG * 1.05;
        for ix in -10_i32..=10 {
            for iz in -10_i32..=10 {
                let x = ix as f32 * 250_000.0;
                let z = iz as f32 * 250_000.0;
                let v = magnetic_variation(seed, x, z);
                assert!(
                    v.abs() <= bound,
                    "magvar {v}° at ({x}, {z}) exceeds ±{bound}°"
                );
            }
        }
    }

    #[test]
    fn toroidal_wrap_continuous() {
        // magnetic_variation(x, z) ≈ magnetic_variation(x + W, z) up to
        // f32 precision at world scale (~1° tolerance matches the
        // broader seam-continuity posture documented in terrain.rs).
        let seed = 42;
        for z in [-100_000.0_f32, 0.0, 1_234_567.0] {
            let here = magnetic_variation(seed, 50_000.0, z);
            let wrapped = magnetic_variation(seed, 50_000.0 + WORLD_EXTENT_M, z);
            assert!(
                (here - wrapped).abs() < 1.0,
                "magvar seam discontinuity at z={z}: {here}° vs {wrapped}°"
            );
        }
    }

    #[test]
    fn latitude_asymmetry_present() {
        // At the "northern peak" (z = W/4), average variation should
        // lean positive (east); at "southern peak" (z = -W/4) negative.
        // fBm noise can dominate at individual points, but the mean
        // across many x samples should show the bias.
        let seed = 42;
        let northern_z = WORLD_EXTENT_M / 4.0;
        let southern_z = -WORLD_EXTENT_M / 4.0;
        let mut mean_north = 0.0_f32;
        let mut mean_south = 0.0_f32;
        let n = 40_i32;
        for i in 0..n {
            let x = i as f32 * (WORLD_EXTENT_M / n as f32);
            mean_north += magnetic_variation(seed, x, northern_z);
            mean_south += magnetic_variation(seed, x, southern_z);
        }
        mean_north /= n as f32;
        mean_south /= n as f32;
        assert!(
            mean_north > mean_south,
            "expected northern bias > southern: north={mean_north}°, south={mean_south}°"
        );
    }
}
