//! Layer W3 — per-cell atmospheric deltas + composed samplers.
//!
//! Produces the **consumer API surface** weather-dependent systems query:
//! cloud density, precipitation rate, visibility, and weather-composed wind +
//! turbulence. Each sampler is a pure function of `(master_seed, terrain,
//! world_xyz, time)`.
//!
//! # Architecture
//!
//! W3 sits on top of W1 ([atmospheric base](super)) and W2
//! ([cell state](super::cell_state)). A query at world coordinate `(x, z)`:
//!
//! 1. Resolves its owning Voronoi cell via [`super::voronoi::cell_at`].
//! 2. Samples the cell's current state via [`super::cell_state::state_at_time_with_terrain`] — type, density, base/top altitude, precip rate, lifecycle phase.
//! 3. Computes a per-cell-type delta (updraft, turbulence contribution, cloud profile).
//! 4. Composes the delta onto the W1 baseline and returns the result.
//!
//! # Edge behavior
//!
//! For v1, cell-edge transitions are **sharp** — the owning-cell delta
//! applies fully; there is no multi-cell blending. This matches real
//! cumulus edges (which are visually sharp) and real frontal passages
//! (where wind shifts abruptly). Smoother blending for wind + precipitation
//! at cell boundaries is a future refinement; the `distance_to_edge` field
//! is available in `CellLookup` for callers that want to feather on their
//! own terms.
//!
//! # Scope
//!
//! **3.2.3a (this module):** pure-function samplers. No sim-side coupling —
//! that's 3.2.3b. No chop wiring — that's 3.2.3c.

use glam::Vec3;

use super::{cell_state, voronoi, wind};
use crate::terrain::HeightSampler;

/// Baseline clear-air horizontal visibility (m). Represents "can see to the
/// horizon" in good conditions; caps visibility in the absence of weather.
pub const BASELINE_VISIBILITY_M: f32 = 50_000.0;

/// Horizontal visibility inside dense cloud (m). Picked to match "can see
/// the wingtip but nothing beyond" — ~100m is typical inside a cumulus core.
pub const CLOUD_INTERIOR_VISIBILITY_M: f32 = 100.0;

/// Horizontal visibility in heavy rain (m). ~1.5 km matches heavy-shower
/// real-world observation.
pub const HEAVY_RAIN_VISIBILITY_M: f32 = 1_500.0;

/// Peak updraft speed inside a nimbus core (m/s). Real cumulonimbus cores
/// produce 10–30 m/s updrafts; we use 5 m/s to keep flight-model response
/// playable at v1. Tuneable.
pub const UPDRAFT_NIMBUS_MAX_MPS: f32 = 5.0;

/// Peak updraft speed inside a fair-weather cumulus (m/s). Real fair-weather
/// cumulus updrafts are 1–3 m/s.
pub const UPDRAFT_CUMULUS_MAX_MPS: f32 = 1.5;

/// Peak updraft speed inside a squall cell (m/s).
pub const UPDRAFT_SQUALL_MAX_MPS: f32 = 3.0;

/// Turbulence intensity added inside a nimbus core on top of W1 baseline.
pub const TURBULENCE_DELTA_NIMBUS: f32 = 2.0;

/// Turbulence intensity added inside a squall cell.
pub const TURBULENCE_DELTA_SQUALL: f32 = 3.0;

/// Turbulence intensity added inside a frontal cell.
pub const TURBULENCE_DELTA_FRONTAL: f32 = 1.0;

/// Reference precipitation rate at which visibility drops to
/// `HEAVY_RAIN_VISIBILITY_M` (mm/hr). Below this rate, visibility degrades
/// linearly from baseline.
pub const HEAVY_PRECIP_REFERENCE_MM_PER_HR: f32 = 15.0;

/// Cloud density at world `(x, y_altitude, z)` at sim time `time_s`.
///
/// Returns a fraction in `[0, 1]`. Outside the owning cell's `[base, top]`
/// altitude band, or in clear air, the result is 0. Within the band, the
/// cell's peak density is scaled by a parabolic altitude profile (peak at
/// mid-height for convective types; flat for stratus).
#[must_use]
pub fn cloud_density_at<T: HeightSampler>(
    master_seed: u64,
    terrain: &T,
    x_m: f32,
    y_altitude_m: f32,
    z_m: f32,
    time_s: f32,
) -> f32 {
    let lookup = voronoi::cell_at(master_seed, x_m, z_m);
    let state =
        cell_state::state_at_time_with_terrain(master_seed, terrain, lookup.cell_id, time_s);
    if matches!(state.cell_type, cell_state::CellType::ClearAir) {
        return 0.0;
    }
    let profile = altitude_profile(
        state.cell_type,
        y_altitude_m,
        state.base_altitude_m,
        state.top_altitude_m,
    );
    (state.density * profile).clamp(0.0, 1.0)
}

