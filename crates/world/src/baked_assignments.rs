//! Eager bake of sub-biome feature assignments with self-spacing
//! enforcement (Phase 3.6.3c.1c).
//!
//! Self-spacing rules (`SelfSpacingRules.min_km`) require knowing what
//! features are already placed where during placement — global state
//! the lazy per-cell evaluator can't satisfy without recursive dry-runs.
//! This module provides an eager bake pass that materializes per-cell
//! assignments + per-feature spatial hashes, replacing the unevaluated
//! 3.6.3c.1 stub with correct enforcement.
//!
//! # Pipeline
//!
//! 1. Build `BakedAssignments::bake(catalog, terrain, hydrology, hub_catalog)`.
//! 2. Process cells in **master-seed-keyed permuted order** (see scan-
//!    order rationale below).
//! 3. For each cell, sample candidates with placement-rule + self-
//!    spacing checks; reroll up to 8 attempts; fall back to `plain`.
//! 4. On accept: insert position into the per-feature spatial hash +
//!    write u16-interned feature id to the per-cell array.
//! 5. After bake, queries hit the cache: O(1) lookup by `CellId`.
//!
//! # Scan-order — master-seed-keyed permutation (PINNED 3.6.3c.1c)
//!
//! Bridson cell IDs are spatially correlated (depth-first walk from
//! initial seed → low IDs cluster near origin, high IDs far away).
//! Naive `0..n_cells` ascending order would let the low-ID region
//! "win" all contested high-spacing features (e.g. `hot_springs` with
//! 50 km min_km) because they get processed first; the geographically-
//! distant high-ID cells reject due to a hash already populated.
//!
//! The fix is a deterministic permutation: each cell gets a hash key
//! `splitmix64(master_seed, cell_id)`; cells are processed in order of
//! that key. Same master_seed → same permutation → bit-identical output
//! across platforms. Removes geographic bias while preserving full
//! determinism. See MAPGEN § Bake performance budget.
//!
//! # Memory budget
//!
//! - u16-interned feature IDs: ~3.9 M cells × 2 bytes = ~8 MB
//! - Lookup table: ~50 unique feature IDs × ~30 byte strings = ~1.5 KB
//! - Per-feature spatial hashes (18 features w/ self_spacing rule):
//!   sparse `HashMap<(i32,i32), Vec<(f32,f32)>>` — total ~5-10 MB
//! - **Total: ~20 MB** (vs ~150 MB for non-interned `String`-cached).
//!
//! Memory posture matches user-pinned discipline of staying lean (memo
//! `feedback_no_lint_bypass` and broader project ethos: not a Minecraft-
//! style multi-GB world state).
//!
//! # Parallelization candidate
//!
//! Per-cell rule evaluation is read-only against
//! terrain / hydrology / hub_catalog / climate samplers — embarrassingly
//! parallel via `rayon::par_iter`. Self-spacing-check + spatial-hash-
//! insert phase is serial state mutation. Clean split: parallel phase
//! produces `Vec<(CellId, candidate_attempts)>`; serial phase walks
//! permuted order doing self-spacing + retries.
//!
//! Estimated 3-4× speedup. Adding `rayon` is a one-line dep change. Ship
//! serial first; measure; add parallel if wall-clock > 30 s.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::doc_markdown)]

use std::collections::HashMap;

use crate::hub_catalog::HubCatalog;
use crate::hydrology::HydrologyGraph;
use crate::sub_biome::{
    assign_feature_for_bake, build_placement_context_external, BiomeSource, FeatureAssignment,
    PlacementRulesContext, SelfSpacingState, SubBiomeLibraries, FALLBACK_FEATURE_ID,
};
use crate::terrain::{HeightSampler, Terrain};
use crate::voronoi_cells::{CellId, SubBiomeCellCatalog};

