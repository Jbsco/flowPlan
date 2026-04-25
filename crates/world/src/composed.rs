//! L1 + L3 heightfield composition (Phase 3.6.3a).
//!
//! Bundles the world-bake state (terrain, hydrology, sub-biome
//! partition, hub catalog, hub-foundation overrides) into a single
//! [`WorldRefs`] handle so consumers can query composed heights via
//! one call instead of threading five references through each call
//! site.
//!
//! Composition: `final_height(x, z) = L1_height(x, z) + L3_delta(x, z)`.
//!
//! L3 delta dispatches per-cell via [`crate::feature_delta::compute_delta_at`]
//! — hub foundation has priority over natural features; hydrology
//! continuity invariant suppresses elevation-raising deltas on water
//! cells.
//!
//! # Consumer contract
//!
//! - **Bake side**: construct once per world load. `L3` and `hub_catalog`
//!   bakes are both deterministic per master seed, so `WorldRefs`
//!   transitively is.
//! - **Query side**: threading a `&WorldRefs` through render / sim /
//!   visual-bench replaces the multi-argument mess. Future sim-side
//!   integration (2.5b.7 ground contact, render-side height sampling)
//!   consumes this handle.

use crate::feature_delta::{compute_delta_at, HubFoundationMap};
use crate::hub_catalog::HubCatalog;
use crate::hydrology::HydrologyGraph;
use crate::surface_class::{surface_at, SurfaceClass};
use crate::terrain::{HeightSampler, Terrain};
use crate::voronoi_cells::SubBiomeCellCatalog;

/// Bundle of world-bake references. Lifetimes keep callers honest —
/// all components outlive this handle, and construction is O(1)
/// (just borrows; no copying).
#[derive(Copy, Clone)]
pub struct WorldRefs<'a> {
    pub terrain: &'a Terrain,
    pub hydrology: &'a HydrologyGraph,
    pub sub_biome: &'a SubBiomeCellCatalog,
    pub hubs: &'a HubCatalog,
    pub foundations: &'a HubFoundationMap,
}

impl<'a> WorldRefs<'a> {
    /// Build the handle from the individual bakes. All components
    /// should share the same `master_seed`; we accept them as refs
    /// without re-checking — caller's responsibility.
    #[must_use]
    pub const fn new(
        terrain: &'a Terrain,
        hydrology: &'a HydrologyGraph,
        sub_biome: &'a SubBiomeCellCatalog,
        hubs: &'a HubCatalog,
        foundations: &'a HubFoundationMap,
    ) -> Self {
        Self {
            terrain,
            hydrology,
            sub_biome,
            hubs,
            foundations,
        }
    }

    /// L1 + L3 composed height at `(x, z)`. Full-resolution (all
    /// octaves included).
    #[must_use]
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let base = self.terrain.height(x, z);
        let delta = compute_delta_at(
            x,
            z,
            base,
            self.sub_biome,
            self.foundations,
            self.hubs,
            self.hydrology,
            self.terrain.seed(),
        );
        base + delta
    }

    /// L1 + L3 composed height with mipmap-aware L1 sampling. Skips
    /// L1 octaves finer than `min_spacing_m`; L3 delta is always
    /// computed (L3 features are cell-level, their detail is
    /// inherent at cell scale and doesn't participate in the
    /// octave pyramid).
    #[must_use]
    pub fn height_at_scale(&self, x: f32, z: f32, min_spacing_m: f32) -> f32 {
        let base = self.terrain.height_at_scale(x, z, min_spacing_m);
        let delta = compute_delta_at(
            x,
            z,
            base,
            self.sub_biome,
            self.foundations,
            self.hubs,
            self.hydrology,
            self.terrain.seed(),
        );
        base + delta
    }

    /// L3 delta only (no L1 base). Useful for visual-bench diff maps
    /// and diagnostic tools.
    #[must_use]
    pub fn delta_at(&self, x: f32, z: f32) -> f32 {
        let base = self.terrain.height(x, z);
        compute_delta_at(
            x,
            z,
            base,
            self.sub_biome,
            self.foundations,
            self.hubs,
            self.hydrology,
            self.terrain.seed(),
        )
    }

    /// Surface classification at `(x, z)`.
    #[must_use]
    pub fn surface_at(&self, x: f32, z: f32) -> SurfaceClass {
        surface_at(
            x,
            z,
            self.terrain.seed(),
            self.terrain,
            self.hydrology,
            self.sub_biome,
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::many_single_char_names
)]
mod tests {
    use super::*;

    /// Canonical bake setup. Expensive (~30 s total for all four
    /// bakes); run only in release tests.
    fn bake_canonical() -> (
        Terrain,
        HydrologyGraph,
        SubBiomeCellCatalog,
        HubCatalog,
        HubFoundationMap,
    ) {
        let seed = 0x0F10_1A11_u64;
        let terrain = Terrain::from_seed(seed);
        let hydrology = HydrologyGraph::bake(seed, &terrain);
        let sub_biome = SubBiomeCellCatalog::bake(seed);
        let hubs = HubCatalog::bake(seed, &terrain, &hydrology);
        let foundations = HubFoundationMap::build(&hubs, &sub_biome);
        (terrain, hydrology, sub_biome, hubs, foundations)
    }

