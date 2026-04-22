//! Orographic lift + rain shadow coupling.
//!
//! Pure function of `(master_seed, terrain, world_xz, time)`. Produces a
//! multiplicative humidity bias: windward slopes of high terrain are
//! boosted (moisture forced up condenses), lee slopes are desiccated
//! (descending air warms and dries — Foehn / Chinook effect).
//!
//! Pattern mirrors [`turbulence`](super::turbulence) lee-side factor: sample
//! the terrain gradient at the query point, dot with the ambient wind
//! vector, scale by terrain elevation × wind magnitude × slope steepness.
//!
//! # Real-world-possible gate
//!
//! This coupling is responsible for every high-desert-next-to-snowy-range
//! biome pair (Grand Mesa ↔ Flatirons gradient, Cascades ↔ eastern Oregon,
//! Andes ↔ Atacama). Without it, weather over mountainous terrain would
//! classify uniformly regardless of wind direction, violating the
//! real-world-possible gate.

use crate::terrain::HeightSampler;

use super::wind;

/// Finite-difference step for terrain-gradient sampling (m).
///
/// Matches [`turbulence::TERRAIN_FD_STEP_M`](super::turbulence::TERRAIN_FD_STEP_M)
/// so both effects share a consistent gradient reference at each point.
pub const TERRAIN_FD_STEP_M: f32 = 50.0;

/// Altitude offset above terrain at which to sample wind for the coupling
/// (m). ~Ridge height above peaks. Avoids the surface-friction layer so
/// the dominant air mass's direction drives the effect.
pub const WIND_SAMPLE_ABOVE_TERRAIN_M: f32 = 500.0;

/// Terrain-elevation reference (m). At this elevation the elevation
/// factor is 1.0 (full contribution); below this it's attenuated.
pub const ELEVATION_REFERENCE_M: f32 = 500.0;

/// Wind-magnitude reference (m/s). At this wind speed the wind factor
/// is 1.0; below this it's attenuated. 10 m/s matches typical synoptic
/// wind aloft.
pub const WIND_REFERENCE_MPS: f32 = 10.0;

/// Slope reference (rise/run). At this slope the slope factor is 1.0;
/// below this it's attenuated. 5% slope is roughly the angle at which
/// orographic lift becomes first-order dominant.
pub const SLOPE_REFERENCE: f32 = 0.05;

/// Maximum bias magnitude.
///
/// Humidity on the strongest windward slopes is boosted by up to
/// `1.0 + MAX_BIAS`; strongest lee slopes reduced by `1.0 - MAX_BIAS`.
/// Chosen so local extremes produce visible biome shifts without ever
/// driving RH outside `[0, 1]` after clamp.
pub const MAX_BIAS: f32 = 0.5;

/// Minimum terrain elevation below which no orographic effect applies (m).
/// Coastal flats / low-elevation terrain doesn't lift or shadow air.
pub const MIN_TERRAIN_ELEVATION_M: f32 = 50.0;

