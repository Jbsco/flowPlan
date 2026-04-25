//! Feature groups — multi-cell linear features (Phase 3.6.3c.3a).
//!
//! Roads, rail corridors, and other features that span multiple Voronoi
//! cells along a shared centerline polyline. Cells along the polyline
//! become group members; the L3 delta engine computes a depression
//! based on perpendicular distance from the polyline rather than
//! per-cell radial falloff (which can't represent "a thing that runs
//! through several cells").
//!
//! # Authoring layer
//!
//! Group construction is bake-time, deterministic per `master_seed`.
//! For 3.6.3c.3a, the only group type is **`Road`** — connects each
//! hub to its k-nearest neighbor hubs via straight polylines (Steiner-
//! tree-lite). Hubs reflect the regional skeleton of human settlement,
//! so this puts roads where roads belong without authoring per-pair
//! routes.
//!
//! Future group types (3.6.3c.3b/c):
//! - `Rail` — same shape as road, different delta (raised berm), tier-
//!   gated to Major↔Regional pairs.
//! - `ResidentialCluster` — spillover-driven k-nearest cells around a
//!   `dwelling_cluster` core (hub emergent upgrade memory).
//!
//! # Query layer
//!
//! `FeatureGroupBundle` is the post-bake artifact. Two access patterns:
//!
//! 1. **Per-cell membership**: `bundle.groups_for_cell(cell_id)` returns
//!    the group ids the cell belongs to (typically 0-1; 2+ at road
//!    intersections). Used by the L3 engine to dispatch group deltas.
//! 2. **Per-group geometry**: `bundle.group(group_id)` returns the
//!    polyline + metadata. The delta function reads the polyline to
//!    compute perpendicular distance.
//!
//! # Determinism
//!
//! Pure function of `(master_seed, hub_catalog, sub_biome_catalog)`.
//! Hub iteration order is `HubId` ascending; nearest-neighbor selection
//! uses a stable distance comparison (toroidal Euclidean). Bit-identical
//! per seed.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::doc_markdown)]

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use flowplan_core::{wrap_coord, WORLD_EXTENT_M};

use crate::hub_catalog::{HubCatalog, HubId, HubTier};
use crate::hydrology::{HydrologyGraph, WaterClass};
use crate::terrain::{HeightSampler, Terrain};
use crate::voronoi_cells::{CellId, SubBiomeCellCatalog};

/// Stable group identifier. Assigned in seed order at bake time.
pub type FeatureGroupId = u32;

/// Hub-eligibility predicate signature — used by the per-kind config
/// table in `FeatureGroupBundle::build_road_network`.
type EligibilityFn = fn(&crate::hub_catalog::HubEntry) -> bool;

/// Group kinds for L3 multi-cell features.
///
/// Five road-tier variants (`RoughDirt` … `Highway`) plus `Rail`
/// (separate ladder, not road-comparable). Road tiers are `Ord`-
/// comparable via [`Self::road_tier`] — `RoughDirt` < `Dirt` <
/// `Gravel` < `Paved` < `Highway`.
///
/// Per-kind dispatch reads geometry constants via accessors below.
/// 3.6.3c.3c added the 5-tier road ladder; v1 used 4 kinds with `Road`
/// as the umbrella paved-tier. The rename `Road` → `Paved` is reflected
/// at every call site.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FeatureGroupKind {
    /// Two-rut track — barely a road. Wagon ruts, ATV trail. Narrowest
    /// band, lightest depression. Emitted by very rural features
    /// (`homestead_clearing`, `abandoned_homestead`).
    RoughDirt,
    /// Dirt road — single-lane graded earth surface. Standard rural
    /// access road. Emitted by `remote_compound`, `ranch`.
    Dirt,
    /// Gravel road — improved with crushed-stone surface. Mid-tier;
    /// "infrastructure brought in" upgrade above Dirt. Emitted by
    /// `dwelling_cluster`, `farmland_parcel`.
    Gravel,
    /// Single-lane paved road. Connects Regional / Local hubs and
    /// emitted by larger developments (`ghost_town`, `adobe_village`).
    Paved,
    /// Two-lane highway. Connects Major hubs. Visually most prominent
    /// road type.
    Highway,
    /// Rail corridor connecting Major hubs. Raised berm (positive
    /// delta) rather than depression. Not on the road-tier ladder.
    Rail,
}

