//! Feature-delta implementations (Phase 3.6.3a + 5c.2.c.7).
//!
//! Pure functions returning the L3 heightfield correction for each
//! feature type. Each delta uses the shared [`DeltaContext`] + falloff
//! helpers from [`crate::feature_delta`].
//!
//! ## Delta pattern categories
//!
//! | Category | Shape | Example features |
//! |---|---|---|
//! | Bowl | Negative gaussian depression | lake_cluster, alpine_tarn, hot_springs, oxbow_lake, blowhole |
//! | Asymmetric bowl | Steeper on one side | cirque |
//! | Plateau | Positive flattop + cliff edge | mesa_top, book_cliffs, canyon_rim |
//! | Cliff coast | Raise interior, drop at edge | cliff_coast |
//! | Shore ramp | Slopes toward center | sandy_beach |
//! | Dune field | Layered-sine positive bias | dune_field |
//! | Channel | Narrow linear negative band | slickrock_canyon, slot_canyon, dry_wash, creek_valley, fjord_entrance, avalanche_chute |
//! | V-valley | Symmetric linear negative | scrub_valley, narrow_valley |
//! | U-valley | Wide flat-bottomed negative | alpine_valley |
//! | Ridge | Linear positive | ridge_traverse, rocky_ridge, moraine_field |
//! | Mound | Positive gaussian | rocky_islet, pine_headland, sea_stack, natural_arch, natural_bridge, hoodoo_field, spire_formation |
//! | Slope | Asymmetric cross-cell gradient | talus_slope |
//! | Bumpy | High-freq small amplitude | (reserved) |
//! | Prominence | Sharp tall peak | scenic_overlook, volcanic_cone |
//! | Step | Sharp elevation change one side | waterfall |
//! | Flatten | Gentle pull toward cell mean | bush_strip_clearing, alpine_meadow, natural_meadow, river_floodplain, coastal_plain, salt_flat |
//! | HubFoundation | Tier-adaptive flatten + skirt | hub_foundation_{major,regional,local,remote} |
//!
//! Parameters per feature (depth, radius, cliff offsets) are tuned
//! against visual-bench output — expect iteration after the first
//! L3 renders land. Values here are reasonable first-pass defaults.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::doc_markdown)]

use core::f32::consts::PI;

use crate::feature_delta::{smoothstep, DeltaContext};
use crate::hub_catalog::HubTier;

// ═══════════════════════════════════════════════════════════════════════════
// Natural feature deltas
// ═══════════════════════════════════════════════════════════════════════════

