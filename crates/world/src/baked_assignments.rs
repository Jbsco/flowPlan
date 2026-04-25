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
//! # Parallelization (SHIPPED)
//!
//! `bake` runs as **Phase A in parallel** (per-cell rule evaluation,
//! read-only against terrain / hydrology / hub_catalog / climate
//! samplers — embarrassingly parallel via `rayon::par_iter`) followed
//! by **Phase B in serial** (master-seed-keyed permuted-order walk;
//! self-spacing checks + spatial-hash inserts).
//!
//! Determinism: master-seed-keyed permutation drives Phase B order, so
//! the same master_seed produces bit-identical output regardless of
//! thread count or scheduling. Phase A's parallel iteration order is
//! irrelevant because each cell is independent + the consumer (Phase
//! B) re-orders.
//!
//! Measured at canonical seed: **275 s serial → 33 s 8-core parallel
//! = 8.3× speedup**. `rayon` is in `DESIGN.md` tech-stack, world-bake
//! hot paths only — never on render or audio per CODING_RULES § real-
//! time discipline.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::doc_markdown)]

use std::collections::HashMap;

use rayon::prelude::*;

use crate::feature_instances::{compose_instances_for_feature, FeatureInstance};
use crate::hub_catalog::HubCatalog;
use crate::hydrology::HydrologyGraph;
use crate::sub_biome::{
    build_placement_context_external, compute_bake_candidates_for_cell, BiomeSource,
    FeatureAssignment, PlacementRulesContext, SelfSpacingState, SubBiomeLibraries,
    FALLBACK_FEATURE_ID,
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
    /// Per-cell feature-instance lists (3.6.3c.5c.1). Sparse — only
    /// cells whose feature has an authored primitive composition have
    /// an entry. Memory: ~100-400 bytes per populated cell × thousands
    /// of populated cells = tens of MB total across all features.
    ///
    /// Instances are computed at bake time so render-side can stream
    /// via the cell active set without re-running per-cell logic.
    /// Palette adaptation + cell-seed RNG variation are baked in.
    per_cell_instances: HashMap<CellId, Vec<FeatureInstance>>,
    /// Bake statistics.
    stats: BakeStats,
}

impl BakedAssignments {
    /// Run the eager bake pass — parallel phase A + serial phase B.
    ///
    /// **Phase A (parallel via `rayon::par_iter`)**: per-cell, build
    /// `PlacementContext` + per-cell hub cache + a list of candidate
    /// retries. Each retry candidate is the feature the RNG would
    /// pick at that attempt, after passing all rules EXCEPT
    /// self-spacing (which is deferred to phase B for sequential
    /// state mutation). Read-only against terrain / hydrology /
    /// hub_catalog / climate samplers — embarrassingly parallel.
    ///
    /// **Phase B (serial in master-seed-keyed permuted order)**: walk
    /// per-cell candidate lists; first non-self-spacing-violating
    /// candidate wins; insert into `SelfSpacingState` + write u16
    /// feature index to per-cell array.
    ///
    /// Determinism: master-seed-keyed permutation drives Phase B
    /// order, so the same master_seed produces bit-identical output
    /// regardless of thread count or scheduling.
    ///
    /// Wall-clock measured + recorded in [`BakeStats::bake_wallclock_ms`].
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
        let libraries = SubBiomeLibraries::shared();

        // ---- Build feature-id intern table --------------------------
        let (feature_ids, feature_id_index) = build_feature_id_intern_table(libraries);

        // ---- Compute permuted scan order ----------------------------
        let mut order: Vec<(u64, CellId)> = (0..n_cells as CellId)
            .map(|cell_id| (mix_seed(master_seed, u64::from(cell_id)), cell_id))
            .collect();
        order.sort_unstable_by_key(|&(k, _)| k);

        // ---- Phase A: parallel per-cell candidate generation -------
        let precomputed = run_phase_a(
            master_seed,
            n_cells,
            catalog,
            terrain,
            hydrology,
            hub_catalog,
            libraries,
            &feature_id_index,
        );

        // ---- Phase B: serial self-spacing merge in permuted order ---
        let mut per_cell: Vec<u16> = vec![FALLBACK_INDEX; n_cells];
        let mut per_cell_source: Vec<BiomeSource> =
            vec![BiomeSource::Primary(crate::biome::BiomeClass::CoastalBoreal); n_cells];
        let mut spatial_hashes = SelfSpacingState::new();
        let mut stats = BakeStats {
            cell_count: n_cells as u32,
            ..BakeStats::default()
        };