impl FeatureGroupKind {
    /// Feature-id string used by the RON catalog + delta dispatch.
    /// Keep aligned with `data/world/sub_biome_features.ron` ids.
    #[must_use]
    pub fn feature_id(self) -> &'static str {
        match self {
            Self::RoughDirt => "rough_dirt_segment",
            Self::Dirt => "dirt_road_segment",
            Self::Gravel => "gravel_road_segment",
            Self::Paved => "road_segment",
            Self::Highway => "highway_intersection",
            Self::Rail => "rail_corridor_segment",
        }
    }

    /// Half-width of the depression / berm band (m). Outside this
    /// distance the delta is 0; inside it smoothsteps to the centerline.
    #[must_use]
    pub const fn half_width_m(self) -> f32 {
        match self {
            Self::RoughDirt => 8.0,
            Self::Dirt => 15.0,
            Self::Gravel => 18.0,
            Self::Paved => 25.0,
            Self::Highway => 40.0,
            Self::Rail => 20.0,
        }
    }

    /// Depth (m) at the centerline. Negative = depression (cut into
    /// terrain), positive = raised berm. Smoothsteps to 0 at the band
    /// edge.
    #[must_use]
    pub const fn depth_m(self) -> f32 {
        match self {
            Self::RoughDirt => -0.4,
            Self::Dirt => -0.6,
            Self::Gravel => -1.0,
            Self::Paved => -1.5,
            Self::Highway => -3.0,
            Self::Rail => 1.5,
        }
    }

    /// Inter-segment cap — connections longer than this are not built.
    /// Keeps the network plausible (no 500 km RoughDirt tracks).
    /// Pass-2 feature-emit paths use the same cap as a length budget;
    /// paths that exhaust the budget before reaching the network fade
    /// out at the dead end.
    #[must_use]
    pub const fn max_length_m(self) -> f32 {
        match self {
            Self::RoughDirt => 30_000.0,
            Self::Dirt => 60_000.0,
            Self::Gravel => 80_000.0,
            Self::Paved => 120_000.0,
            Self::Highway => 250_000.0,
            Self::Rail => 200_000.0,
        }
    }

    /// Road-tier index for `Ord`-comparison. `Some(0..=4)` for the
    /// five road kinds; `None` for `Rail` (off-ladder).
    ///
    /// Used by `road_emit` features to find compatible-or-higher-tier
    /// targets to path toward — a `Gravel` emitter accepts `Gravel`,
    /// `Paved`, or `Highway` cells as path destinations but not `Dirt`
    /// or `RoughDirt`.
    #[allow(clippy::doc_markdown)]
    #[must_use]
    pub const fn road_tier(self) -> Option<u8> {
        match self {
            Self::RoughDirt => Some(0),
            Self::Dirt => Some(1),
            Self::Gravel => Some(2),
            Self::Paved => Some(3),
            Self::Highway => Some(4),
            Self::Rail => None,
        }
    }

    /// True for road-ladder kinds (everything except `Rail`).
    #[must_use]
    pub const fn is_road(self) -> bool {
        self.road_tier().is_some()
    }
}

/// One multi-cell linear feature. Stored once in the bundle; cells that
/// the polyline passes through reference it by id.
///
/// **Polyline coordinate convention**: vertices are stored in literal
/// (un-wrapped) world coords. A polyline that crosses the toroidal seam
/// will have one vertex at literal `~0` and the next at literal
/// `~WORLD_EXTENT_M` rather than vertices that wrap mid-edge — the
/// delta evaluator wraps query points before distance computation so
/// the seam is invisible to consumers.
#[derive(Clone, Debug)]
pub struct FeatureGroup {
    pub id: FeatureGroupId,
    pub kind: FeatureGroupKind,
    /// Ordered polyline vertices in world coords (`(x, z)` pairs).
    /// Always length ≥ 2.
    pub polyline: Vec<(f32, f32)>,
    /// Half-width of the depression band (m). Outside this distance
    /// from the polyline the delta is 0; inside, smoothstep-falls toward
    /// the polyline.
    pub half_width_m: f32,
}

/// Per-cell membership entry.
///
/// `group_id` identifies the polyline; `seg_idx` is the representative
/// vertex index — segment `[seg_idx, seg_idx+1]` is the one closest to
/// the cell. The L3 delta evaluator only checks segments near
/// `seg_idx` — at most 3 distance computes per cell-group pair
/// instead of iterating the full polyline.
#[derive(Copy, Clone, Debug)]
pub struct GroupMembership {
    pub group_id: FeatureGroupId,
    pub seg_idx: u16,
}

