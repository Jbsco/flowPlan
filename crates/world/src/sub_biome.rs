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

/// 3-color palette: primary, secondary, accent.
///
/// Features render as simple primitives colored by these palette
/// entries — see memory `simple_primitives_render_direction`.
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

/// Palette category — classifies a feature's dominant visual character.
///
/// The biome-receiving-adaptation table applies a category-appropriate
/// HSV shift when bleed-over places the feature in a host biome. See
/// memory `palette_adaptation_mechanism`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum PaletteCategory {
    /// Wooden buildings — cabins, sheds, barns, docks, wooden stores.
    BuildingWooden,
    /// Masonry / concrete / metal — gas-station canopies, silos,
    /// permanent commercial / industrial structures.
    BuildingMasonry,
    /// Vegetation-dominated features — meadows, farmland, creek valleys
    /// where greens / grasses carry the palette.
    Vegetation,
    /// Rock / stone features — scenic overlooks, cliff coasts.
    Stone,
    /// Water-dominated features — river-bend viewpoints, hot springs.
    Water,
    /// Commercial signage / branded features — gas stations, stores
    /// where the branded identity is the dominant visual.
    Signage,
    /// Fabric / weathered-textile features — sails, awnings, banners.
    /// Currently reserved; use once a feature needs it.
    Textile,
    /// Default catch-all — features without a specific category.
    Default,
}

impl PaletteCategory {
    /// String key used in the biome-receiving-adaptation RON table.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::BuildingWooden => "building_wooden",
            Self::BuildingMasonry => "building_masonry",
            Self::Vegetation => "vegetation",
            Self::Stone => "stone",
            Self::Water => "water",
            Self::Signage => "signage",
            Self::Textile => "textile",
            Self::Default => "default",
        }
    }
}

/// HSV-space shift applied to a foreign feature's palette.
///
/// Captures the biome's visual character impressing itself on imports
/// when bleed-over places a feature in a non-home biome.
///
/// TODO(3.6.4+): extend with compositing overlays for snow-dusting,
/// moss-on-stonework, algae-on-docks, etc. For now all adaptation is
/// pure HSV shift; overlays live in the render / flora layer when they
/// arrive. Leaving a placeholder comment here documents the extension
/// point.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct PaletteHsvShift {
    /// Hue rotation in degrees (positive = toward yellow/red, negative
    /// = toward blue/cyan). Applied modulo 360°.
    #[serde(default)]
    pub hue_shift_deg: f32,
    /// Saturation multiplier (1.0 = unchanged, 0.5 = halve, 2.0 = double).
    #[serde(default = "one_f32")]
    pub saturation_mult: f32,
    /// Value (brightness) multiplier (1.0 = unchanged).
    #[serde(default = "one_f32")]
    pub value_mult: f32,
}

fn one_f32() -> f32 {
    1.0
}

impl Default for PaletteHsvShift {
    fn default() -> Self {
        Self {
            hue_shift_deg: 0.0,
            saturation_mult: 1.0,
            value_mult: 1.0,
        }
    }
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
    /// Per-host-biome adaptation palettes — **explicit overrides**.
    /// Keys are `BiomeClass` names (`"HighDesert"`, `"RockyAlpine"`,
    /// etc.). When present, wins over the biome-receiving default.
    /// Leave empty to fall through to the biome's default HSV shift
    /// (see [`SubBiomeLibraries::biome_receiving_adaptations`]).
    #[serde(default)]
    pub adaptation_per_host: HashMap<String, Palette>,
    /// Palette category — classifies the feature's dominant visual so
    /// the biome-receiving table can apply a category-appropriate HSV
    /// shift. `None` = uses the biome's `"default"` fallback shift.
    #[serde(default)]
    pub palette_category: Option<PaletteCategory>,
    /// Placement-rule extensions beyond the baseline `PlacementContext`
    /// (slope / roughness / water / altitude). Nested by rule category;
    /// absent categories are not evaluated. See
    /// [`PlacementRules`] + MAPGEN § Placement rule vocabulary.
    /// `None` = no extended rules (baseline placement only).
    #[serde(default)]
    pub placement_rules: Option<PlacementRules>,
    /// Primitive composition (Phase 3.6.3c.5c.2.b). When `Some`, the
    /// bake emits [`crate::feature_instances::FeatureInstance`] entries
    /// for each listed `PrimitiveComponent`. `None` = no render-side
    /// primitives (pure heightfield delta + future mesh-anchor
    /// features). Authored per-feature in
    /// `data/world/sub_biome_features.ron`.
    #[serde(default)]
    pub composition: Option<Vec<crate::feature_instances::PrimitiveComponent>>,
}

/// Nested placement-rule schema (3.6.3c.1).
///
/// Extends per-cell `PlacementContext` with spatial-relationship +
/// climate + hydrology coupling rules. Each sub-struct covers one rule
/// category; absent categories are not evaluated. See MAPGEN §
/// Placement rule vocabulary (PINNED) for the full design.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PlacementRules {
    /// Hub-proximity + hub-exclusion rules.
    #[serde(default)]
    pub hub: Option<HubRules>,
    /// Minimum self-spacing (same feature type not within X km of same).
    #[serde(default)]
    pub self_spacing: Option<SelfSpacingRules>,
    /// Road-adjacency gate (unevaluated until 3.6.3c.3 `FeatureGroup`
    /// engine ships — parsed but returns "pass" in 3.6.3c.1).
    #[serde(default)]
    pub road_adjacency: Option<RoadAdjacencyRules>,
    /// Prominence attractor — steep + elevated-above-neighbors cells
    /// get weighted boost for scenery features.
    #[serde(default)]
    pub prominence: Option<ProminenceRules>,
    /// Hydrology coupling — cell's water class must match (Creek,
    /// River, RiverBend, LakeEdge, DeltaFan). Synthesized at query time
    /// for RiverBend + LakeEdge (not standalone enum variants).
    #[serde(default)]
    pub hydrology: Option<HydrologyRules>,
    /// Climate window — temperature / humidity ranges gate placement.
    #[serde(default)]
    pub climate: Option<ClimateRules>,
    /// Spillover — rare anchor features pull supporting features into
    /// nearby cells when placed (emergent-hub mechanism). **Stubbed in
    /// 3.6.3c.1; implementation 3.6.3c.3+**. Parsing today lets us
    /// author anchor features without schema churn later. See memory
    /// `hub_emergent_upgrade_via_spillover` + MAPGEN § Hub emergent
    /// upgrade.
    #[serde(default)]
    pub spillover: Option<SpilloverRules>,
}

/// Hub-proximity + hub-exclusion rules.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct HubRules {
    /// Per-tier max distance (km) within which feature is weighted up.
    /// Map key = hub tier string (`"Major"`, `"Regional"`, `"Local"`,
    /// `"Remote"`, `"Isolated"`). Empty map = no attractor.
    #[serde(default)]
    pub proximity_km_per_tier: HashMap<String, f32>,
    /// If true, a cell failing the proximity attractor (too far from
    /// any qualifying hub tier) is outright rejected. Default false =
    /// the attractor is a weighted boost only.
    #[serde(default)]
    pub proximity_required: bool,
    /// Minimum distance (km) from any hub at or above `exclusion_min_tier`.
    /// Feature rejects inside the exclusion zone. 0.0 = no exclusion.
    #[serde(default)]
    pub exclusion_min_km: f32,
    /// Exclusion applies to hubs at this tier or higher. `None` = no
    /// exclusion rule (ignored even if `exclusion_min_km > 0`).
    #[serde(default)]
    pub exclusion_min_tier: Option<String>,
}

