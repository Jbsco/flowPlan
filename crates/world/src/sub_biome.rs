//! Sub-biome feature libraries (Phase 3.6.2).
//!
//! Per-biome authored catalogs of sub-biome feature types (`mesa_top`,
//! `lake_cluster`, `alpine_tarn`, etc.) with placement constraints.
//! Each Voronoi cell from the L2 partition gets assigned a feature
//! type at bake time by sampling from its parent biome's library
//! (with biome-edge bleed-over + placement-constraint rejection).
//!
//! Libraries live in [`SUB_BIOME_FEATURES_RON`], embedded at compile
//! time via `include_str!` and parsed to [`SubBiomeLibraries`] on
//! first access (cached via `OnceLock`). Adding new features is a
//! pure content edit of `data/world/sub_biome_features.ron`.
//!
//! # Placement constraints
//!
//! Each feature declares:
//! - `slope_tolerance_max_deg` — reject cell if terrain slope exceeds
//! - `roughness_tolerance_m` — reject cell if heightfield variance
//!   across cell footprint exceeds (currently proxied by cell-center
//!   elevation vs. nearest-neighbor delta; refined in 3.6.3+)
//! - `water_adjacency` — Required / Forbidden / None (checked via
//!   hydrology graph)
//! - `altitude_range_m` — cell altitude must fit
//! - `selection_weight` — relative RNG weight for selection
//! - `is_bleed_over_candidate` — can appear in adjacent biomes when
//!   a seed sits in a biome-transition zone
//!
//! # Bleed-over
//!
//! Seeds whose biome-weight primary is < 0.8 are in a transition
//! zone. Their feature selection can draw from adjacent biomes' libraries
//! (`is_bleed_over_candidate: true` entries only). Produces the
//! "lake_cluster in sheltered desert valley" effect pinned in
//! `biome_edge_bleed_over` memory.
//!
//! # Determinism
//!
//! Pure function of `(master_seed, cell_id, biome_weights, terrain,
//! hydrology)`. Bit-identical per seed.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::doc_markdown)]

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::biome::{biome_weights_at, BiomeClass, BiomeWeights};

/// Embedded RON content for the sub-biome feature libraries.
const SUB_BIOME_FEATURES_RON: &str = include_str!("../../../data/world/sub_biome_features.ron");

/// Water-adjacency requirement for a sub-biome feature.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
pub enum WaterAdjacency {
    /// Must be adjacent to a water body (lake / river / sea).
    Required,
    /// Must NOT be adjacent to water (deserts on mesas, etc.).
    Forbidden,
    /// No constraint.
    None,
}

/// RGB color triple (0-255 per channel). Authored in RON as `(r, g, b)`.
pub type PaletteColor = (u8, u8, u8);

/// 3-color palette: primary (dominant), secondary (supporting), accent
/// (highlight). Features render as simple primitives colored by these
/// palette entries — see memory `simple_primitives_render_direction`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Palette {
    /// Primary color — dominant visual tone (building walls, tree
    /// canopy, ground cover).
    #[serde(default)]
    pub primary: PaletteColor,
    /// Secondary color — supporting detail (trim, shadows, accents).
    #[serde(default)]
    pub secondary: PaletteColor,
    /// Accent color — highlights (roofing, flowers, trim contrast).
    #[serde(default)]
    pub accent: PaletteColor,
}

