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
/// Originally 30 km (midpoint of MAPGEN's 20–50 km target range). Adjusted
/// to 25 km (Phase 3.6.0) so `WORLD_EXTENT_M / GRID_SPACING_M = 200` is
/// exactly integer — required for mod-N toroidal Voronoi wrap at the seam.
/// 17% reduction stays well within the 20–50 km feature-size target range;
/// cell count world-wide is 200 × 200 = 40,000 (matches MAPGEN's ~11K
/// scaled up by the lower spacing).
pub const GRID_SPACING_M: f32 = 25_000.0;

/// Number of Voronoi grid cells per world side. `WORLD_EXTENT_M / GRID_SPACING_M`.
/// Must be integer for mod-N wrap to produce clean toroidal identity.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss)]
pub const GRID_CELL_COUNT: i32 = (flowplan_core::WORLD_EXTENT_M / GRID_SPACING_M) as i32;

/// Weight-field spacing (m) for variable cell size (Phase 3.6.0+).
///
/// Per `MAPGEN.md § Variable Voronoi cell size`, each cell gets a
/// per-seed weight sampled from a coarse Perlin field at this scale.
/// Cells with higher weights win more territory in the power-diagram
/// metric; cells with lower weights shrink or get absorbed. Weight-field
/// at 500 km means "this 500 km region favors large cells; that region
/// favors small cells" — produces natural mix of compact convective
/// cells and sprawling frontal systems at ~20× the base 25 km cell size.
const WEIGHT_FIELD_SPACING_M: f32 = 500_000.0;

/// Amplitude of per-cell weight (m²). Power-diagram metric subtracts
/// weight from squared distance; scale so max weight ≈ (GRID_SPACING × 0.8)²
/// gives ~2-3× size variation without creating degenerate-tiny cells.
const WEIGHT_AMPLITUDE_M2: f32 = 4.0e8; // ≈ (20 km)²

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

/// Per-cell weight for the power-diagram metric (Phase 3.6.0+). Sampled
/// from a coarse Perlin field at the seed position so weights vary
/// smoothly across the world — regional areas favor larger cells or
/// smaller cells rather than checker-board alternation.
fn cell_weight(master_seed: u64, seed_pos: Vec2) -> f32 {
    let weight_seed = super::field_seed(
        super::weather_seed(master_seed),
        u64::from_le_bytes(*b"vcell_wt"),
    );
    // Perlin in [−0.707, 0.707]; shift to [0, 1] so all weights are
    // non-negative (power-diagram metric requires weight ≥ 0).
    let raw = crate::noise::perlin_2d(weight_seed, 0, seed_pos.x, seed_pos.y, WEIGHT_FIELD_SPACING_M);
    let normalized = (raw + 0.707) / 1.414; // → [0, 1]
    normalized.clamp(0.0, 1.0) * WEIGHT_AMPLITUDE_M2
}

