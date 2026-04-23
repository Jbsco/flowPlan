//! Hydrology graph (Phase 3.6.1d).
//!
//! Pre-baked, read-only spatial water classification + per-water-body
//! surface elevation. Computed once at world-load from the L1 terrain
//! sampler; queried in O(1) thereafter via [`HydrologyGraph::water_surface_at`].
//!
//! # Architecture
//!
//! Hydrology is **global** — rivers can't decide downhill from local
//! information. We address this by baking once at session load:
//!
//! 1. **Coarse heightfield** at [`BAKE_RESOLUTION_M`] (2.5 km / cell,
//!    [`BAKE_GRID_N`]² = 4 M cells over the 5000 km world). Sampled via
//!    [`Terrain::height_at_scale`] with `min_spacing_m = BAKE_RESOLUTION_M`
//!    so the bake gets mipmap-correct (anti-aliased) elevations rather
//!    than the per-pixel noise speckle that finer grids would alias.
//! 2. **Priority-flood** (Barnes/Lehman/Mulla 2014, "Priority-flood: an
//!    optimal depression-filling and watershed-labeling algorithm"). One
//!    pass O(N log N) yields per-cell `(filled_elevation, d8_flow_dir,
//!    basin_id)`. The filled elevation is the spillway level — water in
//!    a basin would brim over to this level absent evaporation.
//! 3. **Flow accumulation** in topological order, weighted by per-cell
//!    `precip_index` (a per-biome dimensionless value 0.15-1.0). Each
//!    cell adds its weighted precipitation to its downstream neighbor;
//!    sinks accumulate the catchment-wide inflow.
//! 4. **Wetness equilibrium** (the real-world-possible gate applied to
//!    hydrology): per-basin water surface area `A_w = inflow / evap_index`.
//!    Solving `area_below(h) == A_w` against the basin's hypsometric
//!    curve gives the equilibrium fill elevation, capped at the spillway.
//!    Replaces the naive "always fill to spillway" — desert basins sit
//!    mostly dry, alpine cirques brim, coastal basins overflow into
//!    rivers, all from the seed.
//! 5. **River / creek / dry-channel classification** by tracking residual
//!    flow along the downstream walk. River = flow survives channel
//!    evaporation with margin. Creek = thinner perennial. DryChannel =
//!    drainage path with no surface water (wadi / wash geometry only).
//! 6. **Coastline polyline** = sea-level contour via Marching Squares on
//!    the coarse grid.
//!
//! # Toroidal wrap
//!
//! Cell-index neighbor lookups go through [`wrap_cell_index`] — basins,
//! flow paths, and rivers cross the world seam transparently. A basin
//! that wraps the seam appears as one connected component in the graph
//! even though a flat-map projection would show it as two regions.
//!
//! # Determinism
//!
//! Pure function of `(master_seed, terrain)`. Bit-identical bake on every
//! machine that produces bit-identical terrain output. Folded into the
//! determinism contract via the bake-hash invariant.
//!
//! # API
//!
//! Consumers query [`HydrologyGraph::water_surface_at`] at any world
//! coordinate. The existing `world::water_height` becomes a thin wrapper
//! that adds the chop term on top of the surface elevation. Buoyancy /
//! flight model inherit lake-elevation correctness without changes.

// Hydrology is a math-heavy module with many intentional f32<->usize<->u32
// casts for grid indexing, doc paragraphs describing multi-step algorithms,
// and heap/queue entries with single-char names (a, b, c) for algorithm
// clarity. Allowed at module level to match the existing convention in
// `noise.rs` / `terrain.rs` / `weather/*.rs`.
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::similar_names)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::too_long_first_doc_paragraph)]
#![allow(clippy::mismatching_type_param_order)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::match_same_arms)]

use core::cmp::Ordering;

use flowplan_core::{wrap_cell_index, WORLD_EXTENT_M};

use crate::biome::{biome_at, BiomeClass};
use crate::terrain::Terrain;

// ═══════════════════════════════════════════════════════════════════════════
// Bake resolution constants (Phase 3.6.1d initial calibration)
// ═══════════════════════════════════════════════════════════════════════════

/// Coarse-heightfield cell size for the hydrology bake (m).
///
/// 2500 m chosen as the starting calibration: 2000² = 4 M cells over a
/// 5000 km world, ~64 MB at 4 floats per cell, ~1 s priority-flood
/// runtime. Tighter (1.25 km, 16M cells) gives cleaner river polylines
/// at the player crop but eats RAM. Looser (5 km, 1M cells) is faster
/// but rivers look chunky. Tuning happens via visual-bench artifacts.
pub const BAKE_RESOLUTION_M: f32 = 2_500.0;

/// Coarse-grid side length in cells. Must satisfy
/// `BAKE_GRID_N × BAKE_RESOLUTION_M == WORLD_EXTENT_M` exactly so the
/// grid wraps cleanly at the toroidal seam.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub const BAKE_GRID_N: usize = (WORLD_EXTENT_M / BAKE_RESOLUTION_M) as usize;

const _: () = assert!(BAKE_GRID_N * (BAKE_RESOLUTION_M as usize) == WORLD_EXTENT_M as usize);

/// Total coarse-grid cell count.
pub const BAKE_CELL_COUNT: usize = BAKE_GRID_N * BAKE_GRID_N;

/// Sea-level reference elevation (m). Cells at-or-below this are
/// classified as part of the world ocean basin (basin id 0). Matches
/// the global y=0 water plane used by the existing render + buoyancy.
pub const SEA_LEVEL_M: f32 = 0.0;

/// Sea-basin sentinel id. Basin 0 is reserved for the world ocean (the
/// connected union of all sea-level cells across the seam-wrap).
pub const SEA_BASIN_ID: u32 = 0;

/// Minimum spillway-vs-sink elevation drop to classify a basin as
/// holding a "lake" (m). Smaller drops are interpreted as "drainage
/// passes through a low spot" and produce no lake — only the river /
/// creek along the bottom (memory: `aircraft_refit_emergent_gameplay`
/// — dry biomes are first-class).
pub const MIN_LAKE_DEPTH_M: f32 = 5.0;

/// Minimum basin area (in coarse cells) to classify as a lake at all.
/// 4 cells × 6.25 km² = 25 km² area floor. Smaller depressions count
/// as "puddles" and don't get a `LakeBasin` entry.
pub const MIN_LAKE_AREA_CELLS: u32 = 4;

/// River classification threshold (dimensionless flow, post-evaporation).
///
/// Cells whose downstream-walk residual exceeds this are perennial
/// rivers. Calibrated to ~500 cells of upstream catchment × average
/// precip index ≈ 500 × 0.5 = 250; scale up for "named river on a
/// map" density (~thousands of km² catchment). Rivers covering more
/// than ~1-2% of world cells is a warning sign.
pub const RIVER_FLOW_THRESHOLD: f32 = 400.0;

/// Creek vs dry-channel boundary (dimensionless residual flow).
///
/// ~100 cells × 0.5 precip average = 50 residual floor. Creeks
/// should cover ~2-4% of cells (dense network), rivers ~0.5-1%.
pub const RIVER_TO_CREEK_THRESHOLD: f32 = 40.0;

/// Per-cell evaporation in the path-integrated residual-flow pass.
///
/// At each cell on a downstream walk, residual loses
/// `CHANNEL_EVAP_PER_CELL × biome.evap_index` to evaporation. Tuned
/// so high-desert (evap=1.0, precip=0.15) loses net flow per cell
/// (0.25 > 0.15 → shrinking) while coastal-boreal (evap=0.3,
/// precip=1.0) gains per cell (0.075 < 1.0 → growing). This is what
/// produces the DryChannel classification — desert drainage paths
/// that exist topologically but evaporate before reaching the sink.
pub const CHANNEL_EVAP_PER_CELL: f32 = 0.25;

/// Hydrology subsystem tag for future per-subsystem seed derivation
/// (parallel to `weather::weather_seed`). Reserved.
#[allow(dead_code)]
const HYDROLOGY_TAG: u64 = u64::from_le_bytes(*b"hydrolog");

// ═══════════════════════════════════════════════════════════════════════════
// Per-biome hydrology priors (precip + evap dimensionless indices)
// ═══════════════════════════════════════════════════════════════════════════

/// Per-biome precipitation + evaporation indices for the wetness model.
///
/// **Precipitation** is dimensionless 0..1, scaling the per-cell water
/// inflow. **Evaporation** is dimensionless 0..1+, scaling per-unit-area
/// surface evaporation. The ratio precip/evap is what determines lake
/// fill: high ratio → brimming, low ratio → dry.
///
/// Calibrated to match real-world catchment ratios per
/// [MAPGEN.md § Altitude envelope](../../../MAPGEN.md#altitude-envelope--real-world-possible-gate)
/// + the `aircraft_refit_emergent_gameplay` design memory (dry biomes
/// are first-class destinations, not under-served).
#[derive(Copy, Clone, Debug)]
pub struct BiomeHydrology {
    /// Precipitation rate index (dimensionless, ~0.1 desert to ~1.0 boreal).
    pub precip_index: f32,
    /// Evaporation rate index (dimensionless, ~0.3 boreal to ~1.0 desert).
    pub evap_index: f32,
}