/// Bundle of feature groups + per-cell membership index.
///
/// Built once at world bake. Read-only at query time; cheap to share
/// (held by `WorldRefsArc`).
#[derive(Clone, Debug, Default)]
pub struct FeatureGroupBundle {
    groups: Vec<FeatureGroup>,
    /// `cell_id` → list of `(group_id, segment_index)` — each entry is
    /// a polyline that passes through this cell at the given segment.
    /// Most cells: 0 entries. Cells on a road: 1 entry. Intersections: 2+.
    membership: HashMap<CellId, Vec<GroupMembership>>,
}

impl FeatureGroupBundle {
    /// Empty bundle — used when group construction is disabled.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build the full road network from the hub catalog. 3.6.3c.3b
    /// emits four kinds — `Highway` / `Road` / `DirtRoad` / `Rail` —
    /// each with its own hub-tier eligibility, k-nearest budget, and
    /// length cap. See [`FeatureGroupKind`] accessors for per-kind
    /// constants.
    ///
    /// Wrap-aware: polylines that span the toroidal seam keep one
    /// endpoint shifted by `±WORLD_EXTENT_M` so segment length matches
    /// the geodesic.
    #[must_use]
    pub fn build_road_network(hubs: &HubCatalog, sub_biome: &SubBiomeCellCatalog) -> Self {
        let mut groups: Vec<FeatureGroup> = Vec::new();
        let mut connected: std::collections::HashSet<(HubId, HubId, FeatureGroupKind)> =
            std::collections::HashSet::new();

        // Per-kind connection rules: (kind, eligibility predicate, k-nearest budget).
        // Pass 1 — hub-to-hub trunk roads. Feature-emitted access roads
        // (`RoughDirt` / `Dirt` / `Gravel` per-tier) are added by Pass 2
        // (`extend_with_feature_emit_roads`).
        let configs: &[(FeatureGroupKind, EligibilityFn, usize)] = &[
            (FeatureGroupKind::Highway, is_major_hub, 2),
            (FeatureGroupKind::Paved, is_major_or_regional_hub, 2),
            (FeatureGroupKind::Dirt, is_local_hub, 2),
            (FeatureGroupKind::Rail, is_major_hub, 1),
        ];

        for &(kind, eligible_pred, k_nearest) in configs {
            let max_length = kind.max_length_m();
            let half_width = kind.half_width_m();

            let eligible: Vec<&crate::hub_catalog::HubEntry> =
                hubs.hubs().iter().filter(|h| eligible_pred(h)).collect();

            for (i, hub_a) in eligible.iter().enumerate() {
                let mut neighbors: Vec<(usize, f32)> = eligible
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(j, hub_b)| {
                        let d = toroidal_distance_m(hub_a.approx_world_pos, hub_b.approx_world_pos);
                        (j, d)
                    })
                    .filter(|(_, d)| *d <= max_length)
                    .collect();
                neighbors.sort_by(|a, b| {
                    a.1.partial_cmp(&b.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.0.cmp(&b.0))
                });
                for (j, _) in neighbors.into_iter().take(k_nearest) {
                    let hub_b = eligible[j];
                    let pair = if hub_a.id <= hub_b.id {
                        (hub_a.id, hub_b.id, kind)
                    } else {
                        (hub_b.id, hub_a.id, kind)
                    };
                    if connected.contains(&pair) {
                        continue;
                    }
                    connected.insert(pair);

                    let polyline = straight_polyline_wrap_aware(
                        hub_a.approx_world_pos,
                        hub_b.approx_world_pos,
                    );
                    let group_id = groups.len() as FeatureGroupId;
                    groups.push(FeatureGroup {
                        id: group_id,
                        kind,
                        polyline,
                        half_width_m: half_width,
                    });
                }
            }
        }

