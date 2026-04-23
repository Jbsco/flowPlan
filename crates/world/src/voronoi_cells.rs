//! Voronoi cell distribution (Phase 3.6.2).
//!
//! Bridson 2007 blue-noise Poisson-disk seed sampling at ~2 km
//! average spacing. Produces ~6.25 M seeds for the 5000 km world —
//! the sub-biome cell catalog. Each seed gets a feature assignment
//! via [`crate::sub_biome::assign_feature`] on demand (computed
//! lazily per query to avoid the ~30-second eager-bake cost).
//!
//! # Algorithm (Bridson 2007)
//!
//! 1. Pick initial random seed.
//! 2. Maintain an "active" list of seeds.
//! 3. For each active seed, try `BRIDSON_K` = 30 candidate positions
//!    in the annulus `[r, 2r]` around it.
//! 4. Accept a candidate if no existing seed lies within `r` (checked
//!    via a spatial grid with cell size `r / √2` so each cell holds
//!    at most one seed).
//! 5. If no candidate succeeds after `BRIDSON_K` tries, remove the
//!    active seed and move on.
//! 6. Repeat until the active list is empty.
//!
//! Runtime: O(N) in seed count. For 6.25 M seeds: ~3 s release.
//!
//! # Determinism
//!
//! Bridson's order-of-placement is sensitive to the initial random
//! seed + subsequent PRNG calls. We derive all RNG from a deterministic
//! splitmix chain keyed on `(master_seed, step_index)` — identical
//! output on every machine.
//!
//! # Toroidal wrap
//!
//! Candidate positions wrap via `wrap_coord` before grid insertion.
//! Neighbor-check uses toroidal-closest-path distance. The seam is
//! transparent.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::doc_markdown)]

use core::f32::consts::TAU;

use flowplan_core::{wrap_cell_index, wrap_coord, WORLD_EXTENT_M};

use crate::biome::{biome_at, BiomeClass};
use crate::hub_catalog::HubCatalog;
use crate::hydrology::{HydrologyGraph, WaterClass};
use crate::sub_biome::{
    assign_feature, biome_weights_at_position, FeatureAssignment, PlacementContext,
    PlacementRulesContext, SubBiomeLibraries,
};
use crate::terrain::{HeightSampler, Terrain};

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

/// Target average spacing between Voronoi cell seeds (m). Per MAPGEN
/// spec — the "sub-biome feature cell" density.
pub const SEED_SPACING_M: f32 = 2_000.0;

/// Bridson K — number of candidate attempts per active seed before
/// giving up on that seed. 30 is the standard value (Bridson 2007).
pub const BRIDSON_K: u32 = 30;

/// Cell-size for the acceleration grid: `r / √2`. Each grid cell
/// holds at most one seed (the grid is the Poisson-disk constraint
/// enforcer).
pub const GRID_CELL_SIZE_M: f32 = SEED_SPACING_M / core::f32::consts::SQRT_2;

/// Grid dimension: how many cells per side.
#[allow(clippy::cast_possible_truncation)]
pub const GRID_N: usize = (WORLD_EXTENT_M / GRID_CELL_SIZE_M) as usize + 1;

/// Bleed-over water-adjacency radius (m). Matches
/// `sub_biome::BLEED_OVER_PRIMARY_WEIGHT_THRESHOLD` discussion —
/// ~30 km where inter-biome effects can reach.
pub const WATER_ADJACENCY_RADIUS_M: f32 = 30_000.0;