/// Voronoi cell owning world coordinate `(x, z)`. Static (no drift).
///
/// Uses an **additively-weighted power-diagram metric**: distance to a
/// seed is `||p − seed||² − weight(seed)` where `weight` is a per-cell
/// Perlin-sampled value in `[0, WEIGHT_AMPLITUDE_M2]`. Cells with higher
/// weights win more territory — produces organic cell-size variation at
/// ~500 km regional scale (compact cells here, sprawling cells there)
/// rather than uniform grid-aligned structure.
///
/// Toroidal-wrap-aware (Phase 3.6.0): cell IDs wrap via mod-N so queries
/// near the seam correctly identify the same cell whether approached from
/// `x < 0` or `x > WORLD_EXTENT_M`.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn cell_at(master_seed: u64, x_m: f32, z_m: f32) -> CellLookup {
    // Identify the grid cell containing the query point; search a 5×5
    // neighborhood so weighted cells (which can "reach" further than a
    // uniform cell) are still captured. 3×3 was sufficient for the
    // uniform-metric case; weighted needs the extra margin because a
    // high-weight cell can dominate its neighbors.
    let ix0 = libm::floorf(x_m / GRID_SPACING_M) as i32;
    let iz0 = libm::floorf(z_m / GRID_SPACING_M) as i32;

    let mut best = f32::INFINITY; // in weighted-distance² (can be negative)
    let mut second = f32::INFINITY;
    let mut best_id = CellId { ix: 0, iz: 0 };
    let mut best_pos = Vec2::ZERO;
    let mut best_d_euclid = 0.0_f32; // for reporting distance_to_seed

    let n = i64::from(GRID_CELL_COUNT);
    let half_world = flowplan_core::WORLD_EXTENT_M * 0.5;

    for dx in -2..=2_i32 {
        for dz in -2..=2_i32 {
            let ix_raw = ix0 + dx;
            let iz_raw = iz0 + dz;
            let id = CellId {
                ix: flowplan_core::wrap_cell_index(i64::from(ix_raw), n) as i32,
                iz: flowplan_core::wrap_cell_index(i64::from(iz_raw), n) as i32,
            };
            let mut pos = seed_position(master_seed, id);
            if pos.x - x_m > half_world {
                pos.x -= flowplan_core::WORLD_EXTENT_M;
            } else if pos.x - x_m < -half_world {
                pos.x += flowplan_core::WORLD_EXTENT_M;
            }
            if pos.y - z_m > half_world {
                pos.y -= flowplan_core::WORLD_EXTENT_M;
            } else if pos.y - z_m < -half_world {
                pos.y += flowplan_core::WORLD_EXTENT_M;
            }
            let d2 = (pos.x - x_m) * (pos.x - x_m) + (pos.y - z_m) * (pos.y - z_m);
            let weight = cell_weight(master_seed, pos);
            let d2_weighted = d2 - weight;
            if d2_weighted < best {
                second = best;
                best = d2_weighted;
                best_id = id;
                best_pos = pos;
                best_d_euclid = libm::sqrtf(d2.max(0.0));
            } else if d2_weighted < second {
                second = d2_weighted;
            }
        }
    }

    // distance_to_edge in weighted-distance-space is (second - best) / 2;
    // scale roughly back to meters via sqrt. This is approximate for edge
    // feathering purposes — exact geometric edge distance under power
    // diagrams requires more math but isn't needed here.
    let weighted_edge_gap = (second - best).max(0.0);
    let distance_to_edge = libm::sqrtf(weighted_edge_gap) * 0.5;

    CellLookup {
        cell_id: best_id,
        seed_pos: best_pos,
        distance_to_seed: best_d_euclid,
        distance_to_edge,
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
        // Post Phase 3.6.0: cell IDs are canonical in [0, GRID_CELL_COUNT);
        // test iterates through canonical range rather than negative values.
        let expected_bound = JITTER_FRACTION * GRID_SPACING_M + 1.0;
        for ix in 0..11 {
            for iz in 0..11 {
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
        // Post Phase 3.6.0: use canonical [0, N) indices.
        for ix in 0..7 {
            for iz in 0..7 {
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
    fn cell_at_wraps_across_seam() {
        // Post Phase 3.6.0: query near x=0 vs x=WORLD_EXTENT_M should
        // resolve to the SAME canonical cell ID. Verifies toroidal wrap
        // in the Voronoi partition.
        use flowplan_core::WORLD_EXTENT_M;
        for &(x, z) in &[(0.5, 0.5), (123.4, 567.8), (10_000.0, -5_000.0)] {
            let near_origin = cell_at(42, x, z);
            let wrapped_east = cell_at(42, x + WORLD_EXTENT_M, z);
            let wrapped_north = cell_at(42, x, z + WORLD_EXTENT_M);
            assert_eq!(near_origin.cell_id, wrapped_east.cell_id, "east seam at {x},{z}");
            assert_eq!(near_origin.cell_id, wrapped_north.cell_id, "north seam at {x},{z}");
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
        // should own the drifted query. Sample multiple points — after the
        // 3.6.1a fBm refactor the wind field shifted (depends on pressure
        // gradient which is now fBm, not rotated bands), so a fixed-point
        // test could hit a locally-calm spot. Scan for at least ONE point
        // where 6h drift shifts cell — confirms drift mechanism works.
        let altitude = 2_000.0;
        let mut any_drift = false;
        for ix in 0..10 {
            for iz in 0..10 {
                let x = 120_000.0 + ix as f32 * 30_000.0;
                let z = 90_000.0 + iz as f32 * 30_000.0;
                let lookup_0 = cell_at_drifted(42, x, altitude, z, 0.0);
                let lookup_6h = cell_at_drifted(42, x, altitude, z, 21_600.0);
                if lookup_0.cell_id != lookup_6h.cell_id {
                    any_drift = true;
                    break;
                }
            }
            if any_drift {
                break;
            }
        }
        assert!(any_drift, "no cell in sample grid drifted after 6h advection");
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