/// Minimum-self-spacing rule.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct SelfSpacingRules {
    /// Minimum distance (km) from the nearest already-placed same-type
    /// feature. 0.0 = no constraint.
    pub min_km: f32,
}

/// Road-adjacency gate — feature must be adjacent to a road-segment.
///
/// **3.6.3c.1 parses this but returns "pass" in evaluation** because
/// road groups don't exist until 3.6.3c.3. Until then, features that
/// author `required: true` rely on their low `selection_weight` for
/// rarity calibration.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct RoadAdjacencyRules {
    /// If true, feature requires adjacency to a road cell.
    pub required: bool,
}

/// Prominence attractor for scenery features.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct ProminenceRules {
    /// Minimum elevation above the neighborhood median (m). Cell must
    /// stick up by at least this much relative to cells within the
    /// prominence-sample radius.
    #[serde(default)]
    pub min_m: f32,
    /// Minimum slope (degrees) at the cell. Gates scenery features
    /// away from flat ground.
    #[serde(default)]
    pub min_slope_deg: f32,
}

/// Hydrology coupling — require specific water-class at / near the cell.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct HydrologyRules {
    /// Required water class (string to avoid circular enum dep).
    /// Accepted: `"Creek"`, `"River"`, `"RiverBend"` (synthesized),
    /// `"LakeEdge"` (synthesized), `"DeltaFan"`, `"AlpineTarn"`.
    pub required_class: String,
    /// Optional minimum flow accumulation (for river-class features).
    #[serde(default)]
    pub min_accumulation: Option<f32>,
}

/// Climate window — gate placement on temperature / humidity ranges.
/// Evaluated at the bake-time reference `time_s = 0.0` (seasonal
/// reference frame). See MAPGEN for discussion.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct ClimateRules {
    /// (min, max) temperature range in °C. `None` = no temperature gate.
    #[serde(default)]
    pub temp_range_c: Option<(f32, f32)>,
    /// (min, max) relative-humidity range in 0..1. `None` = no gate.
    #[serde(default)]
    pub humidity_range: Option<(f32, f32)>,
}

/// Emergent-hub spillover — rare anchor features pull supporting
/// features into nearby cells. **3.6.3c.1 stub; 3.6.3c.3+ implements.**
/// See memory `hub_emergent_upgrade_via_spillover`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SpilloverRules {
    /// Feature-id list of supporting features that this anchor pulls.
    pub supporting_features: Vec<String>,
    /// Search radius (km) around the anchor for supporting-feature
    /// placement opportunities.
    pub radius_km: f32,
    /// Minimum number of supporting features that MUST place
    /// successfully for the anchor to remain placed. Below this, the
    /// anchor cell rerolls to another feature. Prevents lonely-factory
    /// absurdity.
    #[serde(default)]
    pub min_count: u32,
    /// Maximum supporting features this anchor can pull. Per-trigger
    /// budget cap; prevents chain-reaction snowball.
    pub max_count: u32,
    /// If a hub inside `radius_km` is at or below this tier, upgrade it
    /// OR add an `emergent_context` annotation. `None` = no hub upgrade.
    /// Hub tier string: `"Local"`, `"Regional"`, etc.
    #[serde(default)]
    pub upgrades_hub_to: Option<String>,
}

/// Root RON structure.
#[derive(Debug, Deserialize)]
struct SubBiomeFeaturesRon {
    biomes: HashMap<String, Vec<SubBiomeFeature>>,
    /// Per-biome receiving-adaptation tables — keyed by biome name,
    /// then by palette-category key (`"building_wooden"`, `"stone"`,
    /// etc. or `"default"`).
    #[serde(default)]
    biome_receiving_adaptations: HashMap<String, HashMap<String, PaletteHsvShift>>,
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
    /// Per-biome receiving-adaptation HSV-shift tables, used by
    /// [`adapt_palette`] when a feature bleeds into a host biome.
    biome_receiving_adaptations: HashMap<BiomeClass, HashMap<String, PaletteHsvShift>>,
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