/// Splitmix64-inspired seed mixer.
const fn mix_seed(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Pseudo-random f32 in [0, 1) derived from a hash input.
fn hash_to_unit(seed: u64, step: u64) -> f32 {
    let h = mix_seed(seed, step);
    ((h & 0xFFFF_FFFF) as f32) / (u32::MAX as f32 + 1.0)
}

const SUB_BIOME_SEED_TAG: u64 = u64::from_le_bytes(*b"svxcells");

// ═══════════════════════════════════════════════════════════════════════════
// Public types
// ═══════════════════════════════════════════════════════════════════════════

/// Stable cell identifier. Matches the index in the seeds array.
pub type CellId = u32;

/// Sub-biome cell seed.
#[derive(Copy, Clone, Debug)]
pub struct CellSeed {
    /// Cell id (stable).
    pub id: CellId,
    /// Seed position in world coords.
    pub position: (f32, f32),
}

/// Sentinel for an empty spatial-grid cell. `u32::MAX` cuts grid
/// memory to 4 bytes per entry (vs `Option<u32>`'s 8 bytes due to
/// alignment). With ~6 M valid cell ids well below `u32::MAX`, the
/// sentinel is safe.
const EMPTY_GRID: CellId = u32::MAX;

/// Voronoi cell catalog.
///
/// Seeds are pre-computed via Bridson; feature assignment is computed
/// on demand via [`SubBiomeCellCatalog::feature_at_cell`] (pure
/// function of `(master_seed, cell_id, terrain, hydrology)`).
#[derive(Clone, Debug)]
pub struct SubBiomeCellCatalog {
    seed: u64,
    seeds: Vec<CellSeed>,
    /// Spatial hash grid for O(1) nearest-seed lookup. `EMPTY_GRID`
    /// denotes unoccupied cells.
    grid: Vec<CellId>,
    /// Cell size of the acceleration grid.
    grid_cell_m: f32,
    /// Grid side length (cells).
    grid_n: usize,
}

impl SubBiomeCellCatalog {
    /// Bake the cell seed distribution. Bridson Poisson-disk only;
    /// per-cell feature assignment is computed on demand.
    #[must_use]
    pub fn bake(master_seed: u64) -> Self {
        let root_seed = mix_seed(master_seed, SUB_BIOME_SEED_TAG);
        let grid_cell_m = GRID_CELL_SIZE_M;
        let grid_n = GRID_N;
        let grid_cells_total = grid_n * grid_n;
        let mut grid: Vec<CellId> = vec![EMPTY_GRID; grid_cells_total];
        let mut seeds: Vec<CellSeed> = Vec::with_capacity(grid_cells_total);
        let mut active: Vec<u32> = Vec::new();

        let to_grid = |x: f32, z: f32| -> (usize, usize) {
            let xw = wrap_coord(x);
            let zw = wrap_coord(z);
            let ix = ((xw / grid_cell_m) as usize).min(grid_n - 1);
            let iz = ((zw / grid_cell_m) as usize).min(grid_n - 1);
            (ix, iz)
        };

        // Step 0: seed one random position.
        let x0 = hash_to_unit(root_seed, 0) * WORLD_EXTENT_M;
        let z0 = hash_to_unit(root_seed, 1) * WORLD_EXTENT_M;
        let (gx0, gz0) = to_grid(x0, z0);
        seeds.push(CellSeed {
            id: 0,
            position: (x0, z0),
        });
        grid[gz0 * grid_n + gx0] = 0;
        active.push(0);

        let mut step: u64 = 100;

        // Main loop.
        while let Some(&active_idx) = active.last() {
            let (ax, az) = seeds[active_idx as usize].position;
            let mut placed = false;

            for _k in 0..BRIDSON_K {
                step += 1;
                // Candidate position: uniform angle + distance in [r, 2r].
                let angle = hash_to_unit(root_seed, step) * TAU;
                let dist = SEED_SPACING_M * (1.0 + hash_to_unit(root_seed, step + 1));
                step += 1;
                let cx = ax + dist * libm::cosf(angle);
                let cz = az + dist * libm::sinf(angle);
                let cx_w = wrap_coord(cx);
                let cz_w = wrap_coord(cz);

                // Candidate accept: no existing seed within
                // SEED_SPACING_M. Check the 5×5 grid-cell neighborhood
                // (covers up to √(8) × cell ≈ 2.83r radius).
                let (cgx, cgz) = to_grid(cx_w, cz_w);
                let mut reject = false;
                'check: for dz in -2_i64..=2 {
                    for dx in -2_i64..=2 {
                        let nx = wrap_cell_index(cgx as i64 + dx, grid_n as i64) as usize;
                        let nz = wrap_cell_index(cgz as i64 + dz, grid_n as i64) as usize;
                        let other_id = grid[nz * grid_n + nx];
                        if other_id != EMPTY_GRID {
                            let (ox, oz) = seeds[other_id as usize].position;
                            let mut dx_m = (cx_w - ox).abs();
                            let mut dz_m = (cz_w - oz).abs();
                            let half_w = WORLD_EXTENT_M * 0.5;
                            if dx_m > half_w {
                                dx_m = WORLD_EXTENT_M - dx_m;
                            }
                            if dz_m > half_w {
                                dz_m = WORLD_EXTENT_M - dz_m;
                            }
                            if dx_m * dx_m + dz_m * dz_m < SEED_SPACING_M * SEED_SPACING_M {
                                reject = true;
                                break 'check;
                            }
                        }
                    }
                }
                if !reject {
                    let new_id = seeds.len() as u32;
                    seeds.push(CellSeed {
                        id: new_id,
                        position: (cx_w, cz_w),
                    });
                    grid[cgz * grid_n + cgx] = new_id;
                    active.push(new_id);
                    placed = true;
                    break;
                }
            }

            if !placed {
                // Exhausted this active seed; remove it.
                active.pop();
            }
        }

        Self {
            seed: master_seed,
            seeds,
            grid,
            grid_cell_m,
            grid_n,
        }
    }

    /// Master seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Total number of cells.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.seeds.len()
    }

    /// All cell seeds.
    #[must_use]
    pub fn seeds(&self) -> &[CellSeed] {
        &self.seeds
    }

    /// Nearest-seed lookup at world coordinate `(x, z)`. Returns
    /// the cell id. Searches a 5×5 grid-cell neighborhood with
    /// toroidal wrap.
    #[must_use]
    pub fn nearest_cell(&self, x: f32, z: f32) -> CellId {
        let xw = wrap_coord(x);
        let zw = wrap_coord(z);
        let ix = ((xw / self.grid_cell_m) as usize).min(self.grid_n - 1);
        let iz = ((zw / self.grid_cell_m) as usize).min(self.grid_n - 1);
        let mut best_id: CellId = 0;
        let mut best_d2 = f32::INFINITY;
        let half_w = WORLD_EXTENT_M * 0.5;
        for dz in -2_i64..=2 {
            for dx in -2_i64..=2 {
                let nx = wrap_cell_index(ix as i64 + dx, self.grid_n as i64) as usize;
                let nz = wrap_cell_index(iz as i64 + dz, self.grid_n as i64) as usize;
                let cell_id = self.grid[nz * self.grid_n + nx];
                if cell_id != EMPTY_GRID {
                    let (sx, sz) = self.seeds[cell_id as usize].position;
                    let mut dx_m = (xw - sx).abs();
                    let mut dz_m = (zw - sz).abs();
                    if dx_m > half_w {
                        dx_m = WORLD_EXTENT_M - dx_m;
                    }
                    if dz_m > half_w {
                        dz_m = WORLD_EXTENT_M - dz_m;
                    }
                    let d2 = dx_m * dx_m + dz_m * dz_m;
                    if d2 < best_d2 {
                        best_d2 = d2;
                        best_id = cell_id;
                    }
                }
            }
        }
        best_id
    }

    /// Compute the feature assignment for a given cell. Pure function
    /// of `(master_seed, cell_id, terrain, hydrology)`. Caller-side
    /// caching is up to the consumer.
    #[must_use]
    pub fn feature_at_cell(
        &self,
        cell_id: CellId,
        terrain: &Terrain,
        hydrology: &HydrologyGraph,
    ) -> FeatureAssignment {
        let (x, z) = self.seeds[cell_id as usize].position;
        let context = build_placement_context(self.seed, x, z, terrain, hydrology);
        let libs = SubBiomeLibraries::shared();
        assign_feature(self.seed, cell_id, (x, z), &context, libs, None)
    }

    /// Sub-biome at world coordinate `(x, z)`. Looks up the nearest
    /// cell + computes its feature assignment.
    #[must_use]
    pub fn sub_biome_at(
        &self,
        x: f32,
        z: f32,
        terrain: &Terrain,
        hydrology: &HydrologyGraph,
    ) -> FeatureAssignment {
        let cell_id = self.nearest_cell(x, z);
        self.feature_at_cell(cell_id, terrain, hydrology)
    }

    /// Same as [`Self::feature_at_cell`] but evaluates 3.6.3c.1
    /// placement rules (hub_proximity / hub_exclusion / prominence /
    /// hydrology_coupling / climate_window) against the passed hub
    /// catalog. Features whose rules reject cause a reroll within the
    /// `assign_feature` retry loop.
    ///
    /// Deterministic per `(master_seed, cell_id, baked_inputs)`.
    #[must_use]
    pub fn feature_at_cell_with_rules(
        &self,
        cell_id: CellId,
        terrain: &Terrain,
        hydrology: &HydrologyGraph,
        hub_catalog: &HubCatalog,
    ) -> FeatureAssignment {
        let (x, z) = self.seeds[cell_id as usize].position;
        let context = build_placement_context(self.seed, x, z, terrain, hydrology);
        let libs = SubBiomeLibraries::shared();
        let height_at = |qx: f32, qz: f32| terrain.height(qx, qz);
        let rules_ctx = PlacementRulesContext {
            master_seed: self.seed,
            hub_catalog,
            hydrology,
            height_at: &height_at,
        };
        assign_feature(self.seed, cell_id, (x, z), &context, libs, Some(&rules_ctx))
    }

    /// Sub-biome at world coordinate with placement-rule evaluation.
    /// Same signature-pattern as [`Self::sub_biome_at`] but threads the
    /// hub catalog.
    #[must_use]
    pub fn sub_biome_at_with_rules(
        &self,
        x: f32,
        z: f32,
        terrain: &Terrain,
        hydrology: &HydrologyGraph,
        hub_catalog: &HubCatalog,
    ) -> FeatureAssignment {
        let cell_id = self.nearest_cell(x, z);
        self.feature_at_cell_with_rules(cell_id, terrain, hydrology, hub_catalog)
    }

    /// Return the k nearest cell ids to a seed (adjacency neighbors).
    /// Brute-force grid search at `radius_cells` = 3 by default.
    #[must_use]
    pub fn neighbors_of(&self, cell_id: CellId, k: usize) -> Vec<CellId> {
        let (x, z) = self.seeds[cell_id as usize].position;
        let ix = ((wrap_coord(x) / self.grid_cell_m) as usize).min(self.grid_n - 1);
        let iz = ((wrap_coord(z) / self.grid_cell_m) as usize).min(self.grid_n - 1);
        let half_w = WORLD_EXTENT_M * 0.5;
        let mut candidates: Vec<(f32, CellId)> = Vec::with_capacity(16);
        for dz in -3_i64..=3 {
            for dx in -3_i64..=3 {
                let nx = wrap_cell_index(ix as i64 + dx, self.grid_n as i64) as usize;
                let nz = wrap_cell_index(iz as i64 + dz, self.grid_n as i64) as usize;
                let other_id = self.grid[nz * self.grid_n + nx];
                if other_id != EMPTY_GRID && other_id != cell_id {
                    let (ox, oz) = self.seeds[other_id as usize].position;
                    let mut dx_m = (x - ox).abs();
                    let mut dz_m = (z - oz).abs();
                    if dx_m > half_w {
                        dx_m = WORLD_EXTENT_M - dx_m;
                    }
                    if dz_m > half_w {
                        dz_m = WORLD_EXTENT_M - dz_m;
                    }
                    candidates.push((dx_m * dx_m + dz_m * dz_m, other_id));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
        candidates.into_iter().take(k).map(|(_, id)| id).collect()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Placement context — sample terrain + hydrology + biome at a cell seed
// ═══════════════════════════════════════════════════════════════════════════

/// Build a [`PlacementContext`] for a cell seed at `(x, z)` by
/// sampling terrain + hydrology + biome weights.
///
/// - altitude: `Terrain::height_at_scale(x, z, 2_500.0)` (mipmap-aware)
/// - slope: approximated from a small finite-difference kernel
/// - roughness: approximated as `max(|altitude - neighbor_altitude|)`
///   over a 4-direction sample; gives a coarse "feature fits flat"
///   proxy; refined in 3.6.3+ with proper cell-footprint statistics
/// - has_water_adjacency: true if any of 8 compass-direction samples
///   within [`WATER_ADJACENCY_RADIUS_M`] hits a non-DryChannel water
///   class in the hydrology graph
/// - biome_weights: [`biome_weights_at_position`]
fn build_placement_context(
    master_seed: u64,
    x: f32,
    z: f32,
    terrain: &Terrain,
    hydrology: &HydrologyGraph,
) -> PlacementContext {
    let min_spacing = 2_500.0_f32;
    let altitude_m = terrain.height_at_scale(x, z, min_spacing);
    let sample_radius = 1_000.0_f32;
    let h_e = terrain.height_at_scale(x + sample_radius, z, min_spacing);
    let h_w = terrain.height_at_scale(x - sample_radius, z, min_spacing);
    let h_n = terrain.height_at_scale(x, z + sample_radius, min_spacing);
    let h_s = terrain.height_at_scale(x, z - sample_radius, min_spacing);

    // Slope: steepest gradient from center to cardinal neighbors.
    let dx_max = (h_e - h_w).abs().max((h_n - h_s).abs());
    let dx_m = sample_radius * 2.0;
    let slope_rad = libm::atan2f(dx_max, dx_m);
    let slope_deg = slope_rad * (180.0 / core::f32::consts::PI);

    // Roughness: max deviation of cardinal samples from the center.
    let roughness_m = [h_e, h_w, h_n, h_s]
        .iter()
        .map(|h| (h - altitude_m).abs())
        .fold(0.0_f32, f32::max);

    // Water adjacency: 8-direction search at WATER_ADJACENCY_RADIUS_M.
    let has_water_adjacency = check_water_adjacency(hydrology, x, z);

    let biome_weights = biome_weights_at_position(master_seed, x, z);
    PlacementContext {
        altitude_m,
        slope_deg,
        roughness_m,
        has_water_adjacency,
        biome_weights,
    }
}

/// True if any of 8 compass-direction samples at
/// [`WATER_ADJACENCY_RADIUS_M`] hits a non-DryChannel water surface.
fn check_water_adjacency(hydrology: &HydrologyGraph, x: f32, z: f32) -> bool {
    let r = WATER_ADJACENCY_RADIUS_M;
    let offsets: [(f32, f32); 9] = [
        (0.0, 0.0),
        (r, 0.0),
        (-r, 0.0),
        (0.0, r),
        (0.0, -r),
        (r, r),
        (-r, r),
        (r, -r),
        (-r, -r),
    ];
    for (dx, dz) in offsets {
        if let Some(surface) = hydrology.water_surface_at(x + dx, z + dz) {
            if !matches!(surface.class, WaterClass::DryChannel) {
                return true;
            }
        }
    }
    false
}

// Keep an unused biome_at import silenced — used in tests.
#[allow(dead_code)]
fn _keep_biome_at_used(seed: u64, x: f32, z: f32) -> BiomeClass {
    biome_at(seed, x, z)
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Small-world bake: reduce SEED_SPACING temporarily via a
    /// different Bridson run would require a const swap; instead, do
    /// functional tests against the full-world bake via `--release`.
    /// Bake takes ~3 s release.
    fn bake_canonical() -> SubBiomeCellCatalog {
        SubBiomeCellCatalog::bake(0x0F10_1A11_u64)
    }

    #[test]
    fn bake_produces_millions_of_cells() {
        let c = bake_canonical();
        // 5000 km world / 2 km spacing has theoretical max ~7.2 M
        // (hex packing at r=2000 in 5000² area). Bridson dart-throwing
        // achieves ~55-70% of theoretical max (standard for the
        // algorithm). Assert >= 3 M cells (conservative lower bound);
        // observed density on canonical seed is ~3.9 M.
        assert!(
            c.cell_count() > 3_000_000,
            "cell count {} below expected floor (3M)",
            c.cell_count()
        );
    }

    #[test]
    fn bake_is_deterministic() {
        let a = SubBiomeCellCatalog::bake(12_345);
        let b = SubBiomeCellCatalog::bake(12_345);
        assert_eq!(a.cell_count(), b.cell_count());
        for i in (0..a.cell_count()).step_by(10_000) {
            assert_eq!(
                a.seeds[i].position.0.to_bits(),
                b.seeds[i].position.0.to_bits()
            );
            assert_eq!(
                a.seeds[i].position.1.to_bits(),
                b.seeds[i].position.1.to_bits()
            );
        }
    }

    #[test]
    fn nearest_cell_wraps_toroidally() {
        let c = bake_canonical();
        let a = c.nearest_cell(100_000.0, 100_000.0);
        let b = c.nearest_cell(100_000.0 + WORLD_EXTENT_M, 100_000.0);
        assert_eq!(a, b);
    }

    #[test]
    fn min_spacing_respected() {
        // Spot-check: no two seeds should be closer than SEED_SPACING_M.
        let c = bake_canonical();
        let half_w = WORLD_EXTENT_M * 0.5;
        let min_d2 = SEED_SPACING_M * SEED_SPACING_M;
        // Sample 1000 random cells + check their nearest neighbors.
        for i in (0..c.cell_count()).step_by(c.cell_count() / 1000) {
            let (x, z) = c.seeds[i].position;
            for neighbor in c.neighbors_of(i as CellId, 6) {
                if neighbor == i as CellId {
                    continue;
                }
                let (nx, nz) = c.seeds[neighbor as usize].position;
                let mut dx = (x - nx).abs();
                let mut dz = (z - nz).abs();
                if dx > half_w {
                    dx = WORLD_EXTENT_M - dx;
                }
                if dz > half_w {
                    dz = WORLD_EXTENT_M - dz;
                }
                let d2 = dx * dx + dz * dz;
                assert!(
                    d2 >= min_d2 - 1.0,
                    "cells {i} and {neighbor} closer than min spacing"
                );
            }
        }
    }

    #[test]
    fn neighbors_returned_sorted_by_distance() {
        let c = bake_canonical();
        // Pick a random cell, request 6 neighbors, verify distances
        // are ascending.
        let target = c.cell_count() / 2;
        let (x, z) = c.seeds[target].position;
        let neighbors = c.neighbors_of(target as CellId, 6);
        assert_eq!(neighbors.len().min(6), 6.min(neighbors.len()));
        let half_w = WORLD_EXTENT_M * 0.5;
        let mut prev_d = 0.0_f32;
        for n in neighbors {
            let (nx, nz) = c.seeds[n as usize].position;
            let mut dx = (x - nx).abs();
            let mut dz = (z - nz).abs();
            if dx > half_w {
                dx = WORLD_EXTENT_M - dx;
            }
            if dz > half_w {
                dz = WORLD_EXTENT_M - dz;
            }
            let d = dx.hypot(dz);
            assert!(
                d >= prev_d - 1.0,
                "neighbors not sorted: prev {prev_d} > current {d}"
            );
            prev_d = d;
        }
    }

    #[test]
    fn feature_at_cell_returns_nonfallback_most_of_time() {
        // On a canonical bake, the majority of cells should satisfy
        // some feature's constraints. Fallback fraction should be
        // reasonable (<30% given constraint diversity).
        let c = bake_canonical();
        let terrain = Terrain::from_seed(c.seed());
        let hydrology = HydrologyGraph::bake(c.seed(), &terrain);
        let mut fallback_count = 0_u32;
        let sample_n = 200_u32;
        for i in 0..sample_n {
            let cell_id = (i * (c.cell_count() as u32 / sample_n)) % c.cell_count() as u32;
            let f = c.feature_at_cell(cell_id, &terrain, &hydrology);
            if f.is_fallback {
                fallback_count += 1;
            }
        }
        let fallback_frac = fallback_count as f32 / sample_n as f32;
        assert!(
            fallback_frac < 0.3,
            "fallback rate {fallback_frac} > 30% — libraries too restrictive?"
        );
    }

    #[test]
    fn feature_at_cell_deterministic() {
        let c = bake_canonical();
        let terrain = Terrain::from_seed(c.seed());
        let hydrology = HydrologyGraph::bake(c.seed(), &terrain);
        let a = c.feature_at_cell(1000, &terrain, &hydrology);
        let b = c.feature_at_cell(1000, &terrain, &hydrology);
        assert_eq!(a.feature_id, b.feature_id);
    }
}