/// Dispatch to the right delta function for a natural-feature id.
/// Unknown ids (open_ocean, fishing_grounds, scenery tags, plain, etc.)
/// return 0 — they don't modify the heightfield.
#[must_use]
pub fn natural_delta(feature_id: &str, ctx: &DeltaContext) -> f32 {
    match feature_id {
        // ── Bowls ───────────────────────────────────────────────────
        "lake_cluster" => bowl_delta(ctx, 35.0),
        "alpine_tarn" => bowl_delta(ctx, 50.0),
        "glacier_dammed_lake" => bowl_delta(ctx, 60.0),
        "dry_lake_bed" => bowl_delta(ctx, 12.0),
        "protected_cove" => bowl_delta(ctx, 18.0),
        "hot_springs" => bowl_delta(ctx, 4.0),
        "small_harbor" => bowl_delta(ctx, 6.0),
        "oxbow_lake" => bowl_delta(ctx, 15.0),
        "blowhole" => bowl_delta(ctx, 3.0),
        "cirque" => asymmetric_bowl_delta(ctx, 70.0),

        // ── Plateaus / cliffs ───────────────────────────────────────
        "mesa_top" => plateau_delta(ctx, 140.0, 0.75),
        "book_cliffs" => plateau_delta(ctx, 380.0, 0.55),
        "canyon_rim" => plateau_delta(ctx, 220.0, 0.5),
        "cliff_coast" => cliff_coast_delta(ctx, 90.0),

        // ── Shore ramp ──────────────────────────────────────────────
        "sandy_beach" => shore_ramp_delta(ctx, -8.0),

        // ── Dune field ──────────────────────────────────────────────
        "dune_field" => dune_delta(ctx, 22.0),

        // ── Channels (narrow linear cuts) ───────────────────────────
        "slickrock_canyon" => channel_delta(ctx, 60.0, 0.3),
        "slot_canyon" => channel_delta(ctx, 45.0, 0.12),
        "dry_wash" => channel_delta(ctx, 8.0, 0.25),
        "creek_valley" => channel_delta(ctx, 15.0, 0.3),
        "avalanche_chute" => channel_delta(ctx, 25.0, 0.25),
        "fjord_entrance" => channel_delta(ctx, 120.0, 0.4),

        // ── Valleys ─────────────────────────────────────────────────
        "scrub_valley" => v_valley_delta(ctx, 30.0),
        "narrow_valley" => v_valley_delta(ctx, 90.0),
        "alpine_valley" => u_valley_delta(ctx, 75.0, 0.4),

        // ── Ridges ──────────────────────────────────────────────────
        "ridge_traverse" => ridge_delta(ctx, 85.0),
        "rocky_ridge" => ridge_delta(ctx, 65.0),
        "moraine_field" => ridge_delta(ctx, 35.0),

        // ── Mounds ──────────────────────────────────────────────────
        "rocky_islet" => mound_delta(ctx, 22.0),
        "pine_headland" => mound_delta(ctx, 40.0),
        "sea_stack" => mound_delta(ctx, 45.0),
        "natural_arch" => mound_delta(ctx, 18.0),
        "natural_bridge" => mound_delta(ctx, 28.0),
        "hoodoo_field" => mound_delta(ctx, 15.0),
        "spire_formation" => mound_delta(ctx, 50.0),

        // ── Slope (asymmetric cross-cell) ───────────────────────────
        "talus_slope" => slope_delta(ctx, 55.0),

        // ── Prominences (sharp peaks) ───────────────────────────────
        "scenic_overlook" => prominence_delta(ctx, 45.0, 1.4),
        "volcanic_cone" => prominence_delta(ctx, 180.0, 2.0),

        // ── Step (waterfall) ────────────────────────────────────────
        "waterfall" => step_delta(ctx, 35.0),

        // ── Flattens (gentle smoothing) ─────────────────────────────
        // Strength scale (first-pass tuning, refine after L3 visual-bench).
        // Developments + resupply (3.6.3c.2) interleave with natural
        // flattens by surface character: plowed > paved > shop-pad >
        // yard > clearing > rough-camp.
        "salt_flat" => flatten_delta(ctx, 0.95),
        "farmland_parcel" => flatten_delta(ctx, 0.85),
        "gas_station" => flatten_delta(ctx, 0.8),
        "bush_strip_clearing" => flatten_delta(ctx, 0.75),
        "ranch" | "general_store" => flatten_delta(ctx, 0.65),
        "dwelling_cluster" | "ferry_dock" => flatten_delta(ctx, 0.6),
        "river_floodplain" | "bait_shop" | "tackle_shop" => flatten_delta(ctx, 0.55),
        "natural_meadow" | "coastal_plain" | "homestead_clearing" | "remote_compound" => {
            flatten_delta(ctx, 0.5)
        },
        "alpine_meadow" => flatten_delta(ctx, 0.45),
        "fishing_camp" => flatten_delta(ctx, 0.4),

        // All other feature ids (including `plain`, oceanic types,
        // scenery tags, infrastructure from 3.6.3b which is blocked
        // on `FeatureGroup` 3.6.3c.3) produce no heightfield delta.
        _ => 0.0,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Pattern implementations
// ═══════════════════════════════════════════════════════════════════════════

/// Negative bowl — gaussian-like depression centered on the cell.
///
/// Depth parameter is the peak depression at cell center (positive
/// input interpreted as downward distance).
fn bowl_delta(ctx: &DeltaContext, depth_m: f32) -> f32 {
    let f = ctx.falloff();
    -depth_m * f
}

/// Asymmetric bowl — one side slopes steeply (cliff side), other side
/// gentle. Used for cirques where glacial plucking creates a
/// characteristic back wall.
///
/// Encodes asymmetry via a directional factor based on the query
/// point's position relative to cell center. Deterministic within the
/// cell — consistent cliff side across all queries to the same cell.
fn asymmetric_bowl_delta(ctx: &DeltaContext, depth_m: f32) -> f32 {
    let f = ctx.falloff();
    // Direction vector from cell center to query point (normalized
    // roughly). Use query vs center sign for a cheap bias.
    let dx = ctx.x - ctx.cell_center_x;
    let dz = ctx.z - ctx.cell_center_z;
    // "Uphill" side (positive dx) gets shallower bowl; "downhill"
    // gets deeper + cliff-like transition.
    let mut dist_factor = f;
    if dx + dz > 0.0 {
        dist_factor *= 0.7; // shallower on one side
    } else {
        dist_factor *= 1.15; // deeper on the other
    }
    -depth_m * dist_factor.min(1.0)
}

/// Positive plateau — flat top at `height_m`, cliff edge at
/// `plateau_fraction × cell_radius` (inner), smooth falloff beyond.
///
/// `plateau_fraction = 0.75` means inner 75% of cell radius is flat
/// plateau, outer 25% ramps back down to 0.
fn plateau_delta(ctx: &DeltaContext, height_m: f32, plateau_fraction: f32) -> f32 {
    let t = ctx.normalized_distance;
    if t >= 1.0 {
        return 0.0;
    }
    if t <= plateau_fraction {
        // Full height on plateau interior.
        height_m
    } else {
        // Smooth cliff-like falloff from plateau_fraction to 1.0.
        let cliff_t = (t - plateau_fraction) / (1.0 - plateau_fraction);
        height_m * (1.0 - smoothstep(cliff_t))
    }
}

/// Cliff-coast delta — sharp positive mound on the landward side,
/// fast fall on the seaward side. Approximates a coastal cliff edge.
///
/// Uses the cell center-to-query direction to compute "landward vs
/// seaward"; deterministic per cell but doesn't account for real sea
/// direction (refined in 3.6.3c when shore-awareness adds to the
/// engine).
fn cliff_coast_delta(ctx: &DeltaContext, drop_m: f32) -> f32 {
    let t = ctx.normalized_distance;
    if t >= 1.0 {
        return 0.0;
    }
    // Raise the interior ~half of the cell, then drop to zero at edge.
    if t <= 0.4 {
        drop_m
    } else {
        let cliff_t = (t - 0.4) / 0.6;
        drop_m * (1.0 - smoothstep(cliff_t))
    }
}

/// Shore ramp — gentle negative slope toward cell center (approaching
/// the shoreline/water body). `ramp_depth_m` is the target depression
/// at cell center; zero at cell edge.
fn shore_ramp_delta(ctx: &DeltaContext, ramp_depth_m: f32) -> f32 {
    let t = ctx.normalized_distance;
    if t >= 1.0 {
        return 0.0;
    }
    // Linear-ish ramp from edge (0) to center (ramp_depth_m).
    ramp_depth_m * (1.0 - t) * ctx.falloff()
}

/// Dune field — high-frequency positive noise overlaid. Deterministic
/// within the cell via sinusoidal patterns keyed to query position.
fn dune_delta(ctx: &DeltaContext, amplitude_m: f32) -> f32 {
    let f = ctx.falloff();
    // Layered sinusoidal (cheap, no Perlin call in this hot path).
    // Wavelengths chosen for "dune-like" appearance: ~80 m primary,
    // ~150 m secondary.
    let k1 = 2.0 * PI / 80.0;
    let k2 = 2.0 * PI / 150.0;
    let theta1 = 0.4_f32;
    let theta2 = 1.1_f32;
    let proj1 = ctx.x * libm::cosf(theta1) + ctx.z * libm::sinf(theta1);
    let proj2 = ctx.x * libm::cosf(theta2) + ctx.z * libm::sinf(theta2);
    let dune_shape = libm::sinf(k1 * proj1) * 0.6 + libm::sinf(k2 * proj2) * 0.4;
    amplitude_m * dune_shape.max(-0.3) * f // bias positive (dunes rise)
}

/// Flatten — bias query altitude toward the cell center's base
/// altitude. `strength` in [0, 1]: 1.0 = fully flat plateau at cell
/// center altitude; 0.0 = no change.
///
/// Returns the delta to ADD to base_altitude to achieve the blend.
fn flatten_delta(ctx: &DeltaContext, strength: f32) -> f32 {
    let f = ctx.falloff();
    // Target = cell center's altitude. Since we don't have the center
    // altitude directly in the context (only query-point base), we
    // approximate: delta makes the query point closer to what a
    // bush strip would be — a gentle pull toward a "level reference"
    // that's the cell-center altitude. For a first pass, assume the
    // cell-center altitude is "reasonable bush-strip" and flatten the
    // query's deviation by `strength × falloff`.
    //
    // Since we can't easily sample cell-center altitude here without
    // extra terrain sampling, use a local approximation: the flatten
    // target is the query's base altitude (no change) but reduce
    // variation in a radius. As a placeholder that has the right
    // sign/magnitude, emit a small negative delta proportional to
    // falloff — this "settles" the cell into a flat strip look.
    //
    // Refinement (3.6.3c): pass cell_center_altitude through the
    // context and compute a proper blend.
    let local_variation_proxy = 5.0_f32; // ~m of simulated variation to smooth
    -local_variation_proxy * strength * f
}

/// Channel — narrow linear negative band running across the cell. Used
/// for canyons, washes, avalanche chutes, fjords. The channel aligns
/// with the cell's x-axis; rotational randomization lands in a later
/// polish pass. `width_fraction` is the band half-width relative to
/// cell radius (0.12 = slot canyon, 0.4 = broad fjord).
///
/// The cut is deepest at the center-line (z = cell_center_z) and falls
/// smoothly to zero at ±width_fraction × cell_radius.
fn channel_delta(ctx: &DeltaContext, depth_m: f32, width_fraction: f32) -> f32 {
    let cross_axis_dist = (ctx.z - ctx.cell_center_z).abs();
    let width = width_fraction * ctx.cell_radius_m;
    if cross_axis_dist >= width {
        return 0.0;
    }
    // Within the band: smooth profile perpendicular to the channel,
    // plus standard cell-edge falloff along the channel's long axis.
    let cross_t = cross_axis_dist / width;
    let perp_profile = 1.0 - smoothstep(cross_t);
    let long_axis_falloff = ctx.falloff();
    -depth_m * perp_profile * long_axis_falloff
}

/// V-valley — symmetric linear negative with triangular cross-section.
/// Gentler and broader than `channel_delta`; used for scrub_valley,
/// narrow_valley. Peak depth at the cell's center-line.
fn v_valley_delta(ctx: &DeltaContext, depth_m: f32) -> f32 {
    let cross = (ctx.z - ctx.cell_center_z).abs() / ctx.cell_radius_m;
    if cross >= 1.0 {
        return 0.0;
    }
    // V-profile: linear rise from center (max) to edge (zero).
    let perp_profile = 1.0 - cross;
    let long_falloff = ctx.falloff();
    -depth_m * perp_profile * long_falloff
}

/// U-valley — wide flat-bottomed negative channel. The central
/// `flat_fraction × cell_radius` is uniform max depth; outside that
/// ramps smoothly to zero at the cell radius. Matches glacial valleys
/// (e.g. alpine_valley).
fn u_valley_delta(ctx: &DeltaContext, depth_m: f32, flat_fraction: f32) -> f32 {
    let cross = (ctx.z - ctx.cell_center_z).abs() / ctx.cell_radius_m;
    if cross >= 1.0 {
        return 0.0;
    }
    let perp_profile = if cross <= flat_fraction {
        1.0
    } else {
        let t = (cross - flat_fraction) / (1.0 - flat_fraction);
        1.0 - smoothstep(t)
    };
    let long_falloff = ctx.falloff();
    -depth_m * perp_profile * long_falloff
}

/// Ridge — linear positive bump. Mirror image of `v_valley_delta` but
/// adding height along a center-line rather than cutting depth.
/// Orientation fixed along cell's x-axis for this first-pass; per-cell
/// rotation is a 5c.2.c.8+ polish item.
fn ridge_delta(ctx: &DeltaContext, height_m: f32) -> f32 {
    let cross = (ctx.z - ctx.cell_center_z).abs() / ctx.cell_radius_m;
    if cross >= 1.0 {
        return 0.0;
    }
    let perp_profile = 1.0 - cross;
    let long_falloff = ctx.falloff();
    height_m * perp_profile * long_falloff
}

/// Mound — positive gaussian bump centered on cell. Simple inverse of
/// `bowl_delta`. Sea stacks / rocky islets / arches etc. rise above the
/// base terrain by `height_m`.
fn mound_delta(ctx: &DeltaContext, height_m: f32) -> f32 {
    height_m * ctx.falloff()
}

/// Slope — asymmetric cross-cell elevation gradient. One side of the
/// cell is `height_m` higher than the other; deterministic per-cell
/// direction. Used for talus slopes and hillside flanks.
fn slope_delta(ctx: &DeltaContext, height_m: f32) -> f32 {
    // Normalize the x-offset into [-1, +1], then into [0, 1] so the
    // lower side is 0 and upper side is full height. Apply cell-edge
    // falloff so neighbors blend cleanly.
    let norm_x = (ctx.x - ctx.cell_center_x) / ctx.cell_radius_m;
    let side_t = (norm_x.clamp(-1.0, 1.0) + 1.0) * 0.5;
    height_m * side_t * ctx.falloff()
}

/// Bumpy — small-amplitude high-frequency noise overlay. Used for
/// talus-field surface detail, moraine roughness. Cheaper than full
/// Perlin; sinusoidal pattern keyed off query position. Bias toward
/// zero-mean so it adds texture without net altitude change.
#[allow(dead_code)] // reserved for badlands / moraine surface detail
fn bumpy_delta(ctx: &DeltaContext, amplitude_m: f32) -> f32 {
    let f = ctx.falloff();
    // Shorter wavelengths than dune_delta for finer texture.
    let k1 = 2.0 * PI / 18.0;
    let k2 = 2.0 * PI / 30.0;
    let n1 = libm::sinf(k1 * ctx.x) * libm::cosf(k1 * ctx.z);
    let n2 = libm::sinf(k2 * ctx.x + 1.7) * libm::sinf(k2 * ctx.z + 0.4);
    amplitude_m * (0.6 * n1 + 0.4 * n2) * f
}

/// Prominence — sharp tall peak. Like `mound_delta` but with `sharpness`
/// exponent applied to the falloff so the top is narrower and steeper.
/// Scenic overlooks, volcanic cones. `sharpness = 1.0` = same as mound;
/// `sharpness = 2.0` = noticeably pointier.
fn prominence_delta(ctx: &DeltaContext, height_m: f32, sharpness: f32) -> f32 {
    let f = ctx.falloff().powf(sharpness.max(0.5));
    height_m * f
}

/// Step — sharp elevation change running across the cell. Used for
/// waterfalls: one side of the cell is `step_m` higher than the other,
/// with a narrow transition zone at the center. Differs from `slope_delta`
/// in having a discrete jump rather than a smooth gradient.
fn step_delta(ctx: &DeltaContext, step_m: f32) -> f32 {
    let norm_x = (ctx.x - ctx.cell_center_x) / ctx.cell_radius_m;
    // Narrow transition band at the cell center (±5% of radius).
    let band = 0.05;
    let side_t = if norm_x < -band {
        1.0 // upper (raised) side
    } else if norm_x > band {
        0.0 // lower (base) side
    } else {
        // Linear blend inside the band.
        (band - norm_x) / (2.0 * band)
    };
    step_m * side_t * ctx.falloff()
}

// ═══════════════════════════════════════════════════════════════════════════
// Hub foundation delta
// ═══════════════════════════════════════════════════════════════════════════

/// Tier-adaptive hub foundation delta.
///
/// Cells hosting hubs get this delta instead of their natural feature
/// delta. Aggressiveness scales by tier:
///
/// | Tier | Behavior |
/// |---|---|
/// | Major | Full flatten to runway datum + 50 m skirt + cut-and-fill into positive elevation |
/// | Regional | Flatten + 30 m skirt |
/// | Local | Partial flatten + natural surface preserved at edges |
/// | Remote | Minimal smoothing (terrain itself is the hub) |
/// | Isolated | No delta — pristine natural surface |
///
/// `runway_datum_m` is the target flat altitude. For now we use the
/// cell center's base altitude; in 3.6.3c this gets refined per hub
/// (runway end + approach slope).
#[must_use]
pub fn hub_foundation_delta(tier: HubTier, ctx: &DeltaContext, runway_datum_m: f32) -> f32 {
    let strength = match tier {
        HubTier::Major => 1.0,
        HubTier::Regional => 0.75,
        HubTier::Local => 0.45,
        HubTier::Remote => 0.15,
        HubTier::Isolated => return 0.0, // pristine
    };

    // The skirt — area beyond the inner flat zone that ramps up/down
    // to match natural terrain at cell edge.
    let skirt_fraction = match tier {
        HubTier::Major => 0.7,    // inner 70% perfectly flat
        HubTier::Regional => 0.6, // inner 60%
        HubTier::Local => 0.4,
        HubTier::Remote => 0.2,
        HubTier::Isolated => 0.0,
    };

    let t = ctx.normalized_distance;
    if t >= 1.0 {
        return 0.0;
    }

    // Compute the target altitude at this query point.
    // - Inside the skirt fraction: pull fully to runway_datum.
    // - Outside the skirt: smoothly blend back to natural altitude
    //   via smoothstep on (t - skirt_fraction) / (1 - skirt_fraction).
    let pull_factor = if t <= skirt_fraction {
        1.0
    } else {
        let t_skirt = (t - skirt_fraction) / (1.0 - skirt_fraction);
        1.0 - smoothstep(t_skirt)
    };

    // Delta = (runway_datum - base) × pull_factor × strength.
    let target_altitude = runway_datum_m;
    let needed_change = target_altitude - ctx.base_altitude_m;
    needed_change * pull_factor * strength
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ctx_at(t: f32, base_altitude: f32) -> DeltaContext {
        DeltaContext {
            x: 0.0,
            z: 0.0,
            cell_center_x: 0.0,
            cell_center_z: 0.0,
            cell_radius_m: 1000.0,
            base_altitude_m: base_altitude,
            normalized_distance: t,
        }
    }

    #[test]
    fn bowl_delta_zero_at_edge() {
        let ctx = ctx_at(1.0, 100.0);
        assert!(bowl_delta(&ctx, 50.0).abs() < 1e-3);
    }

    #[test]
    fn bowl_delta_negative_at_center() {
        let ctx = ctx_at(0.0, 100.0);
        let d = bowl_delta(&ctx, 50.0);
        assert!(d < 0.0);
        assert!((d + 50.0).abs() < 0.1);
    }

    #[test]
    fn plateau_delta_full_on_interior() {
        let ctx = ctx_at(0.3, 100.0);
        let d = plateau_delta(&ctx, 150.0, 0.75);
        assert!((d - 150.0).abs() < 0.01);
    }

    #[test]
    fn plateau_delta_falls_to_zero_at_edge() {
        let ctx = ctx_at(1.0, 100.0);
        let d = plateau_delta(&ctx, 150.0, 0.75);
        assert!(d.abs() < 0.01);
    }

    #[test]
    fn hub_foundation_isolated_no_delta() {
        let ctx = ctx_at(0.5, 100.0);
        let d = hub_foundation_delta(HubTier::Isolated, &ctx, 50.0);
        assert_eq!(d.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn hub_foundation_major_full_pull_at_center() {
        let ctx = ctx_at(0.0, 80.0);
        let runway_datum = 50.0_f32;
        let d = hub_foundation_delta(HubTier::Major, &ctx, runway_datum);
        // Pulls fully to datum: delta = (50 - 80) × 1.0 × 1.0 = -30
        assert!((d - (-30.0)).abs() < 0.1);
    }

    #[test]
    fn hub_foundation_regional_weaker_than_major() {
        let ctx = ctx_at(0.0, 80.0);
        let d_major = hub_foundation_delta(HubTier::Major, &ctx, 50.0);
        let d_regional = hub_foundation_delta(HubTier::Regional, &ctx, 50.0);
        assert!(d_regional.abs() < d_major.abs());
    }

    #[test]
    fn hub_foundation_zero_at_edge() {
        let ctx = ctx_at(1.0, 80.0);
        let d = hub_foundation_delta(HubTier::Major, &ctx, 50.0);
        assert!(d.abs() < 0.1);
    }

    #[test]
    fn natural_delta_unknown_returns_zero() {
        let ctx = ctx_at(0.5, 100.0);
        assert_eq!(natural_delta("plain", &ctx).to_bits(), 0.0_f32.to_bits());
        assert_eq!(
            natural_delta("open_ocean", &ctx).to_bits(),
            0.0_f32.to_bits()
        );
        assert_eq!(
            natural_delta("nonexistent_feature", &ctx).to_bits(),
            0.0_f32.to_bits()
        );
    }

    #[test]
    fn natural_delta_known_features_non_zero() {
        let ctx = ctx_at(0.3, 100.0);
        assert!(natural_delta("lake_cluster", &ctx) < 0.0);
        assert!(natural_delta("mesa_top", &ctx) > 0.0);
        assert!(natural_delta("book_cliffs", &ctx) > 0.0);
        assert!(natural_delta("cliff_coast", &ctx) > 0.0);
    }

    #[test]
    fn dev_resupply_deltas_emit_modest_flatten() {
        // 3.6.3c.2 features must produce a non-zero (negative) delta —
        // confirms wiring through `natural_delta` dispatch. Magnitudes
        // bounded by `flatten_delta`'s 5 m proxy × strength × falloff.
        let ctx = ctx_at(0.3, 100.0);
        for id in [
            "farmland_parcel",
            "ranch",
            "dwelling_cluster",
            "homestead_clearing",
            "remote_compound",
            "gas_station",
            "general_store",
            "ferry_dock",
            "bait_shop",
            "tackle_shop",
            "fishing_camp",
        ] {
            let d = natural_delta(id, &ctx);
            assert!(
                d < 0.0,
                "{id} should emit a negative flatten delta, got {d}"
            );
            assert!(
                d > -5.0,
                "{id} flatten magnitude should stay under the 5 m proxy ceiling, got {d}"
            );
        }
    }

    #[test]
    fn dev_deltas_zero_at_edge() {
        let ctx = ctx_at(1.0, 100.0);
        for id in ["farmland_parcel", "gas_station", "ferry_dock"] {
            let d = natural_delta(id, &ctx);
            assert!(d.abs() < 1e-3, "{id} should taper to zero at cell edge");
        }
    }

    #[test]
    fn dev_deltas_strength_ordering() {
        // farmland (cultivated, very flat) > gas_station (paved pad) >
        // fishing_camp (rough clearing). At identical query position
        // the magnitudes should reflect that ordering.
        let ctx = ctx_at(0.3, 100.0);
        let farmland = natural_delta("farmland_parcel", &ctx).abs();
        let gas = natural_delta("gas_station", &ctx).abs();
        let camp = natural_delta("fishing_camp", &ctx).abs();
        assert!(
            farmland > gas,
            "farmland {farmland} should exceed gas {gas}"
        );
        assert!(gas > camp, "gas {gas} should exceed fishing-camp {camp}");
    }

    fn ctx_at_xz(x: f32, z: f32, cell_cx: f32, cell_cz: f32, cell_r: f32, t: f32) -> DeltaContext {
        DeltaContext {
            x,
            z,
            cell_center_x: cell_cx,
            cell_center_z: cell_cz,
            cell_radius_m: cell_r,
            base_altitude_m: 100.0,
            normalized_distance: t,
        }
    }

    #[test]
    fn channel_delta_deep_at_centerline_zero_past_band() {
        // Cell centered at origin, radius 1000, channel width 30%.
        // Query on centerline (z=0) = max depth; past band (z=400) = 0.
        let on_line = ctx_at_xz(0.0, 0.0, 0.0, 0.0, 1000.0, 0.0);
        let past = ctx_at_xz(0.0, 400.0, 0.0, 0.0, 1000.0, 0.0);
        let d_line = channel_delta(&on_line, 60.0, 0.3);
        let d_past = channel_delta(&past, 60.0, 0.3);
        assert!(
            d_line < -50.0,
            "expected deep cut at centerline, got {d_line}"
        );
        assert!(
            d_past.abs() < 1e-3,
            "expected no cut past band, got {d_past}"
        );
    }

    #[test]
    fn v_valley_delta_linear_profile() {
        let on_line = ctx_at_xz(0.0, 0.0, 0.0, 0.0, 1000.0, 0.0);
        let at_half = ctx_at_xz(0.0, 500.0, 0.0, 0.0, 1000.0, 0.5);
        let at_edge = ctx_at_xz(0.0, 999.0, 0.0, 0.0, 1000.0, 0.999);
        assert!(v_valley_delta(&on_line, 50.0) < -40.0);
        // Half distance → half depth (before falloff factor makes it
        // shallower still).
        assert!(v_valley_delta(&at_half, 50.0) > v_valley_delta(&on_line, 50.0));
        assert!(v_valley_delta(&at_edge, 50.0).abs() < 1.0);
    }

    #[test]
    fn u_valley_delta_flat_floor() {
        // Within flat_fraction → uniform max depth (modulo long-axis falloff).
        let near_center = ctx_at_xz(0.0, 100.0, 0.0, 0.0, 1000.0, 0.1);
        let in_flat = ctx_at_xz(0.0, 300.0, 0.0, 0.0, 1000.0, 0.3);
        let d_center = u_valley_delta(&near_center, 50.0, 0.4);
        let d_flat = u_valley_delta(&in_flat, 50.0, 0.4);
        // Both within flat_fraction=0.4 of cell_radius — should be
        // nearly equal (after long-axis falloff).
        assert!(d_center < 0.0);
        assert!(d_flat < 0.0);
    }

    #[test]
    fn ridge_delta_positive() {
        let on_line = ctx_at_xz(0.0, 0.0, 0.0, 0.0, 1000.0, 0.0);
        let d = ridge_delta(&on_line, 80.0);
        assert!(d > 0.0, "ridge should raise terrain");
    }

    #[test]
    fn mound_delta_positive_at_center() {
        let ctx = ctx_at(0.0, 100.0);
        let d = mound_delta(&ctx, 50.0);
        assert!((d - 50.0).abs() < 0.1);
    }

    #[test]
    fn slope_delta_asymmetric() {
        let left = ctx_at_xz(-500.0, 0.0, 0.0, 0.0, 1000.0, 0.5);
        let right = ctx_at_xz(500.0, 0.0, 0.0, 0.0, 1000.0, 0.5);
        let dl = slope_delta(&left, 60.0);
        let dr = slope_delta(&right, 60.0);
        assert!(dr > dl, "right side should be higher than left");
    }

    #[test]
    fn prominence_delta_sharper_than_mound() {
        // Sharpness > 1 should produce a smaller value mid-cell than
        // the equivalent mound.
        let ctx = ctx_at(0.5, 100.0);
        let mound = mound_delta(&ctx, 100.0);
        let prom = prominence_delta(&ctx, 100.0, 2.0);
        assert!(prom < mound, "sharper prominence should be lower at mid-t");
    }

    #[test]
    fn step_delta_sides_differ() {
        let upper = ctx_at_xz(-500.0, 0.0, 0.0, 0.0, 1000.0, 0.5);
        let lower = ctx_at_xz(500.0, 0.0, 0.0, 0.0, 1000.0, 0.5);
        let du = step_delta(&upper, 40.0);
        let dl = step_delta(&lower, 40.0);
        assert!(du > 20.0, "upper side should be raised");
        assert!(dl.abs() < 1e-3, "lower side should be at zero");
    }
}