/// Single sub-biome feature catalog entry.
#[derive(Clone, Debug, Deserialize)]
pub struct SubBiomeFeature {
    /// Stable string id (snake_case). Used as a cross-system key.
    pub id: String,
    /// Max terrain slope at cell seed (degrees).
    pub slope_tolerance_max_deg: f32,
    /// Max heightfield variance across cell footprint (m).
    pub roughness_tolerance_m: f32,
    /// Water-adjacency constraint.
    pub water_adjacency: WaterAdjacency,
    /// (min, max) altitude range (m). Cell altitude must lie within.
    pub altitude_range_m: (f32, f32),
    /// Relative selection weight within the parent biome's library.
    pub selection_weight: f32,
    /// Can this feature appear in adjacent biomes via the bleed-over
    /// mechanism? False for strongly-biome-specific features
    /// (terracotta_port, glacier_tongue, etc.).
    pub is_bleed_over_candidate: bool,
    /// Home palette (native biome). Optional — absent entries render
    /// via biome-derived fallback colors until palette authoring lands.
    #[serde(default)]
    pub home_palette: Option<Palette>,
    /// Per-host-biome adaptation palettes. Keys are
    /// `BiomeClass` names (`"HighDesert"`, `"RockyAlpine"`, etc.).
    /// When this feature appears in a host biome via bleed-over, the
    /// render color blends from `home_palette` toward
    /// `adaptation_per_host[host]` per `blend_factor` (see memory
    /// `palette_adaptation_mechanism`).
    #[serde(default)]
    pub adaptation_per_host: HashMap<String, Palette>,
}

/// Root RON structure.
#[derive(Debug, Deserialize)]
struct SubBiomeFeaturesRon {
    biomes: HashMap<String, Vec<SubBiomeFeature>>,
}

/// Per-biome feature libraries, parsed once from the embedded RON.
/// Indexed by [`BiomeClass`]. An additional `oceanic` library holds
/// sea-cell features (not tied to a land biome).
#[derive(Debug)]
pub struct SubBiomeLibraries {
    high_desert: Vec<SubBiomeFeature>,
    rocky_alpine: Vec<SubBiomeFeature>,
    coastal_boreal: Vec<SubBiomeFeature>,
    mediterranean: Vec<SubBiomeFeature>,
    oceanic: Vec<SubBiomeFeature>,
}

impl SubBiomeLibraries {
    /// Shared library instance. Parsed lazily on first access.
    #[must_use]
    pub fn shared() -> &'static Self {
        static INSTANCE: OnceLock<SubBiomeLibraries> = OnceLock::new();
        INSTANCE.get_or_init(Self::load)
    }

    /// Load + parse the embedded RON. Panics at startup if the RON
    /// file is malformed — a build-time authoring bug, not a runtime
    /// error class.
    fn load() -> Self {
        let root: SubBiomeFeaturesRon = ron::from_str(SUB_BIOME_FEATURES_RON)
            .unwrap_or_else(|e| panic!("failed to parse sub_biome_features.ron: {e}"));
        Self {
            high_desert: root.biomes.get("HighDesert").cloned().unwrap_or_default(),
            rocky_alpine: root.biomes.get("RockyAlpine").cloned().unwrap_or_default(),
            coastal_boreal: root
                .biomes
                .get("CoastalBoreal")
                .cloned()
                .unwrap_or_default(),
            mediterranean: root
                .biomes
                .get("MediterraneanArchipelago")
                .cloned()
                .unwrap_or_default(),
            oceanic: root.biomes.get("Oceanic").cloned().unwrap_or_default(),
        }
    }

    /// The Oceanic feature library (for sea cells below
    /// [`OPEN_WATER_ALTITUDE_THRESHOLD_M`]).
    #[must_use]
    pub fn oceanic(&self) -> &[SubBiomeFeature] {
        &self.oceanic
    }

    /// All features for the given biome.
    #[must_use]
    pub fn for_biome(&self, biome: BiomeClass) -> &[SubBiomeFeature] {
        match biome {
            BiomeClass::HighDesert => &self.high_desert,
            BiomeClass::RockyAlpine => &self.rocky_alpine,
            BiomeClass::CoastalBoreal => &self.coastal_boreal,
            BiomeClass::MediterraneanArchipelago => &self.mediterranean,
        }
    }

    /// Count of features per biome.
    #[must_use]
    pub fn count_per_biome(&self, biome: BiomeClass) -> usize {
        self.for_biome(biome).len()
    }

    /// Total feature-entry count across all biomes (including
    /// oceanic).
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.high_desert.len()
            + self.rocky_alpine.len()
            + self.coastal_boreal.len()
            + self.mediterranean.len()
            + self.oceanic.len()
    }

    /// Look up a feature by (biome, id). Returns None if absent.
    #[must_use]
    pub fn find(&self, biome: BiomeClass, id: &str) -> Option<&SubBiomeFeature> {
        self.for_biome(biome).iter().find(|f| f.id == id)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Placement context (inputs for feature selection)
// ═══════════════════════════════════════════════════════════════════════════

/// Inputs evaluated at a candidate cell seed position during feature
/// assignment.
#[derive(Copy, Clone, Debug)]
pub struct PlacementContext {
    /// Sea-level-adjusted elevation sampled at the seed position (m).
    pub altitude_m: f32,
    /// Slope in degrees sampled at the seed (max over a small kernel).
    pub slope_deg: f32,
    /// Roughness proxy — max terrain variance across cell footprint (m).
    pub roughness_m: f32,
    /// True if a water body (lake, river, sea) exists within the
    /// bleed-over-threshold distance.
    pub has_water_adjacency: bool,
    /// Biome-weight profile at the seed (for bleed-over evaluation).
    pub biome_weights: BiomeWeights,
}

impl PlacementContext {
    /// True iff the feature's placement constraints pass.
    #[must_use]
    pub fn satisfies(&self, feature: &SubBiomeFeature) -> bool {
        if self.slope_deg > feature.slope_tolerance_max_deg {
            return false;
        }
        if self.roughness_m > feature.roughness_tolerance_m {
            return false;
        }
        let (alt_lo, alt_hi) = feature.altitude_range_m;
        if self.altitude_m < alt_lo || self.altitude_m > alt_hi {
            return false;
        }
        match feature.water_adjacency {
            WaterAdjacency::Required => self.has_water_adjacency,
            WaterAdjacency::Forbidden => !self.has_water_adjacency,
            WaterAdjacency::None => true,
        }
    }
}

/// Which biome library to sample from — the primary biome or an
/// adjacent biome via bleed-over.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BiomeSource {
    /// Primary parent biome.
    Primary(BiomeClass),
    /// Adjacent biome contributing via bleed-over (only candidates
    /// with `is_bleed_over_candidate: true` are considered).
    BleedOver {
        host: BiomeClass,
        source: BiomeClass,
    },
}

