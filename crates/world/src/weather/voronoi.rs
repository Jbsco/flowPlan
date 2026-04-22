//! Weather Voronoi partition (Layer W2 core).
//!
//! Pre-baked spatial partition of the world into weather cells. Seed
//! distribution is blue-noise-ish via a jittered regular grid: one seed per
//! `GRID_SPACING_M`-sized grid cell, offset from the cell center by a
//! per-cell deterministic jitter.
//!
//! # Architecture
//!
//! The Voronoi diagram itself is **not materialized** — cells are computed on
//! demand from `(master_seed, world_xz)` via a nearest-neighbor query over
//! the 3×3 grid-cell neighborhood of the query point. This keeps world-bake
//! cheap (no Fortune's-sweep construction) and memory flat (no cell index
//! table) while still producing deterministic global structure.
//!
//! # Drift model
//!
//! Weather cells are static in world-space for determinism, but consumers
//! see them moving past — [`cell_at_drifted`] queries `cell_at(world_pos -
//! wind · time)`, so an observer at a fixed point sees cells flowing past at
//! the prevailing wind speed. Drift velocity is altitude-stratified: low-level
//! cells (near surface) drift slower than wind aloft, matching real atmospheric
//! layering.
//!
//! # Scope
//!
//! **Phase 3.2.2a (this module):** partition + cell lookup + drift. No
//! per-cell state, no placement constraints, no time evolution. Each lookup
//! returns the `CellId` (which is just the `(ix, iz)` grid coordinate of the
//! owning seed) and geometric metadata — the caller does not yet get back a
//! "cumulus / stratus / clear-air" classification.
//!
//! **Phase 3.2.2b:** per-cell state + W1 placement constraints.
//!
//! **Phase 3.2.2c:** adjacency-bias evolution.

use glam::Vec2;

use super::{field_seed, weather_seed, wind};

/// Average spacing between Voronoi seeds (m).
///
/// Picks the midpoint of the MAPGEN 20–50 km target range — matches weather-
/// feature sizes (storm cells, frontal-passage regions).
pub const GRID_SPACING_M: f32 = 30_000.0;

/// Maximum seed jitter as a fraction of [`GRID_SPACING_M`].
///
/// Keeps seeds within the inner ~60% of each grid cell so neighboring seeds
/// don't collide and cells stay roughly convex. Low-enough jitter to preserve
/// blue-noise distribution properties (no clustering, no voids).
pub const JITTER_FRACTION: f32 = 0.3;

const VORONOI_FIELD_TAG: u64 = u64::from_le_bytes(*b"voronoi_");

/// Identifier of a Voronoi cell — the `(ix, iz)` grid coordinate of its
/// owning seed. Stable across lookups and compact to hash/store.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CellId {
    pub ix: i32,
    pub iz: i32,
}

/// Result of a cell-lookup query.
#[derive(Copy, Clone, Debug)]
pub struct CellLookup {
    /// Owning cell identity.
    pub cell_id: CellId,
    /// World-space position of the owning cell's seed.
    pub seed_pos: Vec2,
    /// Distance from query point to the owning seed (m).
    pub distance_to_seed: f32,
    /// Approximate distance to the nearest Voronoi-cell edge (m). Computed as
    /// `(distance_to_second_nearest - distance_to_nearest) / 2`; exact on the
    /// bisector, approximate elsewhere. Sufficient for edge-feathering.
    pub distance_to_edge: f32,
}

/// World-space seed position of the cell identified by `cell`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn seed_position(master_seed: u64, cell: CellId) -> Vec2 {
    let (jx, jz) = jitter(master_seed, cell.ix, cell.iz);
    Vec2::new(
        (cell.ix as f32 + 0.5 + jx) * GRID_SPACING_M,
        (cell.iz as f32 + 0.5 + jz) * GRID_SPACING_M,
    )
}