/// Precipitation rate (mm/hr) at world `(x, y_altitude, z)` at sim time
/// `time_s`. Falls within the cell's horizontal footprint below cloud base;
/// zero outside the cell or above the cloud.
#[must_use]
pub fn precipitation_rate_at<T: HeightSampler>(
    master_seed: u64,
    terrain: &T,
    x_m: f32,
    y_altitude_m: f32,
    z_m: f32,
    time_s: f32,
) -> f32 {
    let lookup = voronoi::cell_at(master_seed, x_m, z_m);
    let state =
        cell_state::state_at_time_with_terrain(master_seed, terrain, lookup.cell_id, time_s);
    if matches!(state.cell_type, cell_state::CellType::ClearAir)
        || state.precip_rate_mm_per_hr <= 0.0
    {
        return 0.0;
    }
    // Precipitation reaches the ground; a pilot at altitude below cloud base
    // is in the rain shaft. Above cloud base, we're above the rain (no
    // precip at the query altitude).
    if y_altitude_m > state.base_altitude_m {
        return 0.0;
    }
    state.precip_rate_mm_per_hr
}

/// Effective horizontal visibility (m) at world `(x, y_altitude, z)` at sim
/// time `time_s`. Reduced by cloud density and by precipitation rate;
/// returns whichever is tighter.
#[must_use]
pub fn visibility_at<T: HeightSampler>(
    master_seed: u64,
    terrain: &T,
    x_m: f32,
    y_altitude_m: f32,
    z_m: f32,
    time_s: f32,
) -> f32 {
    let cloud_d = cloud_density_at(master_seed, terrain, x_m, y_altitude_m, z_m, time_s);
    let precip = precipitation_rate_at(master_seed, terrain, x_m, y_altitude_m, z_m, time_s);

    // Cloud-interior reduction: lerp from baseline to cloud-interior based on
    // density.
    let vis_from_cloud = lerp(BASELINE_VISIBILITY_M, CLOUD_INTERIOR_VISIBILITY_M, cloud_d);

    // Precipitation reduction: scale by heavy-precip reference, clamp to [0, 1].
    let precip_t = (precip / HEAVY_PRECIP_REFERENCE_MM_PER_HR).clamp(0.0, 1.0);
    let vis_from_precip = lerp(BASELINE_VISIBILITY_M, HEAVY_RAIN_VISIBILITY_M, precip_t);

    vis_from_cloud.min(vis_from_precip)
}

/// Wind vector at world `(x, y_altitude, z)` at sim time `time_s`.
///
/// Composes the W1 baseline wind with the owning cell's W3 delta (updraft /
/// downdraft, gust-front shift). Vertical component non-zero inside convective
/// cells.
#[must_use]
pub fn wind_at_with_weather<T: HeightSampler>(
    master_seed: u64,
    terrain: &T,
    x_m: f32,
    y_altitude_m: f32,
    z_m: f32,
    time_s: f32,
) -> Vec3 {
    let base = wind::wind_at(master_seed, x_m, y_altitude_m, z_m, time_s);
    let lookup = voronoi::cell_at(master_seed, x_m, z_m);
    let state =
        cell_state::state_at_time_with_terrain(master_seed, terrain, lookup.cell_id, time_s);
    let delta = wind_delta(
        state.cell_type,
        y_altitude_m,
        state.base_altitude_m,
        state.top_altitude_m,
        state.density,
    );
    base + delta
}

/// Turbulence intensity at world `(x, y_altitude, z)` at sim time `time_s`,
/// composing the W1 terrain-induced baseline with the owning cell's W3
/// convective / frontal contribution.
#[must_use]
pub fn turbulence_at_with_weather<T: HeightSampler>(
    master_seed: u64,
    terrain: &T,
    x_m: f32,
    y_altitude_m: f32,
    z_m: f32,
    time_s: f32,
) -> f32 {
    let base =
        super::turbulence::turbulence_at(master_seed, terrain, x_m, y_altitude_m, z_m, time_s);
    let lookup = voronoi::cell_at(master_seed, x_m, z_m);
    let state =
        cell_state::state_at_time_with_terrain(master_seed, terrain, lookup.cell_id, time_s);
    let delta = turbulence_delta(
        state.cell_type,
        y_altitude_m,
        state.base_altitude_m,
        state.top_altitude_m,
        state.density,
    );
    (base + delta).min(super::turbulence::MAX_TURBULENCE_INTENSITY)
}