impl BiomeSource {
    /// Biome whose feature library we're actually sampling.
    #[must_use]
    pub const fn feature_source_biome(&self) -> BiomeClass {
        match self {
            Self::Primary(b) => *b,
            Self::BleedOver { source, .. } => *source,
        }
    }

    /// Biome that ultimately "hosts" this cell (the cell's geographic
    /// parent). Same as feature_source for Primary; for BleedOver,
    /// it's the host (local) biome.
    #[must_use]
    pub const fn host_biome(&self) -> BiomeClass {
        match self {
            Self::Primary(b) => *b,
            Self::BleedOver { host, .. } => *host,
        }
    }

    /// True if this cell is a bleed-over result.
    #[must_use]
    pub const fn is_bleed_over(&self) -> bool {
        matches!(self, Self::BleedOver { .. })
    }
}

/// Threshold below which a cell is in a "biome-transition zone" and
/// eligible for bleed-over feature selection.
///
/// Biome weights are normalized to sum to 1.0. The **primary** weight
/// = the largest component. A primary weight of 1.0 means the cell is
/// deep in one biome; 0.25 means the cell is at a 4-biome tri/quad
/// junction. Cells with primary weight below this threshold open up
/// to bleed-over candidates from adjacent biomes.
pub const BLEED_OVER_PRIMARY_WEIGHT_THRESHOLD: f32 = 0.8;