        for (_, cell_id) in order {
            let cell = &precomputed[cell_id as usize];
            per_cell_source[cell_id as usize] = cell.source;
            if cell.source.is_bleed_over() {
                stats.bleed_over_count += 1;
            }

            // Open-water cells: oceanic features have no self_spacing
            // rules. Take the first attempt (or FALLBACK_INDEX if none
            // — but oceanic library is permissive, so first attempt
            // almost always wins).
            if cell.is_open_water {
                let idx = cell
                    .attempts
                    .first()
                    .map_or(FALLBACK_INDEX, |a| a.feature_id_index);
                per_cell[cell_id as usize] = idx;
                if idx == FALLBACK_INDEX {
                    stats.fallback_count += 1;
                }
                continue;
            }

            // Land cells: walk attempts in order, accept first that
            // doesn't violate self-spacing. Insert into hash on accept.
            let mut accepted = false;
            for attempt in &cell.attempts {
                if attempt.self_spacing_min_km > 0.0
                    && spatial_hashes.would_violate(
                        feature_ids[attempt.feature_id_index as usize].as_str(),
                        cell.pos,
                        attempt.self_spacing_min_km,
                    )
                {
                    stats.self_spacing_reject_count += 1;
                    continue;
                }
                if attempt.self_spacing_min_km > 0.0 {
                    spatial_hashes.insert(
                        feature_ids[attempt.feature_id_index as usize].as_str(),
                        cell.pos,
                    );
                }
                per_cell[cell_id as usize] = attempt.feature_id_index;
                accepted = true;
                break;
            }
            if !accepted {
                per_cell[cell_id as usize] = FALLBACK_INDEX;
                stats.fallback_count += 1;
            }
        }

        // ---- Phase C: per-cell instance generation (3.6.3c.5c.1) ----
        // For each cell whose accepted feature has an authored primitive
        // composition, generate its instance list (uses cell-seed RNG
        // + palette adaptation + terrain-height anchoring). Sparse:
        // only populated cells stored.
        let per_cell_instances = run_phase_c(
            master_seed,
            n_cells,
            &per_cell,
            &per_cell_source,
            &feature_ids,
            catalog,
            libraries,
            terrain,
        );