/// Altitude profile within a cell's `[base, top]` band. Returns a scalar
/// in `[0, 1]` that scales the cell's peak density to produce the local
/// density at altitude `y`.
fn altitude_profile(
    cell_type: cell_state::CellType,
    y_altitude_m: f32,
    base_altitude_m: f32,
    top_altitude_m: f32,
) -> f32 {
    if y_altitude_m < base_altitude_m || y_altitude_m > top_altitude_m {
        return 0.0;
    }
    let depth = top_altitude_m - base_altitude_m;
    if depth < 1.0 {
        return 0.0;
    }
    let t = (y_altitude_m - base_altitude_m) / depth;
    match cell_type {
        cell_state::CellType::ClearAir => 0.0,
        // Flat horizontal layer — density uniform across the thin vertical span.
        cell_state::CellType::Stratus => 1.0,
        // Convective / frontal: parabolic peak at mid-depth.
        _ => {
            let centered = 1.0 - (2.0 * t - 1.0) * (2.0 * t - 1.0);
            centered.max(0.0)
        },
    }
}

/// Per-cell-type wind delta — nonzero for convective cells with a vertical
/// component (updraft inside the cloud column).
fn wind_delta(
    cell_type: cell_state::CellType,
    y_altitude_m: f32,
    base_altitude_m: f32,
    top_altitude_m: f32,
    density_scale: f32,
) -> Vec3 {
    let profile = altitude_profile(cell_type, y_altitude_m, base_altitude_m, top_altitude_m);
    if profile <= 0.0 {
        return Vec3::ZERO;
    }
    let updraft_peak = match cell_type {
        cell_state::CellType::Cumulus => UPDRAFT_CUMULUS_MAX_MPS,
        cell_state::CellType::Nimbus => UPDRAFT_NIMBUS_MAX_MPS,
        cell_state::CellType::Squall => UPDRAFT_SQUALL_MAX_MPS,
        cell_state::CellType::Frontal => UPDRAFT_CUMULUS_MAX_MPS * 0.5,
        cell_state::CellType::Stratus | cell_state::CellType::ClearAir => 0.0,
    };
    // Density scales the updraft — a dissipating cell produces less lift.
    Vec3::new(0.0, updraft_peak * profile * density_scale, 0.0)
}