/// Determine the biome source (primary vs bleed-over) for feature
/// selection at a cell. Uses the biome weights at the cell position.
///
/// The bleed-over biome is chosen deterministically from the weight
/// distribution — second-largest weight above a minimum threshold,
/// picked by hashing `(master_seed, cell_id)` against the adjacent
/// biome candidates.
#[must_use]
pub fn pick_biome_source(weights: &BiomeWeights, bleed_rng: u32) -> BiomeSource {
    let (primary_biome, primary_weight) = primary_biome(weights);

    if primary_weight >= BLEED_OVER_PRIMARY_WEIGHT_THRESHOLD {
        return BiomeSource::Primary(primary_biome);
    }

    // Transition zone — pick a bleed-over source from the
    // non-primary biomes with non-trivial weight.
    let candidates: Vec<(BiomeClass, f32)> = [
        (BiomeClass::HighDesert, weights.high_desert),
        (BiomeClass::RockyAlpine, weights.rocky_alpine),
        (BiomeClass::CoastalBoreal, weights.coastal_boreal),
        (BiomeClass::MediterraneanArchipelago, weights.mediterranean),
    ]
    .iter()
    .copied()
    .filter(|(b, w)| *b != primary_biome && *w > 0.05)
    .collect();

    if candidates.is_empty() {
        return BiomeSource::Primary(primary_biome);
    }

    // Weighted pick from candidates (by weight). Deterministic via
    // bleed_rng input.
    let total_weight: f32 = candidates.iter().map(|(_, w)| *w).sum();
    let roll = ((bleed_rng as f32) / (u32::MAX as f32 + 1.0)) * total_weight;
    let mut acc = 0.0_f32;
    for (b, w) in &candidates {
        acc += *w;
        if roll <= acc {
            return BiomeSource::BleedOver {
                host: primary_biome,
                source: *b,
            };
        }
    }
    // Fallback (unreachable given positive weights, but safe).
    BiomeSource::Primary(primary_biome)
}

/// Return the primary biome + its weight. Primary = largest weight.
fn primary_biome(weights: &BiomeWeights) -> (BiomeClass, f32) {
    let mut best = BiomeClass::CoastalBoreal;
    let mut best_w = weights.coastal_boreal;
    if weights.high_desert > best_w {
        best = BiomeClass::HighDesert;
        best_w = weights.high_desert;
    }
    if weights.rocky_alpine > best_w {
        best = BiomeClass::RockyAlpine;
        best_w = weights.rocky_alpine;
    }
    if weights.mediterranean > best_w {
        best = BiomeClass::MediterraneanArchipelago;
        best_w = weights.mediterranean;
    }
    (best, best_w)
}

// ═══════════════════════════════════════════════════════════════════════════
// Feature assignment
// ═══════════════════════════════════════════════════════════════════════════