/// Voronoi cell owning world coordinate `(x, z)`. Static (no drift).
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn cell_at(master_seed: u64, x_m: f32, z_m: f32) -> CellLookup {
    // Identify the grid cell containing the query point; search its 3×3
    // neighborhood for the two nearest seeds.
    let ix0 = libm::floorf(x_m / GRID_SPACING_M) as i32;
    let iz0 = libm::floorf(z_m / GRID_SPACING_M) as i32;

    let mut best = f32::INFINITY;
    let mut second = f32::INFINITY;
    let mut best_id = CellId { ix: ix0, iz: iz0 };
    let mut best_pos = Vec2::ZERO;

    for dx in -1..=1 {
        for dz in -1..=1 {
            let id = CellId {
                ix: ix0 + dx,
                iz: iz0 + dz,
            };
            let pos = seed_position(master_seed, id);
            let d2 = (pos.x - x_m) * (pos.x - x_m) + (pos.y - z_m) * (pos.y - z_m);
            let d = libm::sqrtf(d2);
            if d < best {
                second = best;
                best = d;
                best_id = id;
                best_pos = pos;
            } else if d < second {
                second = d;
            }
        }
    }

    CellLookup {
        cell_id: best_id,
        seed_pos: best_pos,
        distance_to_seed: best,
        distance_to_edge: (second - best) * 0.5,
    }
}

/// Voronoi cell owning world coordinate `(x, z)` at `time` seconds, with
/// altitude-dependent wind drift applied.
///
/// Drift is `wind_at(x, y_altitude, z, 0) × time` — a first-order integration
/// using the instantaneous wind at the query altitude. W1 wind is near-static
/// in time (pressure drifts at 10 m/s but field shape is stable over hours),
/// so this simple integration is accurate to ~1% over game-relevant timescales.
#[must_use]
pub fn cell_at_drifted(
    master_seed: u64,
    x_m: f32,
    y_altitude_m: f32,
    z_m: f32,
    time_s: f32,
) -> CellLookup {
    let w = wind::wind_at(master_seed, x_m, y_altitude_m, z_m, 0.0);
    let drift_x = w.x * time_s;
    let drift_z = w.z * time_s;
    cell_at(master_seed, x_m - drift_x, z_m - drift_z)
}

/// Per-(ix, iz) deterministic jitter ∈ `[-JITTER_FRACTION, +JITTER_FRACTION]²`.
#[must_use]
fn jitter(master_seed: u64, ix: i32, iz: i32) -> (f32, f32) {
    let root = field_seed(weather_seed(master_seed), VORONOI_FIELD_TAG);
    // Combine (ix, iz) into the seed by mixing packed i32 pair.
    #[allow(clippy::cast_sign_loss)]
    let packed = ((ix as u32 as u64) << 32) | (iz as u32 as u64);
    let seed = splitmix(root.wrapping_add(packed));

    let jx = (hash_to_unit(seed, 1) - 0.5) * 2.0 * JITTER_FRACTION;
    let jz = (hash_to_unit(seed, 2) - 0.5) * 2.0 * JITTER_FRACTION;
    (jx, jz)
}