        // 3. Build per-cell membership. Walk each polyline by sampling
        //    every ~half_width meters and recording the cell at each
        //    sample. Coarse but adequate — group lookup happens once
        //    per L3-query cell, not per polyline-vertex.
        let mut membership: HashMap<CellId, Vec<GroupMembership>> = HashMap::new();
        for group in &groups {
            let step_m = group.half_width_m * 1.5;
            let mut visited: std::collections::HashSet<CellId> = std::collections::HashSet::new();
            for (seg_idx, window) in group.polyline.windows(2).enumerate() {
                let (x0, z0) = window[0];
                let (x1, z1) = window[1];
                let dx = x1 - x0;
                let dz = z1 - z0;
                let length = dx.hypot(dz).max(1.0);
                let n_steps = (length / step_m).ceil() as usize + 1;
                let seg_idx_u16 = seg_idx.min(u16::MAX as usize) as u16;
                for s in 0..=n_steps {
                    let t = s as f32 / n_steps as f32;
                    let x = wrap_coord(x0 + dx * t);
                    let z = wrap_coord(z0 + dz * t);
                    let cell = sub_biome.nearest_cell(x, z);
                    if visited.insert(cell) {
                        membership.entry(cell).or_default().push(GroupMembership {
                            group_id: group.id,
                            seg_idx: seg_idx_u16,
                        });
                    }
                }
            }
        }