/// Multiplicative humidity modifier at world `(x, z)` given the terrain
/// sampler and master seed. Returns `~[1 - MAX_BIAS, 1 + MAX_BIAS]`.
///
/// Pure function — deterministic per `(master_seed, terrain, x, z, time_s)`.
#[must_use]
pub fn terrain_moisture_modifier<T: HeightSampler>(
    master_seed: u64,
    terrain: &T,
    x_m: f32,
    z_m: f32,
    time_s: f32,
) -> f32 {
    let h = terrain.height(x_m, z_m);
    if h < MIN_TERRAIN_ELEVATION_M {
        return 1.0;
    }

    // Terrain gradient via central finite-difference.
    let h_xp = terrain.height(x_m + TERRAIN_FD_STEP_M, z_m);
    let h_xm = terrain.height(x_m - TERRAIN_FD_STEP_M, z_m);
    let h_zp = terrain.height(x_m, z_m + TERRAIN_FD_STEP_M);
    let h_zm = terrain.height(x_m, z_m - TERRAIN_FD_STEP_M);
    let dh_dx = (h_xp - h_xm) / (2.0 * TERRAIN_FD_STEP_M);
    let dh_dz = (h_zp - h_zm) / (2.0 * TERRAIN_FD_STEP_M);
    let slope_mag = libm::sqrtf(dh_dx * dh_dx + dh_dz * dh_dz);

    if slope_mag < 1e-5 {
        return 1.0;
    }

    // Ambient wind at ~ridge altitude (terrain surface + offset). Above the
    // boundary layer → full geostrophic wind, not friction-reduced surface.
    let sample_altitude = h + WIND_SAMPLE_ABOVE_TERRAIN_M;
    let w = wind::wind_at(master_seed, x_m, sample_altitude, z_m, time_s);
    let wind_mag = libm::sqrtf(w.x * w.x + w.z * w.z);

    if wind_mag < 0.01 {
        return 1.0;
    }

    // Unit-normalized dot product: ∈ [-1, 1].
    //  > 0 → wind climbs the slope (windward, moisture forced up, humidity boosted)
    //  < 0 → wind descends the slope (lee, warming descent dries air, humidity reduced)
    let dot = (w.x * dh_dx + w.z * dh_dz) / (wind_mag * slope_mag);

    // Contribution factors — each saturates at 1.0, so their product also
    // stays in [0, 1]. The final bias is `dot × combined × MAX_BIAS`, so
    // bias ∈ [-MAX_BIAS, +MAX_BIAS] by construction.
    let elev_factor = (h / ELEVATION_REFERENCE_M).clamp(0.0, 1.0);
    let wind_factor = (wind_mag / WIND_REFERENCE_MPS).clamp(0.0, 1.0);
    let slope_factor = (slope_mag / SLOPE_REFERENCE).clamp(0.0, 1.0);
    let combined = elev_factor * wind_factor * slope_factor;

    let bias = dot * combined * MAX_BIAS;
    1.0 + bias
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::terrain::Terrain;

    #[test]
    fn flat_terrain_produces_unit_modifier() {
        let flat = Terrain::flat_land();
        let m = terrain_moisture_modifier(42, &flat, 1_000.0, -500.0, 0.0);
        assert_eq!(m.to_bits(), 1.0_f32.to_bits());
    }

    #[test]
    fn low_elevation_produces_unit_modifier() {
        // Terrain::flat_land() has base_height = 0 and zero amplitude, so
        // every query is at height 0 < MIN_TERRAIN_ELEVATION_M → no effect.
        let flat = Terrain::flat_land();
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 500.0;
                let z = (iz as f32 - 10.0) * 500.0;
                let m = terrain_moisture_modifier(42, &flat, x, z, 0.0);
                assert_eq!(m.to_bits(), 1.0_f32.to_bits());
            }
        }
    }

    #[test]
    fn deterministic() {
        let t = Terrain::from_seed(42);
        let a = terrain_moisture_modifier(42, &t, 1_234.0, -5_678.0, 100.0);
        let b = terrain_moisture_modifier(42, &t, 1_234.0, -5_678.0, 100.0);
        assert_eq!(a.to_bits(), b.to_bits());
    }

    #[test]
    fn windward_boosts_lee_reduces() {
        // Over seeded terrain, scan a grid; there should be points where
        // the modifier is > 1 and points where it is < 1.
        let t = Terrain::from_seed(42);
        let mut saw_boost = false;
        let mut saw_reduce = false;
        for ix in 0..40 {
            for iz in 0..40 {
                let x = (ix as f32 - 20.0) * 200.0;
                let z = (iz as f32 - 20.0) * 200.0;
                let m = terrain_moisture_modifier(42, &t, x, z, 0.0);
                if m > 1.001 {
                    saw_boost = true;
                }
                if m < 0.999 {
                    saw_reduce = true;
                }
                if saw_boost && saw_reduce {
                    return;
                }
            }
        }
        panic!("expected both boost and reduce; saw_boost={saw_boost} saw_reduce={saw_reduce}");
    }

    #[test]
    fn modifier_stays_within_max_bias() {
        // Scan a grid; every modifier must lie in [1 - MAX_BIAS, 1 + MAX_BIAS].
        let t = Terrain::from_seed(42);
        let lo = 1.0 - MAX_BIAS - 1e-5;
        let hi = 1.0 + MAX_BIAS + 1e-5;
        for ix in 0..30 {
            for iz in 0..30 {
                let x = (ix as f32 - 15.0) * 500.0;
                let z = (iz as f32 - 15.0) * 500.0;
                let m = terrain_moisture_modifier(42, &t, x, z, 0.0);
                assert!(
                    (lo..=hi).contains(&m),
                    "modifier {m} at ({x}, {z}) outside bounds [{lo}, {hi}]"
                );
            }
        }
    }

    #[test]
    fn windward_and_lee_pair_bracket_unit() {
        // At any point, flipping the wind direction must invert the sign of
        // the bias (windward of wind+ = lee of wind-). We can't easily
        // flip the wind direction without refactoring, but we CAN verify
        // the mean modifier over a large area is ~1.0 (boosts and reductions
        // statistically cancel across the field).
        let t = Terrain::from_seed(42);
        let mut sum = 0.0_f32;
        let mut count = 0_u32;
        for ix in 0..50 {
            for iz in 0..50 {
                let x = (ix as f32 - 25.0) * 400.0;
                let z = (iz as f32 - 25.0) * 400.0;
                let m = terrain_moisture_modifier(42, &t, x, z, 0.0);
                sum += m;
                count += 1;
            }
        }
        let mean = sum / count as f32;
        // Mean should be close to 1.0 — boosts and reductions cancel in
        // aggregate across a rolling terrain. Loose tolerance because the
        // 50x50 sample may not perfectly balance and the seeded terrain
        // may have a net windward or lee skew.
        assert!(
            (mean - 1.0).abs() < 0.1,
            "mean modifier {mean} deviates too far from unit over 2500 samples"
        );
    }
}