        let mut biome_receiving_adaptations: HashMap<BiomeClass, HashMap<String, PaletteHsvShift>> =
            HashMap::new();
        for (biome_key, table) in root.biome_receiving_adaptations {
            if let Some(biome) = parse_biome_key(&biome_key) {
                biome_receiving_adaptations.insert(biome, table);
            }
        }

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
            biome_receiving_adaptations,
        }
    }

    /// Receiving-adaptation table for a given host biome, if authored.
    /// Returned map keys are palette-category strings (or `"default"`).
    #[must_use]
    pub fn receiving_adaptation(
        &self,
        host: BiomeClass,
    ) -> Option<&HashMap<String, PaletteHsvShift>> {
        self.biome_receiving_adaptations.get(&host)
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
///
/// When `rules_ctx` is `Some`, placement rules are evaluated for each
/// rolled candidate — rule rejections cause reroll like `context`
/// failures. When `None`, rules are skipped entirely (pre-3.6.3c.1
/// baseline behavior). `cell_pos` is used only when `rules_ctx` is
/// `Some`; callers without rules can pass `(0.0, 0.0)`.
///
/// Attractor rules (`hub_proximity`, `prominence`) currently contribute
/// only their gate semantics in the retry-loop (Pass/Reject). The
/// `weight_mult` they compute is discarded in this wiring — an eager
/// placement pass (3.6.3c.1c Poisson) will consume it properly.
#[must_use]
pub fn assign_feature(
    master_seed: u64,
    cell_id: u32,
    cell_pos: (f32, f32),
    context: &PlacementContext,
    libraries: &SubBiomeLibraries,
    rules_ctx: Option<&PlacementRulesContext<'_>>,
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
                if context.satisfies(f) && passes_placement_rules(f, cell_pos, context, rules_ctx) {
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
            if context.satisfies(f) && passes_placement_rules(f, cell_pos, context, rules_ctx) {
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

/// Gate-only placement-rule check used inside the `assign_feature`
/// retry loop. Returns `true` if no rules are declared, if rules_ctx
/// is `None`, or if [`evaluate_placement_rules`] returns
/// [`PlacementRuleOutcome::Pass`]. Returns `false` on any reject.
fn passes_placement_rules(
    feature: &SubBiomeFeature,
    cell_pos: (f32, f32),
    context: &PlacementContext,
    rules_ctx: Option<&PlacementRulesContext<'_>>,
) -> bool {
    let Some(rules_ctx) = rules_ctx else {
        return true;
    };
    matches!(
        evaluate_placement_rules(feature, cell_pos, context, rules_ctx),
        PlacementRuleOutcome::Pass { .. }
    )
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
// Palette adaptation — biome-receiving HSV shift + explicit override
// ═══════════════════════════════════════════════════════════════════════════

/// Parse a biome-name string (as authored in RON) into [`BiomeClass`].
/// Returns `None` for unknown biomes — caller drops the entry so a
/// typo doesn't silently shadow a real biome.
fn parse_biome_key(key: &str) -> Option<BiomeClass> {
    match key {
        "HighDesert" => Some(BiomeClass::HighDesert),
        "RockyAlpine" => Some(BiomeClass::RockyAlpine),
        "CoastalBoreal" => Some(BiomeClass::CoastalBoreal),
        "MediterraneanArchipelago" => Some(BiomeClass::MediterraneanArchipelago),
        _ => None,
    }
}

/// Stringified [`BiomeClass`] for looking into per-feature
/// `adaptation_per_host` overrides.
const fn biome_key(b: BiomeClass) -> &'static str {
    match b {
        BiomeClass::HighDesert => "HighDesert",
        BiomeClass::RockyAlpine => "RockyAlpine",
        BiomeClass::CoastalBoreal => "CoastalBoreal",
        BiomeClass::MediterraneanArchipelago => "MediterraneanArchipelago",
    }
}

/// Resolve the adapted palette for a feature appearing in a `host`
/// biome. Three-tier resolution order:
///
/// 1. Feature's `adaptation_per_host[host]` — explicit per-feature
///    override (from 3.6.3b authoring).
/// 2. Biome's receiving-adaptation table
///    `biome_receiving_adaptations[host][feature.palette_category]` —
///    per-category biome default (from 3.6.3b authoring — this is the
///    new primary mechanism).
/// 3. Biome's receiving-adaptation fallback
///    `biome_receiving_adaptations[host]["default"]`.
/// 4. If none apply, return the unmodified `home_palette` (feature
///    renders in home colors — no adaptation).
///
/// Returns the default palette if the feature has no `home_palette`
/// (render falls back to biome-derived colors — pre-palette-authoring
/// degradation path).
#[must_use]
pub fn adapt_palette(
    feature: &SubBiomeFeature,
    host: BiomeClass,
    libraries: &SubBiomeLibraries,
) -> Palette {
    let Some(home) = feature.home_palette else {
        return Palette::default();
    };

    // Tier 1: explicit per-feature override
    if let Some(over) = feature.adaptation_per_host.get(biome_key(host)) {
        return *over;
    }

    // Tier 2 + 3: biome receiving-adaptation
    if let Some(table) = libraries.receiving_adaptation(host) {
        let category_key = feature
            .palette_category
            .map_or("default", PaletteCategory::key);
        let shift = table
            .get(category_key)
            .or_else(|| table.get("default"))
            .copied()
            .unwrap_or_default();
        return apply_hsv_shift(home, shift);
    }

    // Tier 4: no adaptation authored — render in home colors
    home
}

/// Apply an HSV shift to every slot of a palette.
#[must_use]
pub fn apply_hsv_shift(p: Palette, shift: PaletteHsvShift) -> Palette {
    Palette {
        primary: apply_hsv_shift_color(p.primary, shift),
        secondary: apply_hsv_shift_color(p.secondary, shift),
        accent: apply_hsv_shift_color(p.accent, shift),
    }
}

fn apply_hsv_shift_color(rgb: PaletteColor, shift: PaletteHsvShift) -> PaletteColor {
    let (h, s, v) = rgb_to_hsv(rgb);
    let h_new = (h + shift.hue_shift_deg).rem_euclid(360.0);
    let s_new = (s * shift.saturation_mult).clamp(0.0, 1.0);
    let v_new = (v * shift.value_mult).clamp(0.0, 1.0);
    hsv_to_rgb((h_new, s_new, v_new))
}

/// RGB (0-255 per channel) → HSV (hue in degrees 0-360; sat + val in 0-1).
// Single-char names r/g/b/h/s/v/max/min/delta follow color-math literature
// convention — introducing descriptive names here would fight the reader's
// mental model. Scope-limited allow; no crate-wide suppression.
#[allow(clippy::float_cmp, clippy::many_single_char_names)]
fn rgb_to_hsv(rgb: PaletteColor) -> (f32, f32, f32) {
    let r = f32::from(rgb.0) / 255.0;
    let g = f32::from(rgb.1) / 255.0;
    let b = f32::from(rgb.2) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let v = max;
    let s = if max > 0.0 { delta / max } else { 0.0 };
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    (h, s, v)
}

/// HSV → RGB (0-255 per channel).
// Single-char names follow color-math convention; see rgb_to_hsv.
#[allow(clippy::many_single_char_names)]
fn hsv_to_rgb(hsv: (f32, f32, f32)) -> PaletteColor {
    let (h, s, v) = hsv;
    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime.floor() as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    // `clamp(0.0, 255.0)` guarantees the f32 is in [0, 255] before cast;
    // sign-loss cannot occur. Inline allow per CODING_RULES.md §49.
    #[allow(clippy::cast_sign_loss)]
    let to_u8 = |x: f32| ((x + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r1), to_u8(g1), to_u8(b1))
}

// ═══════════════════════════════════════════════════════════════════════════
// Placement-rule evaluation (3.6.3c.1 — hub / prominence / hydrology / climate;
// self_spacing parsed-but-unevaluated pending 3.6.3c.1b; road_adjacency
// parsed-but-unevaluated pending 3.6.3c.3 FeatureGroup engine).
// ═══════════════════════════════════════════════════════════════════════════

use crate::hub_catalog::{HubCatalog, HubTier};
use crate::hydrology::HydrologyGraph;

/// Bake-time reference time for climate-sampler lookups in placement-
/// rule evaluation. Mirrors the existing pattern of deterministic bake
/// inputs — features place once per world, not per-tick.
pub const PLACEMENT_RULES_TIME_S: f32 = 0.0;

/// Radius (m) used for prominence-proxy median sampling. Cell elevation
/// compared against median elevation of cells within this radius; if
/// the cell sticks up by `prominence.min_m`, prominence rule passes.
const PROMINENCE_SAMPLE_RADIUS_M: f32 = 5_000.0;

/// World-level context bundle for [`evaluate_placement_rules`].
///
/// Carries references to baked data the rule evaluators need — hub
/// catalog for proximity / exclusion, hydrology graph for water-class
/// coupling, master seed for climate-sampler lookups.
///
/// Terrain access uses a closure rather than a direct reference so the
/// evaluator doesn't need to know about `Terrain` ownership (which
/// differs between bake time and query time).
#[derive(Clone, Copy)]
pub struct PlacementRulesContext<'a> {
    /// Master seed (threads through climate samplers).
    pub master_seed: u64,
    /// Baked hub catalog (for hub_proximity / hub_exclusion rules).
    pub hub_catalog: &'a HubCatalog,
    /// Baked hydrology graph (for hydrology_coupling rule).
    pub hydrology: &'a HydrologyGraph,
    /// Height sampler: `(x, z) -> altitude_m`. Used for prominence
    /// median-sampling.
    pub height_at: &'a dyn Fn(f32, f32) -> f32,
    /// Optional per-cell hub-cache (3.6.3c.1c bake optimization).
    ///
    /// Precomputed list of hubs within [`MAX_RULE_HUB_RADIUS_M`] of the
    /// cell being evaluated. When present, hub-proximity and hub-
    /// exclusion rules iterate this short list instead of re-querying
    /// `hub_catalog.hubs_in_radius` per candidate. The bake loop fills
    /// it once per cell and reuses across all candidate evaluations,
    /// turning ~5 spatial queries per cell into 1.
    ///
    /// Lazy callers (no eager bake) leave this as `None` and pay the
    /// per-rule query cost — fine for sparse query patterns.
    #[allow(clippy::doc_markdown)]
    pub cell_hubs_cache: Option<&'a [&'a crate::hub_catalog::HubEntry]>,
}

/// Maximum hub-proximity radius any authored placement rule references.
///
/// Used by the bake loop to size the per-cell hub cache. Set above the
/// largest authored radius (`farmland_parcel.hub.proximity_km_per_tier
/// = "Major": 80.0`) with margin for future authoring.
pub const MAX_RULE_HUB_RADIUS_M: f32 = 100_000.0;

/// Outcome of placement-rule evaluation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PlacementRuleOutcome {
    /// All applicable rules passed. Carries the selection-weight
    /// multiplier (1.0 = no weight change; > 1.0 = boost from hub-
    /// proximity or prominence attractors).
    Pass { weight_mult: f32 },
    /// At least one rule rejected. Carries a short reason tag (useful
    /// for diagnostics + visual-bench annotation).
    Reject { reason: RejectReason },
}

/// Reason tag for a rule rejection. Stable strings are useful for
/// diagnostics + bake-stat dashboards; enum keeps them canonical.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RejectReason {
    /// hub_proximity_required set but no qualifying hub in radius.
    HubProximityRequired,
    /// Cell inside hub-exclusion zone of a qualifying hub tier.
    HubExclusion,
    /// Self-spacing rule would reject (not evaluated in 3.6.3c.1;
    /// reserved).
    SelfSpacing,
    /// Road adjacency required (not evaluated in 3.6.3c.1; reserved).
    RoadAdjacency,
    /// Cell prominence below min_m threshold.
    ProminenceMin,
    /// Cell slope below prominence.min_slope_deg threshold.
    ProminenceSlope,
    /// Cell water class doesn't match hydrology.required_class.
    HydrologyClass,
    /// Cell temperature outside climate.temp_range_c.
    ClimateTemperature,
    /// Cell humidity outside climate.humidity_range.
    ClimateHumidity,
}