        Self { groups, membership }
    }

    /// Look up groups whose polyline passes through this cell. Most
    /// cells: empty slice.
    #[must_use]
    pub fn groups_for_cell(&self, cell_id: CellId) -> &[GroupMembership] {
        self.membership.get(&cell_id).map_or(&[], Vec::as_slice)
    }

    /// Look up a group by id. Out-of-range returns `None`.
    #[must_use]
    pub fn group(&self, id: FeatureGroupId) -> Option<&FeatureGroup> {
        self.groups.get(id as usize)
    }

    /// Total number of groups.
    #[must_use]
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// True if no groups baked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// All groups — diagnostic / visual-bench access.
    #[must_use]
    pub fn groups(&self) -> &[FeatureGroup] {
        &self.groups
    }

    /// Total cell-membership entries (sum of `groups_for_cell` across
    /// all cells). Diagnostic.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.membership.values().map(Vec::len).sum()
    }

    /// Set of cells whose polyline membership flags them as "on a road".
    /// Used by `requires_road_adjacency` placement-rule evaluation.
    /// Returns a snapshot — caller can store + query without re-iter.
    ///
    /// 3.6.3c.3b: strict membership only. A cell is "on a road" iff
    /// the polyline literally passed through it during bake. Future
    /// extension (3.6.3c.3c+) could expand outward via the sub-biome
    /// adjacency graph for "near a road within N km" semantics.
    #[must_use]
    pub fn road_adjacent_cells(&self) -> HashSet<CellId> {
        self.membership.keys().copied().collect()
    }

    /// Pass 2 — extend the bundle with feature-emit access roads
    /// (Phase 3.6.3c.3c). For each placed feature instance whose
    /// authored `road_emit` is `Some(kind)`, A* a path from the
    /// feature's cell to the nearest cell already on a network of
    /// equal-or-higher tier (or hub-tier feature). Paths are appended
    /// to `self`; per-cell membership grows.
    ///
    /// Inputs:
    /// - `road_emit_cells`: per-cell list of `(emit_kind, count)` —
    ///   collected from `BakedAssignments` by the caller. We only
    ///   need to know which cells have road-emitting features and at
    ///   what tier; how many instances live there doesn't change the
    ///   road geometry.
    /// - `terrain` / `hydrology`: pathfinder cost inputs.
    ///
    /// Algorithm: per source cell, A* over the Voronoi-cell adjacency
    /// graph (`SubBiomeCellCatalog::neighbors_of`) with cost =
    /// segment-length + slope-penalty + water-block + feature-soft-
    /// cost. Heuristic = toroidal-Euclidean distance to nearest target.
    /// Length-cap = `kind.max_length_m()`; if exhausted, emit a
    /// fade-out polyline (depth smoothsteps to 0 at the dead end).
    pub fn extend_with_feature_emit_roads(
        &mut self,
        road_emit_cells: &[(CellId, FeatureGroupKind)],
        sub_biome: &SubBiomeCellCatalog,
        terrain: &Terrain,
        hydrology: &HydrologyGraph,
    ) {
        // Group emit cells by kind so we build all paths of one tier
        // together — same target set per kind (network at >= tier).
        let mut by_kind: HashMap<FeatureGroupKind, Vec<CellId>> = HashMap::new();
        for &(cell_id, kind) in road_emit_cells {
            by_kind.entry(kind).or_default().push(cell_id);
        }

        // Process tiers in DESCENDING order so higher-tier paths exist
        // before lower-tier paths look for them as targets. A Gravel
        // emitter that connects to a Paved trunk needs the Paved trunk
        // already in the network when we evaluate Gravel.
        let mut kinds_sorted: Vec<FeatureGroupKind> = by_kind.keys().copied().collect();
        kinds_sorted.sort_by_key(|k| std::cmp::Reverse(k.road_tier().unwrap_or(0)));

        for kind in kinds_sorted {
            let Some(my_tier) = kind.road_tier() else {
                continue;
            };
            let target_set = self.target_cells_for_tier(my_tier);
            // Lower tiers inherit higher-tier emit cells via
            // `target_cells_for_tier` after the higher tier ran in
            // descending-tier order — chain-emergent connectivity
            // forms automatically. Same-tier seeding would self-
            // target (every emit source is in the target set, paths
            // collapse to 1-cell stubs), so we deliberately don't
            // add this kind's own emit cells.
            if target_set.is_empty() {
                continue;
            }

            // Multi-source Dijkstra from all targets, length-capped.
            // Produces (distance, predecessor) for every cell within
            // `max_length` of the network. Per-feature path reconstruction
            // walks the predecessor chain in O(path_length). Replaces
            // the per-feature A* (O(target_count) heuristic per pop
            // ballooned the cost; multi-source Dijkstra precomputes
            // exactly the gradient we need).
            let dijkstra = multi_source_dijkstra(&target_set, sub_biome, terrain, hydrology, kind);

            let mut sources = by_kind.get(&kind).cloned().unwrap_or_default();
            sources.sort_unstable(); // deterministic emit order
            for src in sources {
                if target_set.contains(&src) {
                    continue;
                }
                let Some(path_cells) = reconstruct_path(src, &dijkstra, &target_set) else {
                    // No path within length cap reached the network —
                    // fade-out path emission deferred to a polish item.
                    continue;
                };
                let polyline = polyline_from_cells(&path_cells, sub_biome);
                let group_id = self.groups.len() as FeatureGroupId;
                self.groups.push(FeatureGroup {
                    id: group_id,
                    kind,
                    polyline,
                    half_width_m: kind.half_width_m(),
                });
                // Each path-cell sits at vertex i; the closest segment
                // is `[i, i+1]` (or `[i-1, i]` for the last cell).
                for (i, cell) in path_cells.iter().enumerate() {
                    let seg_idx = i.min(path_cells.len().saturating_sub(2));
                    let seg_idx_u16 = seg_idx.min(u16::MAX as usize) as u16;
                    self.membership
                        .entry(*cell)
                        .or_default()
                        .push(GroupMembership {
                            group_id,
                            seg_idx: seg_idx_u16,
                        });
                }
            }
        }
    }

    /// Cells whose membership includes a group of road-tier ≥ `min_tier`.
    /// Used as the target set for Pass-2 A* pathing.
    fn target_cells_for_tier(&self, min_tier: u8) -> HashSet<CellId> {
        let mut out: HashSet<CellId> = HashSet::new();
        for (cell, mems) in &self.membership {
            for m in mems {
                if let Some(group) = self.groups.get(m.group_id as usize) {
                    if let Some(t) = group.kind.road_tier() {
                        if t >= min_tier {
                            out.insert(*cell);
                            break;
                        }
                    }
                }
            }
        }
        out
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Hub-eligibility predicates
// ═══════════════════════════════════════════════════════════════════════

#[inline]
fn is_major_hub(h: &crate::hub_catalog::HubEntry) -> bool {
    matches!(h.tier, HubTier::Major)
}

#[inline]
fn is_major_or_regional_hub(h: &crate::hub_catalog::HubEntry) -> bool {
    matches!(h.tier, HubTier::Major | HubTier::Regional)
}

#[inline]
fn is_local_hub(h: &crate::hub_catalog::HubEntry) -> bool {
    matches!(h.tier, HubTier::Local)
}

// ═══════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════

fn toroidal_distance_m(a: (f32, f32), b: (f32, f32)) -> f32 {
    let half = WORLD_EXTENT_M * 0.5;
    let mut dx = (a.0 - b.0).abs();
    let mut dz = (a.1 - b.1).abs();
    if dx > half {
        dx = WORLD_EXTENT_M - dx;
    }
    if dz > half {
        dz = WORLD_EXTENT_M - dz;
    }
    dx.hypot(dz)
}

/// Two-vertex polyline from `a` to `b`, choosing whichever endpoint
/// frame keeps the segment shorter than `WORLD_EXTENT_M / 2`. Required
/// because the toroidal world means the "literal-coord straight line"
/// isn't always the geodesic — a hub at literal `(100, 0)` connecting
/// to a hub at literal `(4_999_900, 0)` should run 200 m across the
/// seam, not 4.99 Mm across the world.
fn straight_polyline_wrap_aware(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let half = WORLD_EXTENT_M * 0.5;
    let mut bx = b.0;
    let mut bz = b.1;
    let dx = bx - a.0;
    let dz = bz - a.1;
    if dx > half {
        bx -= WORLD_EXTENT_M;
    } else if dx < -half {
        bx += WORLD_EXTENT_M;
    }
    if dz > half {
        bz -= WORLD_EXTENT_M;
    } else if dz < -half {
        bz += WORLD_EXTENT_M;
    }
    vec![a, (bx, bz)]
}

/// Perpendicular distance (m) from a query point to a polyline,
/// wrap-aware. Returns the minimum perpendicular distance to any
/// segment.
///
/// **Wrap-handling**: polylines are stored in literal coords with all
/// vertices in a single frame (built via `straight_polyline_wrap_aware`
/// for Pass 1 / `polyline_from_cells` for Pass 2). The query point may
/// be in a different literal frame (e.g. the query is at literal
/// `WORLD_EXTENT_M - 100` while the polyline runs from literal `-1000`
/// to `+1000`). We align the query to the polyline's frame once via
/// the first vertex, then do one pass over segments — much cheaper
/// than the prior 9-shift × N-segment evaluator.
#[must_use]
pub fn perpendicular_distance_to_polyline(polyline: &[(f32, f32)], qx: f32, qz: f32) -> f32 {
    if polyline.len() < 2 {
        return f32::INFINITY;
    }
    let (ref_x, ref_z) = polyline[0];
    let half = WORLD_EXTENT_M * 0.5;
    let mut aqx = qx;
    let mut aqz = qz;
    let dx = qx - ref_x;
    let dz = qz - ref_z;
    if dx > half {
        aqx -= WORLD_EXTENT_M;
    } else if dx < -half {
        aqx += WORLD_EXTENT_M;
    }
    if dz > half {
        aqz -= WORLD_EXTENT_M;
    } else if dz < -half {
        aqz += WORLD_EXTENT_M;
    }
    let mut best = f32::INFINITY;
    for window in polyline.windows(2) {
        let (x0, z0) = window[0];
        let (x1, z1) = window[1];
        let d = point_to_segment_distance(aqx, aqz, x0, z0, x1, z1);
        if d < best {
            best = d;
        }
    }
    best
}

fn point_to_segment_distance(qx: f32, qz: f32, x0: f32, z0: f32, x1: f32, z1: f32) -> f32 {
    let dx = x1 - x0;
    let dz = z1 - z0;
    let len_sq = dx * dx + dz * dz;
    if len_sq <= 1e-6 {
        return (qx - x0).hypot(qz - z0);
    }
    let t = ((qx - x0) * dx + (qz - z0) * dz) / len_sq;
    let t_clamped = t.clamp(0.0, 1.0);
    let cx = x0 + t_clamped * dx;
    let cz = z0 + t_clamped * dz;
    (qx - cx).hypot(qz - cz)
}

// ═══════════════════════════════════════════════════════════════════════
// Pass-2 A* pathfinder (3.6.3c.3c)
// ═══════════════════════════════════════════════════════════════════════

/// A* search node — wrapped to make `BinaryHeap` (max-heap) act as a
/// min-heap on `f_score` and stable on `cell_id` for tie-breaking.
#[derive(Clone, Copy, Debug)]
struct AStarNode {
    cell_id: CellId,
    f_score: f32,
}

impl Eq for AStarNode {}
impl PartialEq for AStarNode {
    fn eq(&self, other: &Self) -> bool {
        self.f_score.to_bits() == other.f_score.to_bits() && self.cell_id == other.cell_id
    }
}
impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse: smaller f_score wins (we want a min-heap).
        other
            .f_score
            .partial_cmp(&self.f_score)
            .unwrap_or(Ordering::Equal)
            .then(self.cell_id.cmp(&other.cell_id))
    }
}
impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Output of `multi_source_dijkstra` — `dist[cell]` is the shortest
/// road-cost from `cell` to the nearest target; `successor[cell]` is
/// the next cell along the shortest path TOWARD the network. Walking
/// `successor` from any source converges on a target cell.
struct DijkstraResult {
    dist: HashMap<CellId, f32>,
    successor: HashMap<CellId, CellId>,
}

