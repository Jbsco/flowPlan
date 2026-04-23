//! Shore / surface classification sampler (Phase 3.6.3a).
//!
//! Derives a [`SurfaceClass`] at a world coordinate from the
//! L1 heightfield + hydrology graph + biome + sub-biome feature tag.
//! Unblocks **Milestone 2.5b.7 per-surface friction** (pending since
//! Phase 2.5 ground-contact work).
//!
//! # Classification decision order
//!
//! 1. **Hydrology water class first** — sea / lake / river → `Water`.
//! 2. **Feature-tag override** — `sandy_beach` → `Sand`, `cliff_coast`
//!    → `Cliff`, `dune_field` → `Sand`, `terracotta_port` → `Paved`,
//!    etc. Per-feature mapping handles the most visually-distinct
//!    surface choices.
//! 3. **Gradient-based fallback** — steep slope → `Rock`/`Cliff`;
//!    moderate slope → `Gravel`; gentle → biome-default.
//! 4. **Biome default** — HighDesert → `Dirt`/`Rock`, RockyAlpine at
//!    altitude → `Snow`/`Rock`, CoastalBoreal → `Grass`, Mediterranean
//!    → `Grass`.
//!
//! # Consumer contract
//!
//! 2.5b.7 consumes `surface_at(x, z)` during ground-contact physics
//! to pick per-surface rolling/lateral friction coefficients. Sim-side
//! requirement: pure function, deterministic, reasonably fast (called
//! per-wheel per-tick during contact).

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::match_same_arms)]

use crate::biome::{biome_at, BiomeClass};
use crate::hydrology::{HydrologyGraph, WaterClass};
use crate::sub_biome::FeatureAssignment;
use crate::terrain::{HeightSampler, Terrain};
use crate::voronoi_cells::SubBiomeCellCatalog;

/// Surface classification at a world coordinate. Consumed by
/// `sim::ground_contact` (Phase 2.5b.7 per-surface friction) +
/// render-side palette/material selection.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceClass {
    /// Water surface (sea / lake / river).
    Water,
    /// Sand — beach, dune field, desert wash.
    Sand,
    /// Gravel — rocky shore, loose talus foot, bush-strip surface.
    Gravel,
    /// Rock — exposed bedrock, talus, mesa face.
    Rock,
    /// Cliff — vertical rock face, near-unwalkable.
    Cliff,
    /// Mudflat — tidal mud, river delta, wet river flat.
    Mudflat,
    /// River / delta mouth sediment — soft alluvium.
    Delta,
    /// Ice edge — glacier terminus, frozen lake perimeter.
    IceEdge,
    /// Grass — meadow, lowland, arable land.
    Grass,
    /// Paved — runway, road, port pavement.
    Paved,
    /// Dirt — unpaved road, dry lake bed, scrub ground.
    Dirt,
    /// Snow — alpine + seasonal overlay.
    Snow,
    /// Powder snow — soft seasonal accumulation.
    PowderSnow,
    /// Dry lake — cracked playa, salt flat.
    DryLake,
}

impl SurfaceClass {
    /// Default rolling friction coefficient for wheels (Phase 2.5b.7
    /// stopgap — real coefficients tuned per-aircraft + surface in a
    /// data file when 2.5b.7 ships).
    #[must_use]
    pub const fn default_rolling_friction(self) -> f32 {
        match self {
            Self::Paved => 0.02,
            Self::Dirt => 0.04,
            Self::Gravel => 0.06,
            Self::Grass => 0.08,
            Self::Sand => 0.15,
            Self::Mudflat | Self::Delta => 0.18,
            Self::Snow => 0.10,
            Self::PowderSnow => 0.20,
            Self::DryLake => 0.05,
            Self::Rock | Self::Cliff | Self::IceEdge => 0.04,
            Self::Water => 0.35, // wheel skim / submerged wheel
        }
    }

