//! Ambient environment object generator (Phase 3.6.3c.5c.9).
//!
//! Fills the visual void between placed features with biome-wide
//! cover (forest / shrub / boulders / grass). Generates per-cell
//! ambient instances lazily at streaming cell-enter from a
//! deterministic seed `(master_seed, cell_id, ambient_entry_idx)` —
//! zero persistent storage.
//!
//! See [MAPGEN § Ambient environment objects](../../../MAPGEN.md#ambient-environment-objects-phase-363c5c8)
//! for full design.
//!
//! # Pipeline
//!
//! Called from the app's streaming cell-enter loop for each new cell:
//!
//! 1. Enumerate the biome's [`AmbientEntry`] list.
//! 2. Per entry, compute expected candidate count as
//!    `density_per_km2 × cell_area_km2`.
//! 3. For each candidate, seed a per-instance RNG and sample uniform
//!    disc coords within the cell.
//! 4. Filter: altitude range, slope, noise density threshold.
//! 5. Apply the cell-feature's [`FeatureAmbientControl`] — compute
//!    nearest feature-instance distance, reject or scale by clearing
//!    rule.
//! 6. Per-primitive instance-overlap check (all modes): reject if
//!    candidate is within any feature-instance footprint radius.
//! 7. Emit a [`FeatureInstance`] with biome-default ambient palette
//!    color.
//!
//! # Determinism
//!
//! All RNG is derived from `(master_seed, cell_id, entry_idx,
//! candidate_idx)`. Same seed → bit-identical ambient set. The
//! generator is pure w.r.t. its inputs; callers drive the flow from
//! stable streaming events.
//!
//! # MVP vs follow-up
//!
//! - 5c.2.c.9: generator + clearing modes + basic hash-based
//!   pseudo-noise filter.
//! - 5c.2.c.10: replace pseudo-noise with proper 4-octave fBm
//!   density fields seeded per-entry.
//! - 5c.2.c.11 (this revision): rect-to-point SDF for `Box` /
//!   `FlatQuad` so elongated primitives (airport runways) clear
//!   their actual footprint instead of their bounding circle.
//! - Deferred (5c.2.c.13): 1-ring neighbor-cell lookup for cross-
//!   cell clearing seams — open until visual-check flags them.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::biome::BiomeClass;
use crate::feature_instances::{
    AmbientEntry, ColorSource, FeatureAmbientControl, FeatureInstance, PrimitiveKind,
};
use crate::voronoi_cells::CellId;

/// Per-biome ambient palette (primary = rock / structural,
/// secondary = foliage, accent = highlights). Hardcoded constants
/// for 5c.2.c.9 MVP; 5c.2.c.11 may migrate to RON if authors want
/// to tune. Biome-appropriate natural tones independent of feature
/// palette adaptations.
const AMBIENT_PALETTE: [(BiomeClass, [[u8; 3]; 3]); 4] = [
    (
        BiomeClass::CoastalBoreal,
        [[125, 105, 85], [70, 110, 75], [160, 155, 140]], // brown bark / conifer / rock
    ),
    (
        BiomeClass::HighDesert,
        [[175, 135, 95], [130, 135, 95], [165, 140, 105]], // sandstone / sage / dusty
    ),
    (
        BiomeClass::RockyAlpine,
        [[125, 120, 115], [60, 95, 65], [170, 165, 155]], // granite / fir / talus
    ),
    (
        BiomeClass::MediterraneanArchipelago,
        [[180, 165, 140], [100, 130, 85], [165, 150, 120]], // limestone / olive / beige
    ),
];

fn ambient_palette_for(biome: BiomeClass) -> [[u8; 3]; 3] {
    AMBIENT_PALETTE.iter().find(|(b, _)| *b == biome).map_or(
        [[150, 140, 125], [90, 120, 90], [170, 160, 140]],
        |(_, p)| *p,
    )
}

const TAU: f32 = core::f32::consts::TAU;