/// Multi-source Dijkstra from `targets` over the Voronoi-cell graph,
/// length-capped at `kind.max_length_m()`. Replaces the per-feature A*
/// (whose nearest-target heuristic was O(target_count) per pop —
/// ballooned the cost on real bakes). Single Dijkstra builds a
/// shortest-path-to-network field; per-feature path reconstruction
/// walks successors in O(path_length).
///
/// Cost per edge: toroidal-Euclidean step + slope penalty (5× >25°,
/// 2× >15°). Water-crossing edges are unreachable.
fn multi_source_dijkstra(
    targets: &HashSet<CellId>,
    sub_biome: &SubBiomeCellCatalog,
    terrain: &Terrain,
    hydrology: &HydrologyGraph,
    kind: FeatureGroupKind,
) -> DijkstraResult {
    let max_length = kind.max_length_m();
    let mut dist: HashMap<CellId, f32> = HashMap::new();
    let mut successor: HashMap<CellId, CellId> = HashMap::new();
    let mut open: BinaryHeap<AStarNode> = BinaryHeap::new();

    for &t in targets {
        dist.insert(t, 0.0);
        open.push(AStarNode {
            cell_id: t,
            f_score: 0.0,
        });
    }

    while let Some(AStarNode { cell_id, f_score }) = open.pop() {
        // Stale-entry skip — heap entry for cell with old worse dist.
        let cur_dist = dist.get(&cell_id).copied().unwrap_or(f32::INFINITY);
        if f_score > cur_dist + 1.0 {
            continue;
        }
        if cur_dist > max_length {
            continue;
        }
        let cur_pos = sub_biome.seeds()[cell_id as usize].position;
        let neighbors = sub_biome.neighbors_of(cell_id, 6);
        for &n_id in &neighbors {
            let n_pos = sub_biome.seeds()[n_id as usize].position;
            let Some(edge_cost) = edge_cost_or_block(n_pos, cur_pos, terrain, hydrology) else {
                continue;
            };
            let tentative = cur_dist + edge_cost;
            if tentative > max_length {
                continue;
            }
            let prev = dist.get(&n_id).copied().unwrap_or(f32::INFINITY);
            if tentative < prev {
                dist.insert(n_id, tentative);
                // Successor of `n` (along path TOWARD network) is
                // `cell_id` because cell_id is closer to network.
                successor.insert(n_id, cell_id);
                open.push(AStarNode {
                    cell_id: n_id,
                    f_score: tentative,
                });
            }
        }
    }

    DijkstraResult { dist, successor }
}

