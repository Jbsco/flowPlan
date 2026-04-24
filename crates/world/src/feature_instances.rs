//! Per-cell feature instance data for sub-biome primitive rendering
//! (Phase 3.6.3c.5c.1).
//!
//! A `FeatureInstance` is a single renderable primitive (box, cone,
//! sphere, etc.) placed within a Voronoi cell's world footprint.
//! Instances are computed at bake-time from a cell's feature assignment
//! plus cell-seed RNG plus palette adaptation, and stored sparse in
//! `BakedAssignments` indexed by `CellId`. At render time, the active
//! cell set (3.6.3c.5b Layer 2) drives which cells' instances are live.
//!
//! # Primitive vocabulary
//!
//! Pinned in memory `simple_primitives_render_direction`. MVP (5c.1)
//! ships [`PrimitiveKind::Box`] only; remaining primitives (cone,
//! cylinder, sphere, half-cylinder, flat-quad, stick-tree) extend the
//! enum in 5c.2 without breaking the instance schema.
//!
//! # Feature composition
//!
//! The sub-biome RON entries include render-primitive notes in
//! comments (e.g. `dwelling_cluster = 5-8× box (houses) + 4× cone
//! (trees)`). 5c.1 hardcodes one composition (`dwelling_cluster`) in
//! `compose_instances_for_feature`. 5c.2+ will either parse RON or
//! expand to cover all feature ids.
//!
//! # Determinism
//!
//! Instance generation is a pure function of
//! `(master_seed, cell_id, feature_id, cell_pos, palette)`. Same inputs
//! produce bit-identical instance lists across runs.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use crate::sub_biome::{adapt_palette, BiomeSource, Palette, SubBiomeFeature, SubBiomeLibraries};
use crate::voronoi_cells::CellId;

/// Primitive kind — which hardcoded mesh this instance renders as.
///
/// Extended in 5c.2 with `Cone`, `Cylinder`, `Sphere`, `HalfCylinder`,
/// `FlatQuad`, `StickTree` (composite).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PrimitiveKind {
    /// Axis-aligned box; unit dimensions scaled per-instance. Used for
    /// buildings, sheds, cabins, shops.
    Box,
}

/// One placed primitive instance.
///
/// ~48 bytes per instance; per-cell typical count is 5-10 → ~0.5 KB/cell
/// × 25 K cells with `dwelling_cluster` = ~12 MB baked for this one
/// feature. Sparse storage keeps total in the tens-of-megabytes range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeatureInstance {
    /// Which primitive mesh to render.
    pub primitive: PrimitiveKind,
    /// World-space position (cell-center + local offset).
    pub position: glam::Vec3,
    /// Per-axis scale. For boxes: (width, height, depth) of the building.
    pub scale: glam::Vec3,
    /// Rotation around world Y axis (radians). Walls face plausibly.
    pub rotation_y_rad: f32,
    /// Baked RGB color (u8 per channel) — already palette-adapted via
    /// the 3.6.3b biome-receives HSV shift. Alpha reserved for 5c.3
    /// blend effects.
    pub color: [u8; 4],
}