/// Splitmix64-inspired hash matching the existing pattern in `sub_biome.rs`.
/// Used for the master-seed-keyed permutation order.
const fn mix_seed(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Sentinel for the per-cell assignment index — represents "fallback
/// to `plain`" without occupying a real feature-id slot.
const FALLBACK_INDEX: u16 = 0;

/// Bake-stat counters reported via [`BakedAssignments::stats`].
#[derive(Clone, Debug, Default)]
pub struct BakeStats {
    /// Total cells processed.
    pub cell_count: u32,
    /// Cells that fell back to `plain` after all retries exhausted.
    pub fallback_count: u32,
    /// Cells that ended up with a bleed-over assignment.
    pub bleed_over_count: u32,
    /// Cells rejected by self-spacing (each retry that hit the
    /// spatial-hash adds one).
    pub self_spacing_reject_count: u32,
    /// Total wall-clock for the bake (milliseconds). Populated when
    /// [`BakedAssignments::bake`] runs; useful for parallelization
    /// decisions.
    pub bake_wallclock_ms: u64,
}

/// Eager-baked sub-biome feature assignments with self-spacing.
///
/// Built once per world via [`Self::bake`]. Subsequent queries are
/// O(1) lookups by `CellId`.
pub struct BakedAssignments {
    /// Per-cell u16 index into [`Self::feature_ids`]. `FALLBACK_INDEX`
    /// = `plain`.
    per_cell: Vec<u16>,
    /// Per-cell `BiomeSource` (Primary vs BleedOver). Same length as
    /// `per_cell`.
    per_cell_source: Vec<BiomeSource>,
    /// Interned feature-id strings. Index 0 reserved for the `plain`
    /// fallback.
    feature_ids: Vec<String>,
    /// Reverse lookup: feature_id_str → u16 index. Used at bake time
    /// to map accepted-feature-ids back to interned indices; retained
    /// for post-bake feature-id-to-index queries (visual-bench may need
    /// this when comparing feature distributions).
    feature_id_index: HashMap<String, u16>,
    /// Per-feature spatial hashes — populated during bake; useful for
    /// post-bake spatial queries (e.g. "is there a fishing_camp within
    /// 5 km of this point?"). Empty for features without self_spacing.
    spatial_hashes: SelfSpacingState,
    /// Bake statistics.
    stats: BakeStats,
}

impl BakedAssignments {
    /// Run the eager bake pass.
    ///
    /// Iterates all cells in master-seed-keyed permuted order. For each
    /// cell, evaluates the full placement-rule pipeline (including
    /// self-spacing); writes accepted assignments to the per-cell cache
    /// + per-feature spatial hashes.
    ///
    /// Single-threaded synchronous. Wall-clock measured + recorded in
    /// [`BakeStats::bake_wallclock_ms`].
    #[must_use]
    pub fn bake(
        catalog: &SubBiomeCellCatalog,
        terrain: &Terrain,
        hydrology: &HydrologyGraph,
        hub_catalog: &HubCatalog,
    ) -> Self {
        let start = std::time::Instant::now();
        let master_seed = catalog.master_seed();
        let n_cells = catalog.cell_count();

        // ---- Build feature-id intern table --------------------------
        // Determinism: sort unique IDs alphabetically before assigning indices.
        let libraries = SubBiomeLibraries::shared();
        let mut all_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        all_ids.insert(FALLBACK_FEATURE_ID.to_string());
        for biome in [
            crate::biome::BiomeClass::HighDesert,
            crate::biome::BiomeClass::RockyAlpine,
            crate::biome::BiomeClass::CoastalBoreal,
            crate::biome::BiomeClass::MediterraneanArchipelago,
        ] {
            for f in libraries.for_biome(biome) {
                all_ids.insert(f.id.clone());
            }
        }
        for f in libraries.oceanic() {
            all_ids.insert(f.id.clone());
        }
        // Move FALLBACK to index 0 explicitly.
        let mut feature_ids: Vec<String> = Vec::with_capacity(all_ids.len());
        feature_ids.push(FALLBACK_FEATURE_ID.to_string());
        for id in all_ids {
            if id != FALLBACK_FEATURE_ID {
                feature_ids.push(id);
            }
        }
        let feature_id_index: HashMap<String, u16> = feature_ids
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u16))
            .collect();

        // ---- Compute permuted scan order ----------------------------
        let mut order: Vec<(u64, CellId)> = (0..n_cells as CellId)
            .map(|cell_id| (mix_seed(master_seed, u64::from(cell_id)), cell_id))
            .collect();
        order.sort_unstable_by_key(|&(k, _)| k);

        // ---- Bake-state -----------------------------------------
        let mut per_cell: Vec<u16> = vec![FALLBACK_INDEX; n_cells];
        let mut per_cell_source: Vec<BiomeSource> = vec![
            BiomeSource::Primary(crate::biome::BiomeClass::CoastalBoreal);
            n_cells
        ];
        let mut spatial_hashes = SelfSpacingState::new();
        let mut stats = BakeStats {
            cell_count: n_cells as u32,
            ..BakeStats::default()
        };

        // ---- Per-cell evaluation loop ---------------------------
        let height_at = |x: f32, z: f32| terrain.height(x, z);

        for (_, cell_id) in order {
            let (x, z) = catalog.seeds()[cell_id as usize].position;
            let context = build_placement_context_external(master_seed, x, z, terrain, hydrology);

            // 3.6.3c.1c bake optimization: fetch hubs in MAX_RULE_HUB_RADIUS_M
            // ONCE per cell and reuse across all candidate evaluations.
            // Without this cache the bake explodes to ~minutes per 100 K
            // cells (every candidate's hub-proximity rule re-queries the
            // hub spatial hash).
            let cell_hubs: Vec<&crate::hub_catalog::HubEntry> = hub_catalog.hubs_in_radius(
                x,
                z,
                crate::sub_biome::MAX_RULE_HUB_RADIUS_M,
            );
            let rules_ctx = PlacementRulesContext {
                master_seed,
                hub_catalog,
                hydrology,
                height_at: &height_at,
                cell_hubs_cache: Some(&cell_hubs),
            };

            let assignment = assign_feature_for_bake(
                master_seed,
                cell_id,
                (x, z),
                &context,
                libraries,
                &rules_ctx,
                &mut spatial_hashes,
                &mut stats,
            );

            let idx = *feature_id_index
                .get(&assignment.feature_id)
                .unwrap_or(&FALLBACK_INDEX);
            per_cell[cell_id as usize] = idx;
            per_cell_source[cell_id as usize] = assignment.source;
            if assignment.is_fallback {
                stats.fallback_count += 1;
            }
            if assignment.source.is_bleed_over() {
                stats.bleed_over_count += 1;
            }
        }

        let elapsed = start.elapsed();
        stats.bake_wallclock_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);

        Self {
            per_cell,
            per_cell_source,
            feature_ids,
            feature_id_index,
            spatial_hashes,
            stats,
        }
    }

    /// Look up a cell's baked assignment.
    #[must_use]
    pub fn feature_at_cell(&self, cell_id: CellId) -> FeatureAssignment {
        let idx = self.per_cell[cell_id as usize];
        let source = self.per_cell_source[cell_id as usize];
        FeatureAssignment {
            feature_id: self.feature_ids[idx as usize].clone(),
            source,
            is_fallback: idx == FALLBACK_INDEX,
        }
    }

    /// Same as [`Self::feature_at_cell`] but returns the interned u16
    /// index without cloning a String. Useful for fast scans (visual-
    /// bench, density diagnostics).
    #[must_use]
    pub fn feature_index_at_cell(&self, cell_id: CellId) -> u16 {
        self.per_cell[cell_id as usize]
    }

    /// Resolve a u16 index to its feature-id string.
    #[must_use]
    pub fn feature_id_for(&self, index: u16) -> Option<&str> {
        self.feature_ids.get(index as usize).map(String::as_str)
    }

    /// Look up a feature-id's interned u16 index (`None` if not in the
    /// catalog at bake time).
    #[must_use]
    pub fn index_for_feature_id(&self, feature_id: &str) -> Option<u16> {
        self.feature_id_index.get(feature_id).copied()
    }

    /// Total feature-id count (interned table size).
    #[must_use]
    pub fn unique_feature_count(&self) -> usize {
        self.feature_ids.len()
    }

    /// Bake statistics — wall-clock + reject counters.
    #[must_use]
    pub const fn stats(&self) -> &BakeStats {
        &self.stats
    }

    /// Read-only access to the per-feature spatial hashes (for post-
    /// bake spatial queries).
    #[must_use]
    pub const fn spatial_hashes(&self) -> &SelfSpacingState {
        &self.spatial_hashes
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::terrain::Terrain;

    const TEST_SEED: u64 = 0xF10_1A11;

    /// Bake → assignment lookup invariants. Smoke test only — full
    /// bake against canonical seed is the visual-bench's job.
    #[test]
    #[ignore = "expensive: bakes ~3.9M cells; opt-in via --ignored"]
    fn bake_assignments_determinism() {
        let terrain = Terrain::from_seed(TEST_SEED);
        let hydrology = HydrologyGraph::bake(TEST_SEED, &terrain);
        let hub_catalog = HubCatalog::bake(TEST_SEED, &terrain, &hydrology);
        let catalog = SubBiomeCellCatalog::bake(TEST_SEED);

        let baked_a = BakedAssignments::bake(&catalog, &terrain, &hydrology, &hub_catalog);
        let baked_b = BakedAssignments::bake(&catalog, &terrain, &hydrology, &hub_catalog);

        // Same seed → bit-identical per-cell assignments.
        assert_eq!(baked_a.per_cell, baked_b.per_cell);
        assert_eq!(baked_a.feature_ids, baked_b.feature_ids);
    }
}