/// Cycle / overrun guard for `reconstruct_path` (a multi-source
/// Dijkstra with a 250 km cap can't emit chains beyond ~125 cells in
/// practice; double that as a safety bound).
const MAX_PATH_STEPS: usize = 200;

/// Walk the successor chain from `start` until a target is reached.
/// Returns `[start, ..., target]` or `None` if `start` is outside the
/// Dijkstra's length cap (or the chain doesn't terminate at a target,
/// which shouldn't happen given the multi-source seeding).
fn reconstruct_path(
    start: CellId,
    dijkstra: &DijkstraResult,
    targets: &HashSet<CellId>,
) -> Option<Vec<CellId>> {
    if !dijkstra.dist.contains_key(&start) {
        return None;
    }
    let mut path = vec![start];
    let mut cur = start;
    let mut steps = 0_usize;
    while !targets.contains(&cur) {
        let next = *dijkstra.successor.get(&cur)?;
        if next == cur {
            return None;
        }
        path.push(next);
        cur = next;
        steps += 1;
        if steps > MAX_PATH_STEPS {
            return None;
        }
    }
    Some(path)
}

/// Edge cost between two adjacent Voronoi cells. Returns `None` if the
/// edge crosses water (not traversable).
fn edge_cost_or_block(
    a: (f32, f32),
    b: (f32, f32),
    terrain: &Terrain,
    hydrology: &HydrologyGraph,
) -> Option<f32> {
    let dist = toroidal_distance_m(a, b);
    // Mid-segment water check — sample at the midpoint. Coarser than
    // walking the whole segment but adequate at Voronoi cell scale
    // (~2 km steps). False negatives possible at narrow water crossings
    // — refine in a polish pass.
    let mid = (wrap_coord((a.0 + b.0) * 0.5), wrap_coord((a.1 + b.1) * 0.5));
    let mid_h = HeightSampler::height(terrain, mid.0, mid.1);
    if mid_h < crate::hydrology::SEA_LEVEL_M {
        return None;
    }
    if let Some(surface) = hydrology.water_surface_at(mid.0, mid.1) {
        if !matches!(surface.class, WaterClass::DryChannel) {
            return None;
        }
    }
    // Slope penalty — steep edges cost more.
    let h_a = HeightSampler::height(terrain, wrap_coord(a.0), wrap_coord(a.1));
    let h_b = HeightSampler::height(terrain, wrap_coord(b.0), wrap_coord(b.1));
    let dh = (h_b - h_a).abs();
    let slope_deg = libm::atan2f(dh, dist).to_degrees();
    let slope_mult = if slope_deg > 25.0 {
        5.0
    } else if slope_deg > 15.0 {
        2.0
    } else {
        1.0
    };
    Some(dist * slope_mult)
}