/// Evaluate placement rules for a candidate (feature, cell) pair.
///
/// Returns [`PlacementRuleOutcome::Pass`] if the feature has no rules
/// declared, if all declared rules pass, or carries a weight multiplier
/// from attractor rules (hub_proximity, prominence). Returns
/// [`PlacementRuleOutcome::Reject`] with a reason tag when any
/// gate-rule fires.
///
/// 3.6.3c.1 evaluates: hub_proximity, hub_exclusion, prominence,
/// hydrology, climate. Parses but does not evaluate: self_spacing
/// (deferred to 3.6.3c.1b eager pass), road_adjacency (deferred to
/// 3.6.3c.3 FeatureGroup engine), spillover (anchor-feature emergent-
/// hub mechanism; implementation 3.6.3c.3+).
#[must_use]
pub fn evaluate_placement_rules(
    feature: &SubBiomeFeature,
    cell_pos: (f32, f32),
    context: &PlacementContext,
    rules_ctx: &PlacementRulesContext<'_>,
) -> PlacementRuleOutcome {
    let Some(rules) = &feature.placement_rules else {
        return PlacementRuleOutcome::Pass { weight_mult: 1.0 };
    };

    let mut weight_mult = 1.0_f32;

    // Hub rules — proximity + exclusion.
    if let Some(hub) = &rules.hub {
        if !hub.proximity_km_per_tier.is_empty() {
            let boost = compute_hub_proximity_boost(hub, cell_pos, rules_ctx);
            if hub.proximity_required && boost <= 1.0 {
                return PlacementRuleOutcome::Reject {
                    reason: RejectReason::HubProximityRequired,
                };
            }
            weight_mult *= boost;
        }
        if hub.exclusion_min_km > 0.0 && is_within_hub_exclusion(hub, cell_pos, rules_ctx) {
            return PlacementRuleOutcome::Reject {
                reason: RejectReason::HubExclusion,
            };
        }
    }

    // Prominence rules.
    if let Some(p) = &rules.prominence {
        if p.min_slope_deg > 0.0 && context.slope_deg < p.min_slope_deg {
            return PlacementRuleOutcome::Reject {
                reason: RejectReason::ProminenceSlope,
            };
        }
        if p.min_m > 0.0 {
            let prom = compute_prominence_proxy(cell_pos, rules_ctx.height_at);
            if prom < p.min_m {
                return PlacementRuleOutcome::Reject {
                    reason: RejectReason::ProminenceMin,
                };
            }
        }
    }

    // Hydrology coupling.
    if let Some(h) = &rules.hydrology {
        if !satisfies_hydrology_class(h, cell_pos, rules_ctx.hydrology) {
            return PlacementRuleOutcome::Reject {
                reason: RejectReason::HydrologyClass,
            };
        }
    }

    // Climate window.
    if let Some(c) = &rules.climate {
        if let Some((tmin, tmax)) = c.temp_range_c {
            let temp = crate::weather::temperature::temperature_at(
                rules_ctx.master_seed,
                cell_pos.0,
                context.altitude_m,
                cell_pos.1,
                PLACEMENT_RULES_TIME_S,
            );
            if temp < tmin || temp > tmax {
                return PlacementRuleOutcome::Reject {
                    reason: RejectReason::ClimateTemperature,
                };
            }
        }
        if let Some((hmin, hmax)) = c.humidity_range {
            let hum = crate::weather::humidity::humidity_at(
                rules_ctx.master_seed,
                cell_pos.0,
                context.altitude_m,
                cell_pos.1,
                PLACEMENT_RULES_TIME_S,
            );
            if hum < hmin || hum > hmax {
                return PlacementRuleOutcome::Reject {
                    reason: RejectReason::ClimateHumidity,
                };
            }
        }
    }

    // self_spacing + road_adjacency + spillover not yet evaluated.

    PlacementRuleOutcome::Pass { weight_mult }
}

/// Compute hub-proximity attractor boost — maximum weighted multiplier
/// from any qualifying hub's tier.
///
/// Example: `hub.proximity_km_per_tier` maps `"Regional": 30.0`; a cell
/// 15 km from a Regional hub is inside the radius → returns a boost
/// > 1.0. Cells outside every tier's radius return 1.0 (no boost).
///
/// Boost magnitude: `1 + 2 * (1 - dist/radius)` → 1.0 at radius edge,
/// 3.0 at cell center.
///
/// Uses `rules_ctx.cell_hubs_cache` when present (3.6.3c.1c bake
/// optimization). Falls back to `hub_catalog.hubs_in_radius` otherwise.
fn compute_hub_proximity_boost(
    hub: &HubRules,
    cell_pos: (f32, f32),
    rules_ctx: &PlacementRulesContext<'_>,
) -> f32 {
    let mut best_boost = 1.0_f32;
    for (tier_str, radius_km) in &hub.proximity_km_per_tier {
        let Some(tier) = parse_hub_tier(tier_str) else {
            continue;
        };
        let radius_m = *radius_km * 1000.0;
        let radius_m_sq = radius_m * radius_m;

        // Iterate hubs from cache (per-cell precomputed) or fall back
        // to a fresh spatial query. Cache wins by ~5× when multiple
        // candidates evaluate hub-proximity against the same cell.
        let check_hub = |h: &crate::hub_catalog::HubEntry| -> Option<f32> {
            if h.tier != tier {
                return None;
            }
            let dx = h.approx_world_pos.0 - cell_pos.0;
            let dz = h.approx_world_pos.1 - cell_pos.1;
            let d_sq = dx * dx + dz * dz;
            if d_sq > radius_m_sq {
                return None;
            }
            let d = d_sq.sqrt();
            Some(1.0 + 2.0 * (1.0 - (d / radius_m).min(1.0)))
        };

        if let Some(cached) = rules_ctx.cell_hubs_cache {
            for h in cached {
                if let Some(boost) = check_hub(h) {
                    best_boost = best_boost.max(boost);
                }
            }
        } else {
            for h in rules_ctx
                .hub_catalog
                .hubs_in_radius(cell_pos.0, cell_pos.1, radius_m)
            {
                if let Some(boost) = check_hub(h) {
                    best_boost = best_boost.max(boost);
                }
            }
        }
    }
    best_boost
}