        let elapsed = start.elapsed();
        stats.bake_wallclock_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);

        Self {
            per_cell,
            per_cell_source,
            feature_ids,
            feature_id_index,
            spatial_hashes,
            per_cell_instances,
            stats,
        }
    }

    /// Look up a cell's baked feature instances. Returns empty slice
    /// for cells whose feature has no primitive composition (5c.1 MVP
    /// supports `dwelling_cluster` only; 5c.2+ extends).
    ///
    /// Consumed by the active-cell-set streaming loop in game_app per
    /// 3.6.3c.5c.1 design.
    #[must_use]
    pub fn instances_at_cell(&self, cell_id: CellId) -> &[FeatureInstance] {
        self.per_cell_instances
            .get(&cell_id)
            .map_or(&[], Vec::as_slice)
    }

    /// Total placed instance count across all cells. Useful for
    /// bake-stat diagnostics.
    #[must_use]
    pub fn total_instance_count(&self) -> usize {
        self.per_cell_instances.values().map(Vec::len).sum()
    }

    /// Number of cells with at least one baked instance. Useful for
    /// diagnosing "where are the populated cells?" after a bake.
    #[must_use]
    pub fn populated_cell_count(&self) -> usize {
        self.per_cell_instances.len()
    }

    /// Iterator over populated cells sorted by `CellId` (deterministic).
    /// Consumed at bake time to emit sample coordinates the player can
    /// fly to when verifying the 3.6.3c.5c.1 render path.
    pub fn populated_cells(&self) -> impl Iterator<Item = (CellId, &[FeatureInstance])> {
        let mut ids: Vec<CellId> = self.per_cell_instances.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .map(move |id| (id, self.per_cell_instances[&id].as_slice()))
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

/// Phase-A worker. Per-cell parallel candidate generation, no shared
/// mutable state. Returns `Vec<PrecomputedCell>` indexed by `CellId`
/// (NOT permuted order — phase B does the permuted walk).
///
/// Read-only against terrain / hydrology / hub_catalog / climate
/// samplers. Embarrassingly parallel via `rayon::par_iter`.
#[allow(clippy::too_many_arguments)]
fn run_phase_a(
    master_seed: u64,
    n_cells: usize,
    catalog: &SubBiomeCellCatalog,
    terrain: &Terrain,
    hydrology: &HydrologyGraph,
    hub_catalog: &HubCatalog,
    libraries: &SubBiomeLibraries,
    feature_id_index: &HashMap<String, u16>,
) -> Vec<PrecomputedCell> {
    let height_at_local = move |x: f32, z: f32| terrain.height(x, z);
    (0..n_cells as CellId)
        .into_par_iter()
        .map(|cell_id| {
            let (x, z) = catalog.seeds()[cell_id as usize].position;
            let context = build_placement_context_external(master_seed, x, z, terrain, hydrology);
            let cell_hubs: Vec<&crate::hub_catalog::HubEntry> =
                hub_catalog.hubs_in_radius(x, z, crate::sub_biome::MAX_RULE_HUB_RADIUS_M);
            let rules_ctx = PlacementRulesContext {
                master_seed,
                hub_catalog,
                hydrology,
                height_at: &height_at_local,
                cell_hubs_cache: Some(&cell_hubs),
            };
            let outcome = compute_bake_candidates_for_cell(
                master_seed,
                cell_id,
                (x, z),
                &context,
                libraries,
                &rules_ctx,
            );
            let attempts: Vec<CandidateAttempt> = outcome
                .attempts
                .into_iter()
                .map(|a| CandidateAttempt {
                    feature_id_index: *feature_id_index
                        .get(&a.feature_id)
                        .unwrap_or(&FALLBACK_INDEX),
                    self_spacing_min_km: a.self_spacing_min_km,
                })
                .collect();
            PrecomputedCell {
                pos: (x, z),
                source: outcome.source,
                attempts,
                is_open_water: outcome.is_open_water,
            }
        })
        .collect()
}

/// Phase-C worker. Per-cell feature-instance generation. Read-only
/// against the output of phases A + B + the terrain sampler (for
/// Y-anchoring). Embarrassingly parallel via `rayon::par_iter`.
///
/// 5c.1 MVP: only cells whose accepted feature has an authored
/// primitive composition contribute an entry. All others are skipped
/// (sparse storage). `AUTHORED_FEATURE_IDS` is the early-out filter so
/// the 3.9 M-cell scan doesn't pay the cost of a library lookup per
/// skipped cell. The terrain sampler is invoked per placed instance
/// (typically 5-8 per populated cell) at bake time so the render
/// side never re-samples.
#[allow(clippy::too_many_arguments)]
fn run_phase_c(
    master_seed: u64,
    n_cells: usize,
    per_cell: &[u16],
    per_cell_source: &[BiomeSource],
    feature_ids: &[String],
    catalog: &SubBiomeCellCatalog,
    libraries: &SubBiomeLibraries,
    terrain: &Terrain,
) -> HashMap<CellId, Vec<FeatureInstance>> {
    // Phase C generates instances for any feature with an authored RON
    // `composition`. `compose_instances_for_feature` early-returns
    // `Vec::new()` when `composition` is `None`, so the
    // `instances.is_empty()` check below filters composition-less
    // features (natural / scenery / unauthored) without an explicit
    // whitelist. Library lookup per-cell is O(1) HashMap; the cost
    // (~100 ms across the 3.9 M-cell scan) is negligible vs the bake's
    // ~30 s total.
    let height_at = |x: f32, z: f32| terrain.height(x, z);

    (0..n_cells as CellId)
        .into_par_iter()
        .filter_map(|cell_id| {
            let idx = per_cell[cell_id as usize];
            if idx == FALLBACK_INDEX {
                return None;
            }
            let feature_id = feature_ids[idx as usize].as_str();
            let source = per_cell_source[cell_id as usize];
            let pos = catalog.seeds()[cell_id as usize].position;

            // Resolve the feature. Try the source biome first; fall
            // back to oceanic (sea cells). At most one of the two
            // succeeds because feature_id intern table only contains
            // ids from libraries we actually sampled.
            let feature = libraries
                .find(source.feature_source_biome(), feature_id)
                .or_else(|| libraries.oceanic().iter().find(|f| f.id == feature_id))?;

            // Skip features without authored compositions before calling
            // the composer (saves the per-component RNG setup; equivalent
            // to the empty-instances check below but cheaper).
            feature.composition.as_ref()?;

            let instances = compose_instances_for_feature(
                master_seed,
                cell_id,
                pos,
                feature,
                source,
                libraries,
                &height_at,
            );
            if instances.is_empty() {
                None
            } else {
                Some((cell_id, instances))
            }
        })
        .collect()
}

/// Build the deterministic feature-id intern table.
///
/// Returns the interned `Vec<String>` (`FALLBACK_FEATURE_ID` at index 0)
/// plus the reverse-lookup map. Sorted-by-name internally so indices
/// are bit-identical across runs.
fn build_feature_id_intern_table(
    libraries: &SubBiomeLibraries,
) -> (Vec<String>, HashMap<String, u16>) {
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
    (feature_ids, feature_id_index)
}

/// Per-cell precomputed bake input — output of phase A, consumed by
/// phase B. Carries the cell position, biome source, and the list of
/// candidate retries (each with its self-spacing min_km annotated).
struct PrecomputedCell {
    pos: (f32, f32),
    source: BiomeSource,
    attempts: Vec<CandidateAttempt>,
    is_open_water: bool,
}

/// One retry candidate from phase A. The feature index + the
/// self-spacing min_km (0.0 if no rule) is enough for phase B to do
/// its sequential merge without re-querying anything.
struct CandidateAttempt {
    feature_id_index: u16,
    self_spacing_min_km: f32,
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
