//! Per-cell feature instance data for sub-biome primitive rendering
//! (Phase 3.6.3c.5c).
//!
//! A [`FeatureInstance`] is a single renderable primitive (box, cone,
//! cylinder, sphere) placed within a Voronoi cell's world footprint.
//! Instances are computed at bake-time from a cell's feature assignment
//! plus cell-seed RNG plus palette adaptation, and stored sparse in
//! `BakedAssignments` indexed by `CellId`. At render time, the active
//! cell set (3.6.3c.5b Layer 2) drives which cells' instances are live.
//!
//! # Primitive vocabulary
//!
//! Pinned in memory `simple_primitives_render_direction`. 5c.2.a ships
//! `Box` / `Cone` / `Cylinder` / `Sphere`; later milestones may add
//! `HalfCylinder`, `FlatQuad`, `StickTree` (composite) without breaking
//! the [`FeatureInstance`] schema.
//!
//! # Feature composition (5c.2.b — RON-driven)
//!
//! Each [`crate::sub_biome::SubBiomeFeature`] carries an optional
//! `composition: Vec<PrimitiveComponent>`. [`compose_instances_for_feature`]
//! iterates the list and delegates to [`emit_component`] for each
//! entry, producing the concrete [`FeatureInstance`]s.
//!
//! A [`PrimitiveComponent`] declares:
//! - `kind` — which primitive mesh
//! - `count` — (min, max) inclusive instance count
//! - `base_size_m` — per-axis base dimensions
//! - `scale_jitter` — per-instance uniform-scale multiplier range
//! - `color_source` — which palette slot (or explicit RGB)
//! - `placement` — spatial pattern (`Single` / `RadialCluster`; more in 5c.2.c)
//! - `rng_stream` — distinct per-component RNG offset so components
//!   don't correlate
//!
//! Features with `composition: None` emit no instances. 5c.2.b authors
//! `dwelling_cluster` (5-8 houses + 3-6 trees); 5c.2.c extends to the
//! remaining 17 developments / resupply / scenery features.
//!
//! # Determinism
//!
//! Instance generation is a pure function of
//! `(master_seed, cell_id, feature, cell_pos, palette)`. Same inputs
//! produce bit-identical instance lists across runs.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use serde::Deserialize;

use crate::sub_biome::{adapt_palette, BiomeSource, Palette, SubBiomeFeature, SubBiomeLibraries};
use crate::voronoi_cells::CellId;

/// Primitive kind — which hardcoded mesh this instance renders as.
///
/// 5c.2.a ships `Box` / `Cone` / `Cylinder` / `Sphere`. 5c.2.c.1 adds
/// `HalfCylinder` (quonset-hut hangar roofs) + `FlatQuad` (runways /
/// tarmac / road surfaces). Later milestones may add `StickTree`
/// (composite) per memory `simple_primitives_render_direction`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, Deserialize)]
pub enum PrimitiveKind {
    /// Axis-aligned box; unit dimensions scaled per-instance. Used for
    /// buildings, sheds, cabins, shops.
    Box,
    /// Unit cone, apex-up. Trees (conifer canopy), spires, signage
    /// cones, rooftop details.
    Cone,
    /// Unit cylinder. Pillars, silos, gas-station support columns,
    /// tree trunks, fence posts.
    Cylinder,
    /// Unit sphere (smooth-shaded). Boulders, floats, decorative
    /// spherical features.
    Sphere,
    /// Half-cylinder (quonset / hangar roof shape). Flat base on -Y,
    /// arc roof with apex at +Y, length runs along Z. Used for
    /// hangars, quonset huts, arched roofs.
    HalfCylinder,
    /// Flat axis-aligned quad in the XZ plane, +Y normal. Used for
    /// runway surfaces, tarmac, apron slabs, road segments, signage
    /// panels. Zero Y-extent in the unit mesh — set `base_size_m.y`
    /// in the composition to encode anti-z-fight lift above ground.
    FlatQuad,
}

impl PrimitiveKind {
    /// All variants in a stable order — used to enumerate primitive
    /// pipelines in the renderer. Keep aligned with the enum
    /// declaration so grouping/lookup is consistent.
    pub const ALL: &'static [Self] = &[
        Self::Box,
        Self::Cone,
        Self::Cylinder,
        Self::Sphere,
        Self::HalfCylinder,
        Self::FlatQuad,
    ];
}