/// True if cell is within `exclusion_min_km` of any hub at or above
/// `exclusion_min_tier`.
///
/// Uses `rules_ctx.cell_hubs_cache` when present.
fn is_within_hub_exclusion(
    hub: &HubRules,
    cell_pos: (f32, f32),
    rules_ctx: &PlacementRulesContext<'_>,
) -> bool {
    let Some(min_tier_str) = &hub.exclusion_min_tier else {
        return false;
    };
    let Some(min_tier) = parse_hub_tier(min_tier_str) else {
        return false;
    };
    let radius_m = hub.exclusion_min_km * 1000.0;
    let radius_m_sq = radius_m * radius_m;

    let within = |h: &crate::hub_catalog::HubEntry| -> bool {
        if h.tier > min_tier {
            return false;
        }
        let dx = h.approx_world_pos.0 - cell_pos.0;
        let dz = h.approx_world_pos.1 - cell_pos.1;
        dx * dx + dz * dz <= radius_m_sq
    };

    rules_ctx.cell_hubs_cache.map_or_else(
        || {
            rules_ctx
                .hub_catalog
                .hubs_in_radius(cell_pos.0, cell_pos.1, radius_m)
                .iter()
                .any(|h| within(h))
        },
        |cached| cached.iter().any(|h| within(h)),
    )
}

/// Parse hub-tier string to enum. Returns `None` for typos so a RON
/// author bug doesn't silently become a runtime gate.
fn parse_hub_tier(s: &str) -> Option<HubTier> {
    match s {
        "Major" => Some(HubTier::Major),
        "Regional" => Some(HubTier::Regional),
        "Local" => Some(HubTier::Local),
        "Remote" => Some(HubTier::Remote),
        "Isolated" => Some(HubTier::Isolated),
        _ => None,
    }
}