/// Deterministic splitmix64-inspired hash (matches the pattern in
/// `weather::mod::mix64` + `magnetic_variation::mix_seed`).
const fn mix_seed(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Sub-biome tag for catalog fields.
const SUB_BIOME_FIELD_TAG: u64 = u64::from_le_bytes(*b"subbiome");

/// Max re-roll attempts when constraint rejection hits. Falls back to
/// the generic `plain` feature identifier if exhausted.
const MAX_FEATURE_ROLL_ATTEMPTS: u32 = 8;

/// Generic fallback feature id used when no library entry satisfies
/// the placement context. Emitted as-is into the catalog; 3.6.3
/// feature-delta renderer treats unknown ids as "no delta".
pub const FALLBACK_FEATURE_ID: &str = "plain";

/// Open-water feature id fallback for sea cells.
///
/// Assigned to cells whose seed sits clearly below sea level when the
/// Oceanic library rollup fails (very rare). The regular path now
/// samples oceanic feature types (open_ocean, fishing_grounds, etc.)
/// rather than emitting this tag directly — `open_water` remains only
/// as a safety-net fallback.
///
/// Rendered in 3.6.3 as the global water surface — no per-cell
/// geometry delta needed.
pub const OPEN_WATER_FEATURE_ID: &str = "open_water";

/// Altitude threshold (m) below which a cell is short-circuited to
/// [`OPEN_WATER_FEATURE_ID`]. `-5 m` catches both deep sea and
/// tide-flat / fjord-bottom cells.
pub const OPEN_WATER_ALTITUDE_THRESHOLD_M: f32 = -5.0;

/// Result of a feature-assignment pass for one cell.
#[derive(Clone, Debug)]
pub struct FeatureAssignment {
    /// Final feature id (from library, or `plain` fallback).
    pub feature_id: String,
    /// Which biome the feature was drawn from (primary or bleed-over
    /// source).
    pub source: BiomeSource,
    /// True if the generic `plain` fallback was used (constraints
    /// rejected all library candidates).
    pub is_fallback: bool,
}

/// Assign a sub-biome feature to a cell.
///
/// Algorithm:
/// 1. Determine [`BiomeSource`] via weights + bleed-over RNG roll.
/// 2. Pick a candidate feature from the chosen biome's library via
///    weighted-RNG sampling.
/// 3. Check [`PlacementContext::satisfies`] against constraints.
/// 4. If fails, re-roll up to `MAX_FEATURE_ROLL_ATTEMPTS` times.
/// 5. If exhausted, return the `plain` fallback.
///
/// Deterministic per `(master_seed, cell_id)`.
#[must_use]
pub fn assign_feature(
    master_seed: u64,
    cell_id: u32,
    context: &PlacementContext,
    libraries: &SubBiomeLibraries,
) -> FeatureAssignment {
    let field_seed = mix_seed(master_seed, SUB_BIOME_FIELD_TAG);
    let bleed_rng = mix_seed(field_seed, u64::from(cell_id) | (1_u64 << 32)) as u32;
    let source = pick_biome_source(&context.biome_weights, bleed_rng);

    // Open-water short-circuit: cells clearly below sea level sample
    // from the Oceanic library rather than falling through the land-
    // biome rollup (which would hit `plain` for nearly all deep-sea
    // cells). The `source` biome still reflects the dominant land-
    // biome climate here — useful for future water-over-biome palette
    // effects (sea off a Mediterranean coast looks different than sea
    // off an Alaskan one). User-pinned direction: oceanic features
    // host NPC ships / fishing / interactive encounters at later
    // phases (see `oceanic_features_npcs` memory).
    if context.altitude_m < OPEN_WATER_ALTITUDE_THRESHOLD_M {
        let oceanic = libraries.oceanic();
        for attempt in 0..MAX_FEATURE_ROLL_ATTEMPTS {
            let roll_rng = mix_seed(
                field_seed,
                u64::from(cell_id) | (u64::from(attempt) << 48) | (1 << 60),
            );
            if let Some(f) = sample_feature(oceanic, roll_rng, false) {
                if context.satisfies(f) {
                    return FeatureAssignment {
                        feature_id: f.id.clone(),
                        source,
                        is_fallback: false,
                    };
                }
            }
        }
        // If oceanic library can't satisfy (unlikely — open_ocean is
        // very permissive), fall back to the generic open_water tag.
        return FeatureAssignment {
            feature_id: OPEN_WATER_FEATURE_ID.to_string(),
            source,
            is_fallback: false,
        };
    }

    let source_biome = source.feature_source_biome();
    let lib = libraries.for_biome(source_biome);

    for attempt in 0..MAX_FEATURE_ROLL_ATTEMPTS {
        let roll_rng = mix_seed(field_seed, u64::from(cell_id) | (u64::from(attempt) << 48));
        let feature = sample_feature(lib, roll_rng, source.is_bleed_over());
        if let Some(f) = feature {
            if context.satisfies(f) {
                return FeatureAssignment {
                    feature_id: f.id.clone(),
                    source,
                    is_fallback: false,
                };
            }
        }
    }
    // Fallback.
    FeatureAssignment {
        feature_id: FALLBACK_FEATURE_ID.to_string(),
        source,
        is_fallback: true,
    }
}

/// Sample a feature from a library by selection_weight, filtering to
/// only bleed-over-eligible features when `require_bleed_over_candidate`
/// is true.
fn sample_feature(
    lib: &[SubBiomeFeature],
    rng_u64: u64,
    require_bleed_over_candidate: bool,
) -> Option<&SubBiomeFeature> {
    let filtered: Vec<&SubBiomeFeature> = lib
        .iter()
        .filter(|f| !require_bleed_over_candidate || f.is_bleed_over_candidate)
        .collect();
    if filtered.is_empty() {
        return None;
    }
    let total_weight: f32 = filtered.iter().map(|f| f.selection_weight).sum();
    let roll = ((rng_u64 & 0xFFFF_FFFF) as f32 / (u32::MAX as f32 + 1.0)) * total_weight;
    let mut acc = 0.0_f32;
    for f in &filtered {
        acc += f.selection_weight;
        if roll <= acc {
            return Some(f);
        }
    }
    filtered.last().copied()
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper: biome weights at a coordinate
// ═══════════════════════════════════════════════════════════════════════════

/// Convenience: look up biome weights at a world coordinate. Thin
/// wrapper around [`biome_weights_at`] for callers who only want the
/// weights (the sub-biome catalog uses these as placement inputs).
#[must_use]
pub fn biome_weights_at_position(master_seed: u64, x: f32, z: f32) -> BiomeWeights {
    biome_weights_at(master_seed, x, z)
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn libraries_parse_from_ron() {
        let libs = SubBiomeLibraries::shared();
        assert!(libs.count_per_biome(BiomeClass::HighDesert) >= 8);
        assert!(libs.count_per_biome(BiomeClass::RockyAlpine) >= 6);
        assert!(libs.count_per_biome(BiomeClass::CoastalBoreal) >= 8);
        assert!(libs.count_per_biome(BiomeClass::MediterraneanArchipelago) >= 6);
        assert!(libs.total_count() >= 30);
    }

    #[test]
    fn highdesert_book_cliffs_present() {
        // User-flagged addition; must be in the authored list.
        let libs = SubBiomeLibraries::shared();
        assert!(
            libs.find(BiomeClass::HighDesert, "book_cliffs").is_some(),
            "HighDesert must include book_cliffs per authoring direction"
        );
    }

    #[test]
    fn each_biome_has_expected_core_features() {
        let libs = SubBiomeLibraries::shared();
        assert!(libs.find(BiomeClass::HighDesert, "mesa_top").is_some());
        assert!(libs.find(BiomeClass::RockyAlpine, "alpine_tarn").is_some());
        assert!(libs
            .find(BiomeClass::CoastalBoreal, "lake_cluster")
            .is_some());
        assert!(libs
            .find(BiomeClass::MediterraneanArchipelago, "protected_cove")
            .is_some());
    }

    #[test]
    fn placement_context_satisfies_uses_all_constraints() {
        let feature = SubBiomeFeature {
            id: "test".into(),
            slope_tolerance_max_deg: 5.0,
            roughness_tolerance_m: 50.0,
            water_adjacency: WaterAdjacency::None,
            altitude_range_m: (100.0, 500.0),
            selection_weight: 1.0,
            is_bleed_over_candidate: false,
            home_palette: None,
            adaptation_per_host: HashMap::new(),
        };
        // All pass.
        let ok = PlacementContext {
            altitude_m: 300.0,
            slope_deg: 3.0,
            roughness_m: 30.0,
            has_water_adjacency: false,
            biome_weights: BiomeWeights::default(),
        };
        assert!(ok.satisfies(&feature));
        // Slope fails.
        let slope_fail = PlacementContext {
            slope_deg: 10.0,
            ..ok
        };
        assert!(!slope_fail.satisfies(&feature));
        // Altitude low.
        let alt_low = PlacementContext {
            altitude_m: 50.0,
            ..ok
        };
        assert!(!alt_low.satisfies(&feature));
        // Altitude high.
        let alt_high = PlacementContext {
            altitude_m: 600.0,
            ..ok
        };
        assert!(!alt_high.satisfies(&feature));
        // Roughness fails.
        let rough_fail = PlacementContext {
            roughness_m: 80.0,
            ..ok
        };
        assert!(!rough_fail.satisfies(&feature));
    }

    #[test]
    fn water_adjacency_semantics() {
        let req = SubBiomeFeature {
            id: "req".into(),
            slope_tolerance_max_deg: 90.0,
            roughness_tolerance_m: 1e6,
            water_adjacency: WaterAdjacency::Required,
            altitude_range_m: (-1e6, 1e6),
            selection_weight: 1.0,
            is_bleed_over_candidate: false,
            home_palette: None,
            adaptation_per_host: HashMap::new(),
        };
        let forb = SubBiomeFeature {
            water_adjacency: WaterAdjacency::Forbidden,
            ..req.clone()
        };
        let with_water = PlacementContext {
            altitude_m: 0.0,
            slope_deg: 0.0,
            roughness_m: 0.0,
            has_water_adjacency: true,
            biome_weights: BiomeWeights::default(),
        };
        let no_water = PlacementContext {
            has_water_adjacency: false,
            ..with_water
        };
        assert!(with_water.satisfies(&req));
        assert!(!no_water.satisfies(&req));
        assert!(!with_water.satisfies(&forb));
        assert!(no_water.satisfies(&forb));
    }

    #[test]
    fn pick_biome_source_primary_when_dominant() {
        let weights = BiomeWeights {
            coastal_boreal: 0.9,
            rocky_alpine: 0.05,
            high_desert: 0.02,
            mediterranean: 0.03,
        };
        let source = pick_biome_source(&weights, 0);
        assert!(matches!(
            source,
            BiomeSource::Primary(BiomeClass::CoastalBoreal)
        ));
    }

    #[test]
    fn pick_biome_source_bleed_over_in_transition() {
        // 55/40 split — primary weight below threshold, bleed-over
        // should engage.
        let weights = BiomeWeights {
            coastal_boreal: 0.55,
            rocky_alpine: 0.40,
            high_desert: 0.03,
            mediterranean: 0.02,
        };
        // Try several RNG rolls; at least one should produce BleedOver.
        let mut saw_bleed = false;
        for rng in 0..100 {
            let source = pick_biome_source(&weights, rng * 1_000_003);
            if source.is_bleed_over() {
                saw_bleed = true;
                break;
            }
        }
        assert!(saw_bleed, "expected bleed-over with 55/40 weights");
    }

    #[test]
    fn assign_feature_returns_fallback_when_no_constraints_fit() {
        let libs = SubBiomeLibraries::shared();
        // Pathological context: altitude outside any feature's range,
        // extreme slope. Expect fallback.
        let ctx = PlacementContext {
            altitude_m: 10_000.0, // way above any altitude range
            slope_deg: 89.0,
            roughness_m: 1_000_000.0,
            has_water_adjacency: false,
            biome_weights: BiomeWeights {
                coastal_boreal: 1.0,
                ..Default::default()
            },
        };
        let result = assign_feature(42, 0, &ctx, libs);
        assert!(result.is_fallback);
        assert_eq!(result.feature_id, FALLBACK_FEATURE_ID);
    }

    #[test]
    fn assign_feature_deterministic() {
        let libs = SubBiomeLibraries::shared();
        let ctx = PlacementContext {
            altitude_m: 500.0,
            slope_deg: 5.0,
            roughness_m: 50.0,
            has_water_adjacency: true,
            biome_weights: BiomeWeights {
                coastal_boreal: 1.0,
                ..Default::default()
            },
        };
        let a = assign_feature(42, 1234, &ctx, libs);
        let b = assign_feature(42, 1234, &ctx, libs);
        assert_eq!(a.feature_id, b.feature_id);
        assert_eq!(a.source, b.source);
        assert_eq!(a.is_fallback, b.is_fallback);
    }

    #[test]
    #[allow(clippy::items_after_statements)]
    fn assign_feature_different_seeds_can_differ() {
        let libs = SubBiomeLibraries::shared();
        let ctx = PlacementContext {
            altitude_m: 500.0,
            slope_deg: 5.0,
            roughness_m: 50.0,
            has_water_adjacency: true,
            biome_weights: BiomeWeights {
                coastal_boreal: 1.0,
                ..Default::default()
            },
        };
        // Across 50 different cell ids we should see at least 3
        // distinct features (otherwise the RNG sampling is broken).
        use std::collections::HashSet;
        let mut seen: HashSet<String> = HashSet::new();
        for cell_id in 0..50 {
            let r = assign_feature(42, cell_id, &ctx, libs);
            if !r.is_fallback {
                seen.insert(r.feature_id);
            }
        }
        assert!(
            seen.len() >= 3,
            "feature sampling produced only {} distinct features across 50 cells",
            seen.len()
        );
    }
}