/// Splitmix64-inspired hash — matches the `mix_seed` pattern used
/// across the world crate for deterministic PRNG chains.
const fn mix_seed(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn rand_unit(seed: u64, step: u64) -> f32 {
    let h = mix_seed(seed, step);
    ((h & 0xFFFF_FFFF) as f32) / (u32::MAX as f32 + 1.0)
}

/// Per-biome per-entry fBm density field (5c.2.c.10). 4 octaves of
/// Perlin seeded by the entry's tag (derived from `master_seed` +
/// biome + `entry_idx`). Raw output is normalized to `[0, 1]` so the
/// authored `noise_threshold` in RON maps intuitively (0 = always
/// spawn, 0.5 = half the field clusters, 0.9 = very sparse).
///
/// Same signature as the 5c.2.c.9 pseudo-noise it replaces; no caller
/// changes required.
fn density_noise(x: f32, z: f32, scale_m: f32, entry_tag: u64) -> f32 {
    // 4 octaves at the entry's base wavelength. Raw fBm range is
    // roughly [-1.4, 1.4]; map to [0, 1].
    let raw = crate::noise::fbm_2d(entry_tag, 0, x, z, scale_m, 4);
    ((raw / 2.8) + 0.5).clamp(0.0, 1.0)
}

/// Smoothstep — cubic ease from 0 to 1 over `t ∈ [0, 1]`.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Circular-approximation footprint radius for curved primitives
/// (`Cylinder`, `Sphere`, `Cone`, `HalfCylinder`). `Box` / `FlatQuad`
/// go through rotated-rect math in [`signed_distance_to_footprint`]
/// and never call this.
fn circular_footprint_radius(_kind: PrimitiveKind, scale: glam::Vec3) -> f32 {
    scale.x.max(scale.z) * 0.5
}

/// Signed distance from `(x, z)` to a feature instance's XZ-plane
/// footprint. Negative inside, zero at boundary, positive outside.
///
/// Elongated rect primitives (`Box`, `FlatQuad`) use a rotated-rect
/// test oriented by `rotation_y_rad` — critical for long runways
/// (`airport_major` 1500 m × 45 m) where a circular approximation
/// would either mis-clear 1500 m of cross-runway space or fail to
/// clear 200 m either end.
///
/// Curved primitives (`Cylinder`, `Sphere`, `Cone`, `HalfCylinder`)
/// keep the cheap circle-based distance.
#[allow(clippy::many_single_char_names)] // x, z, dx, dz are standard math names
fn signed_distance_to_footprint(x: f32, z: f32, feature: &FeatureInstance) -> f32 {
    let dx = x - feature.position.x;
    let dz = z - feature.position.z;
    match feature.primitive {
        PrimitiveKind::Box | PrimitiveKind::FlatQuad => {
            // Rotate (dx, dz) into the primitive's local frame. The
            // inverse of a CCW rotation by θ on a point is:
            //   (dx·cos(θ) + dz·sin(θ), -dx·sin(θ) + dz·cos(θ)).
            let (sin, cos) = feature.rotation_y_rad.sin_cos();
            let lx = dx * cos + dz * sin;
            let lz = -dx * sin + dz * cos;
            let hx = feature.scale.x * 0.5;
            let hz = feature.scale.z * 0.5;
            let qx = lx.abs() - hx;
            let qz = lz.abs() - hz;
            // Axis-aligned rectangle SDF (Inigo Quilez).
            let outside = qx.max(0.0).hypot(qz.max(0.0));
            let inside = qx.max(qz).min(0.0);
            outside + inside
        },
        PrimitiveKind::Cylinder
        | PrimitiveKind::Sphere
        | PrimitiveKind::Cone
        | PrimitiveKind::HalfCylinder => {
            let r = circular_footprint_radius(feature.primitive, feature.scale);
            dx.hypot(dz) - r
        },
    }
}

/// Distance from `(x, z)` to the nearest feature instance's footprint
/// edge. Returns `f32::INFINITY` when there are no feature instances.
/// Used by the clearing-mode logic.
fn nearest_feature_distance(x: f32, z: f32, features: &[FeatureInstance]) -> f32 {
    let mut min_d = f32::INFINITY;
    for f in features {
        let d = signed_distance_to_footprint(x, z, f);
        if d < min_d {
            min_d = d;
        }
    }
    min_d
}

/// True if `(x, z)` falls strictly within any feature-instance
/// footprint. Always rejected regardless of clearing mode.
fn is_within_any_footprint(x: f32, z: f32, features: &[FeatureInstance]) -> bool {
    features
        .iter()
        .any(|f| signed_distance_to_footprint(x, z, f) < 0.0)
}

/// Bounding-circle footprint radius — retained for the airport-runway
/// regression test that demonstrates what the rotated-rect improvement
/// avoids. Prefer [`signed_distance_to_footprint`] in hot paths.
#[cfg(test)]
fn bounding_footprint_radius(_kind: PrimitiveKind, scale: glam::Vec3) -> f32 {
    scale.x.max(scale.z) * 0.5
}

/// Default bounding-box half-extent for candidate sampling per cell
/// (m). The generator samples uniformly in `cell_pos ± this` square,
/// then keeps only candidates whose nearest seed is the cell being
/// generated. Sized generously to cover the largest realistic
/// power-diagram cell (max ~3 km radius observed in spawn area; we
/// use 4 km to be safe). Authors who want even larger cells can pass
/// a per-cell extent through `generate_ambient_for_cell`.
const AMBIENT_BBOX_HALF_M: f32 = 4_000.0;

/// Ground-Y anchoring: for each primitive kind return the
/// (center, scale) adjustment so bases sit on terrain.
fn ground_anchor_y(kind: PrimitiveKind, size_m: f32, ground_y: f32) -> (f32, glam::Vec3) {
    if kind == PrimitiveKind::FlatQuad {
        // Zero-thickness quad at y=0 local; lift 5 cm above ground to
        // avoid z-fighting with terrain.
        let scale = glam::Vec3::new(size_m, 0.1, size_m);
        (ground_y + 0.05, scale)
    } else {
        // Uniform scale; base rests on ground, center at ground + h/2.
        let scale = glam::Vec3::splat(size_m);
        (ground_y + size_m * 0.5, scale)
    }
}

/// Generate the ambient-cover instance list for a single Voronoi
/// cell. Called from the app's streaming cell-enter path for each
/// newly-active cell.
///
/// Arguments:
/// - `master_seed`: world seed.
/// - `cell_id`: the cell entering the active set.
/// - `cell_pos`: `(x, z)` world-space cell center.
/// - `biome`: host biome of the cell (drives ambient-entry lookup +
///   biome-default palette).
/// - `entries`: the biome's `AmbientEntry` list (typically from
///   `SubBiomeLibraries::ambient_for_biome(biome)`).
/// - `feature_instances`: the cell's baked feature instances
///   (consulted for overlap + clearing).
/// - `ambient_control`: the cell-assigned feature's control mode.
///   `None` (the default Option) = fully wild.
/// - `height_at`: terrain sampler for altitude + ground anchor.
/// - `is_water`: closure returning `true` if `(x, z)` is under a
///   water surface (sea / lake / river).
/// - `is_in_cell`: closure returning `true` if `(x, z)` belongs to
///   THIS cell's Voronoi polygon. Typically wraps
///   `SubBiomeCellCatalog::nearest_cell(x, z) == cell_id`. Tests can
///   pass `|_, _| true` to disable polygon clipping. This replaces
///   the disc-radius approximation: per-cell candidates fall in the
///   `cell_pos ± AMBIENT_BBOX_HALF_M` bounding box and are kept
///   only if they belong to this cell — zero overlap with neighbors,
///   zero gaps in coverage.
///
/// Returns `Vec<FeatureInstance>` tagged with primitive kind + color;
/// the renderer pipeline consumes them via the same upload path as
/// feature instances.
#[must_use]
#[allow(clippy::too_many_lines)] // linear per-candidate pipeline; splitting hurts readability
#[allow(clippy::too_many_arguments)] // per-cell context is what it is
pub fn generate_ambient_for_cell(
    master_seed: u64,
    cell_id: CellId,
    cell_pos: (f32, f32),
    biome: BiomeClass,
    entries: &[AmbientEntry],
    feature_instances: &[FeatureInstance],
    ambient_control: Option<FeatureAmbientControl>,
    height_at: &dyn Fn(f32, f32) -> f32,
    is_water: &dyn Fn(f32, f32) -> bool,
    is_in_cell: &dyn Fn(f32, f32) -> bool,
) -> Vec<FeatureInstance> {
    if entries.is_empty() {
        return Vec::new();
    }

    // Bounding-box area in km² — drives candidate count. The
    // is_in_cell filter rejects candidates that belong to neighbor
    // cells, so per-cell instance count ≈ density × cell area
    // regardless of how big the bounding box is.
    let bbox_side_m = AMBIENT_BBOX_HALF_M * 2.0;
    let bbox_area_km2 = bbox_side_m * bbox_side_m / 1_000_000.0;
    let palette = ambient_palette_for(biome);
    let control = ambient_control.unwrap_or(FeatureAmbientControl::None);
    let mut out = Vec::new();

    for (entry_idx, entry) in entries.iter().enumerate() {
        let target = (entry.density_per_km2 * bbox_area_km2).ceil() as u32;
        let entry_tag = mix_seed(0x4a47_45be_4143_4f56, entry_idx as u64); // "AMBIENT\0"-ish
        let entry_rng = mix_seed(mix_seed(master_seed, u64::from(cell_id)), entry_idx as u64);

        for i in 0..target {
            let cand_rng = mix_seed(entry_rng, u64::from(i));

            // Uniform sample in the bounding-box square.
            let ux = rand_unit(cand_rng, 1) * 2.0 - 1.0; // [-1, 1]
            let uz = rand_unit(cand_rng, 2) * 2.0 - 1.0;
            let px = cell_pos.0 + ux * AMBIENT_BBOX_HALF_M;
            let pz = cell_pos.1 + uz * AMBIENT_BBOX_HALF_M;

            // Voronoi-polygon clip — keep only candidates whose
            // nearest seed is THIS cell. Cheap O(1) catalog lookup
            // (5×5 grid neighborhood). Eliminates overlap with
            // neighbor cells AND gaps between disc samples.
            if !is_in_cell(px, pz) {
                continue;
            }

            // Water filter — cheap O(1) hydrology lookup, runs first
            // so we skip altitude sampling for water candidates.
            if is_water(px, pz) {
                continue;
            }

            // Altitude filter.
            let alt = height_at(px, pz);
            if alt < entry.altitude_range_m.0 || alt > entry.altitude_range_m.1 {
                continue;
            }

            // Slope filter (computed via 4-sample finite-difference
            // gradient magnitude; cheap, adequate for MVP).
            let offset = 5.0;
            let h_e = height_at(px + offset, pz);
            let h_w = height_at(px - offset, pz);
            let h_n = height_at(px, pz + offset);
            let h_s = height_at(px, pz - offset);
            let dh_dx = (h_e - h_w) / (2.0 * offset);
            let dh_dz = (h_n - h_s) / (2.0 * offset);
            let slope_deg = dh_dx.hypot(dh_dz).atan().to_degrees();
            if slope_deg > entry.slope_max_deg {
                continue;
            }

            // Density-field modulation. 4-octave fBm seeded per-entry;
            // raw range normalized to `[0, 1]`.
            //
            // `noise_floor` sets the minimum spawn weight — with
            // `noise_floor > 0` the below-threshold region still
            // spawns at reduced density / size, producing uniform
            // cover with continuous density variation (no circular
            // patches). See MAPGEN § Density modulation vs gating
            // (PINNED 2026-04-24).
            let noise = density_noise(px, pz, entry.noise_scale_m, entry_tag);
            let taper = entry.noise_taper.max(1e-4);
            let t_raw = (noise - (entry.noise_threshold - taper * 0.5)) / taper;
            let floor = entry.noise_floor.clamp(0.0, 1.0);
            let patch_weight = floor + (1.0 - floor) * smoothstep(t_raw);
            if patch_weight <= 0.0 {
                continue;
            }
            if rand_unit(cand_rng, 6) > patch_weight {
                continue;
            }

            // Instance-overlap (all modes): candidate rejected if
            // strictly inside any feature-instance footprint.
            if is_within_any_footprint(px, pz, feature_instances) {
                continue;
            }

            // Clearing-mode size factor. Rejects are handled inline.
            let size_factor = match control {
                FeatureAmbientControl::None => 1.0,
                FeatureAmbientControl::HardClearing { clear_radius_m } => {
                    let d = nearest_feature_distance(px, pz, feature_instances);
                    if d < clear_radius_m {
                        continue;
                    }
                    1.0
                },
                FeatureAmbientControl::TaperedClearing {
                    clear_radius_m,
                    taper_radius_m,
                } => {
                    let d = nearest_feature_distance(px, pz, feature_instances);
                    if d < clear_radius_m {
                        continue;
                    }
                    if d < taper_radius_m {
                        let t = (d - clear_radius_m) / (taper_radius_m - clear_radius_m).max(1e-3);
                        smoothstep(t)
                    } else {
                        1.0
                    }
                },
                FeatureAmbientControl::InverseTaperedClearing {
                    clear_radius_m,
                    taper_radius_m,
                } => {
                    let d = nearest_feature_distance(px, pz, feature_instances);
                    if d < clear_radius_m {
                        0.3 // pioneer sprout
                    } else if d < taper_radius_m {
                        let t = (d - clear_radius_m) / (taper_radius_m - clear_radius_m).max(1e-3);
                        0.3 + 0.7 * smoothstep(t)
                    } else {
                        1.0
                    }
                },
            };

            // Size pick. Multiplied by `patch_weight` so edge
            // candidates shrink toward 70% of authored size, fully
            // feathering the cluster boundary.
            let base_size = entry.size_range_m.0
                + rand_unit(cand_rng, 3) * (entry.size_range_m.1 - entry.size_range_m.0);
            let jitter = entry.scale_jitter.0
                + rand_unit(cand_rng, 4) * (entry.scale_jitter.1 - entry.scale_jitter.0);
            let edge_fade = 0.7 + 0.3 * patch_weight;
            let size = base_size * jitter * size_factor * edge_fade;
            if size < 0.1 {
                continue; // below visible-threshold — skip the upload
            }

            // Color resolution. Ambient uses per-biome default palette,
            // not the cell-feature's adapted palette.
            let rgb = match entry.color_source {
                ColorSource::Primary => palette[0],
                ColorSource::Secondary => palette[1],
                ColorSource::Accent => palette[2],
                ColorSource::Explicit(rgb) => rgb,
            };

            let rot_y = rand_unit(cand_rng, 5) * TAU;
            let (py, scale_vec) = ground_anchor_y(entry.kind, size, alt);

            out.push(FeatureInstance {
                primitive: entry.kind,
                position: glam::Vec3::new(px, py, pz),
                scale: scale_vec,
                rotation_y_rad: rot_y,
                color: [rgb[0], rgb[1], rgb[2], 255],
            });

            // Composite stack layers (5c.2.c.12b). Each layer spawns
            // one derived instance offset + scaled relative to the
            // base. Enables trunks, skirts, crowns, boulder piles.
            for (layer_idx, layer) in entry.stacks.iter().enumerate() {
                let layer_rng = mix_seed(cand_rng, 0x4c41_5945_5200_0000 ^ layer_idx as u64);
                let xz = layer.xz_scale.0
                    + rand_unit(layer_rng, 1) * (layer.xz_scale.1 - layer.xz_scale.0);
                let ys =
                    layer.y_scale.0 + rand_unit(layer_rng, 2) * (layer.y_scale.1 - layer.y_scale.0);
                let yoff_ratio = layer.y_offset_ratio.0
                    + rand_unit(layer_rng, 3) * (layer.y_offset_ratio.1 - layer.y_offset_ratio.0);
                let layer_rot_jitter =
                    (rand_unit(layer_rng, 4) * 2.0 - 1.0) * layer.rotation_y_jitter_rad;

                // Layer scale in world meters. FlatQuad keeps its
                // thin-plate y, other kinds get independent xz vs y.
                let (layer_scale, layer_center_y_offset) = match layer.kind {
                    PrimitiveKind::FlatQuad => (
                        glam::Vec3::new(size * xz, 0.1, size * xz),
                        0.05, // same y-lift as base FlatQuad
                    ),
                    _ => (
                        glam::Vec3::new(size * xz, size * ys, size * xz),
                        size * ys * 0.5, // center at base + half height
                    ),
                };
                let layer_center_y = alt + size * yoff_ratio + layer_center_y_offset;

                // Layer color resolution — independent palette pick.
                let layer_rgb = match layer.color_source {
                    ColorSource::Primary => palette[0],
                    ColorSource::Secondary => palette[1],
                    ColorSource::Accent => palette[2],
                    ColorSource::Explicit(rgb) => rgb,
                };

                out.push(FeatureInstance {
                    primitive: layer.kind,
                    position: glam::Vec3::new(px, layer_center_y, pz),
                    scale: layer_scale,
                    rotation_y_rad: rot_y + layer_rot_jitter,
                    color: [layer_rgb[0], layer_rgb[1], layer_rgb[2], 255],
                });
            }
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn const_height(h: f32) -> impl Fn(f32, f32) -> f32 {
        move |_x, _z| h
    }

    fn no_water(_x: f32, _z: f32) -> bool {
        false
    }

    /// Default polygon clip for tests: accept everything (single-cell
    /// world). Real callers wrap `catalog.nearest_cell(x, z) ==
    /// cell_id`.
    fn always_in_cell(_x: f32, _z: f32) -> bool {
        true
    }

    fn boreal_forest_entry() -> AmbientEntry {
        AmbientEntry {
            kind: PrimitiveKind::Cone,
            color_source: ColorSource::Secondary,
            density_per_km2: 100.0,
            size_range_m: (3.0, 8.0),
            scale_jitter: (0.9, 1.1),
            noise_scale_m: 400.0,
            noise_threshold: 0.0, // everything passes (max of pseudo-noise is 0.7)
            noise_taper: 0.0,     // hard threshold for test determinism
            noise_floor: 0.0,     // classic gating for test determinism
            altitude_range_m: (-100.0, 3000.0),
            slope_max_deg: 60.0,
            rng_stream: 0,
            stacks: Vec::new(),
        }
    }

    #[test]
    fn generator_produces_instances_on_flat_ground() {
        let entries = vec![boreal_forest_entry()];
        let insts = generate_ambient_for_cell(
            42,
            0,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &entries,
            &[],
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );
        assert!(
            !insts.is_empty(),
            "should emit ambient on unblocked flat ground"
        );
        for inst in &insts {
            assert_eq!(inst.primitive, PrimitiveKind::Cone);
            assert_eq!(inst.color[3], 255);
            // Ground-anchored: y = ground + scale.y / 2 = 100 + size/2.
            let expected_y = 100.0 + inst.scale.y * 0.5;
            assert!((inst.position.y - expected_y).abs() < 1e-3);
        }
    }

    #[test]
    fn altitude_filter_rejects_out_of_range() {
        let mut entry = boreal_forest_entry();
        entry.altitude_range_m = (0.0, 50.0); // ground is 100, everything rejected
        let insts = generate_ambient_for_cell(
            42,
            0,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &[entry],
            &[],
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );
        assert!(insts.is_empty());
    }

    #[test]
    fn generator_deterministic() {
        let entries = vec![boreal_forest_entry()];
        let a = generate_ambient_for_cell(
            42,
            123,
            (200.0, -400.0),
            BiomeClass::CoastalBoreal,
            &entries,
            &[],
            None,
            &const_height(50.0),
            &no_water,
            &always_in_cell,
        );
        let b = generate_ambient_for_cell(
            42,
            123,
            (200.0, -400.0),
            BiomeClass::CoastalBoreal,
            &entries,
            &[],
            None,
            &const_height(50.0),
            &no_water,
            &always_in_cell,
        );
        assert_eq!(a.len(), b.len());
        for (l, r) in a.iter().zip(b.iter()) {
            assert_eq!(l.position, r.position);
            assert_eq!(l.scale, r.scale);
        }
    }

    #[test]
    fn hard_clearing_rejects_ambient_in_radius() {
        // Place a large feature at cell center → hard clearing 100 m.
        // All ambient within 100 m gets dropped. Lots outside should
        // remain.
        let entries = vec![boreal_forest_entry()];
        let feature = FeatureInstance {
            primitive: PrimitiveKind::Box,
            position: glam::Vec3::new(0.0, 2.5, 0.0),
            scale: glam::Vec3::new(10.0, 5.0, 10.0),
            rotation_y_rad: 0.0,
            color: [150, 100, 80, 255],
        };
        let insts = generate_ambient_for_cell(
            42,
            0,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &entries,
            &[feature],
            Some(FeatureAmbientControl::HardClearing {
                clear_radius_m: 100.0,
            }),
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );
        for inst in &insts {
            let d = inst.position.x.hypot(inst.position.z);
            // Clearing is distance-to-edge; 10×10 box → ~5 m half-extent
            // → min distance-to-center ≈ 100 + 5 along axis. Keep the
            // assertion loose since corner-direction candidates are ~107.
            assert!(
                d >= 100.0,
                "ambient instance found too close to cleared feature: d={d}"
            );
        }
    }

    #[test]
    fn tapered_clearing_scales_down_near_feature() {
        let entries = vec![boreal_forest_entry()];
        let feature = FeatureInstance {
            primitive: PrimitiveKind::Box,
            position: glam::Vec3::new(0.0, 2.5, 0.0),
            scale: glam::Vec3::new(10.0, 5.0, 10.0),
            rotation_y_rad: 0.0,
            color: [150, 100, 80, 255],
        };
        // Taper from 30 to 100 m: inside 30 rejected, at 100+ full size.
        let insts = generate_ambient_for_cell(
            42,
            0,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &entries,
            &[feature],
            Some(FeatureAmbientControl::TaperedClearing {
                clear_radius_m: 30.0,
                taper_radius_m: 100.0,
            }),
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );

        // Partition: close = distance < 60, far = distance > 100.
        let (close, far): (Vec<&FeatureInstance>, Vec<&FeatureInstance>) =
            insts.iter().partition(|i| {
                let d = i.position.x.hypot(i.position.z);
                d < 60.0
            });
        // Require at least one of each bucket for the test to be meaningful.
        if !close.is_empty() && !far.is_empty() {
            let avg_close: f32 = close.iter().map(|i| i.scale.y).sum::<f32>() / close.len() as f32;
            let avg_far: f32 = far.iter().map(|i| i.scale.y).sum::<f32>() / far.len() as f32;
            assert!(
                avg_close <= avg_far + 0.5,
                "close ambient avg {avg_close} should be <= far avg {avg_far}"
            );
        }
    }

    #[test]
    fn elongated_quad_uses_rotated_rect_not_bounding_circle() {
        // Airport runway: 1500 m × 45 m FlatQuad, unrotated.
        // A circular bounding-radius approach would clear 750 m
        // cross-runway (wrong). Rect SDF clears only the 45 m width.
        let entries = vec![boreal_forest_entry()];
        let runway = FeatureInstance {
            primitive: PrimitiveKind::FlatQuad,
            position: glam::Vec3::new(0.0, 0.05, 0.0),
            scale: glam::Vec3::new(1500.0, 0.1, 45.0),
            rotation_y_rad: 0.0,
            color: [70, 70, 70, 255],
        };
        let insts = generate_ambient_for_cell(
            42,
            0,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &entries,
            core::slice::from_ref(&runway),
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );

        // No ambient should land ON the runway rect (|x| < 750, |z| < 22.5).
        for inst in &insts {
            let on_runway = inst.position.x.abs() < 750.0 && inst.position.z.abs() < 22.5;
            assert!(
                !on_runway,
                "ambient placed on runway at ({}, {})",
                inst.position.x, inst.position.z
            );
        }

        // A bounding-circle approach would have cleared the entire
        // 750 m radius; verify we DO allow ambient at moderate cross-
        // runway distance (say 60 m from centerline, well outside 22.5).
        // This is the business end of the rotated-rect improvement.
        let has_near_cross = insts.iter().any(|i| {
            i.position.x.abs() < 700.0 && i.position.z.abs() > 60.0 && i.position.z.abs() < 150.0
        });
        assert!(
            has_near_cross,
            "rotated-rect should allow ambient 60 m cross-runway (bounding-circle would block)"
        );

        // Sanity-check bounding-footprint radius for the runway is
        // 750 m — confirms a circle-only approach would have over-cleared.
        assert!((bounding_footprint_radius(runway.primitive, runway.scale) - 750.0).abs() < 1.0);
    }

    #[test]
    fn rotated_rect_clears_along_orientation() {
        // Same runway, rotated 90° — now wide axis is along Z.
        // Ambient should be blocked for |z| < 750 (not |x|).
        let entries = vec![boreal_forest_entry()];
        let runway = FeatureInstance {
            primitive: PrimitiveKind::FlatQuad,
            position: glam::Vec3::new(0.0, 0.05, 0.0),
            scale: glam::Vec3::new(1500.0, 0.1, 45.0),
            rotation_y_rad: core::f32::consts::FRAC_PI_2,
            color: [70, 70, 70, 255],
        };
        let insts = generate_ambient_for_cell(
            42,
            0,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &entries,
            &[runway],
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );
        for inst in &insts {
            let on_runway = inst.position.z.abs() < 750.0 && inst.position.x.abs() < 22.5;
            assert!(!on_runway, "rotated runway overlap missed");
        }
    }

    #[test]
    fn noise_floor_keeps_below_threshold_candidates_alive() {
        // High threshold (0.7) with floor=0 should produce few
        // instances (most noise values reject). With floor=0.5, the
        // below-threshold area still spawns at 50% base rate, so the
        // count jumps significantly.
        let mut gated = boreal_forest_entry();
        gated.density_per_km2 = 80.0;
        gated.noise_threshold = 0.7;
        gated.noise_taper = 0.15;
        gated.noise_floor = 0.0;
        let mut modulated = gated.clone();
        modulated.noise_floor = 0.5;

        let gated_out = generate_ambient_for_cell(
            99,
            21,
            (1234.0, -5678.0),
            BiomeClass::CoastalBoreal,
            &[gated],
            &[],
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );
        let modulated_out = generate_ambient_for_cell(
            99,
            21,
            (1234.0, -5678.0),
            BiomeClass::CoastalBoreal,
            &[modulated],
            &[],
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );

        // Modulated should produce MORE instances (below-threshold
        // candidates are kept probabilistically, not rejected).
        assert!(
            modulated_out.len() > gated_out.len() * 2,
            "floor=0.5 should spawn noticeably more than floor=0 at the same threshold: \
             gated={} modulated={}",
            gated_out.len(),
            modulated_out.len()
        );
    }

    #[test]
    fn soft_taper_reduces_spawn_count_vs_hard_threshold() {
        // Threshold of 0.5 on an entry with hard taper (0.0) lets
        // through half the field on average. Same threshold with a
        // wide taper (0.3) probabilistically accepts some sub-0.5
        // candidates and probabilistically rejects some above-0.5
        // ones — total count similar but edge smoother. Verify the
        // soft-taper version produces a non-empty output AND has
        // size variation at its low end (evidence of edge fade).
        let mut hard = boreal_forest_entry();
        hard.noise_threshold = 0.5;
        hard.noise_taper = 0.0;
        let mut soft = hard.clone();
        soft.noise_taper = 0.3;

        let hard_out = generate_ambient_for_cell(
            11,
            99,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &[hard],
            &[],
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );
        let soft_out = generate_ambient_for_cell(
            11,
            99,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &[soft],
            &[],
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );

        // Both produce output.
        assert!(!hard_out.is_empty());
        assert!(!soft_out.is_empty());

        // Soft-taper output should include at least one instance
        // scaled below 0.9× its maximum — edge-fade evidence.
        let max_y = soft_out.iter().map(|i| i.scale.y).fold(0.0_f32, f32::max);
        let has_small = soft_out.iter().any(|i| i.scale.y < max_y * 0.85);
        assert!(
            has_small,
            "soft-taper should produce edge instances smaller than interior"
        );
    }

    #[test]
    fn stacks_emit_derived_instances_per_base_candidate() {
        // Base cone with two stacks (skirt + crown) → each candidate
        // emits 3 instances (1 base + 2 stacks). Verify we get 3x the
        // output of an equivalent no-stacks entry.
        use crate::feature_instances::AmbientStackLayer;
        let base = boreal_forest_entry();
        let mut with_stacks = base.clone();
        with_stacks.stacks = vec![
            AmbientStackLayer {
                kind: PrimitiveKind::Cone,
                color_source: ColorSource::Secondary,
                xz_scale: (1.3, 1.3),
                y_scale: (0.4, 0.4),
                y_offset_ratio: (0.0, 0.0),
                rotation_y_jitter_rad: 0.0,
            },
            AmbientStackLayer {
                kind: PrimitiveKind::Cylinder,
                color_source: ColorSource::Primary,
                xz_scale: (0.15, 0.15),
                y_scale: (0.5, 0.5),
                y_offset_ratio: (-0.3, -0.3),
                rotation_y_jitter_rad: 0.0,
            },
        ];

        let base_insts = generate_ambient_for_cell(
            42,
            7,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &[base],
            &[],
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );
        let stacked_insts = generate_ambient_for_cell(
            42,
            7,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &[with_stacks],
            &[],
            None,
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );

        assert_eq!(
            stacked_insts.len(),
            base_insts.len() * 3,
            "each candidate should emit base + 2 stacks"
        );

        // Each base instance is followed by its two stacks, in order.
        for triple in stacked_insts.chunks_exact(3) {
            let (base_inst, skirt, trunk) = (&triple[0], &triple[1], &triple[2]);
            // Skirt: wider (xz=1.3) than base (xz=1.0). Cone.
            assert_eq!(skirt.primitive, PrimitiveKind::Cone);
            assert!(skirt.scale.x > base_inst.scale.x);
            // Trunk: narrower than base, shorter, Cylinder.
            assert_eq!(trunk.primitive, PrimitiveKind::Cylinder);
            assert!(trunk.scale.x < base_inst.scale.x);
            assert!(trunk.scale.y < base_inst.scale.y);
        }
    }

    #[test]
    fn water_filter_rejects_candidates_under_water_surface() {
        // Terrain at +5 m (above entry's -10 floor) but flagged as
        // water. The altitude filter would accept these; the water
        // filter must reject them. Simulates elevated-lake case where
        // terrain height > 0 but water sits on top.
        let entries = vec![boreal_forest_entry()];
        let everywhere_water = |_x: f32, _z: f32| true;
        let insts = generate_ambient_for_cell(
            42,
            0,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &entries,
            &[],
            None,
            &const_height(5.0),
            &everywhere_water,
            &always_in_cell,
        );
        assert!(
            insts.is_empty(),
            "water filter must reject all candidates regardless of altitude"
        );
    }

    #[test]
    fn instance_overlap_rejects_exact_position() {
        let entries = vec![boreal_forest_entry()];
        // Tiny feature — only exact overlap should be rejected.
        let feature = FeatureInstance {
            primitive: PrimitiveKind::Cylinder,
            position: glam::Vec3::new(100.0, 5.0, 100.0),
            scale: glam::Vec3::new(2.0, 10.0, 2.0),
            rotation_y_rad: 0.0,
            color: [150, 100, 80, 255],
        };
        let insts = generate_ambient_for_cell(
            42,
            0,
            (0.0, 0.0),
            BiomeClass::CoastalBoreal,
            &entries,
            &[feature],
            None, // wild — no clearing
            &const_height(100.0),
            &no_water,
            &always_in_cell,
        );
        // Verify no ambient is INSIDE the 1m cylinder footprint
        // (scale.x / 2 = 1 m).
        for inst in &insts {
            let dx = inst.position.x - 100.0;
            let dz = inst.position.z - 100.0;
            let d = dx.hypot(dz);
            assert!(
                d >= 1.0 - 1e-3,
                "ambient should not overlap feature footprint"
            );
        }
    }
}