/// Which palette slot drives a primitive's color (5c.2.b RON schema).
///
/// `Explicit([r, g, b])` escape hatch for features where the palette
/// doesn't apply (signage paint, branded red accents, etc.). Alpha
/// stays hardcoded at 255 — alpha is reserved for later blend effects
/// and should not vary per-composition-entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum ColorSource {
    /// Use `palette.primary` — typical for structural elements
    /// (building walls, plinths, main masses).
    Primary,
    /// Use `palette.secondary` — foliage / organic accent / trim.
    Secondary,
    /// Use `palette.accent` — sign trim, highlight detail.
    Accent,
    /// Override with a literal RGB triple. Use sparingly; RON-authored
    /// feature compositions should prefer palette slots so the
    /// 3.6.3b palette-adaptation pipeline applies.
    Explicit([u8; 3]),
}

/// Placement pattern for a primitive component (5c.2.b + 5c.2.c.1 RON
/// schema).
///
/// Determines how the component's `count` instances are spatially
/// arranged within the cell's ~200 m footprint. Extend with new
/// variants as feature compositions demand (e.g. `GridField { spacing,
/// extent }` for farmland rows).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub enum PlacementPattern {
    /// Single instance at a fixed offset from cell center. Used for
    /// hero features that should sit at a stable position (anchor
    /// buildings, signage, single boulders).
    Single {
        /// World-space (x, z) offset from cell center (m).
        offset_m: (f32, f32),
    },
    /// Randomly scattered within a disc of given radius from cell
    /// center. Uses square-root distribution for area-uniform
    /// placement. Used for clustered features (houses, trees, scattered
    /// rocks).
    RadialCluster {
        /// Disc radius (m) within which instances are distributed.
        radius_m: f32,
    },
    /// Instances placed along a straight line. Used for hangars
    /// arranged along a taxiway, runway edge lights, perimeter fence
    /// posts, linear building rows. The `count` drawn from
    /// [`PrimitiveComponent::count`] determines how many instances
    /// actually emit; authors set `count: (10, 10)` for exactly 10
    /// with no variance.
    LinearChain {
        /// World-space (x, z) offset from cell center for the first
        /// instance.
        start_m: (f32, f32),
        /// Line direction measured counterclockwise from +X (radians).
        /// `0.0` = +X, `PI/2` = +Z, `PI` = -X, etc. Kept as a scalar
        /// (not a unit vector) for RON authoring simplicity.
        direction_rad: f32,
        /// Distance between consecutive instances (m). Total chain
        /// length = `(count - 1) × spacing_m`.
        spacing_m: f32,
    },
}

/// Feature-parent-controller for ambient objects in surrounding area
/// (5c.2.c.8 schema). Default `None` — features opt IN to clearing.
///
/// See [MAPGEN § Ambient environment objects](../../../MAPGEN.md#ambient-environment-objects-phase-363c5c8)
/// for full design + the per-feature classification table.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub enum FeatureAmbientControl {
    /// No clearing. Ambient objects appear at full size right up to
    /// feature primitives; only per-primitive instance-overlap is
    /// rejected. For natural / remote / rustic features — the default
    /// behavior when `ambient_control` is omitted from RON.
    None,

    /// Tapered clearing. Inside `clear_radius_m`: no ambient. Between
    /// `clear_radius_m` and `taper_radius_m`: ambient size scales 0→1
    /// via smoothstep so trees shrink as they approach the feature
    /// edge. Beyond `taper_radius_m`: full size. For cultivated /
    /// groomed features (airports, dwellings, farmland, marinas).
    TaperedClearing {
        clear_radius_m: f32,
        taper_radius_m: f32,
    },

    /// Hard binary clearing. No ambient within `clear_radius_m`; full
    /// size everywhere else. For small sanitized features (gas
    /// stations, helipads, watchtowers, mining camps).
    HardClearing { clear_radius_m: f32 },

    /// Inverse tapered clearing — pioneering growth on/near feature.
    /// Ambient size ramps UP with distance from feature (tiny sprouts
    /// near, full size far). Reserved for future authoring use cases
    /// (`ancient_ruins` ivy, `abandoned_homestead` saplings on
    /// foundation). Mechanism: inside `clear_radius_m` → full size at
    /// 0 scale climbing; between clear and taper → size
    /// `smoothstep((taper-D)/(taper-clear))`; beyond taper → full
    /// size again. For MVP, `None` + biome ambient defaults usually
    /// covers the "nature reclaims" look without this variant.
    InverseTaperedClearing {
        clear_radius_m: f32,
        taper_radius_m: f32,
    },
}