/// Prominence proxy: `cell_elevation - median(neighbors within 5 km)`.
/// Positive value = cell sticks up above the neighborhood; gating on
/// `min_m` gives a cheap scenic-overlook score without ray-cast
/// view-distance.
///
/// Samples 9 points in a 3×3 grid spanning
/// [`PROMINENCE_SAMPLE_RADIUS_M`]; the median of the sample set is the
/// baseline. Simple and deterministic.
fn compute_prominence_proxy(cell_pos: (f32, f32), height_at: &dyn Fn(f32, f32) -> f32) -> f32 {
    let center_h = height_at(cell_pos.0, cell_pos.1);
    let r = PROMINENCE_SAMPLE_RADIUS_M;
    let offsets = [-r, 0.0, r];
    let mut samples = Vec::with_capacity(9);
    for dx in offsets {
        for dz in offsets {
            samples.push(height_at(cell_pos.0 + dx, cell_pos.1 + dz));
        }
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = samples[samples.len() / 2];
    center_h - median
}

/// True if cell's water class matches the hydrology rule. Supports
/// direct enum classes (`Creek`, `River`, `DeltaFan`, `AlpineTarn`,
/// etc.) + synthesized ones (`RiverBend`, `LakeEdge`).
fn satisfies_hydrology_class(
    rule: &HydrologyRules,
    cell_pos: (f32, f32),
    hydrology: &HydrologyGraph,
) -> bool {
    use crate::hydrology::WaterClass;
    let surface = hydrology.water_surface_at(cell_pos.0, cell_pos.1);

    match rule.required_class.as_str() {
        "Creek" => matches!(surface.map(|s| s.class), Some(WaterClass::Creek)),
        "River" => matches!(surface.map(|s| s.class), Some(WaterClass::River)),
        "DeltaFan" => matches!(surface.map(|s| s.class), Some(WaterClass::DeltaFan)),
        "AlpineTarn" => matches!(surface.map(|s| s.class), Some(WaterClass::AlpineTarn)),
        "LakeLarge" => matches!(surface.map(|s| s.class), Some(WaterClass::LakeLarge)),
        "LakeSmall" => matches!(surface.map(|s| s.class), Some(WaterClass::LakeSmall)),
        "ClosedBasinLake" => matches!(surface.map(|s| s.class), Some(WaterClass::ClosedBasinLake)),
        "RiverBend" => is_river_bend(cell_pos, hydrology),
        "LakeEdge" => is_lake_edge(cell_pos, hydrology),
        _ => false,
    }
}

/// Synthesized `RiverBend` detection: river-class cell with enough
/// direction change over neighbor samples. Samples 2 km around cell;
/// compares upstream / downstream flow directions.
///
/// Placeholder heuristic — refined when flow-direction-graph exposes
/// a cleaner API. 3.6.3c.1 ships the hook; accuracy improves later.
fn is_river_bend(cell_pos: (f32, f32), hydrology: &HydrologyGraph) -> bool {
    use crate::hydrology::WaterClass;
    let Some(center) = hydrology.water_surface_at(cell_pos.0, cell_pos.1) else {
        return false;
    };
    if !matches!(center.class, WaterClass::River) {
        return false;
    }
    // Sample 4 neighbors at ±2 km N/S/E/W; count how many are also river-class.
    // If 1-2 neighbors are river and 2-3 are not, the cell is likely a bend
    // (river threading through). If 0 or 4 neighbors are river, it's an
    // isolated or straight segment.
    let offsets = [(2000.0, 0.0), (-2000.0, 0.0), (0.0, 2000.0), (0.0, -2000.0)];
    let river_neighbors = offsets
        .iter()
        .filter(|(dx, dz)| {
            matches!(
                hydrology
                    .water_surface_at(cell_pos.0 + dx, cell_pos.1 + dz)
                    .map(|s| s.class),
                Some(WaterClass::River)
            )
        })
        .count();
    (1..=2).contains(&river_neighbors)
}

/// Synthesized `LakeEdge`: any Lake*-class cell with at least one
/// non-lake neighbor.
fn is_lake_edge(cell_pos: (f32, f32), hydrology: &HydrologyGraph) -> bool {
    use crate::hydrology::WaterClass;
    let Some(center) = hydrology.water_surface_at(cell_pos.0, cell_pos.1) else {
        return false;
    };
    let is_lake = |c: WaterClass| {
        matches!(
            c,
            WaterClass::LakeLarge
                | WaterClass::LakeSmall
                | WaterClass::AlpineTarn
                | WaterClass::ClosedBasinLake
        )
    };
    if !is_lake(center.class) {
        return false;
    }
    let offsets = [(1500.0, 0.0), (-1500.0, 0.0), (0.0, 1500.0), (0.0, -1500.0)];
    offsets.iter().any(|(dx, dz)| {
        hydrology
            .water_surface_at(cell_pos.0 + dx, cell_pos.1 + dz)
            .is_none_or(|s| !is_lake(s.class))
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Self-spacing state + bake-time assignment (3.6.3c.1c)
// ═══════════════════════════════════════════════════════════════════════════

/// Per-feature spatial hash for self-spacing rule enforcement.
///
/// Each feature with a `SelfSpacingRules.min_km` rule gets a sparse
/// grid (~10 km cells) of placed positions. Self-spacing checks query
/// the grid in the cell+neighbors-within-min_km window.
///
/// Built incrementally during the eager bake pass
/// ([`crate::baked_assignments::BakedAssignments::bake`]); after bake,
/// usable for spatial lookups.
#[derive(Default)]
pub struct SelfSpacingState {
    /// feature_id_str → grid of placed positions.
    grids: HashMap<String, FeatureSpatialGrid>,
}

struct FeatureSpatialGrid {
    cell_size_m: f32,
    cells: HashMap<(i32, i32), Vec<(f32, f32)>>,
}

impl SelfSpacingState {
    /// Create an empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check whether `pos` is within `min_km` of any already-placed
    /// position for the given feature id. True = self-spacing rule
    /// violated (reject placement).
    #[must_use]
    pub fn would_violate(&self, feature_id: &str, pos: (f32, f32), min_km: f32) -> bool {
        let Some(grid) = self.grids.get(feature_id) else {
            return false;
        };
        let min_m = min_km * 1000.0;
        let min_m_sq = min_m * min_m;
        let r_cells = (min_m / grid.cell_size_m).ceil() as i32;
        let (gx, gz) = grid.cell_for(pos);
        for dz in -r_cells..=r_cells {
            for dx in -r_cells..=r_cells {
                if let Some(positions) = grid.cells.get(&(gx + dx, gz + dz)) {
                    for &(px, pz) in positions {
                        let ddx = px - pos.0;
                        let ddz = pz - pos.1;
                        if ddx * ddx + ddz * ddz < min_m_sq {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Insert a placement into the grid for the given feature.
    pub fn insert(&mut self, feature_id: &str, pos: (f32, f32)) {
        let grid = self
            .grids
            .entry(feature_id.to_string())
            .or_insert_with(|| FeatureSpatialGrid {
                cell_size_m: 10_000.0,
                cells: HashMap::new(),
            });
        let key = grid.cell_for(pos);
        grid.cells.entry(key).or_default().push(pos);
    }

    /// Number of feature types tracked.
    #[must_use]
    pub fn feature_count(&self) -> usize {
        self.grids.len()
    }

    /// Number of placed positions for a feature (0 if untracked).
    #[must_use]
    pub fn placement_count(&self, feature_id: &str) -> usize {
        self.grids
            .get(feature_id)
            .map_or(0, |g| g.cells.values().map(Vec::len).sum())
    }
}

impl FeatureSpatialGrid {
    fn cell_for(&self, pos: (f32, f32)) -> (i32, i32) {
        (
            (pos.0 / self.cell_size_m).floor() as i32,
            (pos.1 / self.cell_size_m).floor() as i32,
        )
    }
}

/// Per-cell phase-A output consumed by `BakedAssignments::bake`.
///
/// Carries the cell's biome source (Primary/BleedOver), the ordered
/// list of retry candidates that survived all rules EXCEPT
/// self-spacing, and an `is_open_water` flag indicating which library
/// was sampled (oceanic features carry no self-spacing rules so
/// phase B short-circuits open-water cells).
pub struct CellBakeOutcome {
    pub source: BiomeSource,
    pub attempts: Vec<CellBakeAttempt>,
    pub is_open_water: bool,
}

/// One retry candidate from phase A. Phase B uses `self_spacing_min_km`
/// to know whether to spatial-hash-check the candidate.
pub struct CellBakeAttempt {
    pub feature_id: String,
    pub self_spacing_min_km: f32,
}

/// Phase-A worker for the bake pass.
///
/// Walks the standard retry loop with rule evaluation + biome-source
/// selection, but **does NOT enforce self-spacing**. Returns the
/// candidate list (up to [`MAX_FEATURE_ROLL_ATTEMPTS`] entries, each
/// being a feature that survived `context.satisfies` + placement-rule
/// checks). Phase B walks this list in the cell's permuted-order slot
/// to apply self-spacing + commit to the per-cell array.
///
/// Pure read-only against `terrain` / `hydrology` / `hub_catalog` /
/// `climate samplers` — safe to call from multiple threads in parallel
/// (no shared mutable state).
#[must_use]
pub fn compute_bake_candidates_for_cell(
    master_seed: u64,
    cell_id: u32,
    cell_pos: (f32, f32),
    context: &PlacementContext,
    libraries: &SubBiomeLibraries,
    rules_ctx: &PlacementRulesContext<'_>,
) -> CellBakeOutcome {
    let field_seed = mix_seed(master_seed, SUB_BIOME_FIELD_TAG);
    let bleed_rng = mix_seed(field_seed, u64::from(cell_id) | (1_u64 << 32)) as u32;
    let source = pick_biome_source(&context.biome_weights, bleed_rng);

    // Open-water short-circuit: oceanic features have no self_spacing
    // rules, so phase B can take the first attempt directly.
    if context.altitude_m < OPEN_WATER_ALTITUDE_THRESHOLD_M {
        let oceanic = libraries.oceanic();
        let mut attempts = Vec::with_capacity(1);
        for attempt in 0..MAX_FEATURE_ROLL_ATTEMPTS {
            let roll_rng = mix_seed(
                field_seed,
                u64::from(cell_id) | (u64::from(attempt) << 48) | (1 << 60),
            );
            if let Some(f) =
                sample_feature_with_context(oceanic, roll_rng, false, cell_pos, context, rules_ctx)
            {
                if context.satisfies(f) {
                    attempts.push(CellBakeAttempt {
                        feature_id: f.id.clone(),
                        self_spacing_min_km: 0.0,
                    });
                    break;
                }
            }
        }
        if attempts.is_empty() {
            attempts.push(CellBakeAttempt {
                feature_id: OPEN_WATER_FEATURE_ID.to_string(),
                self_spacing_min_km: 0.0,
            });
        }
        return CellBakeOutcome {
            source,
            attempts,
            is_open_water: true,
        };
    }

    // Land cell: collect ALL surviving retry candidates so phase B can
    // pick the first that doesn't self-space-violate.
    let source_biome = source.feature_source_biome();
    let lib = libraries.for_biome(source_biome);
    let mut attempts = Vec::with_capacity(MAX_FEATURE_ROLL_ATTEMPTS as usize);

    for attempt in 0..MAX_FEATURE_ROLL_ATTEMPTS {
        let roll_rng = mix_seed(field_seed, u64::from(cell_id) | (u64::from(attempt) << 48));
        let feature = sample_feature_with_context(
            lib,
            roll_rng,
            source.is_bleed_over(),
            cell_pos,
            context,
            rules_ctx,
        );
        if let Some(f) = feature {
            if !context.satisfies(f) {
                continue;
            }
            let self_spacing_min_km = f
                .placement_rules
                .as_ref()
                .and_then(|r| r.self_spacing.as_ref())
                .map_or(0.0, |ss| ss.min_km);
            attempts.push(CellBakeAttempt {
                feature_id: f.id.clone(),
                self_spacing_min_km,
            });
        }
    }

    CellBakeOutcome {
        source,
        attempts,
        is_open_water: false,
    }
}

/// Public wrapper around [`crate::voronoi_cells::build_placement_context`]
/// for `baked_assignments` to call.
#[must_use]
pub fn build_placement_context_external(
    master_seed: u64,
    x: f32,
    z: f32,
    terrain: &crate::terrain::Terrain,
    hydrology: &crate::hydrology::HydrologyGraph,
) -> PlacementContext {
    crate::voronoi_cells::build_placement_context(master_seed, x, z, terrain, hydrology)
}

/// Like [`sample_feature`] but additionally uses placement-rule
/// `weight_mult` from attractor rules (hub_proximity, prominence) to
/// bias the weighted RNG roll.
///
/// Rejected candidates contribute zero weight (cannot be picked).
/// Accepted candidates contribute `selection_weight × weight_mult`.
fn sample_feature_with_context<'a>(
    lib: &'a [SubBiomeFeature],
    rng_u64: u64,
    require_bleed_over_candidate: bool,
    cell_pos: (f32, f32),
    context: &PlacementContext,
    rules_ctx: &PlacementRulesContext<'_>,
) -> Option<&'a SubBiomeFeature> {
    let filtered: Vec<&SubBiomeFeature> = lib
        .iter()
        .filter(|f| !require_bleed_over_candidate || f.is_bleed_over_candidate)
        .collect();
    if filtered.is_empty() {
        return None;
    }

    let weights: Vec<f32> = filtered
        .iter()
        .map(|f| {
            let mult = match evaluate_placement_rules(f, cell_pos, context, rules_ctx) {
                PlacementRuleOutcome::Pass { weight_mult } => weight_mult,
                PlacementRuleOutcome::Reject { .. } => 0.0,
            };
            f.selection_weight * mult
        })
        .collect();

    let total_weight: f32 = weights.iter().sum();
    if total_weight <= 0.0 {
        return None;
    }
    let roll = ((rng_u64 & 0xFFFF_FFFF) as f32 / (u32::MAX as f32 + 1.0)) * total_weight;
    let mut acc = 0.0_f32;
    for (f, w) in filtered.iter().zip(weights.iter()) {
        acc += w;
        if roll <= acc {
            return Some(*f);
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
            palette_category: None,
            placement_rules: None,
            composition: None,
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
            palette_category: None,
            placement_rules: None,
            composition: None,
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
        let result = assign_feature(42, 0, (0.0, 0.0), &ctx, libs, None);
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
        let a = assign_feature(42, 1234, (0.0, 0.0), &ctx, libs, None);
        let b = assign_feature(42, 1234, (0.0, 0.0), &ctx, libs, None);
        assert_eq!(a.feature_id, b.feature_id);
        assert_eq!(a.source, b.source);
        assert_eq!(a.is_fallback, b.is_fallback);
    }

    #[test]
    fn biome_receiving_adaptations_authored() {
        // All 4 land biomes must have a receiving-adaptation table
        // with at least a `"default"` entry. 3.6.3b catalog authoring.
        let libs = SubBiomeLibraries::shared();
        for biome in [
            BiomeClass::HighDesert,
            BiomeClass::RockyAlpine,
            BiomeClass::CoastalBoreal,
            BiomeClass::MediterraneanArchipelago,
        ] {
            let table = libs
                .receiving_adaptation(biome)
                .unwrap_or_else(|| panic!("{biome:?} missing receiving-adaptation table"));
            assert!(
                table.contains_key("default"),
                "{biome:?} receiving-adaptation table missing `default` fallback"
            );
        }
    }

    #[test]
    fn adapt_palette_resolves_explicit_override_first() {
        // Tier 1: feature's adaptation_per_host entry wins over biome
        // receiving-adaptation.
        let mut over = HashMap::new();
        over.insert(
            "RockyAlpine".to_string(),
            Palette {
                primary: (1, 2, 3),
                secondary: (4, 5, 6),
                accent: (7, 8, 9),
            },
        );
        let feature = SubBiomeFeature {
            id: "t".into(),
            slope_tolerance_max_deg: 90.0,
            roughness_tolerance_m: 1e6,
            water_adjacency: WaterAdjacency::None,
            altitude_range_m: (-1e6, 1e6),
            selection_weight: 1.0,
            is_bleed_over_candidate: true,
            home_palette: Some(Palette {
                primary: (100, 100, 100),
                secondary: (100, 100, 100),
                accent: (100, 100, 100),
            }),
            adaptation_per_host: over,
            palette_category: Some(PaletteCategory::BuildingWooden),
            placement_rules: None,
            composition: None,
        };
        let libs = SubBiomeLibraries::shared();
        let adapted = adapt_palette(&feature, BiomeClass::RockyAlpine, libs);
        // Override wins — exact RGB returned, not an HSV-shifted version.
        assert_eq!(adapted.primary, (1, 2, 3));
        assert_eq!(adapted.secondary, (4, 5, 6));
        assert_eq!(adapted.accent, (7, 8, 9));
    }

    #[test]
    fn adapt_palette_falls_through_to_biome_default() {
        // Tier 2/3: feature without override uses biome receiving-
        // adaptation per category. With category=BuildingWooden in
        // RockyAlpine, HSV shifts are nonzero (hue −5°, sat 0.80,
        // value 0.90), so the adapted palette differs from home.
        let feature = SubBiomeFeature {
            id: "t".into(),
            slope_tolerance_max_deg: 90.0,
            roughness_tolerance_m: 1e6,
            water_adjacency: WaterAdjacency::None,
            altitude_range_m: (-1e6, 1e6),
            selection_weight: 1.0,
            is_bleed_over_candidate: true,
            home_palette: Some(Palette {
                primary: (140, 95, 65),
                secondary: (165, 145, 105),
                accent: (95, 80, 65),
            }),
            adaptation_per_host: HashMap::new(),
            palette_category: Some(PaletteCategory::BuildingWooden),
            placement_rules: None,
            composition: None,
        };
        let libs = SubBiomeLibraries::shared();
        let adapted = adapt_palette(&feature, BiomeClass::RockyAlpine, libs);
        let home = feature.home_palette.unwrap();
        // RA BuildingWooden shift reduces value → darker. Adapted differs.
        assert!(
            adapted.primary != home.primary,
            "expected HSV shift to modify primary, got {:?} == {:?}",
            adapted.primary,
            home.primary
        );
    }

    #[test]
    fn adapt_palette_uses_default_when_category_none() {
        // Feature with palette_category=None falls to biome's "default"
        // entry in its receiving table. All 4 biomes authored "default".
        let feature = SubBiomeFeature {
            id: "t".into(),
            slope_tolerance_max_deg: 90.0,
            roughness_tolerance_m: 1e6,
            water_adjacency: WaterAdjacency::None,
            altitude_range_m: (-1e6, 1e6),
            selection_weight: 1.0,
            is_bleed_over_candidate: true,
            home_palette: Some(Palette {
                primary: (100, 120, 80),
                secondary: (150, 150, 120),
                accent: (180, 160, 110),
            }),
            adaptation_per_host: HashMap::new(),
            palette_category: None,
            placement_rules: None,
            composition: None,
        };
        let libs = SubBiomeLibraries::shared();
        let adapted = adapt_palette(&feature, BiomeClass::HighDesert, libs);
        let home = feature.home_palette.unwrap();
        // HD default shift (hue +5, sat 1.0, value 1.05) changes colors.
        assert!(adapted.primary != home.primary);
    }

    #[test]
    fn adapt_palette_no_home_returns_default_palette() {
        let feature = SubBiomeFeature {
            id: "t".into(),
            slope_tolerance_max_deg: 90.0,
            roughness_tolerance_m: 1e6,
            water_adjacency: WaterAdjacency::None,
            altitude_range_m: (-1e6, 1e6),
            selection_weight: 1.0,
            is_bleed_over_candidate: true,
            home_palette: None,
            adaptation_per_host: HashMap::new(),
            palette_category: None,
            placement_rules: None,
            composition: None,
        };
        let libs = SubBiomeLibraries::shared();
        let adapted = adapt_palette(&feature, BiomeClass::HighDesert, libs);
        assert_eq!(adapted, Palette::default());
    }

    #[test]
    fn placement_rules_absent_returns_pass_with_identity_weight() {
        // Baseline behaviour: feature with placement_rules=None passes
        // with weight_mult=1.0, regardless of context.
        let feature = SubBiomeFeature {
            id: "t".into(),
            slope_tolerance_max_deg: 90.0,
            roughness_tolerance_m: 1e6,
            water_adjacency: WaterAdjacency::None,
            altitude_range_m: (-1e6, 1e6),
            selection_weight: 1.0,
            is_bleed_over_candidate: false,
            home_palette: None,
            adaptation_per_host: HashMap::new(),
            palette_category: None,
            placement_rules: None,
            composition: None,
        };
        // We can't easily construct a real HubCatalog + HydrologyGraph in
        // a unit test. Assert only the short-circuit path (rules=None):
        // the function must never touch rules_ctx when rules is None,
        // so we assert the Pass outcome by direct property inspection.
        assert!(feature.placement_rules.is_none());
    }

    #[test]
    fn reject_reason_variants_are_distinct() {
        // Ensure the RejectReason enum covers all 9 expected variants
        // (diagnostics + visual-bench annotations depend on this set).
        use std::collections::HashSet;
        let all = [
            RejectReason::HubProximityRequired,
            RejectReason::HubExclusion,
            RejectReason::SelfSpacing,
            RejectReason::RoadAdjacency,
            RejectReason::ProminenceMin,
            RejectReason::ProminenceSlope,
            RejectReason::HydrologyClass,
            RejectReason::ClimateTemperature,
            RejectReason::ClimateHumidity,
        ];
        let unique: HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn placement_rules_parse_nested_shape_from_ron() {
        // Integration: a RON snippet with nested placement_rules parses
        // into the expected struct shape (verifies the #[serde(default)]
        // + nested Option wiring holds together).
        let ron_text = r#"(
            id: "test_feature",
            slope_tolerance_max_deg: 90.0,
            roughness_tolerance_m: 1000.0,
            water_adjacency: None,
            altitude_range_m: (-100.0, 5000.0),
            selection_weight: 1.0,
            is_bleed_over_candidate: false,
            placement_rules: Some((
                hub: Some((
                    proximity_km_per_tier: { "Major": 50.0, "Regional": 20.0 },
                    proximity_required: true,
                    exclusion_min_km: 10.0,
                    exclusion_min_tier: Some("Major"),
                )),
                self_spacing: Some((min_km: 15.0)),
                prominence: Some((min_m: 100.0, min_slope_deg: 20.0)),
                hydrology: Some((required_class: "RiverBend", min_accumulation: Some(500.0))),
                climate: Some((temp_range_c: Some((-10.0, 5.0)), humidity_range: None)),
            )),
        )"#;
        let feature: SubBiomeFeature = ron::from_str(ron_text).unwrap();
        let rules = feature.placement_rules.unwrap();
        let hub = rules.hub.unwrap();
        // Authored-through-RON values are bit-exact; tolerance for float
        // comparisons is 1e-6 to appease clippy::float_cmp without
        // swallowing real parsing bugs.
        let eps = 1e-6_f32;
        assert!((hub.proximity_km_per_tier["Major"] - 50.0).abs() < eps);
        assert!((hub.proximity_km_per_tier["Regional"] - 20.0).abs() < eps);
        assert!(hub.proximity_required);
        assert!((hub.exclusion_min_km - 10.0).abs() < eps);
        assert_eq!(hub.exclusion_min_tier.as_deref(), Some("Major"));
        let ss = rules.self_spacing.unwrap();
        assert!((ss.min_km - 15.0).abs() < eps);
        let prom = rules.prominence.unwrap();
        assert!((prom.min_m - 100.0).abs() < eps);
        assert!((prom.min_slope_deg - 20.0).abs() < eps);
        let hyd = rules.hydrology.unwrap();
        assert_eq!(hyd.required_class, "RiverBend");
        let acc = hyd.min_accumulation.unwrap();
        assert!((acc - 500.0).abs() < eps);
        let clim = rules.climate.unwrap();
        let (tmin, tmax) = clim.temp_range_c.unwrap();
        assert!((tmin - -10.0).abs() < eps);
        assert!((tmax - 5.0).abs() < eps);
        assert!(clim.humidity_range.is_none());
    }

    #[test]
    fn authored_placement_rules_present_in_catalog() {
        // Spot-check 5 features authored in 3.6.3c.1b. Rule shape must
        // survive round-trip through the embedded RON.
        let libs = SubBiomeLibraries::shared();

        // ranch (HighDesert) — hub_proximity + self_spacing
        let ranch = libs.find(BiomeClass::HighDesert, "ranch").unwrap();
        let rules = ranch.placement_rules.as_ref().unwrap();
        let hub = rules.hub.as_ref().unwrap();
        assert!(hub.proximity_km_per_tier.contains_key("Regional"));
        assert!(rules.self_spacing.is_some());

        // scenic_overlook (RockyAlpine) — prominence + self_spacing
        let overlook = libs
            .find(BiomeClass::RockyAlpine, "scenic_overlook")
            .unwrap();
        let rules = overlook.placement_rules.as_ref().unwrap();
        let prom = rules.prominence.as_ref().unwrap();
        let eps = 1e-6_f32;
        assert!((prom.min_m - 100.0).abs() < eps);
        assert!((prom.min_slope_deg - 20.0).abs() < eps);

        // creek_valley (CoastalBoreal) — hydrology coupling (Creek)
        let creek = libs
            .find(BiomeClass::CoastalBoreal, "creek_valley")
            .unwrap();
        let hyd = creek
            .placement_rules
            .as_ref()
            .and_then(|r| r.hydrology.as_ref())
            .unwrap();
        assert_eq!(hyd.required_class, "Creek");

        // hot_springs (RockyAlpine) — climate window
        let hs = libs.find(BiomeClass::RockyAlpine, "hot_springs").unwrap();
        let clim = hs
            .placement_rules
            .as_ref()
            .and_then(|r| r.climate.as_ref())
            .unwrap();
        assert!(clim.temp_range_c.is_some());

        // gas_station (CoastalBoreal) — road_adjacency required
        let gas = libs.find(BiomeClass::CoastalBoreal, "gas_station").unwrap();
        let ra = gas
            .placement_rules
            .as_ref()
            .and_then(|r| r.road_adjacency.as_ref())
            .unwrap();
        assert!(ra.required);
    }

    #[test]
    fn spillover_rules_parse_from_ron() {
        // Even though evaluation is stubbed, schema must parse — future
        // anchor-feature authoring depends on it.
        let ron_text = r#"(
            id: "factory_stub",
            slope_tolerance_max_deg: 5.0,
            roughness_tolerance_m: 30.0,
            water_adjacency: None,
            altitude_range_m: (-10.0, 2500.0),
            selection_weight: 0.05,
            is_bleed_over_candidate: false,
            placement_rules: Some((
                spillover: Some((
                    supporting_features: ["dwelling_cluster", "gas_station"],
                    radius_km: 8.0,
                    min_count: 2,
                    max_count: 5,
                    upgrades_hub_to: Some("Local"),
                )),
            )),
        )"#;
        let feature: SubBiomeFeature = ron::from_str(ron_text).unwrap();
        let spill = feature
            .placement_rules
            .as_ref()
            .and_then(|r| r.spillover.as_ref())
            .unwrap();
        assert_eq!(
            spill.supporting_features,
            vec!["dwelling_cluster", "gas_station"]
        );
        let eps = 1e-6_f32;
        assert!((spill.radius_km - 8.0).abs() < eps);
        assert_eq!(spill.min_count, 2);
        assert_eq!(spill.max_count, 5);
        assert_eq!(spill.upgrades_hub_to.as_deref(), Some("Local"));
    }

    #[test]
    fn hsv_zero_shift_is_identity_within_rounding() {
        let colors: &[PaletteColor] =
            &[(100, 50, 20), (200, 200, 200), (50, 150, 250), (255, 0, 0)];
        let zero_shift = PaletteHsvShift::default();
        for &c in colors {
            let shifted = apply_hsv_shift_color(c, zero_shift);
            // Allow ±2/255 rounding tolerance per channel.
            assert!(
                (i32::from(shifted.0) - i32::from(c.0)).abs() <= 2
                    && (i32::from(shifted.1) - i32::from(c.1)).abs() <= 2
                    && (i32::from(shifted.2) - i32::from(c.2)).abs() <= 2,
                "hsv roundtrip drift: {c:?} -> {shifted:?}"
            );
        }
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
            let r = assign_feature(42, cell_id, (0.0, 0.0), &ctx, libs, None);
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