/// Per-cell-type turbulence delta — scales with the cell's current density
/// (via profile × density) so dissipating cells produce less buffet.
fn turbulence_delta(
    cell_type: cell_state::CellType,
    y_altitude_m: f32,
    base_altitude_m: f32,
    top_altitude_m: f32,
    density_scale: f32,
) -> f32 {
    let profile = altitude_profile(cell_type, y_altitude_m, base_altitude_m, top_altitude_m);
    if profile <= 0.0 {
        return 0.0;
    }
    let peak = match cell_type {
        cell_state::CellType::Nimbus => TURBULENCE_DELTA_NIMBUS,
        cell_state::CellType::Squall => TURBULENCE_DELTA_SQUALL,
        cell_state::CellType::Frontal => TURBULENCE_DELTA_FRONTAL,
        cell_state::CellType::Cumulus => TURBULENCE_DELTA_NIMBUS * 0.2,
        cell_state::CellType::Stratus | cell_state::CellType::ClearAir => 0.0,
    };
    peak * profile * density_scale
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    a + (b - a) * t
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::terrain::Terrain;

    #[test]
    fn clear_air_cells_produce_zero_deltas() {
        // Find a coordinate where the owning cell is ClearAir.
        let terrain = Terrain::from_seed(42);
        for ix in 0..30 {
            for iz in 0..30 {
                let x = (ix as f32 - 15.0) * 20_000.0;
                let z = (iz as f32 - 15.0) * 20_000.0;
                let lookup = voronoi::cell_at(42, x, z);
                let state =
                    cell_state::state_at_time_with_terrain(42, &terrain, lookup.cell_id, 0.0);
                if matches!(state.cell_type, cell_state::CellType::ClearAir) {
                    let density = cloud_density_at(42, &terrain, x, 1_500.0, z, 0.0);
                    let precip = precipitation_rate_at(42, &terrain, x, 100.0, z, 0.0);
                    let wind_w = wind_at_with_weather(42, &terrain, x, 2_000.0, z, 0.0);
                    let wind_base = wind::wind_at(42, x, 2_000.0, z, 0.0);
                    assert_eq!(density.to_bits(), 0.0_f32.to_bits());
                    assert_eq!(precip.to_bits(), 0.0_f32.to_bits());
                    // Wind composed == wind base (zero delta).
                    assert_eq!(wind_w.x.to_bits(), wind_base.x.to_bits());
                    assert_eq!(wind_w.y.to_bits(), wind_base.y.to_bits());
                    assert_eq!(wind_w.z.to_bits(), wind_base.z.to_bits());
                    return;
                }
            }
        }
        panic!("no ClearAir cell found in sample grid");
    }

    #[test]
    fn cloud_density_in_unit_range() {
        let terrain = Terrain::from_seed(42);
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 25_000.0;
                let z = (iz as f32 - 10.0) * 25_000.0;
                for alt in [0.0, 500.0, 2_000.0, 5_000.0, 10_000.0] {
                    let d = cloud_density_at(42, &terrain, x, alt, z, 0.0);
                    assert!(
                        (0.0..=1.0).contains(&d),
                        "density {d} at ({x}, {alt}, {z}) out of range"
                    );
                }
            }
        }
    }

    #[test]
    fn cloud_density_peaks_above_base_zero_outside() {
        // Find a non-clear cell, verify profile.
        let terrain = Terrain::from_seed(42);
        for ix in 0..30 {
            for iz in 0..30 {
                let x = (ix as f32 - 15.0) * 20_000.0;
                let z = (iz as f32 - 15.0) * 20_000.0;
                let lookup = voronoi::cell_at(42, x, z);
                let state =
                    cell_state::state_at_time_with_terrain(42, &terrain, lookup.cell_id, 0.0);
                if !matches!(state.cell_type, cell_state::CellType::ClearAir)
                    && !matches!(state.cell_type, cell_state::CellType::Stratus)
                    && state.density > 0.1
                {
                    let below =
                        cloud_density_at(42, &terrain, x, state.base_altitude_m - 100.0, z, 0.0);
                    let mid = f32::midpoint(state.base_altitude_m, state.top_altitude_m);
                    let inside = cloud_density_at(42, &terrain, x, mid, z, 0.0);
                    let above =
                        cloud_density_at(42, &terrain, x, state.top_altitude_m + 100.0, z, 0.0);
                    assert_eq!(below.to_bits(), 0.0_f32.to_bits());
                    assert_eq!(above.to_bits(), 0.0_f32.to_bits());
                    assert!(
                        inside > 0.0,
                        "cloud density at mid-layer should be positive"
                    );
                    return;
                }
            }
        }
        // Acceptable if no convective cell exists in the sample — the test
        // doesn't panic, because clear-plus-stratus grids are valid.
    }

    #[test]
    fn visibility_bounded() {
        let terrain = Terrain::from_seed(42);
        for ix in 0..15 {
            for iz in 0..15 {
                let x = (ix as f32 - 7.0) * 25_000.0;
                let z = (iz as f32 - 7.0) * 25_000.0;
                for alt in [0.0, 500.0, 2_000.0, 5_000.0] {
                    let v = visibility_at(42, &terrain, x, alt, z, 0.0);
                    let lo = CLOUD_INTERIOR_VISIBILITY_M - 1.0;
                    let hi = BASELINE_VISIBILITY_M + 1.0;
                    assert!(
                        (lo..=hi).contains(&v),
                        "visibility {v} at ({x}, {alt}, {z}) out of range"
                    );
                }
            }
        }
    }

    #[test]
    fn deltas_deterministic() {
        let terrain = Terrain::from_seed(42);
        let a = cloud_density_at(42, &terrain, 1_000.0, 2_000.0, -500.0, 100.0);
        let b = cloud_density_at(42, &terrain, 1_000.0, 2_000.0, -500.0, 100.0);
        assert_eq!(a.to_bits(), b.to_bits());

        let c = wind_at_with_weather(42, &terrain, 1_000.0, 2_000.0, -500.0, 100.0);
        let d = wind_at_with_weather(42, &terrain, 1_000.0, 2_000.0, -500.0, 100.0);
        assert_eq!(c.y.to_bits(), d.y.to_bits());
    }

    #[test]
    fn nimbus_produces_updraft() {
        // Find a nimbus cell, verify there's an updraft at mid-altitude.
        let terrain = Terrain::from_seed(42);
        for ix in 0..40 {
            for iz in 0..40 {
                let x = (ix as f32 - 20.0) * 15_000.0;
                let z = (iz as f32 - 20.0) * 15_000.0;
                let lookup = voronoi::cell_at(42, x, z);
                let state =
                    cell_state::state_at_time_with_terrain(42, &terrain, lookup.cell_id, 0.0);
                if matches!(state.cell_type, cell_state::CellType::Nimbus) && state.density > 0.1 {
                    let mid = f32::midpoint(state.base_altitude_m, state.top_altitude_m);
                    let w = wind_at_with_weather(42, &terrain, x, mid, z, 0.0);
                    assert!(
                        w.y > 0.1,
                        "nimbus mid-altitude should have updraft; got w.y = {}",
                        w.y
                    );
                    return;
                }
            }
        }
        // Not a failure if no nimbus exists in the sample — seed-dependent.
    }

    #[test]
    fn turbulence_composed_exceeds_baseline_in_active_cells() {
        let terrain = Terrain::from_seed(42);
        for ix in 0..40 {
            for iz in 0..40 {
                let x = (ix as f32 - 20.0) * 15_000.0;
                let z = (iz as f32 - 20.0) * 15_000.0;
                let lookup = voronoi::cell_at(42, x, z);
                let state =
                    cell_state::state_at_time_with_terrain(42, &terrain, lookup.cell_id, 0.0);
                if matches!(
                    state.cell_type,
                    cell_state::CellType::Nimbus | cell_state::CellType::Squall
                ) && state.density > 0.1
                {
                    let mid = f32::midpoint(state.base_altitude_m, state.top_altitude_m);
                    let base =
                        super::super::turbulence::turbulence_at(42, &terrain, x, mid, z, 0.0);
                    let composed = turbulence_at_with_weather(42, &terrain, x, mid, z, 0.0);
                    assert!(
                        composed >= base,
                        "composed turbulence {composed} < baseline {base}"
                    );
                    assert!(composed > base, "active cell should boost turbulence");
                    return;
                }
            }
        }
        // Seed-dependent — not a hard failure.
    }

    #[test]
    fn precipitation_zero_above_cloud_base() {
        let terrain = Terrain::from_seed(42);
        // Find a cell with precip; verify above-base returns 0.
        for ix in 0..40 {
            for iz in 0..40 {
                let x = (ix as f32 - 20.0) * 15_000.0;
                let z = (iz as f32 - 20.0) * 15_000.0;
                let lookup = voronoi::cell_at(42, x, z);
                let state =
                    cell_state::state_at_time_with_terrain(42, &terrain, lookup.cell_id, 0.0);
                if state.precip_rate_mm_per_hr > 0.1 {
                    let below = precipitation_rate_at(
                        42,
                        &terrain,
                        x,
                        state.base_altitude_m - 10.0,
                        z,
                        0.0,
                    );
                    let above = precipitation_rate_at(
                        42,
                        &terrain,
                        x,
                        state.base_altitude_m + 10.0,
                        z,
                        0.0,
                    );
                    assert!(below > 0.0);
                    assert_eq!(above.to_bits(), 0.0_f32.to_bits());
                    return;
                }
            }
        }
    }

    #[test]
    fn altitude_profile_zero_outside_band() {
        let p_below = altitude_profile(cell_state::CellType::Cumulus, 500.0, 1_000.0, 2_000.0);
        let p_above = altitude_profile(cell_state::CellType::Cumulus, 2_500.0, 1_000.0, 2_000.0);
        let p_inside = altitude_profile(cell_state::CellType::Cumulus, 1_500.0, 1_000.0, 2_000.0);
        assert_eq!(p_below.to_bits(), 0.0_f32.to_bits());
        assert_eq!(p_above.to_bits(), 0.0_f32.to_bits());
        assert!(p_inside > 0.0);
    }

    #[test]
    fn altitude_profile_stratus_flat() {
        let p_mid = altitude_profile(cell_state::CellType::Stratus, 500.0, 300.0, 1_000.0);
        let p_base = altitude_profile(cell_state::CellType::Stratus, 300.0, 300.0, 1_000.0);
        let p_top = altitude_profile(cell_state::CellType::Stratus, 1_000.0, 300.0, 1_000.0);
        assert_eq!(p_mid.to_bits(), 1.0_f32.to_bits());
        assert_eq!(p_base.to_bits(), 1.0_f32.to_bits());
        assert_eq!(p_top.to_bits(), 1.0_f32.to_bits());
    }
}