/// One ambient-cover entry in a biome's scatter schema (5c.2.c.8).
///
/// Represents a single layer of scattered biome-wide cover — e.g. a
/// forest-density layer uses `kind: Cone` with a `120 density_per_km2`
/// across `CoastalBoreal`. Multiple entries per biome compose (forest +
/// understory shrubs + boulders).
///
/// See [MAPGEN § Ambient environment objects → RON schema](../../../MAPGEN.md#ambient-environment-objects-phase-363c5c8)
/// for authoring guidelines.
const fn default_noise_taper() -> f32 {
    0.15
}

#[derive(Clone, Debug, Deserialize)]
pub struct AmbientEntry {
    /// Which primitive renders each ambient instance.
    pub kind: PrimitiveKind,
    /// Color source — which palette slot (or explicit RGB) drives the
    /// instance's baked color. Same semantics as
    /// [`PrimitiveComponent.color_source`].
    pub color_source: ColorSource,
    /// Target instance density (instances per square km). Authors tune
    /// via visual-bench iteration; ~50–200 for forest canopy;
    /// ~10–40 for boulders or sparse shrubs.
    pub density_per_km2: f32,
    /// Per-instance size (m). Each instance picks a uniform random
    /// value in this range for its primary dimension (width/diameter/
    /// height depending on primitive). Kept as `(min, max)` tuple.
    pub size_range_m: (f32, f32),
    /// Per-instance uniform-scale jitter applied on top of `size_range`.
    pub scale_jitter: (f32, f32),
    /// fBm wavelength controlling density-field variation (m). Larger
    /// = broader forest/meadow patches; smaller = tighter clustering.
    pub noise_scale_m: f32,
    /// Density-field sample threshold. Candidates with noise above
    /// `noise_threshold + noise_taper/2` always spawn; candidates
    /// below `noise_threshold - noise_taper/2` never spawn; in the
    /// taper band, spawn probability + size fade smoothly via
    /// smoothstep. Higher threshold = sparser coverage.
    pub noise_threshold: f32,
    /// Width of the soft-taper band around `noise_threshold` (in
    /// normalized noise units). Default `0.15` produces ~100 m
    /// feathered patch edges at `noise_scale_m = 400`. Set to `0` for
    /// hard threshold (pre-5c.2.c.12c behavior).
    #[serde(default = "default_noise_taper")]
    pub noise_taper: f32,
    /// Minimum spawn weight even when noise is at its minimum. Default
    /// `0.0` = classic gated behavior (below-threshold candidates are
    /// rejected outright). Raise to 0.2–0.4 for "uniform carpet"
    /// behavior — sparse presence everywhere in the valid zone, dense
    /// peaks where noise is high. See MAPGEN § Density modulation vs
    /// gating (PINNED 2026-04-24) for the project-wide principle.
    #[serde(default)]
    pub noise_floor: f32,
    /// (min, max) altitude range (m). Candidates outside this are
    /// rejected before expensive checks.
    pub altitude_range_m: (f32, f32),
    /// Max terrain slope at the candidate position (degrees). Scrubbed
    /// from steep slopes so shrubs don't hang off cliffs.
    pub slope_max_deg: f32,
    /// Distinct per-entry RNG stream offset so overlapping layers
    /// (forest cones + shrub spheres at the same cell) don't
    /// correlate their placements.
    pub rng_stream: u64,
    /// Optional composite-primitive stack. When non-empty, each
    /// successful candidate emits the base instance PLUS one
    /// derived instance per stack layer. Enables trunks, layered-
    /// skirt pines, boulder piles, shrub clusters, etc. Default
    /// empty = single-primitive (current behavior).
    ///
    /// See [`AmbientStackLayer`] for the per-layer parameters.
    #[serde(default)]
    pub stacks: Vec<AmbientStackLayer>,
}

