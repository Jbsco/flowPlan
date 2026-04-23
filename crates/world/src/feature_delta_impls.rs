//! Feature-delta implementations (Phase 3.6.3a).
//!
//! Pure functions returning the L3 heightfield correction for each
//! feature type. Each delta uses the shared [`DeltaContext`] + falloff
//! helpers from [`crate::feature_delta`].
//!
//! ## Delta pattern categories
//!
//! | Category | Shape | Features |
//! |---|---|---|
//! | Bowl | Negative gaussian-ish depression | lake_cluster, alpine_tarn, cirque, protected_cove, glacier_dammed_lake, dry_lake_bed |
//! | Plateau | Positive flattop + cliff edge | mesa_top, book_cliffs |
//! | Cliff | Sharp altitude drop | cliff_coast |
//! | Ramp | Smooth shore ramp | sandy_beach |
//! | Dune | High-freq positive noise | dune_field |
//! | Flatten | Local bias toward cell mean | bush_strip_clearing |
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
        "cirque" => asymmetric_bowl_delta(ctx, 70.0),

        // ── Plateaus / cliffs ───────────────────────────────────────
        "mesa_top" => plateau_delta(ctx, 140.0, 0.75),
        "book_cliffs" => plateau_delta(ctx, 380.0, 0.55),
        "cliff_coast" => cliff_coast_delta(ctx, 90.0),

        // ── Shore ramp ──────────────────────────────────────────────
        "sandy_beach" => shore_ramp_delta(ctx, -8.0),

        // ── Dune field ──────────────────────────────────────────────
        "dune_field" => dune_delta(ctx, 22.0),

        // ── Local flatten ───────────────────────────────────────────
        "bush_strip_clearing" => flatten_delta(ctx, 0.75),

        // All other feature ids (including `plain`, oceanic types,
        // scenery tags, developments/infra/resupply from 3.6.3b)
        // produce no heightfield delta at this phase.
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
}