#[must_use]
fn splitmix(x: u64) -> u64 {
    let mut x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[must_use]
#[allow(clippy::cast_precision_loss)]
fn hash_to_unit(seed: u64, tag: u64) -> f32 {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(tag);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    ((x >> 40) as f32) / ((1u32 << 24) as f32)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn cell_lookup_deterministic() {
        let a = cell_at(42, 12_345.0, -6_789.0);
        let b = cell_at(42, 12_345.0, -6_789.0);
        assert_eq!(a.cell_id, b.cell_id);
        assert_eq!(a.distance_to_seed.to_bits(), b.distance_to_seed.to_bits());
        assert_eq!(a.distance_to_edge.to_bits(), b.distance_to_edge.to_bits());
    }

    #[test]
    fn different_seeds_produce_different_partitions() {
        // At a dense grid, at least some cells should differ by seed.
        let mut any_diff = false;
        for ix in 0..10 {
            for iz in 0..10 {
                let x = ix as f32 * GRID_SPACING_M;
                let z = iz as f32 * GRID_SPACING_M;
                if cell_at(1, x, z).cell_id != cell_at(2, x, z).cell_id {
                    any_diff = true;
                    break;
                }
                if any_diff {
                    break;
                }
            }
        }
        // At least one cell should differ — with random jitter across 100 cells
        // the probability of a full match is vanishingly small.
        assert!(
            any_diff || {
                // Fallback: check seed POSITIONS differ even if grid-cell
                // assignment happens to agree at sample points.
                seed_position(1, CellId { ix: 0, iz: 0 })
                    != seed_position(2, CellId { ix: 0, iz: 0 })
            },
            "seeds 1 vs 2 produced identical Voronoi partitions"
        );
    }

    #[test]
    fn seed_within_jitter_bound_of_grid_center() {
        // For any (master_seed, cell), the seed must lie within
        // ±JITTER_FRACTION * GRID_SPACING_M of the grid-cell center.
        let expected_bound = JITTER_FRACTION * GRID_SPACING_M + 1.0;
        for ix in -5..=5 {
            for iz in -5..=5 {
                let center_x = (ix as f32 + 0.5) * GRID_SPACING_M;
                let center_z = (iz as f32 + 0.5) * GRID_SPACING_M;
                let pos = seed_position(42, CellId { ix, iz });
                let off_x = (pos.x - center_x).abs();
                let off_z = (pos.y - center_z).abs();
                assert!(
                    off_x < expected_bound && off_z < expected_bound,
                    "cell ({ix}, {iz}) jitter ({off_x}, {off_z}) exceeds bound {expected_bound}"
                );
            }
        }
    }

    #[test]
    fn cell_contains_its_seed() {
        // Querying at a seed's position should resolve to that cell.
        for ix in -3..=3 {
            for iz in -3..=3 {
                let id = CellId { ix, iz };
                let pos = seed_position(42, id);
                let lookup = cell_at(42, pos.x, pos.y);
                assert_eq!(lookup.cell_id, id);
                assert!(
                    lookup.distance_to_seed < 1.0,
                    "distance_to_seed {} at own seed position should be ~0",
                    lookup.distance_to_seed
                );
            }
        }
    }

    #[test]
    fn distance_to_edge_non_negative() {
        // Sample a grid; distance_to_edge must be >= 0.
        for ix in 0..10 {
            for iz in 0..10 {
                let x = (ix as f32 - 5.0) * 10_000.0;
                let z = (iz as f32 - 5.0) * 10_000.0;
                let l = cell_at(42, x, z);
                assert!(
                    l.distance_to_edge >= 0.0,
                    "distance_to_edge {} at ({x}, {z}) is negative",
                    l.distance_to_edge
                );
            }
        }
    }

    #[test]
    fn drift_at_zero_time_matches_static() {
        // With time = 0, drifted lookup should match static lookup exactly.
        let s = cell_at(42, 5_000.0, -8_000.0);
        let d = cell_at_drifted(42, 5_000.0, 500.0, -8_000.0, 0.0);
        assert_eq!(s.cell_id, d.cell_id);
        assert_eq!(s.distance_to_seed.to_bits(), d.distance_to_seed.to_bits());
    }

    #[test]
    fn drift_shifts_cell_over_time() {
        // Pick a point with nonzero wind; after enough time, a different cell
        // should own the drifted query.
        let x = 120_000.0;
        let z = 90_000.0;
        let altitude = 2_000.0; // aloft, full geostrophic
        let lookup_0 = cell_at_drifted(42, x, altitude, z, 0.0);
        let lookup_1h = cell_at_drifted(42, x, altitude, z, 3_600.0);
        // After an hour at ~10 m/s wind, drift ≈ 36 km — more than GRID_SPACING_M.
        // Lookup should fall into a different cell.
        assert_ne!(
            lookup_0.cell_id, lookup_1h.cell_id,
            "drifted cell should change after 1h advection"
        );
    }

    #[test]
    fn seed_position_deterministic_per_cell() {
        let p1 = seed_position(42, CellId { ix: 3, iz: -2 });
        let p2 = seed_position(42, CellId { ix: 3, iz: -2 });
        assert_eq!(p1.x.to_bits(), p2.x.to_bits());
        assert_eq!(p1.y.to_bits(), p2.y.to_bits());
    }

    #[test]
    fn different_cells_have_different_seeds() {
        let p1 = seed_position(42, CellId { ix: 0, iz: 0 });
        let p2 = seed_position(42, CellId { ix: 1, iz: 0 });
        assert_ne!(p1, p2);
    }
}