/// One layer in an [`AmbientEntry`]'s composite-primitive stack.
///
/// Each layer is a primitive offset + scaled relative to the base
/// instance's authored size. Added in Phase 3.6.3c.5c.2.c.12b to
/// enable shape variation (trunk + canopy trees, layered-skirt pines,
/// boulder piles).
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct AmbientStackLayer {
    /// Primitive kind for this layer (can differ from the entry's
    /// base kind — e.g., Cylinder trunk under a Cone canopy).
    pub kind: PrimitiveKind,
    /// Palette-slot source for this layer's baked color.
    pub color_source: ColorSource,
    /// Horizontal (X/Z) scale as a fraction of the candidate's base
    /// size. Random uniform sample in (min, max). Independent of Y
    /// so authors can build squat skirts or tall narrow crowns.
    pub xz_scale: (f32, f32),
    /// Vertical (Y) scale as a fraction of the candidate's base size.
    /// Random uniform sample in (min, max). Cone/Cylinder height;
    /// Sphere behaves as uniform when equal to `xz_scale`.
    pub y_scale: (f32, f32),
    /// Y offset from the candidate's base ground position, expressed
    /// as a fraction of the candidate's base size. `(0, 0)` = stacked
    /// at ground; `(0.4, 0.5)` = stacked halfway up the base; negative
    /// values sink below ground (useful for trunks rooted through the
    /// base center).
    pub y_offset_ratio: (f32, f32),
    /// Per-layer random yaw rotation in radians. Range `[-jitter, +jitter]`.
    /// Set to 0 for aligned stacks; `PI` for fully random orientation.
    pub rotation_y_jitter_rad: f32,
}

/// One entry in a feature's primitive composition (5c.2.b RON schema).
///
/// Declarative replacement for the hardcoded `compose_dwelling_cluster`
/// logic. A feature's `composition: Vec<PrimitiveComponent>` lists
/// each primitive layer; the bake iterates and produces
/// `FeatureInstance`s accordingly.
#[derive(Clone, Debug, Deserialize)]
pub struct PrimitiveComponent {
    /// Which primitive mesh renders each instance.
    pub kind: PrimitiveKind,
    /// Instance count (inclusive `(min, max)`). A random integer in
    /// this range is picked from the cell RNG.
    pub count: (u32, u32),
    /// Base (width, height, depth) in meters at `scale_jitter=1.0`.
    /// For axis-symmetric primitives (cone / cylinder / sphere) width
    /// + depth are typically equal (diameter).
    pub base_size_m: (f32, f32, f32),
    /// Per-instance uniform-scale jitter multiplier range. `(0.8, 1.2)`
    /// = each instance scales uniformly by a random factor in [0.8,
    /// 1.2]. Set to `(1.0, 1.0)` for no jitter.
    pub scale_jitter: (f32, f32),
    /// Color source — which palette slot (or explicit RGB) drives the
    /// instance's baked color.
    pub color_source: ColorSource,
    /// Spatial arrangement of the `count` instances within the cell.
    pub placement: PlacementPattern,
    /// RNG stream offset. Distinct per-component values make
    /// house/tree/etc. placements uncorrelated within a cell. Author
    /// 0 for the first component, 0x100 for the second, 0x200 for the
    /// third, etc. — or any distinct values.
    pub rng_stream: u64,
}

/// One placed primitive instance.
///
/// ~48 bytes per instance; per-cell typical count is 5-10 → ~0.5 KB/cell
/// × 25 K cells with `dwelling_cluster` = ~12 MB baked for this one
/// feature. Sparse storage keeps total in the tens-of-megabytes range.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
    let Some(composition) = feature.composition.as_ref() else {
        return Vec::new();
    };
    let adapted = adapt_palette(feature, source.host_biome(), libraries);
    let seed = mix_seed(master_seed, FEATURE_INSTANCE_TAG);
    let cell_rng = mix_seed(seed, u64::from(cell_id));

    let mut instances = Vec::new();
    for component in composition {
        emit_component(
            cell_rng,
            cell_pos,
            component,
            &adapted,
            height_at,
            &mut instances,
        );
    }
    instances
}