impl BiomeHydrology {
    /// Look up per-biome hydrology priors. Returns the same `BiomeClass`
    /// values used by [`crate::biome::biome_at`].
    #[must_use]
    pub const fn for_class(class: BiomeClass) -> Self {
        match class {
            // High desert: Colorado Plateau / Grand Mesa region. Mostly
            // dry channels + ephemeral wadis; rare filled basins where
            // catchment is huge or biome humidity locally above floor.
            BiomeClass::HighDesert => Self {
                precip_index: 0.15,
                evap_index: 1.0,
            },
            // Rocky alpine: tarns brim, perennial rivers, cirque lakes.
            BiomeClass::RockyAlpine => Self {
                precip_index: 0.7,
                evap_index: 0.4,
            },
            // Coastal boreal: many full lakes, dense river network,
            // fjord-fed deep basins. Floatplane home turf.
            BiomeClass::CoastalBoreal => Self {
                precip_index: 1.0,
                evap_index: 0.3,
            },
            // Mediterranean archipelago: mid-fill lakes, perennial
            // coastal rivers, dry-summer creeks inland.
            BiomeClass::MediterraneanArchipelago => Self {
                precip_index: 0.5,
                evap_index: 0.7,
            },
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Public types — query-facing
// ═══════════════════════════════════════════════════════════════════════════

/// Spatial water classification at a world coordinate. See
/// [`HydrologyGraph::water_surface_at`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum WaterClass {
    /// Open ocean — large connected sea-basin areas far from coast.
    OpenOcean,
    /// Open coastal water — sea-basin within ~30 km of coastline.
    OpenCoastal,
    /// Protected bay / fjord — sea-basin substantially enclosed by terrain
    /// (lower fetch, calmer chop). Exact criterion is bay-enclosure
    /// fraction; placeholder until 3.6.2 cell partition lands.
    ProtectedBay,
    /// Large inland lake (≥ 25 km² × ~5× threshold ≈ 125 km²).
    LakeLarge,
    /// Small inland lake (25-125 km²).
    LakeSmall,
    /// Alpine tarn — small lake at altitude > 1500 m.
    AlpineTarn,
    /// Closed basin lake (no spillway — endorheic). Salt / brackish
    /// flavor candidate at L3 palette adaptation.
    ClosedBasinLake,
    /// Perennial river — channel with strong residual flow after
    /// per-cell evaporation losses.
    River,
    /// Perennial creek — thinner channel with positive but smaller
    /// residual flow.
    Creek,
    /// Dry drainage channel (wadi / wash) — flow path exists in the
    /// graph but evaporation cancels surface water; renders as
    /// terrain-only feature in 3.6.3.
    DryChannel,
    /// Delta / alluvial fan — river-mouth zone where flow transitions
    /// from confined channel to sea / lake outflow. Detected as
    /// River / Creek cells whose downstream is sea or lake, plus a
    /// short upstream fan footprint. Rendered in 3.6.3 as braided
    /// channels + sediment sandbars (per user design insight that the
    /// visible D8 "parallel-diagonal" artifact in flat regions is a
    /// geometric signature worth leaning INTO rather than smoothing
    /// away).
    DeltaFan,
}

/// Water-surface descriptor returned by [`HydrologyGraph::water_surface_at`].
#[derive(Copy, Clone, Debug)]
pub struct WaterSurface {
    /// Water classification.
    pub class: WaterClass,
    /// Surface elevation (m). Sea = [`SEA_LEVEL_M`] (= 0); lake =
    /// equilibrium fill elevation; river / creek = ground + small ε
    /// offset (set by the consumer, usually rendered as terrain-flush).
    pub elevation_m: f32,
    /// Chop amplitude scale (multiplies the global Gerstner / FFT
    /// amplitude). 1.0 baseline. 0.0 for rivers (no waves; flow
    /// shader handles motion). Lakes typically 0.3-0.7 (less fetch
    /// than open sea); open ocean 1.0+ depending on weather coupling.
    pub chop_amplitude_scale: f32,
    /// Body identifier within its class. Sea uses [`SEA_BASIN_ID`]
    /// (= 0); lakes get sequential ids; rivers get sequential ids.
    /// `None` for `DryChannel` (no surface water → no body id).
    pub body_id: Option<u32>,
}

/// Lake basin entry in the hydrology graph.
#[derive(Clone, Debug)]
pub struct LakeBasin {
    /// Basin id (matches the `basin_id` field of cells inside it).
    pub id: u32,
    /// Equilibrium water surface elevation (m).
    pub surface_elevation_m: f32,
    /// Spillway elevation (m). Equal to `surface_elevation_m` if the
    /// basin overflows; higher if wetness equilibrium leaves the lake
    /// below the spillway (closed-basin or low-precipitation case).
    pub spillway_elevation_m: f32,
    /// Sink (lowest cell) elevation (m).
    pub sink_elevation_m: f32,
    /// Total basin footprint area in coarse cells.
    pub area_cells: u32,
    /// Water surface area in coarse cells (≤ `area_cells`).
    pub water_area_cells: u32,
    /// Closed (endorheic) basin — no spillway exists; basin can only
    /// lose water via evaporation. Tagged for L3 palette adaptation
    /// (salt / brackish lakes).
    pub closed_basin: bool,
    /// Classification (which `WaterClass` variant this basin produces).
    pub class: WaterClass,
}

/// Polyline-style river edge entry. Built by walking downstream from
/// each river-classified source cell until reaching sea, lake, or
/// merging with another river.
#[derive(Clone, Debug)]
pub struct RiverEdge {
    /// River id (matches the cell metadata).
    pub id: u32,
    /// Polyline waypoints in world coordinates `(x_m, z_m, elevation_m)`.
    /// At least 2 entries (source + downstream).
    pub waypoints: Vec<(f32, f32, f32)>,
    /// Average residual flow along the polyline (dimensionless).
    /// Drives river width at render time.
    pub flow: f32,
    /// Classification — `River` or `Creek`.
    pub class: WaterClass,
}

/// Polyline coastline segment.
#[derive(Clone, Debug)]
pub struct CoastlineSegment {
    /// Vertices in world coordinates.
    pub vertices: Vec<(f32, f32)>,
}

/// Hydrology graph baked from terrain at world-load time. Read-only
/// after construction; safe to share across threads.
#[derive(Clone, Debug)]
pub struct HydrologyGraph {
    /// Master seed used for the bake.
    seed: u64,
    /// Coarse-grid filled-elevation per cell (m). Length [`BAKE_CELL_COUNT`].
    /// "Filled" = pit-filled to the basin's spillway elevation, so cells
    /// below the spillway get the spillway level here. Used for downhill
    /// flow direction lookups.
    filled_elevation_m: Box<[f32]>,
    /// True (raw) terrain elevation per cell (m). Length [`BAKE_CELL_COUNT`].
    /// Pre-fill — used for hypsometry + visualization.
    raw_elevation_m: Box<[f32]>,
    /// D8 downstream cell index per cell (`u32::MAX` = no downstream
    /// neighbor; only happens for the global ocean sentinel sink).
    d8_downstream: Box<[u32]>,
    /// Per-cell basin membership. Sea cells = [`SEA_BASIN_ID`].
    cell_basin: Box<[u32]>,
    /// Per-cell flow accumulation (weighted by upstream precip indices).
    flow_accumulation: Box<[f32]>,
    /// Per-cell residual flow (after evaporation along the downstream
    /// walk). Drives river / creek / dry-channel classification.
    residual_flow: Box<[f32]>,
    /// Per-cell water classification (None = dry land).
    cell_water_class: Box<[Option<WaterClass>]>,
    /// Per-cell river edge id (`u32::MAX` if the cell isn't on a river).
    cell_river_id: Box<[u32]>,
    /// Lake basin catalog. Index = `LakeBasin::id`.
    lake_basins: Vec<LakeBasin>,
    /// River edge catalog. Index = `RiverEdge::id`.
    river_edges: Vec<RiverEdge>,
    /// Coastline polyline segments at sea level.
    coastline_segments: Vec<CoastlineSegment>,
}

impl HydrologyGraph {
    /// Master seed used to bake this graph.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Number of lake basins in the catalog (excludes the world ocean).
    #[must_use]
    pub fn lake_count(&self) -> usize {
        self.lake_basins.len()
    }

    /// Number of river edges in the catalog.
    #[must_use]
    pub fn river_count(&self) -> usize {
        self.river_edges.len()
    }

    /// All lake basins.
    #[must_use]
    pub fn lakes(&self) -> &[LakeBasin] {
        &self.lake_basins
    }

    /// All river edges.
    #[must_use]
    pub fn rivers(&self) -> &[RiverEdge] {
        &self.river_edges
    }

    /// All coastline polyline segments.
    #[must_use]
    pub fn coastline_segments(&self) -> &[CoastlineSegment] {
        &self.coastline_segments
    }

    /// Per-cell raw terrain elevation array (length [`BAKE_CELL_COUNT`]).
    /// Used by visual-bench overlays + tests; not part of the runtime
    /// query API.
    #[must_use]
    pub fn raw_elevation_grid(&self) -> &[f32] {
        &self.raw_elevation_m
    }

    /// Per-cell flow-accumulation array. Used by visual-bench overlays.
    #[must_use]
    pub fn flow_accumulation_grid(&self) -> &[f32] {
        &self.flow_accumulation
    }

    /// Per-cell residual-flow array. Used by visual-bench overlays.
    #[must_use]
    pub fn residual_flow_grid(&self) -> &[f32] {
        &self.residual_flow
    }

    /// Per-cell basin id array. Used by visual-bench overlays.
    #[must_use]
    pub fn basin_grid(&self) -> &[u32] {
        &self.cell_basin
    }

    /// Per-cell D8 downstream-cell index array. Used by visual-bench
    /// arrow overlays. `u32::MAX` = no downstream (sea-basin sentinel).
    #[must_use]
    pub fn d8_downstream_grid(&self) -> &[u32] {
        &self.d8_downstream
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Cell-index arithmetic + neighbor lookup
// ═══════════════════════════════════════════════════════════════════════════

/// Convert a coarse-grid `(ix, iz)` pair to a flat array index.
#[inline]
#[must_use]
const fn flatten(ix: usize, iz: usize) -> usize {
    iz * BAKE_GRID_N + ix
}

/// Convert a flat array index back to `(ix, iz)`.
#[inline]
#[must_use]
const fn unflatten(idx: usize) -> (usize, usize) {
    (idx % BAKE_GRID_N, idx / BAKE_GRID_N)
}

/// World coordinate at the center of coarse cell `(ix, iz)`. Returns
/// `(x_m, z_m)` in `[0, WORLD_EXTENT_M)`.
#[inline]
#[must_use]
#[allow(clippy::cast_precision_loss)]
fn cell_center_world(ix: usize, iz: usize) -> (f32, f32) {
    let x = (ix as f32 + 0.5) * BAKE_RESOLUTION_M;
    let z = (iz as f32 + 0.5) * BAKE_RESOLUTION_M;
    (x, z)
}

/// World-coordinate `(x, z)` to flat coarse-grid index. Wraps at the
/// toroidal seam.
#[inline]
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn world_to_cell_index(x: f32, z: f32) -> usize {
    let x_wrapped = flowplan_core::wrap_coord(x);
    let z_wrapped = flowplan_core::wrap_coord(z);
    let ix = ((x_wrapped / BAKE_RESOLUTION_M) as usize).min(BAKE_GRID_N - 1);
    let iz = ((z_wrapped / BAKE_RESOLUTION_M) as usize).min(BAKE_GRID_N - 1);
    flatten(ix, iz)
}

/// 8-connected neighbor offsets in `(dix, diz)` form. Order matters
/// only for tie-breaking determinism in priority-flood.
const D8_OFFSETS: [(i64, i64); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// 8-connected neighbor flat indices for cell `(ix, iz)`. Wraps at the
/// toroidal seam via [`wrap_cell_index`].
#[inline]
#[must_use]
#[allow(clippy::cast_possible_wrap)]
fn d8_neighbors(ix: usize, iz: usize) -> [usize; 8] {
    let n = BAKE_GRID_N as i64;
    let mut out = [0_usize; 8];
    for (k, (dix, diz)) in D8_OFFSETS.iter().enumerate() {
        let nx = wrap_cell_index(ix as i64 + *dix, n) as usize;
        let nz = wrap_cell_index(iz as i64 + *diz, n) as usize;
        out[k] = flatten(nx, nz);
    }
    out
}

/// Squared world-distance between cell centers `a` and `b`, accounting
/// for toroidal wrap (the closer-of-two-paths distance per axis). Used
/// to compute D8 slope correctly across the seam. Reserved for future
/// slope-weighted flow accumulation if flow direction heuristics need
/// refining.
#[inline]
#[must_use]
#[allow(dead_code, clippy::cast_precision_loss)]
fn wrapped_squared_distance_m2(a: usize, b: usize) -> f32 {
    let (ax, az) = unflatten(a);
    let (bx, bz) = unflatten(b);
    let n_half = (BAKE_GRID_N / 2) as i64;
    let mut dx = (ax as i64 - bx as i64).abs();
    let mut dz = (az as i64 - bz as i64).abs();
    if dx > n_half {
        dx = BAKE_GRID_N as i64 - dx;
    }
    if dz > n_half {
        dz = BAKE_GRID_N as i64 - dz;
    }
    let dx_m = dx as f32 * BAKE_RESOLUTION_M;
    let dz_m = dz as f32 * BAKE_RESOLUTION_M;
    dx_m * dx_m + dz_m * dz_m
}

// ═══════════════════════════════════════════════════════════════════════════
// Min-heap entry for priority-flood (lowest elevation first; cell index
// breaks ties for determinism)
// ═══════════════════════════════════════════════════════════════════════════

/// Heap entry: `(elevation, cell_index)`. Ordered by elevation ascending,
/// then cell index ascending — both ascending, but Rust's `BinaryHeap` is
/// max-heap, so we negate by wrapping in `Reverse`.
#[derive(Copy, Clone, Debug, PartialEq)]
struct HeapEntry {
    elevation_m: f32,
    cell: u32,
}

impl Eq for HeapEntry {}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Total order: f32 NaN-safe via to_bits-after-canonicalization.
        // We never produce NaN elevations (terrain envelope clamps them),
        // but stay defensive: convert to integer bits for comparison.
        let a = self.elevation_m.to_bits();
        let b = other.elevation_m.to_bits();
        a.cmp(&b).then(self.cell.cmp(&other.cell))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Bake — the one-time graph construction
// ═══════════════════════════════════════════════════════════════════════════

impl HydrologyGraph {
    /// Bake the hydrology graph for the given seed + terrain.
    ///
    /// Cost: ~1 s on a modern CPU (priority-flood dominates; topological
    /// accumulation is O(N); wetness equilibrium is O(N) amortized).
    /// Memory: ~64 MB for the per-cell grids at 2.5 km resolution.
    #[must_use]
    pub fn bake(master_seed: u64, terrain: &Terrain) -> Self {
        let mut g = Self::empty(master_seed);

        // Step 1: coarse heightfield bake. Sample at cell centers using
        // mipmap-aware terrain so we get anti-aliased elevations
        // appropriate for 2.5 km cells.
        for iz in 0..BAKE_GRID_N {
            for ix in 0..BAKE_GRID_N {
                let (x, z) = cell_center_world(ix, iz);
                let h = terrain.height_at_scale(x, z, BAKE_RESOLUTION_M);
                g.raw_elevation_m[flatten(ix, iz)] = h;
                g.filled_elevation_m[flatten(ix, iz)] = h;
            }
        }

        // Step 2: priority-flood (Barnes/Lehman/Mulla 2014).
        priority_flood_d8(&mut g);

        // Step 3: flow accumulation in elevation-sorted order.
        flow_accumulate(&mut g, terrain);

        // Step 4: wetness equilibrium fill per non-sea basin.
        wetness_fill(&mut g, terrain);

        // Step 5: river / creek / dry-channel classification +
        // polyline extraction.
        classify_channels(&mut g);

        // Step 6: coastline polyline extraction at sea level.
        extract_coastline(&mut g);

        g
    }

    /// Construct an empty graph with all per-cell arrays sized.
    fn empty(seed: u64) -> Self {
        let zeros_f32 = vec![0.0_f32; BAKE_CELL_COUNT].into_boxed_slice();
        let zeros_u32 = vec![0_u32; BAKE_CELL_COUNT].into_boxed_slice();
        let max_u32 = vec![u32::MAX; BAKE_CELL_COUNT].into_boxed_slice();
        let nones = vec![None; BAKE_CELL_COUNT].into_boxed_slice();
        Self {
            seed,
            filled_elevation_m: zeros_f32.clone(),
            raw_elevation_m: zeros_f32.clone(),
            d8_downstream: max_u32.clone(),
            cell_basin: zeros_u32.clone(),
            flow_accumulation: zeros_f32.clone(),
            residual_flow: zeros_f32,
            cell_water_class: nones,
            cell_river_id: max_u32,
            lake_basins: Vec::new(),
            river_edges: Vec::new(),
            coastline_segments: Vec::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Priority-flood + D8 flow direction
// ═══════════════════════════════════════════════════════════════════════════

/// Barnes/Lehman/Mulla 2014 priority-flood. Single pass over the coarse
/// grid: seeds the min-heap with all sea cells (raw_elevation ≤
/// [`SEA_LEVEL_M`]) as sinks, then floods outward assigning every
/// non-sea cell a downstream-pointer + pit-filled elevation. The pit-
/// fill step lifts any depression's elevation to its spillway level so
/// the flow direction field is well-defined even inside closed pits.
///
/// Post-pass invariants:
/// - Every cell has `d8_downstream` set (except sea cells, which get
///   `u32::MAX` as the "drains to the global ocean" sentinel).
/// - `filled_elevation_m[c] >= raw_elevation_m[c]` for all cells; strict
///   inequality iff cell is inside a pit that got lifted to the spillway.
/// - Walking downstream from any cell eventually reaches a sea cell.
/// - Every non-sea cell gets basin_id [`SEA_BASIN_ID`] in this pass;
///   lake-basin identification runs in a second pass against cells
///   where `filled > raw`.
fn priority_flood_d8(g: &mut HydrologyGraph) {
    use std::collections::BinaryHeap;

    let mut processed = vec![false; BAKE_CELL_COUNT].into_boxed_slice();
    // Pit queue — FIFO for cells currently being lifted. Using a Vec as
    // a simple deque; push_back = push, pop_front via swap-remove-0 is
    // O(N); instead use an index into a VecDeque.
    let mut pit_queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    // Min-heap: use std::cmp::Reverse to flip BinaryHeap's max-ordering.
    let mut open: BinaryHeap<std::cmp::Reverse<HeapEntry>> = BinaryHeap::new();

    // Seed: every sea cell (raw elevation ≤ SEA_LEVEL_M) becomes a sink.
    // Sea cells' d8_downstream stays u32::MAX (they are the terminal
    // outflow). filled_elevation for sea stays at raw (don't lift sea).
    for cell in 0..BAKE_CELL_COUNT {
        if g.raw_elevation_m[cell] <= SEA_LEVEL_M {
            processed[cell] = true;
            g.cell_basin[cell] = SEA_BASIN_ID;
            g.d8_downstream[cell] = u32::MAX;
            open.push(std::cmp::Reverse(HeapEntry {
                elevation_m: g.raw_elevation_m[cell],
                #[allow(clippy::cast_possible_truncation)]
                cell: cell as u32,
            }));
        }
    }

    // Defensive: if the world has no sea at all (unlikely given our
    // altitude envelope), priority-flood still needs a seed. Fallback:
    // seed from the single lowest cell in the grid. Treat it as a
    // faux-sea sink.
    if open.is_empty() {
        let mut lowest_cell = 0_usize;
        let mut lowest_elev = f32::INFINITY;
        for cell in 0..BAKE_CELL_COUNT {
            if g.raw_elevation_m[cell] < lowest_elev {
                lowest_elev = g.raw_elevation_m[cell];
                lowest_cell = cell;
            }
        }
        processed[lowest_cell] = true;
        g.cell_basin[lowest_cell] = SEA_BASIN_ID;
        g.d8_downstream[lowest_cell] = u32::MAX;
        open.push(std::cmp::Reverse(HeapEntry {
            elevation_m: lowest_elev,
            #[allow(clippy::cast_possible_truncation)]
            cell: lowest_cell as u32,
        }));
    }

    // Main loop: drain Pit first (preserves pit-fill plateau order),
    // then Open (elevation-sorted).
    loop {
        let (c_cell, c_filled) = if let Some(pit_cell) = pit_queue.pop_front() {
            (pit_cell as usize, g.filled_elevation_m[pit_cell as usize])
        } else if let Some(std::cmp::Reverse(entry)) = open.pop() {
            (entry.cell as usize, entry.elevation_m)
        } else {
            break;
        };

        let (cix, ciz) = unflatten(c_cell);
        for n_cell in d8_neighbors(cix, ciz) {
            if processed[n_cell] {
                continue;
            }
            processed[n_cell] = true;
            g.cell_basin[n_cell] = SEA_BASIN_ID;
            #[allow(clippy::cast_possible_truncation)]
            {
                g.d8_downstream[n_cell] = c_cell as u32;
            }
            let n_raw = g.raw_elevation_m[n_cell];
            if n_raw < c_filled {
                // Pit: lift n's filled elevation up to c's plateau.
                g.filled_elevation_m[n_cell] = c_filled;
                #[allow(clippy::cast_possible_truncation)]
                pit_queue.push_back(n_cell as u32);
            } else {
                g.filled_elevation_m[n_cell] = n_raw;
                open.push(std::cmp::Reverse(HeapEntry {
                    elevation_m: n_raw,
                    #[allow(clippy::cast_possible_truncation)]
                    cell: n_cell as u32,
                }));
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Flow accumulation
// ═══════════════════════════════════════════════════════════════════════════

/// Accumulate flow upstream-to-downstream via Kahn's topological sort.
/// Each cell contributes its own per-cell precipitation (weighted by
/// per-cell biome precip_index) + all received upstream contributions,
/// then pushes its total to its downstream neighbor.
///
/// Correctness relies on topological ordering, not elevation ordering —
/// pit-filled flat regions produce equal filled_elevation ties that an
/// elevation sort handles wrong (sibling cells at the same level that
/// flow INTO each other are ordered arbitrarily). Kahn's uses
/// upstream-reference-count to guarantee every cell is processed AFTER
/// all its direct upstream contributors, regardless of elevation.
///
/// Output `flow_accumulation[c]` = total weighted precipitation falling
/// on the catchment area upstream of (and including) cell `c`.
fn flow_accumulate(g: &mut HydrologyGraph, terrain: &Terrain) {
    let _ = terrain; // biome classification doesn't need terrain directly

    // Per-cell precip index (lookup biome at cell center).
    let mut precip = vec![0.0_f32; BAKE_CELL_COUNT].into_boxed_slice();
    for cell in 0..BAKE_CELL_COUNT {
        let (ix, iz) = unflatten(cell);
        let (x, z) = cell_center_world(ix, iz);
        let class = biome_at(g.seed, x, z);
        precip[cell] = BiomeHydrology::for_class(class).precip_index;
    }

    // Initialize accumulation to each cell's own precip.
    for cell in 0..BAKE_CELL_COUNT {
        g.flow_accumulation[cell] = precip[cell];
    }

    // Count upstream contributors per cell (cells that flow INTO this
    // cell directly).
    let mut upstream_count = vec![0_u32; BAKE_CELL_COUNT].into_boxed_slice();
    for cell in 0..BAKE_CELL_COUNT {
        let down = g.d8_downstream[cell];
        if down != u32::MAX {
            upstream_count[down as usize] += 1;
        }
    }

    // Kahn's: seed queue with cells that have 0 upstream contributors
    // (peaks / isolated cells). Process FIFO so cells with lower
    // upstream-count finish before cells with more.
    let mut queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    for cell in 0..BAKE_CELL_COUNT {
        if upstream_count[cell] == 0 {
            #[allow(clippy::cast_possible_truncation)]
            queue.push_back(cell as u32);
        }
    }

    while let Some(cell_u32) = queue.pop_front() {
        let cell = cell_u32 as usize;
        let down = g.d8_downstream[cell];
        if down != u32::MAX {
            let down_usize = down as usize;
            // Push c's accumulation into down's accumulation.
            g.flow_accumulation[down_usize] += g.flow_accumulation[cell];
            // Decrement down's upstream count; enqueue when it hits 0.
            upstream_count[down_usize] -= 1;
            if upstream_count[down_usize] == 0 {
                queue.push_back(down);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Wetness equilibrium fill
// ═══════════════════════════════════════════════════════════════════════════

/// Identify lake basins + compute equilibrium fill per basin.
///
/// A **lake basin candidate** is a contiguous connected component of
/// cells where `filled_elevation > raw_elevation + MIN_LAKE_DEPTH_M`
/// (cell would be underwater if filled to spillway). For each candidate:
///
/// 1. Compute inflow = `Σ flow_accumulation[c]` over cells that drain
///    INTO the basin from outside (detected as cells with downstream
///    inside the basin but themselves outside). Equivalently, the
///    accumulation at the basin's sink.
/// 2. Compute average evaporation index across the basin's cells (biome-
///    weighted).
/// 3. Solve `area_below(h) = inflow / evap_avg` for h by binary-searching
///    the basin's hypsometric curve (sorted cell elevations).
/// 4. Cap h at the spillway; tag `closed_basin` if the equilibrium
///    surface area is less than the full footprint (inflow insufficient
///    to fill).
///
/// Writes:
/// - `cell_basin[c]` = basin id for cells in the water surface
/// - `cell_water_class[c]` = lake-variant for cells in the water surface
/// - Appends to `lake_basins` with full metadata.
fn wetness_fill(g: &mut HydrologyGraph, terrain: &Terrain) {
    let _ = terrain;
    // Step 1: find pit regions (cells where filled > raw by ≥ tiny
    // epsilon). Flood-fill to group them into connected components.
    let mut region_id = vec![u32::MAX; BAKE_CELL_COUNT].into_boxed_slice();
    let mut next_region: u32 = 0;
    let mut regions_cells: Vec<Vec<u32>> = Vec::new();
    let fill_eps = 0.01_f32; // 1 cm tolerance for "cell got lifted"

    for start in 0..BAKE_CELL_COUNT {
        let lifted = g.filled_elevation_m[start] > g.raw_elevation_m[start] + fill_eps;
        if !lifted || region_id[start] != u32::MAX {
            continue;
        }
        // BFS from this cell across contiguous lifted cells.
        let rid = next_region;
        next_region += 1;
        let mut cells: Vec<u32> = Vec::new();
        let mut queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
        #[allow(clippy::cast_possible_truncation)]
        {
            queue.push_back(start as u32);
        }
        region_id[start] = rid;
        while let Some(c_u32) = queue.pop_front() {
            let c = c_u32 as usize;
            cells.push(c_u32);
            let (ix, iz) = unflatten(c);
            for n in d8_neighbors(ix, iz) {
                if region_id[n] != u32::MAX {
                    continue;
                }
                let n_lifted = g.filled_elevation_m[n] > g.raw_elevation_m[n] + fill_eps;
                if !n_lifted {
                    continue;
                }
                region_id[n] = rid;
                #[allow(clippy::cast_possible_truncation)]
                queue.push_back(n as u32);
            }
        }
        regions_cells.push(cells);
    }

    // Step 2: per region, compute metadata + equilibrium fill.
    for (rid_usize, cells) in regions_cells.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let rid = rid_usize as u32 + 1; // reserve 0 for sea
        if cells.len() < MIN_LAKE_AREA_CELLS as usize {
            continue; // too small to be a lake
        }

        // Sink + spillway + hypsometric curve.
        let mut sink_elev = f32::INFINITY;
        let mut spillway = f32::NEG_INFINITY;
        let mut evap_sum: f32 = 0.0;
        let mut cell_elevs: Vec<f32> = Vec::with_capacity(cells.len());
        for &c_u32 in cells {
            let c = c_u32 as usize;
            let raw = g.raw_elevation_m[c];
            let filled = g.filled_elevation_m[c];
            if raw < sink_elev {
                sink_elev = raw;
            }
            if filled > spillway {
                spillway = filled;
            }
            cell_elevs.push(raw);
            // Each basin cell contributes its own precip + any inflow
            // from upstream. `flow_accumulation` captures this directly
            // for the basin's sink cell (the lowest cell in the basin
            // receives everything). Summing across basin cells would
            // double-count; instead, take the SINK's accumulation.
            let (ix, iz) = unflatten(c);
            let (x, z) = cell_center_world(ix, iz);
            let h = BiomeHydrology::for_class(biome_at(g.seed, x, z));
            evap_sum += h.evap_index;
        }
        // Inflow = accumulation at the basin's sink (receives all
        // upstream contributions).
        // Find the cell with min raw elevation inside the basin.
        let mut sink_cell = cells[0] as usize;
        let mut sink_raw = g.raw_elevation_m[sink_cell];
        for &c_u32 in cells {
            let c = c_u32 as usize;
            if g.raw_elevation_m[c] < sink_raw {
                sink_raw = g.raw_elevation_m[c];
                sink_cell = c;
            }
        }
        let inflow: f32 = g.flow_accumulation[sink_cell];

        #[allow(clippy::cast_precision_loss)]
        let evap_avg = evap_sum / cells.len() as f32;

        // Equilibrium water surface area (in cells):
        //   inflow = evap_avg × area × cell_area_m2 × some scale factor
        // But our units are dimensionless precip / evap. Treat the
        // equilibrium as: inflow (summed precip units, one per upstream
        // cell) = evap_avg × surface_cells. Solve:
        //   surface_cells = inflow / evap_avg
        // Capped at basin footprint.
        let surface_cells_float = if evap_avg > 1e-4 {
            (inflow / evap_avg).max(0.0)
        } else {
            f32::INFINITY
        };

        // Find equilibrium fill elevation by binary-searching the
        // hypsometric curve (cell elevations sorted ascending). At
        // elevation E, cells with raw <= E are underwater. Find E such
        // that count(raw <= E) ≈ surface_cells_float.
        cell_elevs.sort_unstable_by_key(|a| a.to_bits());
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let target_idx = surface_cells_float as usize;
        let (fill_elev, closed_basin) = if target_idx >= cells.len() {
            // Inflow sufficient to fill entire basin → brim to spillway.
            (spillway, false)
        } else if target_idx == 0 {
            // Basically no inflow → basin is dry (no lake forms).
            continue;
        } else {
            // Fill to the target_idx-th cell's elevation.
            (
                cell_elevs[target_idx].min(spillway),
                target_idx < cells.len(),
            )
        };

        // Check minimum lake depth.
        if fill_elev < sink_elev + MIN_LAKE_DEPTH_M {
            continue;
        }

        // Classify the lake.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let water_area_cells = target_idx.min(cells.len()) as u32;
        #[allow(clippy::cast_possible_truncation)]
        let area_cells_u32 = cells.len() as u32;
        let class = classify_lake(sink_elev, fill_elev, area_cells_u32, closed_basin);

        // Assign cell classifications for cells in water surface (raw <=
        // fill_elev).
        for &c_u32 in cells {
            let c = c_u32 as usize;
            if g.raw_elevation_m[c] <= fill_elev {
                g.cell_basin[c] = rid;
                g.cell_water_class[c] = Some(class);
            }
        }

        g.lake_basins.push(LakeBasin {
            id: rid,
            surface_elevation_m: fill_elev,
            spillway_elevation_m: spillway,
            sink_elevation_m: sink_elev,
            area_cells: area_cells_u32,
            water_area_cells,
            closed_basin,
            class,
        });
    }

    // Step 3: sea cells get their water class.
    for cell in 0..BAKE_CELL_COUNT {
        if g.raw_elevation_m[cell] <= SEA_LEVEL_M {
            g.cell_water_class[cell] = Some(WaterClass::OpenOcean);
            g.cell_basin[cell] = SEA_BASIN_ID;
        }
    }
}

/// Classify a lake by size + altitude + closed-basin flag.
fn classify_lake(
    sink_elev: f32,
    fill_elev: f32,
    area_cells: u32,
    closed_basin: bool,
) -> WaterClass {
    if closed_basin {
        return WaterClass::ClosedBasinLake;
    }
    // Alpine: any lake with sink above 1500 m.
    if sink_elev >= 1_500.0 && area_cells < 20 {
        return WaterClass::AlpineTarn;
    }
    // Large vs small by footprint area.
    // 20 cells @ 2.5 km = ~125 km². Above that = large.
    let _ = fill_elev;
    if area_cells >= 20 {
        WaterClass::LakeLarge
    } else {
        WaterClass::LakeSmall
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// River / creek / dry-channel classification
// ═══════════════════════════════════════════════════════════════════════════

/// Compute path-integrated residual flow + classify cells as River /
/// Creek / DryChannel.
///
/// The residual-flow pass is a second Kahn's topological walk: each
/// cell receives its upstream residual contributions, adds its own
/// precip, subtracts per-cell evaporation (scaled by the local biome's
/// `evap_index`), clamps at zero (flow can't go negative), then passes
/// the remainder to its downstream neighbor. Over a long path, evap
/// may exceed inflow and the flow dies — that's a DryChannel.
///
/// Classification:
/// - River: residual ≥ `RIVER_FLOW_THRESHOLD`
/// - Creek: `RIVER_TO_CREEK_THRESHOLD` ≤ residual < `RIVER_FLOW_THRESHOLD`
/// - DryChannel: flow_accumulation ≥ `RIVER_TO_CREEK_THRESHOLD` AND
///   residual < `RIVER_TO_CREEK_THRESHOLD` (drainage path exists but
///   surface water evaporates)
/// - Otherwise: dry land (no classification)
///
/// Lake cells are NOT reclassified (they keep their lake class). River
/// polylines are extracted by walking high-accumulation cells from
/// their source downhill until reaching a lake, the sea, or merging
/// into an existing river.
fn classify_channels(g: &mut HydrologyGraph) {
    // Pre-compute per-cell precip + evap indices from biome lookup.
    let mut precip = vec![0.0_f32; BAKE_CELL_COUNT].into_boxed_slice();
    let mut evap = vec![0.0_f32; BAKE_CELL_COUNT].into_boxed_slice();
    for cell in 0..BAKE_CELL_COUNT {
        let (ix, iz) = unflatten(cell);
        let (x, z) = cell_center_world(ix, iz);
        let h = BiomeHydrology::for_class(biome_at(g.seed, x, z));
        precip[cell] = h.precip_index;
        evap[cell] = h.evap_index;
    }

    // Kahn's topological pass for path-integrated residual flow.
    let mut upstream_count = vec![0_u32; BAKE_CELL_COUNT].into_boxed_slice();
    for cell in 0..BAKE_CELL_COUNT {
        let down = g.d8_downstream[cell];
        if down != u32::MAX {
            upstream_count[down as usize] += 1;
        }
    }

    // Clear + re-init residual_flow.
    for cell in 0..BAKE_CELL_COUNT {
        g.residual_flow[cell] = 0.0;
    }

    let mut queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    for cell in 0..BAKE_CELL_COUNT {
        if upstream_count[cell] == 0 {
            #[allow(clippy::cast_possible_truncation)]
            queue.push_back(cell as u32);
        }
    }

    while let Some(cell_u32) = queue.pop_front() {
        let cell = cell_u32 as usize;
        // residual_flow[cell] already has upstream contributions.
        // Add own precip, subtract evap loss (floored at 0).
        let with_precip = g.residual_flow[cell] + precip[cell];
        let evap_loss = evap[cell] * CHANNEL_EVAP_PER_CELL;
        g.residual_flow[cell] = (with_precip - evap_loss).max(0.0);

        // Propagate to downstream.
        let down = g.d8_downstream[cell];
        if down != u32::MAX {
            let down_usize = down as usize;
            g.residual_flow[down_usize] += g.residual_flow[cell];
            upstream_count[down_usize] -= 1;
            if upstream_count[down_usize] == 0 {
                queue.push_back(down);
            }
        }
    }

    // Classification pass — per-cell local decision.
    for cell in 0..BAKE_CELL_COUNT {
        // Skip lake cells — they have a water class already.
        if g.cell_water_class[cell].is_some() {
            continue;
        }
        // Skip sea cells defensively.
        if g.raw_elevation_m[cell] <= SEA_LEVEL_M {
            continue;
        }

        let class = if g.residual_flow[cell] >= RIVER_FLOW_THRESHOLD {
            WaterClass::River
        } else if g.residual_flow[cell] >= RIVER_TO_CREEK_THRESHOLD {
            WaterClass::Creek
        } else if g.flow_accumulation[cell] >= RIVER_TO_CREEK_THRESHOLD {
            // Had positive accumulation pre-evap but got cancelled by
            // the path-integrated evap pass.
            WaterClass::DryChannel
        } else {
            continue; // ordinary dry land
        };
        g.cell_water_class[cell] = Some(class);
    }

    // Pass 1b: DeltaFan detection. A river-mouth cell (River/Creek
    // flowing directly into a sea cell or lake) + its immediate
    // upstream fan footprint gets tagged DeltaFan. This hooks 3.6.3's
    // feature-delta renderer to paint braided channels + sediment
    // sandbars at river mouths. Per user design insight: the parallel-
    // diagonal D8 artifact in flat regions is the geometric signature
    // of real-world delta formation — we lean into it rather than
    // smoothing it away.
    let mut to_tag_as_delta: Vec<u32> = Vec::new();
    for cell in 0..BAKE_CELL_COUNT {
        let is_river_or_creek = matches!(
            g.cell_water_class[cell],
            Some(WaterClass::River | WaterClass::Creek)
        );
        if !is_river_or_creek {
            continue;
        }
        let down = g.d8_downstream[cell];
        let mouth = down == u32::MAX
            || matches!(
                g.cell_water_class[down as usize],
                Some(
                    WaterClass::OpenOcean
                        | WaterClass::OpenCoastal
                        | WaterClass::ProtectedBay
                        | WaterClass::LakeLarge
                        | WaterClass::LakeSmall
                        | WaterClass::AlpineTarn
                        | WaterClass::ClosedBasinLake,
                )
            );
        if !mouth {
            continue;
        }
        // Tag this mouth cell + its immediate upstream D8 neighbors
        // (1-step fan footprint — tight enough to not smear across
        // half the world; 3.6.3 can widen later if needed).
        #[allow(clippy::cast_possible_truncation)]
        to_tag_as_delta.push(cell as u32);
        let (ix, iz) = unflatten(cell);
        for n in d8_neighbors(ix, iz) {
            if matches!(
                g.cell_water_class[n],
                Some(WaterClass::River | WaterClass::Creek | WaterClass::DryChannel)
            ) && g.d8_downstream[n] as usize == cell
            {
                #[allow(clippy::cast_possible_truncation)]
                to_tag_as_delta.push(n as u32);
            }
        }
    }
    for c in to_tag_as_delta {
        g.cell_water_class[c as usize] = Some(WaterClass::DeltaFan);
    }

    // Pass 2: extract river / creek polylines. Find sources (cells
    // classified as River or Creek whose upstream D8 neighbors are NOT
    // River/Creek) and walk downstream.
    let mut next_river_id: u32 = 0;
    for start in 0..BAKE_CELL_COUNT {
        let Some(class @ (WaterClass::River | WaterClass::Creek)) = g.cell_water_class[start]
        else {
            continue;
        };
        if g.cell_river_id[start] != u32::MAX {
            continue; // already part of a polyline
        }
        // Is this cell a source? (no upstream River/Creek neighbor)
        let (ix, iz) = unflatten(start);
        let is_source = d8_neighbors(ix, iz).iter().all(|&n| {
            !matches!(
                g.cell_water_class[n],
                Some(WaterClass::River | WaterClass::Creek)
            ) || g.d8_downstream[n] as usize != start
        });
        if !is_source {
            continue;
        }

        let rid = next_river_id;
        next_river_id += 1;
        let mut waypoints: Vec<(f32, f32, f32)> = Vec::new();
        let mut flow_sum: f32 = 0.0;
        let mut count: u32 = 0;
        let mut cur = start;
        let max_walk = BAKE_CELL_COUNT; // defensive bound against cycles
        for _step in 0..max_walk {
            if g.cell_river_id[cur] != u32::MAX {
                break; // merged into existing polyline
            }
            g.cell_river_id[cur] = rid;
            let (cx, cz) = unflatten(cur);
            let (wx, wz) = cell_center_world(cx, cz);
            // Waypoint elevation uses filled_elevation (what D8 operates
            // on) so polylines are monotonically downhill even across
            // pit-filled flat regions. Raw elevation can dip and rise
            // inside a pit; filled lifts the pit to the spillway so
            // the river path reads as a consistent slope.
            waypoints.push((wx, wz, g.filled_elevation_m[cur]));
            flow_sum += g.residual_flow[cur];
            count += 1;
            let down = g.d8_downstream[cur];
            if down == u32::MAX {
                break; // sea
            }
            // Stop walking if we hit a non-river cell (lake / dry / sea).
            if !matches!(
                g.cell_water_class[down as usize],
                Some(WaterClass::River | WaterClass::Creek)
            ) {
                // Extend one more waypoint for the downstream terminus
                // so the polyline visibly connects to the lake / sea.
                let (dx, dz) = unflatten(down as usize);
                let (wx, wz) = cell_center_world(dx, dz);
                waypoints.push((wx, wz, g.filled_elevation_m[down as usize]));
                break;
            }
            cur = down as usize;
        }
        if waypoints.len() < 2 || count == 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let avg_flow = flow_sum / count as f32;
        g.river_edges.push(RiverEdge {
            id: rid,
            waypoints,
            flow: avg_flow,
            class,
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Coastline polyline extraction (Marching Squares at sea level)
// ═══════════════════════════════════════════════════════════════════════════

/// Run Marching Squares over the coarse heightfield at [`SEA_LEVEL_M`]
/// to produce coastline polyline segments. Each segment is an ordered
/// list of world-coordinate vertices where the terrain crosses the sea
/// level. MS produces short segments (typically 1-2 line endpoints per
/// grid cell); we leave them as-is rather than linking into polylines —
/// the render consumer can link on demand.
///
/// Reuses the MS pattern from `terrain_visual_bench::emit_contour_map_ms_artifacts`.
fn extract_coastline(g: &mut HydrologyGraph) {
    let level = SEA_LEVEL_M;
    let mut segments: Vec<CoastlineSegment> = Vec::new();

    // Iterate over cells (each cell's NE corner participates in one MS
    // square with cell + east + south + south-east neighbors). Skip the
    // last column/row to avoid double-processing with the toroidal wrap
    // neighbor; MS works correctly inside the grid interior.
    #[allow(clippy::cast_precision_loss)]
    let cell_size = BAKE_RESOLUTION_M;
    #[allow(clippy::cast_possible_wrap)]
    let grid_n_i64 = BAKE_GRID_N as i64;

    for iz in 0..BAKE_GRID_N {
        for ix in 0..BAKE_GRID_N {
            // Corners of the MS square, wrap at seam via mod-N.
            let c00 = flatten(ix, iz);
            let c10 = flatten(wrap_cell_index(ix as i64 + 1, grid_n_i64) as usize, iz);
            let c01 = flatten(ix, wrap_cell_index(iz as i64 + 1, grid_n_i64) as usize);
            let c11 = flatten(
                wrap_cell_index(ix as i64 + 1, grid_n_i64) as usize,
                wrap_cell_index(iz as i64 + 1, grid_n_i64) as usize,
            );
            let corners_elev = [
                g.raw_elevation_m[c00],
                g.raw_elevation_m[c10],
                g.raw_elevation_m[c11],
                g.raw_elevation_m[c01],
            ];
            let (x0, z0) = cell_center_world(ix, iz);
            let x1 = x0 + cell_size;
            let z1 = z0 + cell_size;
            let corners_pos = [(x0, z0), (x1, z0), (x1, z1), (x0, z1)];

            let crossings = ms_crossings(&corners_elev, &corners_pos, level);
            match crossings.len() {
                2 => {
                    segments.push(CoastlineSegment {
                        vertices: vec![crossings[0], crossings[1]],
                    });
                },
                4 => {
                    // Saddle: emit both pairs separately (naive pairing).
                    segments.push(CoastlineSegment {
                        vertices: vec![crossings[0], crossings[1]],
                    });
                    segments.push(CoastlineSegment {
                        vertices: vec![crossings[2], crossings[3]],
                    });
                },
                _ => {},
            }
        }
    }

    g.coastline_segments = segments;
}

/// Marching Squares crossings on one cell. Corner order: SW, SE, NE, NW
/// (matches the `corners_elev` / `corners_pos` layout above). Returns
/// interpolated crossing points along the 4 edges (up to 4 crossings
/// total, pairs form line segments).
fn ms_crossings(
    corners_elev: &[f32; 4],
    corners_pos: &[(f32, f32); 4],
    level: f32,
) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(4);
    // Check each of 4 edges for a crossing.
    // Edges: 0-1 (S), 1-2 (E), 2-3 (N), 3-0 (W).
    for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
        let ea = corners_elev[a];
        let eb = corners_elev[b];
        if (ea < level && eb >= level) || (eb < level && ea >= level) {
            let de = eb - ea;
            let t = if de.abs() > 1e-6 {
                ((level - ea) / de).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let (ax, az) = corners_pos[a];
            let (bx, bz) = corners_pos[b];
            out.push((ax + t * (bx - ax), az + t * (bz - az)));
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Query API — water_surface_at + spatial lookups
// ═══════════════════════════════════════════════════════════════════════════

impl HydrologyGraph {
    /// Water surface descriptor at world coordinate `(x, z)`. Returns
    /// `None` for dry land (no water at this point).
    ///
    /// Lookup is O(1): wraps `(x, z)` to the canonical world torus,
    /// indexes into the coarse cell, and reads pre-baked metadata.
    #[must_use]
    pub fn water_surface_at(&self, x: f32, z: f32) -> Option<WaterSurface> {
        let cell = world_to_cell_index(x, z);
        let class = self.cell_water_class[cell]?;

        // Sea cells: surface at SEA_LEVEL_M. Lake cells: surface at
        // their basin's equilibrium elevation. River / creek cells:
        // ground + small offset (set on the consumer side via render
        // shader; here we just report the terrain elevation).
        let basin_id = self.cell_basin[cell];
        let elevation_m = match class {
            WaterClass::OpenOcean | WaterClass::OpenCoastal | WaterClass::ProtectedBay => {
                SEA_LEVEL_M
            },
            WaterClass::LakeLarge
            | WaterClass::LakeSmall
            | WaterClass::AlpineTarn
            | WaterClass::ClosedBasinLake => self
                .lake_basins
                .iter()
                .find(|b| b.id == basin_id)
                .map_or(SEA_LEVEL_M, |b| b.surface_elevation_m),
            WaterClass::River | WaterClass::Creek | WaterClass::DeltaFan => {
                self.raw_elevation_m[cell]
            },
            WaterClass::DryChannel => return None, // no surface water
        };

        let chop_amplitude_scale = match class {
            WaterClass::OpenOcean => 1.0,
            WaterClass::OpenCoastal => 0.8,
            WaterClass::ProtectedBay => 0.4,
            WaterClass::LakeLarge => 0.5,
            WaterClass::LakeSmall | WaterClass::AlpineTarn => 0.3,
            WaterClass::ClosedBasinLake => 0.4,
            WaterClass::River | WaterClass::Creek => 0.0,
            WaterClass::DeltaFan => 0.2, // light chop on river-mouth flats
            WaterClass::DryChannel => 0.0,
        };

        let body_id = match class {
            WaterClass::OpenOcean | WaterClass::OpenCoastal | WaterClass::ProtectedBay => {
                Some(SEA_BASIN_ID)
            },
            WaterClass::LakeLarge
            | WaterClass::LakeSmall
            | WaterClass::AlpineTarn
            | WaterClass::ClosedBasinLake => Some(basin_id),
            WaterClass::River | WaterClass::Creek | WaterClass::DeltaFan => {
                let id = self.cell_river_id[cell];
                if id == u32::MAX {
                    None
                } else {
                    Some(id)
                }
            },
            WaterClass::DryChannel => None,
        };

        Some(WaterSurface {
            class,
            elevation_m,
            chop_amplitude_scale,
            body_id,
        })
    }

    /// Look up a lake basin by id.
    #[must_use]
    pub fn lake(&self, id: u32) -> Option<&LakeBasin> {
        self.lake_basins.iter().find(|b| b.id == id)
    }

    /// Look up a river edge by id.
    #[must_use]
    pub fn river(&self, id: u32) -> Option<&RiverEdge> {
        self.river_edges.iter().find(|r| r.id == id)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::cast_precision_loss, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn bake_grid_constants_consistent() {
        // BAKE_GRID_N × BAKE_RESOLUTION_M must equal WORLD_EXTENT_M.
        let product = BAKE_GRID_N as f32 * BAKE_RESOLUTION_M;
        assert!(
            (product - WORLD_EXTENT_M).abs() < 1e-3,
            "BAKE_GRID_N × BAKE_RESOLUTION_M = {product} ≠ WORLD_EXTENT_M = {WORLD_EXTENT_M}"
        );
        assert_eq!(BAKE_CELL_COUNT, BAKE_GRID_N * BAKE_GRID_N);
    }

    #[test]
    fn cell_center_inside_world() {
        // First and last cells should be inside [0, WORLD_EXTENT_M).
        let (x0, z0) = cell_center_world(0, 0);
        let (x_max, z_max) = cell_center_world(BAKE_GRID_N - 1, BAKE_GRID_N - 1);
        assert!(x0 > 0.0 && x0 < WORLD_EXTENT_M);
        assert!(z0 > 0.0 && z0 < WORLD_EXTENT_M);
        assert!(x_max > 0.0 && x_max < WORLD_EXTENT_M);
        assert!(z_max > 0.0 && z_max < WORLD_EXTENT_M);
    }

    #[test]
    fn flatten_unflatten_round_trip() {
        for ix in [0, 1, 100, BAKE_GRID_N - 1] {
            for iz in [0, 1, 500, BAKE_GRID_N - 1] {
                let (rx, rz) = unflatten(flatten(ix, iz));
                assert_eq!((rx, rz), (ix, iz));
            }
        }
    }

    #[test]
    fn d8_neighbors_wrap_at_seam() {
        // At cell (0, 0), the (-1, -1) neighbor should wrap to
        // (BAKE_GRID_N - 1, BAKE_GRID_N - 1).
        let neighbors = d8_neighbors(0, 0);
        let expected_wrap = flatten(BAKE_GRID_N - 1, BAKE_GRID_N - 1);
        assert_eq!(neighbors[0], expected_wrap);
    }

    #[test]
    fn world_to_cell_index_wraps() {
        let here = world_to_cell_index(100_000.0, 100_000.0);
        let plus_world = world_to_cell_index(100_000.0 + WORLD_EXTENT_M, 100_000.0);
        assert_eq!(here, plus_world);
    }

    #[test]
    fn biome_hydrology_priors_within_bounds() {
        for class in [
            BiomeClass::HighDesert,
            BiomeClass::RockyAlpine,
            BiomeClass::CoastalBoreal,
            BiomeClass::MediterraneanArchipelago,
        ] {
            let h = BiomeHydrology::for_class(class);
            assert!(
                h.precip_index >= 0.0 && h.precip_index <= 1.5,
                "precip_index out of range: {} for {class:?}",
                h.precip_index
            );
            assert!(
                h.evap_index >= 0.0 && h.evap_index <= 2.0,
                "evap_index out of range: {} for {class:?}",
                h.evap_index
            );
        }
    }

    #[test]
    fn empty_graph_constructs_at_full_size() {
        let g = HydrologyGraph::empty(42);
        assert_eq!(g.filled_elevation_m.len(), BAKE_CELL_COUNT);
        assert_eq!(g.raw_elevation_m.len(), BAKE_CELL_COUNT);
        assert_eq!(g.d8_downstream.len(), BAKE_CELL_COUNT);
        assert_eq!(g.cell_basin.len(), BAKE_CELL_COUNT);
        assert_eq!(g.flow_accumulation.len(), BAKE_CELL_COUNT);
        assert_eq!(g.cell_water_class.len(), BAKE_CELL_COUNT);
        assert_eq!(g.lake_basins.len(), 0);
        assert_eq!(g.river_edges.len(), 0);
    }

    #[test]
    fn heap_entry_orders_by_elevation_then_cell() {
        let lo = HeapEntry {
            elevation_m: 100.0,
            cell: 5,
        };
        let mid = HeapEntry {
            elevation_m: 100.0,
            cell: 6,
        };
        let hi = HeapEntry {
            elevation_m: 200.0,
            cell: 1,
        };
        assert!(lo < mid);
        assert!(mid < hi);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Bake integration tests — priority-flood + accumulation + wetness
    // ═══════════════════════════════════════════════════════════════════════
    //
    // These use the real terrain sampler + canonical seed. They're marked
    // `#[ignore]` only if performance becomes an issue; at 2.5 km resolution
    // the bake takes ~1-2 s release, ~5-10 s debug. Acceptable for `cargo
    // test --release`.

    fn bake_canonical() -> (Terrain, HydrologyGraph) {
        let terrain = Terrain::from_seed(0x0F10_1A11_u64);
        let graph = HydrologyGraph::bake(0x0F10_1A11_u64, &terrain);
        (terrain, graph)
    }

    #[test]
    fn bake_produces_sea_cells() {
        let (_, g) = bake_canonical();
        // At 5000 km world with terrain in [-100, +4500] m and sea level
        // at 0, a good chunk of cells should classify as OpenOcean.
        let sea_count = g
            .cell_water_class
            .iter()
            .filter(|c| matches!(**c, Some(WaterClass::OpenOcean)))
            .count();
        assert!(
            sea_count > 0,
            "expected SOME sea cells in canonical world; got 0"
        );
    }

    #[test]
    fn bake_downstream_reaches_sea() {
        // Every non-sea cell's downstream walk must terminate at a sea
        // cell (d8_downstream = u32::MAX). Walk-from-random-start sanity.
        let (_, g) = bake_canonical();
        let mut checked = 0;
        for start in (0..BAKE_CELL_COUNT).step_by(100_000) {
            if g.cell_water_class[start] == Some(WaterClass::OpenOcean) {
                continue;
            }
            let mut cur = start;
            for _ in 0..(BAKE_CELL_COUNT * 2) {
                let down = g.d8_downstream[cur];
                if down == u32::MAX {
                    // Reached sea.
                    checked += 1;
                    break;
                }
                cur = down as usize;
                if g.cell_water_class[cur] == Some(WaterClass::OpenOcean) {
                    checked += 1;
                    break;
                }
            }
        }
        assert!(checked > 0, "no cells reached sea — downstream walk broken");
    }

    #[test]
    fn bake_filled_elevation_dominates_raw() {
        // Priority-flood invariant: filled_elevation >= raw_elevation
        // everywhere.
        let (_, g) = bake_canonical();
        for cell in 0..BAKE_CELL_COUNT {
            assert!(
                g.filled_elevation_m[cell] >= g.raw_elevation_m[cell] - 1e-3,
                "filled < raw at cell {cell}: filled={} raw={}",
                g.filled_elevation_m[cell],
                g.raw_elevation_m[cell]
            );
        }
    }

    #[test]
    fn bake_flow_accumulation_monotonic_downstream() {
        // Accumulation at a downstream cell must be >= accumulation at
        // its upstream contributor (flow is additive). Sample a few
        // cells.
        let (_, g) = bake_canonical();
        let mut checked = 0;
        for cell in (0..BAKE_CELL_COUNT).step_by(10_000) {
            let down = g.d8_downstream[cell];
            if down == u32::MAX {
                continue;
            }
            assert!(
                g.flow_accumulation[down as usize] >= g.flow_accumulation[cell] - 1e-3,
                "accum decreased downstream at cell {cell} -> {down}: up={} down={}",
                g.flow_accumulation[cell],
                g.flow_accumulation[down as usize]
            );
            checked += 1;
        }
        assert!(checked > 0, "no valid cells checked");
    }

    #[test]
    fn bake_is_deterministic() {
        // Same seed produces bit-identical accumulation + classification
        // across bakes.
        let terrain = Terrain::from_seed(12345);
        let a = HydrologyGraph::bake(12345, &terrain);
        let b = HydrologyGraph::bake(12345, &terrain);
        assert_eq!(a.lake_count(), b.lake_count());
        assert_eq!(a.river_count(), b.river_count());
        for cell in (0..BAKE_CELL_COUNT).step_by(1_000) {
            assert_eq!(
                a.filled_elevation_m[cell].to_bits(),
                b.filled_elevation_m[cell].to_bits(),
                "filled not deterministic at cell {cell}"
            );
            assert_eq!(a.cell_basin[cell], b.cell_basin[cell]);
        }
    }

    #[test]
    fn bake_different_seeds_produce_different_graphs() {
        let t1 = Terrain::from_seed(1);
        let t2 = Terrain::from_seed(2);
        let g1 = HydrologyGraph::bake(1, &t1);
        let g2 = HydrologyGraph::bake(2, &t2);
        // Very unlikely both worlds have identical sea-cell count; this
        // is a weak-but-cheap sanity check.
        let sea1 = g1
            .cell_water_class
            .iter()
            .filter(|c| matches!(**c, Some(WaterClass::OpenOcean)))
            .count();
        let sea2 = g2
            .cell_water_class
            .iter()
            .filter(|c| matches!(**c, Some(WaterClass::OpenOcean)))
            .count();
        // Can't assert "definitely different" without specific seeds, but
        // we can assert at least one of the graphs has something
        // non-trivial.
        assert!(sea1 > 0 || sea2 > 0);
    }

    #[test]
    fn water_surface_at_wraps_toroidally() {
        let (_, g) = bake_canonical();
        let here = g.water_surface_at(100_000.0, 100_000.0);
        let plus_world = g.water_surface_at(100_000.0 + WORLD_EXTENT_M, 100_000.0);
        match (here, plus_world) {
            (None, None) => {},
            (Some(a), Some(b)) => {
                assert_eq!(a.class, b.class);
                assert_eq!(a.elevation_m.to_bits(), b.elevation_m.to_bits());
            },
            _ => panic!("wrap produced different presence of water at seam"),
        }
    }

    #[test]
    fn coastline_segments_emitted() {
        let (_, g) = bake_canonical();
        // A world with varied terrain around sea level should produce
        // many coastline segments.
        let segments = g.coastline_segments();
        assert!(
            !segments.is_empty(),
            "no coastline segments produced (terrain has no sea-level crossings?)"
        );
    }

    #[test]
    fn bake_rivers_obey_downhill_slope() {
        // User explicit design constraint: "rivers may have to obey
        // slope". D8 flow direction is downhill-by-construction, so
        // river polylines must be monotonically descending in elevation.
        // Spot-check a sample of river edges.
        let (_, g) = bake_canonical();
        let rivers = g.rivers();
        assert!(!rivers.is_empty(), "no rivers produced");
        let mut checked = 0;
        for r in rivers.iter().take(100) {
            // Elevations across the polyline waypoints must be
            // monotonically non-increasing (downhill). Allow small
            // tolerance for f32 precision.
            for w in r.waypoints.windows(2) {
                let up = w[0].2;
                let down = w[1].2;
                assert!(
                    down <= up + 0.5,
                    "river {} goes uphill: {up} -> {down}",
                    r.id
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no river-waypoint pairs checked");
    }

    #[test]
    fn bake_lake_elevation_bounds_satisfied() {
        // For each lake: sink_elevation <= surface_elevation <=
        // spillway_elevation. Closed basins can have surface <
        // spillway (partial fill); open basins have surface == spillway.
        let (_, g) = bake_canonical();
        for lake in g.lakes() {
            assert!(
                lake.surface_elevation_m >= lake.sink_elevation_m - 1e-3,
                "lake {} surface ({}) below sink ({})",
                lake.id,
                lake.surface_elevation_m,
                lake.sink_elevation_m
            );
            assert!(
                lake.surface_elevation_m <= lake.spillway_elevation_m + 1e-3,
                "lake {} surface ({}) above spillway ({})",
                lake.id,
                lake.surface_elevation_m,
                lake.spillway_elevation_m
            );
            assert!(
                lake.water_area_cells <= lake.area_cells,
                "lake {} water-area ({}) exceeds footprint ({})",
                lake.id,
                lake.water_area_cells,
                lake.area_cells
            );
        }
    }

    #[test]
    fn bake_produces_nontrivial_feature_counts() {
        // Sanity: canonical world should produce at least some lakes +
        // rivers (not zero — proves wetness + classification passes
        // actually did work).
        let (_, g) = bake_canonical();
        assert!(
            g.lake_count() > 0,
            "bake produced zero lakes — wetness fill failed?"
        );
        assert!(
            g.river_count() > 0,
            "bake produced zero rivers — classification failed?"
        );
    }

    #[test]
    fn water_surface_at_lake_returns_lake_elevation() {
        // If we find a lake cell via the lake catalog, water_surface_at
        // at that cell's world position should return the lake's
        // surface elevation.
        let (_, g) = bake_canonical();
        let lake = g.lakes().first().expect("no lakes in canonical bake");
        // Find a cell belonging to this lake in the basin grid.
        let basins = g.basin_grid();
        let mut sample_cell_idx: Option<usize> = None;
        for (idx, &b) in basins.iter().enumerate() {
            if b == lake.id {
                sample_cell_idx = Some(idx);
                break;
            }
        }
        let cell = sample_cell_idx.expect("lake has no cells in basin grid");
        let (ix, iz) = (cell % BAKE_GRID_N, cell / BAKE_GRID_N);
        let (x, z) = cell_center_world(ix, iz);
        let surface = g.water_surface_at(x, z).expect("cell lookup returned None");
        assert_eq!(
            surface.elevation_m.to_bits(),
            lake.surface_elevation_m.to_bits(),
            "water_surface_at at lake cell returned {} instead of {}",
            surface.elevation_m,
            lake.surface_elevation_m
        );
        assert!(matches!(
            surface.class,
            WaterClass::LakeLarge
                | WaterClass::LakeSmall
                | WaterClass::AlpineTarn
                | WaterClass::ClosedBasinLake
        ));
    }

    #[test]
    fn delta_fans_detected_at_river_mouths() {
        // DeltaFan should appear somewhere in a world with river
        // networks; the detection targets River/Creek cells whose
        // downstream is sea or lake. Non-zero count proves the pass
        // runs and the geometric condition is achievable.
        let (_, g) = bake_canonical();
        let mut delta_count = 0_u32;
        for iz in 0..BAKE_GRID_N {
            for ix in 0..BAKE_GRID_N {
                let (x, z) = cell_center_world(ix, iz);
                if matches!(
                    g.water_surface_at(x, z).map(|s| s.class),
                    Some(WaterClass::DeltaFan)
                ) {
                    delta_count += 1;
                }
            }
        }
        assert!(
            delta_count > 0,
            "no DeltaFan cells detected — bake or classification broken"
        );
    }

    #[test]
    fn water_surface_at_sea_returns_sea_level() {
        // Check a known-sea location. We'll find a cell classified as
        // OpenOcean and verify.
        let (_, g) = bake_canonical();
        for iz in 0..BAKE_GRID_N {
            for ix in 0..BAKE_GRID_N {
                let (x, z) = cell_center_world(ix, iz);
                if let Some(s) = g.water_surface_at(x, z) {
                    if matches!(s.class, WaterClass::OpenOcean) {
                        assert_eq!(
                            s.elevation_m.to_bits(),
                            SEA_LEVEL_M.to_bits(),
                            "sea cell at ({x},{z}) reported elevation {} ≠ SEA_LEVEL_M",
                            s.elevation_m
                        );
                        return; // found one, test passes
                    }
                }
            }
        }
        panic!("no sea cells found to verify elevation");
    }
}