/// Convert a sequence of `CellId`s into a polyline of cell-seed
/// positions. Wrap-aware: consecutive cells on opposite sides of the
/// toroidal seam keep one endpoint shifted so the segment is short.
fn polyline_from_cells(cells: &[CellId], sub_biome: &SubBiomeCellCatalog) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(cells.len());
    if cells.is_empty() {
        return out;
    }
    let first = sub_biome.seeds()[cells[0] as usize].position;
    out.push(first);
    let mut prev = first;
    for &cell_id in cells.iter().skip(1) {
        let raw = sub_biome.seeds()[cell_id as usize].position;
        // Wrap raw to be within ±EXTENT/2 of prev.
        let half = WORLD_EXTENT_M * 0.5;
        let mut x = raw.0;
        let mut z = raw.1;
        let dx = x - prev.0;
        let dz = z - prev.1;
        if dx > half {
            x -= WORLD_EXTENT_M;
        } else if dx < -half {
            x += WORLD_EXTENT_M;
        }
        if dz > half {
            z -= WORLD_EXTENT_M;
        } else if dz < -half {
            z += WORLD_EXTENT_M;
        }
        out.push((x, z));
        prev = (x, z);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_bundle_returns_no_groups() {
        let bundle = FeatureGroupBundle::empty();
        assert!(bundle.is_empty());
        assert_eq!(bundle.len(), 0);
        assert!(bundle.groups_for_cell(0).is_empty());
        assert!(bundle.group(0).is_none());
    }

    #[test]
    fn perpendicular_distance_on_polyline_is_zero() {
        let poly = vec![(0.0, 0.0), (1000.0, 0.0)];
        assert!(perpendicular_distance_to_polyline(&poly, 500.0, 0.0) < 1e-3);
        // Endpoints.
        assert!(perpendicular_distance_to_polyline(&poly, 0.0, 0.0) < 1e-3);
        assert!(perpendicular_distance_to_polyline(&poly, 1000.0, 0.0) < 1e-3);
    }

    #[test]
    fn perpendicular_distance_off_polyline() {
        // Polyline along x=0 from (0, 0) to (1000, 0). Query at
        // (500, 50) → 50 m perpendicular.
        let poly = vec![(0.0, 0.0), (1000.0, 0.0)];
        let d = perpendicular_distance_to_polyline(&poly, 500.0, 50.0);
        assert!((d - 50.0).abs() < 1e-3, "expected 50 m, got {d}");
    }

    #[test]
    fn wrap_aware_polyline_picks_short_path() {
        // Hub at literal (100, 0), neighbor at (WORLD_EXTENT_M - 100, 0).
        // Straight literal distance is ~5 Mm; wrap distance is 200 m.
        let a = (100.0, 0.0);
        let b = (WORLD_EXTENT_M - 100.0, 0.0);
        let poly = straight_polyline_wrap_aware(a, b);
        let length: f32 = poly
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .sum();
        assert!(
            length < 1_000.0,
            "wrap-aware polyline should be short, got {length} m"
        );
    }

    #[test]
    fn toroidal_distance_handles_wrap_seam() {
        let a = (100.0, 0.0);
        let b = (WORLD_EXTENT_M - 100.0, 0.0);
        assert!((toroidal_distance_m(a, b) - 200.0).abs() < 1e-3);
    }
}