/// Emit `component.count` instances according to its RON-declared
/// parameters. RNG streams are keyed by `component.rng_stream` so
/// components within one composition don't correlate. Called once per
/// component per cell from [`compose_instances_for_feature`].
fn emit_component(
    cell_rng: u64,
    cell_pos: (f32, f32),
    component: &PrimitiveComponent,
    palette: &Palette,
    height_at: &dyn Fn(f32, f32) -> f32,
    out: &mut Vec<FeatureInstance>,
) {
    let component_rng = mix_seed(cell_rng, component.rng_stream);
    // Integer count in [count.0, count.1] inclusive. Picked from the
    // component RNG so same seed → same count.
    let count_range = component.count.1.saturating_sub(component.count.0) + 1;
    let count = if count_range == 0 {
        component.count.0
    } else {
        let roll = (mix_seed(component_rng, 0xC0) as u32) % count_range;
        component.count.0 + roll
    };

    let base_color = match component.color_source {
        ColorSource::Primary => [palette.primary.0, palette.primary.1, palette.primary.2],
        ColorSource::Secondary => [
            palette.secondary.0,
            palette.secondary.1,
            palette.secondary.2,
        ],
        ColorSource::Accent => [palette.accent.0, palette.accent.1, palette.accent.2],
        ColorSource::Explicit(rgb) => rgb,
    };
    let color = [base_color[0], base_color[1], base_color[2], 255];

    let (base_w, base_h, base_d) = component.base_size_m;
    let (jitter_min, jitter_max) = component.scale_jitter;
    let jitter_span = (jitter_max - jitter_min).max(0.0);

    for i in 0..count {
        let sub_rng = mix_seed(component_rng, u64::from(i));

        let (px, pz) = match component.placement {
            PlacementPattern::Single { offset_m } => {
                (cell_pos.0 + offset_m.0, cell_pos.1 + offset_m.1)
            },
            PlacementPattern::RadialCluster { radius_m } => {
                let angle = rand_unit(sub_rng, 1) * core::f32::consts::TAU;
                let radius = rand_unit(sub_rng, 2).sqrt() * radius_m; // area-uniform
                (
                    cell_pos.0 + radius * libm::cosf(angle),
                    cell_pos.1 + radius * libm::sinf(angle),
                )
            },
            PlacementPattern::LinearChain {
                start_m,
                direction_rad,
                spacing_m,
            } => {
                let offset_along = (i as f32) * spacing_m;
                let dx = offset_along * libm::cosf(direction_rad);
                let dz = offset_along * libm::sinf(direction_rad);
                (cell_pos.0 + start_m.0 + dx, cell_pos.1 + start_m.1 + dz)
            },
        };

        let jitter = jitter_min + rand_unit(sub_rng, 3) * jitter_span;
        let sw = base_w * jitter;
        let sh = base_h * jitter;
        let sd = base_d * jitter;

        // Rotation: Single + RadialCluster use random yaw (houses /
        // trees face plausibly-random directions). LinearChain
        // aligns to its direction axis (hangars along a taxiway /
        // runway edge lights all point the same way). Authors who
        // want random-in-chain can use many Single components with
        // distinct offsets.
        let rot_y = match component.placement {
            PlacementPattern::LinearChain { direction_rad, .. } => direction_rad,
            _ => rand_unit(sub_rng, 4) * core::f32::consts::TAU,
        };
        let ground_y = height_at(px, pz);
        let py = ground_y + sh * 0.5;

        out.push(FeatureInstance {
            primitive: component.kind,
            position: glam::Vec3::new(px, py, pz),
            scale: glam::Vec3::new(sw, sh, sd),
            rotation_y_rad: rot_y,
            color,
        });
    }
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
    fn dwelling_cluster_produces_houses_and_trees() {
        let libs = SubBiomeLibraries::shared();
        let feature = libs
            .find(BiomeClass::CoastalBoreal, "dwelling_cluster")
            .unwrap();
        // 5-8 houses (boxes) + 3-6 trees (cones) = 8-14 total.
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
            let box_count = instances
                .iter()
                .filter(|i| i.primitive == PrimitiveKind::Box)
                .count();
            let cone_count = instances
                .iter()
                .filter(|i| i.primitive == PrimitiveKind::Cone)
                .count();
            assert!(
                (5..=8).contains(&box_count),
                "expected 5-8 house boxes, got {box_count}"
            );
            assert!(
                (3..=6).contains(&cone_count),
                "expected 3-6 tree cones, got {cone_count}"
            );
            for inst in &instances {
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
    fn ron_schema_round_trip_parses() {
        // Validate the 5c.2.b composition schema parses from RON as
        // authored in sub_biome_features.ron. Catches schema drift
        // (adding a field in Rust without updating RON, etc.).
        let ron = r"[
            (
                kind: Box,
                count: (5, 8),
                base_size_m: (8.0, 5.0, 10.0),
                scale_jitter: (0.8, 1.2),
                color_source: Primary,
                placement: RadialCluster(radius_m: 200.0),
                rng_stream: 0,
            ),
            (
                kind: Cone,
                count: (3, 6),
                base_size_m: (4.0, 11.0, 4.0),
                scale_jitter: (0.75, 1.25),
                color_source: Secondary,
                placement: RadialCluster(radius_m: 200.0),
                rng_stream: 256,
            ),
            (
                kind: Cylinder,
                count: (1, 1),
                base_size_m: (1.5, 7.0, 1.5),
                scale_jitter: (1.0, 1.0),
                color_source: Accent,
                placement: Single(offset_m: (10.0, 0.0)),
                rng_stream: 512,
            ),
            (
                kind: Sphere,
                count: (1, 1),
                base_size_m: (3.0, 3.0, 3.0),
                scale_jitter: (1.0, 1.0),
                color_source: Explicit((180, 170, 140)),
                placement: Single(offset_m: (0.0, 0.0)),
                rng_stream: 768,
            ),
        ]";
        let parsed: Vec<PrimitiveComponent> = ron::from_str(ron).unwrap();
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].kind, PrimitiveKind::Box);
        assert_eq!(parsed[1].kind, PrimitiveKind::Cone);
        assert_eq!(parsed[2].kind, PrimitiveKind::Cylinder);
        assert_eq!(parsed[3].kind, PrimitiveKind::Sphere);
        assert!(matches!(
            parsed[2].placement,
            PlacementPattern::Single { .. }
        ));
        assert!(matches!(parsed[3].color_source, ColorSource::Explicit(_)));
    }

    #[test]
    fn authored_features_load_with_nonempty_compositions() {
        // 5c.2.c.2 / .c.3 / .c.4 smoke test — verify the 19 authored
        // features each parse from RON with a composition and produce
        // instances when composed. Catches RON regression + schema
        // drift in one check.
        let libs = SubBiomeLibraries::shared();
        let cases: &[(BiomeClass, &str)] = &[
            // Airports (c.2 + c.3).
            (BiomeClass::CoastalBoreal, "bush_strip"),
            (BiomeClass::CoastalBoreal, "airport_local"),
            (BiomeClass::CoastalBoreal, "airport_regional"),
            (BiomeClass::CoastalBoreal, "airport_major"),
            (BiomeClass::CoastalBoreal, "seaplane_base"),
            // Dwellings + homesteads (c.4).
            (BiomeClass::CoastalBoreal, "dwelling_cluster"),
            (BiomeClass::CoastalBoreal, "abandoned_homestead"),
            (BiomeClass::CoastalBoreal, "remote_compound"),
            (BiomeClass::CoastalBoreal, "homestead_clearing"),
            // Resupply (c.4).
            (BiomeClass::CoastalBoreal, "fishing_camp"),
            (BiomeClass::CoastalBoreal, "tackle_shop"),
            (BiomeClass::CoastalBoreal, "bait_shop"),
            (BiomeClass::CoastalBoreal, "gas_station"),
            (BiomeClass::CoastalBoreal, "general_store"),
            // HighDesert developments (c.4 + c.5 + c.6).
            (BiomeClass::HighDesert, "farmland_parcel"),
            (BiomeClass::HighDesert, "ranch"),
            (BiomeClass::HighDesert, "adobe_village"),
            (BiomeClass::HighDesert, "ghost_town"),
            (BiomeClass::HighDesert, "mining_camp"),
            (BiomeClass::HighDesert, "solar_farm"),
            (BiomeClass::HighDesert, "oil_derrick"),
            (BiomeClass::HighDesert, "wind_farm"),
            (BiomeClass::HighDesert, "grain_elevator"),
            (BiomeClass::HighDesert, "natural_arch"),
            (BiomeClass::HighDesert, "hoodoo_field"),
            // RockyAlpine developments (c.4 + c.5 + c.6).
            (BiomeClass::RockyAlpine, "mountain_lodge"),
            (BiomeClass::RockyAlpine, "fire_lookout_tower"),
            (BiomeClass::RockyAlpine, "mine_entrance"),
            (BiomeClass::RockyAlpine, "alpine_cabin"),
            (BiomeClass::RockyAlpine, "chairlift_terminal"),
            (BiomeClass::RockyAlpine, "campground"),
            (BiomeClass::RockyAlpine, "quarry"),
            (BiomeClass::RockyAlpine, "radio_tower"),
            (BiomeClass::RockyAlpine, "moraine_field"),
            (BiomeClass::RockyAlpine, "geyser_field"),
            (BiomeClass::RockyAlpine, "natural_bridge"),
            (BiomeClass::RockyAlpine, "spire_formation"),
            // CoastalBoreal c.6 additions.
            (BiomeClass::CoastalBoreal, "logging_camp"),
            (BiomeClass::CoastalBoreal, "boathouse"),
            (BiomeClass::CoastalBoreal, "helipad"),
            // Mediterranean developments (c.4 + c.5 + c.6).
            (BiomeClass::MediterraneanArchipelago, "ferry_dock"),
            (BiomeClass::MediterraneanArchipelago, "lighthouse"),
            (BiomeClass::MediterraneanArchipelago, "coastal_village"),
            (BiomeClass::MediterraneanArchipelago, "olive_grove"),
            (BiomeClass::MediterraneanArchipelago, "watchtower"),
            (BiomeClass::MediterraneanArchipelago, "marina"),
            (BiomeClass::MediterraneanArchipelago, "vineyard"),
            (BiomeClass::MediterraneanArchipelago, "ancient_ruins"),
            (BiomeClass::MediterraneanArchipelago, "monastery"),
        ];
        for &(biome, id) in cases {
            let feature = libs
                .find(biome, id)
                .unwrap_or_else(|| panic!("feature missing: {id} in {biome:?}"));
            assert!(
                feature.composition.is_some(),
                "{id} should have a composition"
            );
            let instances = compose_instances_for_feature(
                42,
                1,
                (0.0, 0.0),
                feature,
                BiomeSource::Primary(biome),
                libs,
                &height_stub(0.0),
            );
            assert!(
                !instances.is_empty(),
                "{id} composition produced zero instances"
            );
        }
    }

    #[test]
    fn ambient_entries_load_per_biome() {
        // 5c.2.c.8 smoke test: verify each biome has authored ambient
        // entries + all fields parse. Guards against RON schema drift
        // in the new ambient section.
        let libs = SubBiomeLibraries::shared();
        for biome in [
            BiomeClass::CoastalBoreal,
            BiomeClass::HighDesert,
            BiomeClass::RockyAlpine,
            BiomeClass::MediterraneanArchipelago,
        ] {
            let entries = libs.ambient_for_biome(biome);
            assert!(
                !entries.is_empty(),
                "biome {biome:?} should have ambient entries authored"
            );
            for entry in entries {
                assert!(entry.density_per_km2 > 0.0);
                assert!(entry.size_range_m.0 <= entry.size_range_m.1);
                assert!(entry.noise_scale_m > 0.0);
                assert!(entry.slope_max_deg > 0.0);
            }
        }
    }

    #[test]
    fn feature_ambient_control_variants_present() {
        // 5c.2.c.8: verify the classification ladder — features
        // authored `None` stay `None`, TaperedClearing parses, and
        // HardClearing parses with the right radii.
        let libs = SubBiomeLibraries::shared();

        // `abandoned_homestead` stays wild → None (natural, remote).
        let wild = libs
            .find(BiomeClass::CoastalBoreal, "abandoned_homestead")
            .unwrap();
        assert!(
            wild.ambient_control.is_none(),
            "abandoned_homestead is 'wild' — no ambient clearing"
        );

        // `dwelling_cluster` → TaperedClearing(40, 70).
        let tapered = libs
            .find(BiomeClass::CoastalBoreal, "dwelling_cluster")
            .unwrap();
        match tapered.ambient_control {
            Some(FeatureAmbientControl::TaperedClearing {
                clear_radius_m,
                taper_radius_m,
            }) => {
                assert!((clear_radius_m - 40.0).abs() < 1e-4);
                assert!((taper_radius_m - 70.0).abs() < 1e-4);
            },
            other => panic!("dwelling_cluster should be TaperedClearing, got {other:?}"),
        }

        // `gas_station` → HardClearing(20).
        let hard = libs.find(BiomeClass::CoastalBoreal, "gas_station").unwrap();
        match hard.ambient_control {
            Some(FeatureAmbientControl::HardClearing { clear_radius_m }) => {
                assert!((clear_radius_m - 20.0).abs() < 1e-4);
            },
            other => panic!("gas_station should be HardClearing, got {other:?}"),
        }
    }

    #[test]
    fn airport_major_includes_half_cylinders_and_flat_quads() {
        // Major airport uses the full primitive vocabulary — verify
        // HalfCylinder roofs + FlatQuad runways are both present.
        let libs = SubBiomeLibraries::shared();
        let feature = libs
            .find(BiomeClass::CoastalBoreal, "airport_major")
            .unwrap();
        let instances = compose_instances_for_feature(
            42,
            1,
            (0.0, 0.0),
            feature,
            BiomeSource::Primary(BiomeClass::CoastalBoreal),
            libs,
            &height_stub(0.0),
        );
        let halfcyl_count = instances
            .iter()
            .filter(|i| i.primitive == PrimitiveKind::HalfCylinder)
            .count();
        let flatquad_count = instances
            .iter()
            .filter(|i| i.primitive == PrimitiveKind::FlatQuad)
            .count();
        assert!(
            halfcyl_count >= 8,
            "airport_major should place at least 8 hangar roofs (got {halfcyl_count})"
        );
        assert!(
            flatquad_count >= 3,
            "airport_major should place at least 3 flat surfaces (got {flatquad_count})"
        );
    }

    #[test]
    fn linear_chain_places_evenly_spaced_aligned_instances() {
        // 5 instances along +X at 10 m spacing from cell_pos + (20, 0).
        // Expect: x positions 20, 30, 40, 50, 60; z = 0; rot_y = 0.
        let component = PrimitiveComponent {
            kind: PrimitiveKind::Box,
            count: (5, 5),
            base_size_m: (3.0, 5.0, 3.0),
            scale_jitter: (1.0, 1.0),
            color_source: ColorSource::Primary,
            placement: PlacementPattern::LinearChain {
                start_m: (20.0, 0.0),
                direction_rad: 0.0, // +X
                spacing_m: 10.0,
            },
            rng_stream: 7,
        };
        let palette = Palette {
            primary: (100, 100, 100),
            secondary: (0, 0, 0),
            accent: (0, 0, 0),
        };
        let mut out = Vec::new();
        emit_component(
            mix_seed(1, 2),
            (0.0, 0.0),
            &component,
            &palette,
            &|_, _| 0.0,
            &mut out,
        );
        assert_eq!(out.len(), 5);
        for (i, inst) in out.iter().enumerate() {
            let expected_x = 20.0 + (i as f32) * 10.0;
            assert!(
                (inst.position.x - expected_x).abs() < 1e-4,
                "instance {i} x={} expected {expected_x}",
                inst.position.x
            );
            assert!(inst.position.z.abs() < 1e-4);
            assert!(
                inst.rotation_y_rad.abs() < 1e-4,
                "rotation aligns with direction_rad=0",
            );
        }
    }

    #[test]
    fn single_placement_respects_count_and_offset() {
        // A Single-placement component with count=(1, 1) emits one
        // instance at cell_pos + offset. count=(3, 3) would emit three
        // instances all at the same position (useful for stacking).
        let component = PrimitiveComponent {
            kind: PrimitiveKind::Sphere,
            count: (1, 1),
            base_size_m: (2.0, 2.0, 2.0),
            scale_jitter: (1.0, 1.0),
            color_source: ColorSource::Explicit([10, 20, 30]),
            placement: PlacementPattern::Single {
                offset_m: (5.0, -7.0),
            },
            rng_stream: 42,
        };
        let palette = Palette {
            primary: (0, 0, 0),
            secondary: (0, 0, 0),
            accent: (0, 0, 0),
        };
        let mut out = Vec::new();
        emit_component(
            mix_seed(1, 2),
            (100.0, 200.0),
            &component,
            &palette,
            &|_, _| 0.0,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        let inst = out[0];
        assert_eq!(inst.primitive, PrimitiveKind::Sphere);
        assert!((inst.position.x - 105.0).abs() < 1e-5);
        assert!((inst.position.z - 193.0).abs() < 1e-5);
        assert_eq!(inst.color, [10, 20, 30, 255]);
        // Y = ground + half-height = 0 + 1 = 1.
        assert!((inst.position.y - 1.0).abs() < 1e-5);
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