/// Splitmix64-inspired hash — matches the `mix_seed` pattern used
/// elsewhere in the world crate for deterministic PRNG chains.
const fn mix_seed(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Pseudo-random f32 in [0, 1) from (seed, step).
fn rand_unit(seed: u64, step: u64) -> f32 {
    let h = mix_seed(seed, step);
    ((h & 0xFFFF_FFFF) as f32) / (u32::MAX as f32 + 1.0)
}

/// Domain-tag seed used for feature-instance RNG chains. Distinct from
/// the sub-biome field tag to avoid correlation between feature type
/// selection and instance placement.
const FEATURE_INSTANCE_TAG: u64 = u64::from_le_bytes(*b"instance");

/// Generate the per-cell instance list for the given feature assignment.
///
/// Returns empty vec when the feature has no authored composition
/// yet (5c.1 MVP supports `dwelling_cluster` only). Extended in 5c.2.
///
/// - `master_seed`: world seed
/// - `cell_id`: Voronoi cell id
/// - `cell_pos`: cell seed position in world coords (x, z)
/// - `feature`: the feature placed in this cell (looked up from libraries)
/// - `source`: the cell's `BiomeSource` (Primary / `BleedOver`) — drives palette adaptation
/// - `libraries`: sub-biome libraries (for accessing the receiving-adaptation table)
/// - `height_at`: terrain sampler used to anchor per-instance Y to
///   ground level. Called once per placed instance at bake time so the
///   render side doesn't re-sample per frame. Takes (x, z) → height in
///   meters, same convention as `terrain.height(x, z)`.
#[must_use]
pub fn compose_instances_for_feature(
    master_seed: u64,
    cell_id: CellId,
    cell_pos: (f32, f32),
    feature: &SubBiomeFeature,
    source: BiomeSource,
    libraries: &SubBiomeLibraries,
    height_at: &dyn Fn(f32, f32) -> f32,
) -> Vec<FeatureInstance> {
    // Resolve the adapted palette per 3.6.3b mechanism.
    let adapted = adapt_palette(feature, source.host_biome(), libraries);

    // Per-feature composition — hardcoded for 5c.1 MVP.
    match feature.id.as_str() {
        "dwelling_cluster" => {
            compose_dwelling_cluster(master_seed, cell_id, cell_pos, &adapted, height_at)
        },
        _ => Vec::new(),
    }
}

/// Compose `dwelling_cluster` as 5-8 box-primitives representing houses,
/// placed within a ~200 m radius of the cell center with random
/// rotation + modest scale variation (± 20%).
///
/// Per-house dimensions: 6-10 m wide × 4-6 m tall × 8-12 m deep.
/// Color: primary palette tone (weathered wood / house walls).
/// Y is anchored to terrain height at the instance's (x, z) + half-
/// height so the house base sits on the ground.
fn compose_dwelling_cluster(
    master_seed: u64,
    cell_id: CellId,
    cell_pos: (f32, f32),
    palette: &Palette,
    height_at: &dyn Fn(f32, f32) -> f32,
) -> Vec<FeatureInstance> {
    let seed = mix_seed(master_seed, FEATURE_INSTANCE_TAG);
    let cell_rng = mix_seed(seed, u64::from(cell_id));

    // House count 5-8 (integer, from cell RNG).
    let house_count = 5 + (mix_seed(cell_rng, 0) & 3) as u32; // 5, 6, 7, or 8

    let mut instances = Vec::with_capacity(house_count as usize);
    for i in 0..house_count {
        let sub_rng = mix_seed(cell_rng, u64::from(i));
        // Position: radial offset up to 200 m from cell center.
        let angle = rand_unit(sub_rng, 1) * core::f32::consts::TAU;
        let radius = rand_unit(sub_rng, 2).sqrt() * 200.0; // sqrt for area-uniform
        let ox = radius * libm::cosf(angle);
        let oz = radius * libm::sinf(angle);
        let px = cell_pos.0 + ox;
        let pz = cell_pos.1 + oz;

        // Scale: house-sized with variance.
        let base_w = 8.0;
        let base_h = 5.0;
        let base_d = 10.0;
        let scale_jitter = 0.8 + rand_unit(sub_rng, 3) * 0.4; // 0.8-1.2×
        let sw = base_w * scale_jitter;
        let sh = base_h * scale_jitter;
        let sd = base_d * scale_jitter;

        // Rotation around world Y — houses face plausibly random headings.
        let rot_y = rand_unit(sub_rng, 4) * core::f32::consts::TAU;

        // Anchor to terrain: base of the box sits on the ground at (px, pz),
        // so the box's center (our `position`) is half-height above.
        let ground_y = height_at(px, pz);
        let py = ground_y + sh * 0.5;

        instances.push(FeatureInstance {
            primitive: PrimitiveKind::Box,
            position: glam::Vec3::new(px, py, pz),
            scale: glam::Vec3::new(sw, sh, sd),
            rotation_y_rad: rot_y,
            color: [palette.primary.0, palette.primary.1, palette.primary.2, 255],
        });
    }
    instances
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::biome::BiomeClass;

    /// Test height sampler that returns a known value so tests can
    /// assert terrain anchoring (`py = ground_y + sh/2`).
    fn height_stub(ground: f32) -> impl Fn(f32, f32) -> f32 {
        move |_x, _z| ground
    }

    #[test]
    fn dwelling_cluster_produces_5_to_8_boxes() {
        let libs = SubBiomeLibraries::shared();
        let feature = libs
            .find(BiomeClass::CoastalBoreal, "dwelling_cluster")
            .unwrap();
        // Try multiple cell ids; expect 5-8 each time.
        for cell_id in 0..10 {
            let instances = compose_instances_for_feature(
                42,
                cell_id,
                (100.0 * cell_id as f32, -100.0 * cell_id as f32),
                feature,
                BiomeSource::Primary(BiomeClass::CoastalBoreal),
                libs,
                &height_stub(0.0),
            );
            assert!(
                (5..=8).contains(&instances.len()),
                "expected 5-8 instances, got {}",
                instances.len()
            );
            for inst in &instances {
                assert_eq!(inst.primitive, PrimitiveKind::Box);
                assert!(inst.scale.x > 5.0 && inst.scale.x < 15.0);
                assert!(inst.color[3] == 255);
            }
        }
    }

    #[test]
    fn instance_generation_is_deterministic() {
        let libs = SubBiomeLibraries::shared();
        let feature = libs
            .find(BiomeClass::CoastalBoreal, "dwelling_cluster")
            .unwrap();
        let a = compose_instances_for_feature(
            42,
            1234,
            (500.0, -700.0),
            feature,
            BiomeSource::Primary(BiomeClass::CoastalBoreal),
            libs,
            &height_stub(0.0),
        );
        let b = compose_instances_for_feature(
            42,
            1234,
            (500.0, -700.0),
            feature,
            BiomeSource::Primary(BiomeClass::CoastalBoreal),
            libs,
            &height_stub(0.0),
        );
        assert_eq!(a, b);
    }

    #[test]
    fn unknown_feature_returns_empty() {
        let libs = SubBiomeLibraries::shared();
        let feature = libs.find(BiomeClass::HighDesert, "mesa_top").unwrap();
        let instances = compose_instances_for_feature(
            42,
            0,
            (0.0, 0.0),
            feature,
            BiomeSource::Primary(BiomeClass::HighDesert),
            libs,
            &height_stub(0.0),
        );
        // mesa_top has no composition in 5c.1 MVP.
        assert!(instances.is_empty());
    }

    #[test]
    fn palette_adaptation_applied_to_instance_color() {
        // dwelling_cluster native to CoastalBoreal; bleeding into
        // HighDesert should produce a warmer color than the home palette.
        let libs = SubBiomeLibraries::shared();
        let feature = libs
            .find(BiomeClass::CoastalBoreal, "dwelling_cluster")
            .unwrap();
        let home = compose_instances_for_feature(
            42,
            1,
            (0.0, 0.0),
            feature,
            BiomeSource::Primary(BiomeClass::CoastalBoreal),
            libs,
            &height_stub(0.0),
        );
        let bleed = compose_instances_for_feature(
            42,
            1,
            (0.0, 0.0),
            feature,
            BiomeSource::BleedOver {
                host: BiomeClass::HighDesert,
                source: BiomeClass::CoastalBoreal,
            },
            libs,
            &height_stub(0.0),
        );
        assert_eq!(home.len(), bleed.len());
        // First-instance color should differ (bleed applies HSV shift).
        assert_ne!(home[0].color, bleed[0].color);
    }

    #[test]
    fn instance_y_anchored_to_terrain_height() {
        // Instances should sit with their base on the ground — so
        // position.y = ground_height + half_height_of_box.
        let libs = SubBiomeLibraries::shared();
        let feature = libs
            .find(BiomeClass::CoastalBoreal, "dwelling_cluster")
            .unwrap();
        let ground = 500.0_f32;
        let instances = compose_instances_for_feature(
            42,
            7,
            (0.0, 0.0),
            feature,
            BiomeSource::Primary(BiomeClass::CoastalBoreal),
            libs,
            &height_stub(ground),
        );
        for inst in &instances {
            let expected = ground + inst.scale.y * 0.5;
            assert!(
                (inst.position.y - expected).abs() < 1e-3,
                "expected py={expected}, got {}",
                inst.position.y
            );
        }
    }
}
