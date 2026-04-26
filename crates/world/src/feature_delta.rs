//! Layer 3 feature-delta engine (Phase 3.6.3a).
//!
//! Converts the semantic feature tags from Layer 2 (`sub_biome`) into
//! heightfield deltas — `lake_cluster` carves a bowl, `mesa_top`
//! raises a plateau, `sandy_beach` slopes toward the shore, etc.
//!
//! # Composition
//!
//! `final_height = base_L1(x, z) + Σ feature_delta(x, z, cell)`
//!
//! Each cell's delta falls smoothly to zero at its Voronoi edge (bump
//! / smoothstep falloff) — neighbors don't fight over boundary
//! heights, continuous terrain across Voronoi edges by construction
//! (MAPGEN.md § Layer 3 boundary falloff invariant).
//!
//! # Hydrology continuity invariant
//!
//! Deltas cannot violate the L1 hydrology graph. At query time, if
//! the point sits on a water surface (river / lake / sea) and the
//! delta is elevation-raising (positive), the delta is suppressed to
//! 0. Negative deltas (lake-carving) are allowed on water cells —
//! they align with existing basins rather than fighting them.
//!
//! # Hub foundation priority
//!
//! Cells hosting tier-1/2 hubs get a tier-adaptive `hub_foundation`
//! delta (full flatten + skirt) instead of their natural feature
//! delta. Priority: `hub_foundation` > natural feature > `plain`.
//! Built via [`HubFoundationMap::build`] from hub catalog + sub-biome
//! partition, consulted at query time.
//!
//! # Performance
//!
//! Per-query cost: one `SubBiomeCellCatalog::nearest_cell` (O(1) via
//! spatial hash) + one `HubFoundationMap` lookup (O(1) via HashMap)
//! + delta computation (a few ops) + optional hydrology check (O(1)).
//! Total ~100 ns per query. Fine for 1024² visual-bench render and
//! sim-tick buoyancy query rates.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use flowplan_core::{wrap_coord, WORLD_EXTENT_M};

use crate::baked_assignments::BakedAssignments;
use crate::feature_delta_impls;
use crate::feature_groups::FeatureGroupBundle;
use crate::hub_catalog::{HubCatalog, HubTier};
use crate::hydrology::{HydrologyGraph, WaterClass};
use crate::voronoi_cells::{CellId, SubBiomeCellCatalog};

// ═══════════════════════════════════════════════════════════════════════════
// Hub foundation overrides
// ═══════════════════════════════════════════════════════════════════════════

/// Maps Voronoi cell ids to their hosted hub tier (when any).
///
/// Built once post-bake from the hub catalog + sub-biome partition.
/// Query time: `tier_for(cell_id) -> Option<HubTier>`. Cells hosting
/// hubs get the tier-adaptive hub-foundation delta; cells without get
/// the natural feature delta.
#[derive(Clone, Debug, Default)]
pub struct HubFoundationMap {
    overrides: HashMap<CellId, HubTier>,
}

impl HubFoundationMap {
    /// Build the foundation map by resolving each hub's approximate
    /// position to a sub-biome cell.
    #[must_use]
    pub fn build(hubs: &HubCatalog, sub_biome: &SubBiomeCellCatalog) -> Self {
        let mut overrides = HashMap::new();
        for hub in hubs.hubs() {
            let cell_id = sub_biome.nearest_cell(hub.approx_world_pos.0, hub.approx_world_pos.1);
            // If multiple hubs collide on one cell, the highest tier
            // wins (smaller HubTier enum variant = higher priority
            // per PartialOrd on the tier enum — Major < Regional < ...).
            overrides
                .entry(cell_id)
                .and_modify(|existing: &mut HubTier| {
                    if hub.tier < *existing {
                        *existing = hub.tier;
                    }
                })
                .or_insert(hub.tier);
        }
        Self { overrides }
    }

    /// Look up the tier of a hub hosted at this cell, if any.
    #[must_use]
    pub fn tier_for(&self, cell_id: CellId) -> Option<HubTier> {
        self.overrides.get(&cell_id).copied()
    }