    /// Does ground contact meaningfully apply to this surface?
    ///
    /// Water surfaces still apply (wheel skimming is supported — see
    /// memory `wheel_skimming_design_surface`), but at higher
    /// friction. Cliffs should be avoided entirely — contact is
    /// typically a crash event.
    #[must_use]
    pub const fn is_landable(self) -> bool {
        !matches!(self, Self::Cliff)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Classifier
// ═══════════════════════════════════════════════════════════════════════════

/// Classify the surface at world coordinate `(x, z)`.
///
/// Composes hydrology (water override) + sub-biome feature-tag
/// override + heightfield gradient + biome default into a single
/// [`SurfaceClass`]. Pure function of the world-bake state — Phase
/// 2.5b.7 consumers call this once per ground-contact tick.
#[must_use]
pub fn surface_at(
    x: f32,
    z: f32,
    master_seed: u64,
    terrain: &Terrain,
    hydrology: &HydrologyGraph,
    sub_biome: &SubBiomeCellCatalog,
) -> SurfaceClass {
    // 1. Water override — hydrology class wins first.
    if let Some(surface) = hydrology.water_surface_at(x, z) {
        if !matches!(surface.class, WaterClass::DryChannel) {
            return SurfaceClass::Water;
        }
    }

    // 2. Feature-tag override — look up the cell's feature_id and
    // dispatch to surface type if known.
    let cell_id = sub_biome.nearest_cell(x, z);
    let feature = sub_biome.feature_at_cell(cell_id, terrain, hydrology);
    if let Some(class) = feature_id_to_surface(&feature.feature_id) {
        return class;
    }

    // 3. Gradient-based — steep terrain → Rock/Cliff regardless of
    // biome.
    let sample_radius = 500.0_f32;
    let h_here = terrain.height(x, z);
    let h_east = terrain.height(x + sample_radius, z);
    let h_north = terrain.height(x, z + sample_radius);
    let dh_dx = (h_east - h_here) / sample_radius;
    let dh_dz = (h_north - h_here) / sample_radius;
    let slope_rad = libm::atan2f(dh_dx.hypot(dh_dz), 1.0);
    let slope_deg = slope_rad * (180.0 / core::f32::consts::PI);

    if slope_deg > 60.0 {
        return SurfaceClass::Cliff;
    }
    if slope_deg > 30.0 {
        return SurfaceClass::Rock;
    }

    // 4. Biome default — also incorporates altitude for snow line.
    let biome = biome_at(master_seed, x, z);
    let altitude_m = terrain.height(x, z);
    biome_default_surface(biome, altitude_m, slope_deg)
}

/// Convert a feature_id to its dominant surface class, if the feature
/// has a strong surface signature. Returns `None` for features with
/// no strong surface opinion (lake_cluster, mesa_top — surface comes
/// from the heightfield gradient or biome).
fn feature_id_to_surface(feature_id: &str) -> Option<SurfaceClass> {
    Some(match feature_id {
        // Strong surface signatures
        "sandy_beach" | "dune_field" | "coastal_plain" => SurfaceClass::Sand,
        "cliff_coast" => SurfaceClass::Cliff,
        "slickrock_canyon" | "talus_slope" | "rocky_ridge" | "rocky_islet" | "avalanche_chute" => {
            SurfaceClass::Rock
        },
        "dry_wash" | "arroyo" | "slot_canyon" | "book_cliffs" => SurfaceClass::Rock,
        "dry_lake_bed" => SurfaceClass::DryLake,
        "terracotta_port" | "small_harbor" | "rail_corridor" => SurfaceClass::Paved,
        "sagebrush_plain" | "scrub_valley" | "juniper_stand" | "sandy_port" => SurfaceClass::Dirt,
        "bush_strip_clearing" => SurfaceClass::Gravel,
        "dense_spruce_forest"
        | "pine_headland"
        | "tundra_patch"
        | "alpine_valley"
        | "alpine_meadow"
        | "natural_meadow" => SurfaceClass::Grass,
        "glacier_tongue" | "glacier_dammed_lake" => SurfaceClass::IceEdge,
        "river_floodplain" | "salmon_run_river" => SurfaceClass::Delta,
        // Weak/no surface opinion — fall through to gradient + biome.
        _ => return None,
    })
}

/// Default surface for a biome, given altitude + slope. Snow-line
/// interaction: high-altitude in RockyAlpine → Snow; mid-altitude →
/// Rock; lowland → Grass/Dirt per biome.
fn biome_default_surface(biome: BiomeClass, altitude_m: f32, slope_deg: f32) -> SurfaceClass {
    // Snow line: ~2500 m in RockyAlpine, ~1500 m in CoastalBoreal
    // (driven by biome humidity + temperature proxy).
    let snow_line_m = match biome {
        BiomeClass::RockyAlpine => 2500.0,
        BiomeClass::CoastalBoreal => 1500.0,
        BiomeClass::HighDesert => 3500.0,
        BiomeClass::MediterraneanArchipelago => 4000.0, // rare
    };
    if altitude_m > snow_line_m {
        return SurfaceClass::Snow;
    }

    // Moderate slope → Gravel (loose material accumulates).
    if slope_deg > 15.0 {
        return SurfaceClass::Gravel;
    }

    match biome {
        BiomeClass::HighDesert => SurfaceClass::Dirt,
        BiomeClass::RockyAlpine => {
            if altitude_m > 1800.0 {
                SurfaceClass::Rock
            } else {
                SurfaceClass::Grass
            }
        },
        BiomeClass::CoastalBoreal => SurfaceClass::Grass,
        BiomeClass::MediterraneanArchipelago => SurfaceClass::Grass,
    }
}

/// Unused-import silencer for `FeatureAssignment` — kept in the
/// public interface shape for future refinement (feature assignment
/// could influence classifier weighting once more features land).
#[allow(dead_code)]
fn _keep_feature_assignment_used(_f: FeatureAssignment) {}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn default_rolling_friction_paved_lowest() {
        // Paved should have the lowest friction of the land surfaces.
        let paved = SurfaceClass::Paved.default_rolling_friction();
        let grass = SurfaceClass::Grass.default_rolling_friction();
        let sand = SurfaceClass::Sand.default_rolling_friction();
        assert!(paved < grass);
        assert!(grass < sand);
    }

    #[test]
    fn cliff_is_not_landable() {
        assert!(!SurfaceClass::Cliff.is_landable());
        assert!(SurfaceClass::Sand.is_landable());
        assert!(SurfaceClass::Water.is_landable());
    }

    #[test]
    fn feature_id_to_surface_known_features() {
        assert_eq!(
            feature_id_to_surface("sandy_beach"),
            Some(SurfaceClass::Sand)
        );
        assert_eq!(
            feature_id_to_surface("cliff_coast"),
            Some(SurfaceClass::Cliff)
        );
        assert_eq!(
            feature_id_to_surface("dune_field"),
            Some(SurfaceClass::Sand)
        );
        assert_eq!(
            feature_id_to_surface("talus_slope"),
            Some(SurfaceClass::Rock)
        );
        assert_eq!(
            feature_id_to_surface("bush_strip_clearing"),
            Some(SurfaceClass::Gravel)
        );
        assert_eq!(
            feature_id_to_surface("dense_spruce_forest"),
            Some(SurfaceClass::Grass)
        );
    }

    #[test]
    fn feature_id_to_surface_unknown_returns_none() {
        assert!(feature_id_to_surface("lake_cluster").is_none()); // water handled by hydrology
        assert!(feature_id_to_surface("mesa_top").is_none()); // gradient-dependent
        assert!(feature_id_to_surface("plain").is_none());
    }

    #[test]
    fn biome_default_rocky_alpine_high_is_snow() {
        let class = biome_default_surface(BiomeClass::RockyAlpine, 3000.0, 5.0);
        assert_eq!(class, SurfaceClass::Snow);
    }

    #[test]
    fn biome_default_coastal_boreal_lowland_is_grass() {
        let class = biome_default_surface(BiomeClass::CoastalBoreal, 100.0, 5.0);
        assert_eq!(class, SurfaceClass::Grass);
    }

    #[test]
    fn biome_default_desert_lowland_is_dirt() {
        let class = biome_default_surface(BiomeClass::HighDesert, 800.0, 5.0);
        assert_eq!(class, SurfaceClass::Dirt);
    }

    #[test]
    fn biome_default_steep_is_gravel() {
        // Moderate slope always returns gravel regardless of biome.
        for biome in [
            BiomeClass::HighDesert,
            BiomeClass::RockyAlpine,
            BiomeClass::CoastalBoreal,
            BiomeClass::MediterraneanArchipelago,
        ] {
            let class = biome_default_surface(biome, 500.0, 20.0);
            assert_eq!(class, SurfaceClass::Gravel, "biome {biome:?}");
        }
    }
}