    #[test]
    fn height_at_composes_l1_plus_l3() {
        let (t, h, s, hc, f) = bake_canonical();
        let refs = WorldRefs::new(&t, &h, &s, &hc, &f);
        // Sample at origin: just verify it doesn't panic + returns
        // a finite value in the plausible world-envelope range.
        let composed = refs.height_at(1234.5, -6789.0);
        assert!(composed.is_finite(), "composed height non-finite");
        assert!(
            (-200.0..=5000.0).contains(&composed),
            "composed height {composed} outside plausible envelope"
        );
    }

    #[test]
    fn delta_at_equals_composed_minus_base() {
        let (t, h, s, hc, f) = bake_canonical();
        let refs = WorldRefs::new(&t, &h, &s, &hc, &f);
        for &(x, z) in &[
            (1000.0_f32, -1000.0),
            (50_000.0, 25_000.0),
            (-77_777.0, 123.0),
        ] {
            let base = t.height(x, z);
            let composed = refs.height_at(x, z);
            let delta = refs.delta_at(x, z);
            assert!(
                (composed - (base + delta)).abs() < 1e-3,
                "composed({composed}) != base({base}) + delta({delta}) at ({x}, {z})"
            );
        }
    }

    #[test]
    fn surface_at_returns_water_on_sea() {
        // Pick a spot on clear sea by sampling until we find one.
        // Scan in canonical [0, WORLD_EXTENT_M) coords (1 Mm × 1 Mm
        // window from origin) — post-5c.2.c.12r the noise function
        // pre-wraps, so literal-negative coords sample at their
        // canonical mirror; using canonical-positive coords here
        // keeps the test stable and self-explanatory.
        let (t, h, s, hc, f) = bake_canonical();
        let refs = WorldRefs::new(&t, &h, &s, &hc, &f);
        let mut found = false;
        'outer: for ix in 0..10 {
            for iz in 0..10 {
                let x = ix as f32 * 100_000.0;
                let z = iz as f32 * 100_000.0;
                if t.height(x, z) < -1.0 {
                    // clearly below sea
                    let surface = refs.surface_at(x, z);
                    if surface == SurfaceClass::Water {
                        found = true;
                        break 'outer;
                    }
                }
            }
        }
        assert!(
            found,
            "couldn't find any clear-sea (terrain < -1 AND hydrology=Water) sample in 1 Mm window"
        );
    }

    #[test]
    fn composed_height_respects_hydrology_continuity() {
        // Scan cells on known water — delta should never be positive
        // on them (hydrology continuity invariant).
        let (t, h, s, hc, f) = bake_canonical();
        let refs = WorldRefs::new(&t, &h, &s, &hc, &f);
        let mut samples_checked = 0_u32;
        for ix in 0..30 {
            for iz in 0..30 {
                let x = (ix as f32 - 15.0) * 33_000.0;
                let z = (iz as f32 - 15.0) * 33_000.0;
                // Only check points that are on water.
                let on_water = h
                    .water_surface_at(x, z)
                    .is_some_and(|s| !matches!(s.class, crate::hydrology::WaterClass::DryChannel));
                if !on_water {
                    continue;
                }
                let delta = refs.delta_at(x, z);
                assert!(
                    delta <= 0.0 + 1e-3,
                    "positive L3 delta {delta} on water cell at ({x}, {z}) violates hydrology continuity"
                );
                samples_checked += 1;
                if samples_checked >= 100 {
                    return;
                }
            }
        }
        // If we got here without checking any, the world has no water — unlikely.
        assert!(
            samples_checked > 0,
            "no water samples found for hydrology-continuity check"
        );
    }

    #[test]
    fn hub_foundation_map_populated_from_catalog() {
        let (_, _, s, hc, f) = bake_canonical();
        // Foundation overrides should cover at least the Major +
        // Regional hubs (the tiers with foundation strength > 0 that
        // aren't isolated).
        let meaningful_hubs = hc
            .hubs()
            .iter()
            .filter(|h| !matches!(h.tier, crate::hub_catalog::HubTier::Isolated))
            .count();
        assert!(
            f.count() > 0,
            "hub foundation map has zero overrides despite {meaningful_hubs} non-isolated hubs"
        );
        // Ensure a sampled override maps to a valid cell id.
        let sample_hub = hc
            .hubs()
            .iter()
            .find(|h| matches!(h.tier, crate::hub_catalog::HubTier::Major))
            .expect("at least one Major hub in canonical bake");
        let cell_id = s.nearest_cell(sample_hub.approx_world_pos.0, sample_hub.approx_world_pos.1);
        assert!(
            f.tier_for(cell_id).is_some(),
            "Major hub's cell not in foundation map"
        );
    }
}