    /// Total number of hub-foundation overrides.
    #[must_use]
    pub fn count(&self) -> usize {
        self.overrides.len()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Falloff helpers (shared across delta implementations)
// ═══════════════════════════════════════════════════════════════════════════

/// Bump-function falloff: smoothly 1 at center (t=0) → 0 at edge (t=1).
///
/// `t = distance_from_cell_center / cell_approx_radius`. Values > 1 are
/// outside the cell; return 0.
///
/// Implementation: smoothstep of (1 − t²) for C¹ continuity. Gives a
/// rounded bump that neighbors' deltas won't conflict with at cell
/// boundaries.
#[inline]
#[must_use]
pub fn bump_falloff(t: f32) -> f32 {
    if t >= 1.0 {
        return 0.0;
    }
    let t_clamped = t.clamp(0.0, 1.0);
    let inner = 1.0 - t_clamped * t_clamped;
    // Smoothstep-like: (inner)^2 gives C¹ at center + 0 slope at edge.
    inner * inner
}

/// Smoothstep: 0 at t=0, 1 at t=1, smooth S-curve between.
#[inline]
#[must_use]
pub fn smoothstep(t: f32) -> f32 {
    let t_clamped = t.clamp(0.0, 1.0);
    t_clamped * t_clamped * (3.0 - 2.0 * t_clamped)
}

// ═══════════════════════════════════════════════════════════════════════════
// Delta context
// ═══════════════════════════════════════════════════════════════════════════

/// Inputs to a single feature-delta evaluation.
#[derive(Copy, Clone, Debug)]
pub struct DeltaContext {
    /// Query point.
    pub x: f32,
    pub z: f32,
    /// Cell centroid (seed position).
    pub cell_center_x: f32,
    pub cell_center_z: f32,
    /// Approximate cell radius (m) — distance to nearest neighbor seed.
    pub cell_radius_m: f32,
    /// L1 base elevation at query point (pre-delta).
    pub base_altitude_m: f32,
    /// Normalized distance from cell center to query point, in units
    /// of `cell_radius_m`. In `[0, 1]` inside the cell; > 1 outside
    /// (the query is closer to another cell than to this one, but
    /// the caller may still want the falloff result).
    pub normalized_distance: f32,
}

impl DeltaContext {
    /// Pre-compute the bump falloff for this context.
    #[must_use]
    #[inline]
    pub fn falloff(&self) -> f32 {
        bump_falloff(self.normalized_distance)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Top-level delta query
// ═══════════════════════════════════════════════════════════════════════════

/// Compute the Layer 3 feature delta at `(x, z)` given the full
/// world-bake state.
///
/// Steps:
/// 1. Find nearest sub-biome cell.
/// 2. If the cell hosts a hub (via [`HubFoundationMap`]) → hub
///    foundation delta.
/// 3. Else → natural feature delta from the cell's feature_id.
/// 4. Apply hydrology continuity: if query point is on water AND
///    delta is positive, suppress to 0.
///
/// Returns an additive height correction (m). Sum on top of the L1
/// `Terrain::height_at_scale` result to get the final L3 height.
#[must_use]
pub fn compute_delta_at(
    x: f32,
    z: f32,
    base_altitude_m: f32,
    sub_biome: &SubBiomeCellCatalog,
    foundations: &HubFoundationMap,
    hubs: &HubCatalog,
    hydrology: &HydrologyGraph,
    feature_groups: &FeatureGroupBundle,
    baked: Option<&BakedAssignments>,
    terrain_seed: u64,
) -> f32 {
    let cell_id = sub_biome.nearest_cell(x, z);
    let cell = &sub_biome.seeds()[cell_id as usize];
    let (cx, cz) = cell.position;

    // Approximate cell radius = distance to nearest neighbor seed.
    // One extra lookup per delta query but keeps the falloff correct
    // for variable-size cells.
    let neighbors = sub_biome.neighbors_of(cell_id, 1);
    let radius_m = if let Some(&n_id) = neighbors.first() {
        let (nx, nz) = sub_biome.seeds()[n_id as usize].position;
        toroidal_distance_m((cx, cz), (nx, nz)) * 0.5
    } else {
        1000.0 // fallback — shouldn't happen with our dense Bridson seeding
    };

    let dist_to_center = toroidal_distance_m((cx, cz), (x, z));
    let normalized_distance = dist_to_center / radius_m.max(1.0);

    let ctx = DeltaContext {
        x,
        z,
        cell_center_x: cx,
        cell_center_z: cz,
        cell_radius_m: radius_m,
        base_altitude_m,
        normalized_distance,
    };

    // Hub foundation takes priority over natural feature.
    let raw_delta = if let Some(tier) = foundations.tier_for(cell_id) {
        // Look up the hub at this cell to get the target runway
        // elevation (we use approx_world_pos altitude sampled on L1
        // base terrain as the runway datum).
        let runway_datum = hubs
            .hubs()
            .iter()
            .find(|h| {
                let h_cell = sub_biome.nearest_cell(h.approx_world_pos.0, h.approx_world_pos.1);
                h_cell == cell_id
            })
            .map_or(ctx.base_altitude_m, |_h| ctx.base_altitude_m);
        feature_delta_impls::hub_foundation_delta(tier, &ctx, runway_datum)
    } else {
        // Natural feature: prefer the pre-baked assignment when
        // available (O(1) HashMap lookup). Without baked, fall back
        // to recomputing — `assign_feature` is non-trivial (placement
        // context build + RNG retry loop, ~100 µs/call) so the
        // streaming-mesh rebuild path REQUIRES baked to stay performant
        // (1.4 M vertices × 100 µs = 140 s without baked).
        let feature_id = if let Some(baked_ref) = baked {
            baked_ref.feature_at_cell(cell_id).feature_id
        } else {
            let terrain = crate::terrain::Terrain::from_seed(terrain_seed);
            sub_biome
                .feature_at_cell(cell_id, &terrain, hydrology)
                .feature_id
        };
        feature_delta_impls::natural_delta(&feature_id, &ctx)
    };

    // Hydrology continuity: suppress positive deltas on water cells.
    if raw_delta > 0.0 {
        if let Some(surface) = hydrology.water_surface_at(x, z) {
            if !matches!(surface.class, WaterClass::DryChannel) {
                return 0.0;
            }
        }
    }

    // Layer 3 group deltas — additive on top of the per-cell delta.
    // For 3.6.3c.3a, only roads. Each road is a polyline; the depression
    // band falls smoothly to zero outside `half_width_m`.
    let group_delta = compute_group_delta_at(x, z, cell_id, feature_groups, hydrology);

    raw_delta + group_delta
}

/// Sum of all feature-group deltas at `(x, z)`. For the cells the
/// query-point's nearest-cell belongs to, evaluate each group's
/// delta and sum. Per-kind centerline depth comes from
/// [`crate::feature_groups::FeatureGroupKind::depth_m`].
///
/// Suppressed on water (same continuity invariant as natural deltas):
/// depressions never carve into open water surfaces; berms (positive
/// `depth_m`, e.g. `Rail`) follow the per-cell positive-suppression
/// rule.
fn compute_group_delta_at(
    x: f32,
    z: f32,
    cell_id: CellId,
    feature_groups: &FeatureGroupBundle,
    hydrology: &HydrologyGraph,
) -> f32 {
    let mems = feature_groups.groups_for_cell(cell_id);
    if mems.is_empty() {
        return 0.0;
    }
    let mut total = 0.0_f32;
    for m in mems {
        let Some(group) = feature_groups.group(m.group_id) else {
            continue;
        };
        // Per-cell membership records the segment index nearest the
        // cell's seed. Check at most segments [i-1, i, i+1] — bounds
        // the per-cell cost regardless of polyline length. Without
        // this, long highways would force every membership query to
        // iterate the full polyline (~125 segments × 1.4 M vertices
        // = 175 M seg-distance computes per streaming-mesh rebuild).
        let seg_idx = m.seg_idx as usize;
        let n_segments = group.polyline.len().saturating_sub(1);
        if n_segments == 0 {
            continue;
        }
        let lo = seg_idx.saturating_sub(1);
        let hi = (seg_idx + 1).min(n_segments.saturating_sub(1));

        // Align query to polyline frame once via the first vertex.
        let (ref_x, ref_z) = group.polyline[0];
        let half = flowplan_core::WORLD_EXTENT_M * 0.5;
        let mut aqx = x;
        let mut aqz = z;
        let dx = x - ref_x;
        let dz = z - ref_z;
        if dx > half {
            aqx -= flowplan_core::WORLD_EXTENT_M;
        } else if dx < -half {
            aqx += flowplan_core::WORLD_EXTENT_M;
        }
        if dz > half {
            aqz -= flowplan_core::WORLD_EXTENT_M;
        } else if dz < -half {
            aqz += flowplan_core::WORLD_EXTENT_M;
        }

        let mut best_dist = f32::INFINITY;
        for seg in lo..=hi {
            let p0 = group.polyline[seg];
            let p1 = group.polyline[seg + 1];
            let d = point_to_segment_distance(aqx, aqz, p0, p1);
            if d < best_dist {
                best_dist = d;
            }
        }
        if best_dist >= group.half_width_m {
            continue;
        }
        let t = best_dist / group.half_width_m;
        let falloff = 1.0 - smoothstep(t);
        total += group.kind.depth_m() * falloff;
    }
    // Hydrology continuity: groups don't cut through / rise out of water.
    if total != 0.0 {
        if let Some(surface) = hydrology.water_surface_at(x, z) {
            if !matches!(surface.class, WaterClass::DryChannel) {
                return 0.0;
            }
        }
    }
    total
}

#[inline]
fn point_to_segment_distance(qx: f32, qz: f32, p0: (f32, f32), p1: (f32, f32)) -> f32 {
    let dx = p1.0 - p0.0;
    let dz = p1.1 - p0.1;
    let len_sq = dx * dx + dz * dz;
    if len_sq <= 1e-6 {
        return (qx - p0.0).hypot(qz - p0.1);
    }
    let t = ((qx - p0.0) * dx + (qz - p0.1) * dz) / len_sq;
    let t_clamped = t.clamp(0.0, 1.0);
    let cx = p0.0 + t_clamped * dx;
    let cz = p0.1 + t_clamped * dz;
    (qx - cx).hypot(qz - cz)
}

/// Toroidal-closest-path distance between two world coordinates.
#[inline]
fn toroidal_distance_m(a: (f32, f32), b: (f32, f32)) -> f32 {
    let half_w = WORLD_EXTENT_M * 0.5;
    let mut dx = (wrap_coord(a.0) - wrap_coord(b.0)).abs();
    let mut dz = (wrap_coord(a.1) - wrap_coord(b.1)).abs();
    if dx > half_w {
        dx = WORLD_EXTENT_M - dx;
    }
    if dz > half_w {
        dz = WORLD_EXTENT_M - dz;
    }
    dx.hypot(dz)
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn bump_falloff_properties() {
        // 1.0 at center, 0.0 at edge, monotonic decreasing.
        assert!((bump_falloff(0.0) - 1.0).abs() < 1e-5);
        assert!(bump_falloff(1.0).abs() < 1e-5);
        assert!(bump_falloff(1.5).abs() < 1e-5);
        assert!(bump_falloff(0.5) > bump_falloff(0.8));
        assert!(bump_falloff(0.1) > bump_falloff(0.5));
    }

    #[test]
    fn smoothstep_endpoints() {
        assert!((smoothstep(0.0) - 0.0).abs() < 1e-5);
        assert!((smoothstep(1.0) - 1.0).abs() < 1e-5);
        assert!(smoothstep(0.5) > 0.0 && smoothstep(0.5) < 1.0);
    }

    #[test]
    fn hub_foundation_map_empty_by_default() {
        let map = HubFoundationMap::default();
        assert_eq!(map.count(), 0);
        assert!(map.tier_for(0).is_none());
    }

    #[test]
    fn bump_falloff_c0_at_edge() {
        // falloff should be continuous at t=1 — approach 0 from both
        // sides.
        let eps = 0.001;
        let below = bump_falloff(1.0 - eps);
        let at = bump_falloff(1.0);
        let above = bump_falloff(1.0 + eps);
        assert!(below < 0.01, "below-edge not near zero: {below}");
        assert!((at - 0.0).abs() < 1e-5);
        assert!((above - 0.0).abs() < 1e-5);
    }
}
