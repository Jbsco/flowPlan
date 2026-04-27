//! Unit tests for the cloud-pass math primitives (CM-4 linear remap,
//! CM-6 boundary-blend linearization, CM-10 cel-banding low-end
//! softening). Pure Rust; no GPU required.
//!
//! Each Rust formula MIRRORS the WGSL implementation in
//! `crates/render/src/volumetric.wgsl`. The risk is drift: WGSL changes
//! without a corresponding Rust mirror update would slip past these
//! tests. To keep WGSL → Rust in sync, we also verify the WGSL source
//! contains the canonical formula strings (a parse-test cross-check).
//!
//! These complement the hardware-gated tests:
//! - `cloud_noise_bit_identity.rs`: CPU↔GPU agreement on noise primitives.
//! - `cloud_pass_perf.rs`: end-to-end perf gate on a minimal scene.
//! - `visual_snapshot.rs`: byte-exact PNG comparison (golden image).
//! - `volumetric_wgsl_parses.rs`: WGSL parses + symbol presence.

#![allow(
    clippy::float_cmp,
    clippy::doc_markdown,
    clippy::cast_precision_loss,
    dead_code,
    unused_imports
)]

// Dev-dep markers — render's test target inherits dev-deps it doesn't
// use here but other test files do.
use bytemuck as _;
use flowplan_assets as _;
use flowplan_sim as _;
use glam as _;
use image as _;
use insta as _;
use naga as _;
#[cfg(feature = "shader-hot-reload")]
use notify as _;
use pollster as _;
use wgpu as _;

use flowplan_render::VOLUMETRIC_WGSL_SOURCE;

/// CM-4 linear-remap formula:
/// `cloud_factor = clamp((raw_factor - (1 - cov)) / cov, 0, 1)`.
fn cm4_linear_remap(raw_factor: f32, coverage_strength: f32) -> f32 {
    let coverage_threshold = 1.0 - coverage_strength;
    ((raw_factor - coverage_threshold) / coverage_strength).clamp(0.0, 1.0)
}

/// CM-6 Option-2 one-sqrt linearization for boundary distance:
/// `half_diff ≈ (dsq2 - dsq1) / (4 * sqrt(max(dsq1, 1.0)))`.
fn cm6_half_diff_linearized(dsq1: f32, dsq2: f32) -> f32 {
    let d1 = dsq1.max(1.0).sqrt();
    (dsq2 - dsq1) / (4.0 * d1)
}

/// CM-6 blend weights: `w1 = 0.5 + 0.5 * smoothstep(0, blend_dist, half_diff)`.
fn cm6_blend_w1(half_diff: f32, blend_distance_m: f32) -> f32 {
    let t = (half_diff / blend_distance_m).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    0.5 + 0.5 * smooth
}

/// CM-10 cel-banding with low-end smoothstep softening:
/// `final = max(floor(α * 6) / 6, smoothstep(0, 1/6, α) * 1/6)`.
fn cm10_cel_band(alpha: f32) -> f32 {
    let banded = (alpha * 6.0).floor() / 6.0;
    let t = (alpha / (1.0 / 6.0)).clamp(0.0, 1.0);
    let smooth_low = (t * t * (3.0 - 2.0 * t)) * (1.0 / 6.0);
    banded.max(smooth_low)
}

// ──────────────────────────────────────────────────────────────────
// CM-4 linear-remap tests
// ──────────────────────────────────────────────────────────────────

#[test]
fn cm4_identity_at_full_coverage() {
    // At cov=1.0, the linear remap reduces to raw_factor (identity).
    // This preserves the soft Beer-Lambert + cel-shaded rim that the
    // original smoothstep formulation killed (in-game observation
    // 2026-04-26).
    for raw in [0.0, 0.05, 0.25, 0.5, 0.75, 0.95, 1.0] {
        let cf = cm4_linear_remap(raw, 1.0);
        assert!(
            (cf - raw).abs() < 1e-6,
            "cov=1.0 should be identity: raw={raw} → cloud_factor={cf}"
        );
    }
}

#[test]
fn cm4_gates_below_threshold() {
    // At cov=0.5, threshold=0.5. Noise values below threshold should
    // produce zero cloud (gating). This is the "marginal cells get
    // gaps" behavior.
    let cov = 0.5;
    for raw in [0.0, 0.1, 0.3, 0.49] {
        let cf = cm4_linear_remap(raw, cov);
        assert!(
            cf < 1e-6,
            "raw={raw} below threshold should gate to 0, got {cf}"
        );
    }
}

#[test]
fn cm4_above_threshold_linear() {
    // Above threshold, linear remap. raw=midway gives 0.5; raw=1.0 gives 1.0.
    let cov = 0.5;
    let cf_mid = cm4_linear_remap(0.75, cov); // halfway above threshold
    assert!(
        (cf_mid - 0.5).abs() < 1e-6,
        "midway above threshold should give 0.5: got {cf_mid}"
    );
    let cf_top = cm4_linear_remap(1.0, cov);
    assert!(
        (cf_top - 1.0).abs() < 1e-6,
        "raw=1.0 should saturate to 1: got {cf_top}"
    );
}

#[test]
fn cm4_marginal_cell_top_only() {
    // At cov=0.1, only the top 10% of noise survives.
    let cov = 0.1;
    let cf_lo = cm4_linear_remap(0.85, cov); // below threshold (0.9)
    assert!(
        cf_lo < 1e-6,
        "raw=0.85 with cov=0.1 (threshold=0.9) should gate: got {cf_lo}"
    );
    let cf_hi = cm4_linear_remap(0.95, cov); // halfway above
    assert!(
        (cf_hi - 0.5).abs() < 1e-3,
        "raw=0.95 (halfway in upper 10%) should give ~0.5: got {cf_hi}"
    );
}

// ──────────────────────────────────────────────────────────────────
// CM-6 boundary-blend tests
// ──────────────────────────────────────────────────────────────────

#[test]
fn cm6_blend_at_voronoi_midline_is_half() {
    // At the Voronoi midline d1 == d2 (equidistant from both cells).
    // half_diff = 0; w1 = 0.5 (50/50 blend).
    let dsq = 1_000_000.0_f32; // 1 km²; both cells at 1 km
    let half_diff = cm6_half_diff_linearized(dsq, dsq);
    assert!(
        half_diff.abs() < 1e-3,
        "midline half_diff should be ~0, got {half_diff}"
    );
    let w1 = cm6_blend_w1(half_diff, 5000.0);
    assert!(
        (w1 - 0.5).abs() < 1e-6,
        "midline w1 should be 0.5, got {w1}"
    );
}

#[test]
fn cm6_blend_deep_in_cell1_is_one() {
    // Deep inside cell 1: d1 << d2. half_diff > BLEND_DIST → smoothstep
    // saturates → w1 = 1.0 (pure cell 1).
    let d1 = 100.0_f32;
    let d2 = 25_000.0_f32; // 25 km away
    let half_diff = cm6_half_diff_linearized(d1 * d1, d2 * d2);
    assert!(
        half_diff > 5000.0,
        "should be way past blend distance: {half_diff}"
    );
    let w1 = cm6_blend_w1(half_diff, 5000.0);
    assert!(
        (w1 - 1.0).abs() < 1e-6,
        "deep cell 1 w1 should be 1.0, got {w1}"
    );
}

#[test]
fn cm6_one_sqrt_linearization_accurate_at_boundary() {
    // The (dsq2 - dsq1) / (4 * d1) formula is exact when d1 == d2.
    // Off-boundary, the approximation degrades. Here we verify it's
    // within 10% of the true (d2 - d1)/2 for points near the midline.
    let d1 = 12_400.0_f32; // 12.4 km
    let d2 = 12_600.0_f32; // 12.6 km (200 m offset)
    let true_half_diff = (d2 - d1) * 0.5;
    let approx_half_diff = cm6_half_diff_linearized(d1 * d1, d2 * d2);
    let rel_err = ((approx_half_diff - true_half_diff) / true_half_diff).abs();
    assert!(
        rel_err < 0.05,
        "near-boundary linearization should be within 5%: true={true_half_diff}, approx={approx_half_diff}, err={rel_err}"
    );
}

#[test]
fn cm6_blend_w1_smooth_no_discontinuity() {
    // Sweep half_diff across the smoothstep band; verify no discrete jumps.
    // (Regression guard against the BOUNDARY_BLEND_W2_EARLY_EXIT bug
    // where w2 < 0.05 was suddenly skipped — produced visible V-cuts.)
    let mut prev_w1 = 0.5;
    for n in 0..=100 {
        let half_diff = (n as f32) * 50.0; // 0..5000m
        let w1 = cm6_blend_w1(half_diff, 5000.0);
        assert!(
            (w1 - prev_w1).abs() < 0.05,
            "discontinuity at half_diff={half_diff}: prev={prev_w1}, w1={w1}"
        );
        prev_w1 = w1;
    }
}

// ──────────────────────────────────────────────────────────────────
// CM-10 cel-banding tests
// ──────────────────────────────────────────────────────────────────

#[test]
fn cm10_cel_band_zero_at_alpha_zero() {
    let alpha = cm10_cel_band(0.0);
    assert!(alpha.abs() < 1e-6, "α=0 should map to 0, got {alpha}");
}

#[test]
fn cm10_cel_band_continuous_low_end() {
    // The pre-CM-10 formula had a hard cutoff: α < 1/6 → 0, α ≥ 1/6 → 1/6.
    // The fix is a smoothstep ramp from 0 to 1/6 across α ∈ [0, 1/6].
    // Verify the curve is continuous (no big jumps) across this range.
    let mut prev = 0.0_f32;
    for n in 0..=20 {
        let alpha = (n as f32) * (1.0 / 6.0) / 20.0; // 0..1/6
        let banded = cm10_cel_band(alpha);
        assert!(
            (banded - prev).abs() < 0.05,
            "discontinuity in low end at α={alpha}: prev={prev}, banded={banded}"
        );
        prev = banded;
    }
}

#[test]
fn cm10_cel_band_quantized_above_lowest() {
    // Above α = 1/6, the floor-banding takes over. Verify discrete
    // band values: 2/6, 3/6, 4/6, 5/6.
    for (input_band, expected) in [
        (2_i32, 2.0_f32 / 6.0),
        (3, 3.0 / 6.0),
        (4, 4.0 / 6.0),
        (5, 5.0 / 6.0),
    ] {
        // Sample slightly above the band boundary to ensure the floor
        // moves us into that band (not the edge case below).
        #[allow(clippy::cast_precision_loss)]
        let alpha = (input_band as f32 / 6.0) + 1e-3;
        let banded = cm10_cel_band(alpha);
        assert!(
            (banded - expected).abs() < 1e-3,
            "α={alpha} should give band {expected}, got {banded}"
        );
    }
}

#[test]
fn cm10_cel_band_low_end_smoother_than_floor() {
    // At α = 1/12 (midway through lowest band), the smoothstep ramp
    // should give a value > 0 (was 0 with the hard cutoff).
    let alpha = 1.0 / 12.0;
    let banded = cm10_cel_band(alpha);
    assert!(
        banded > 0.0 && banded < (1.0 / 6.0),
        "low-end softening should give 0 < banded < 1/6 at α=1/12, got {banded}"
    );
}

// ──────────────────────────────────────────────────────────────────
// WGSL ↔ Rust formula cross-check
// ──────────────────────────────────────────────────────────────────

/// Verify the WGSL source contains the canonical CM-4 linear-remap
/// formula. Drift guard — if WGSL changes the formula without
/// updating these tests, the Rust mirror would start lying. This
/// at least catches whole-formula renames.
#[test]
fn wgsl_contains_cm4_linear_remap_formula() {
    assert!(
        VOLUMETRIC_WGSL_SOURCE
            .contains("(raw_factor - coverage_threshold) / cell.coverage_strength"),
        "volumetric.wgsl missing CM-4 linear-remap formula"
    );
}

#[test]
fn wgsl_contains_cm6_one_sqrt_linearization() {
    // Look for the algebraic identity expression. Robust to formatting.
    let normalized: String = VOLUMETRIC_WGSL_SOURCE
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        normalized.contains("(nearest.dsq2-nearest.dsq1)/(4.0*d1)"),
        "volumetric.wgsl missing CM-6 one-sqrt linearization formula"
    );
}

#[test]
fn wgsl_contains_cm10_cel_band_low_smooth() {
    // Look for the new low-end smoothstep softening (regression guard
    // against accidental revert to hard cutoff).
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("smoothstep(0.0, 1.0 / 6.0, accumulated_alpha)"),
        "volumetric.wgsl missing CM-10 cel-band low-end smoothstep softening"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("max(banded_alpha, low_smooth)"),
        "volumetric.wgsl missing CM-10 cel-band max(banded, low_smooth) combiner"
    );
}

// ──────────────────────────────────────────────────────────────────
// CM-9 per-type Worley spacing presence tests
// ──────────────────────────────────────────────────────────────────

#[test]
fn wgsl_worley_spacing_per_type_values_present() {
    // The pinned per-type Worley spacings must be in the WGSL source.
    // (Regression guard — accidental tuning that obliterates per-type
    // differentiation should fail the test.)
    for (label, spacing) in [
        ("Cumulus", "800.0"),
        ("Nimbus", "600.0"),
        ("Stratus", "2000.0"),
        ("Squall", "500.0"),
        ("Frontal", "1000.0"),
        ("ClearAir/haze default", "1500.0"),
    ] {
        assert!(
            VOLUMETRIC_WGSL_SOURCE.contains(spacing),
            "volumetric.wgsl missing CM-9 per-type Worley spacing {spacing} ({label})"
        );
    }
}

#[test]
fn wgsl_clear_air_margin_cap_value() {
    // CM-10 raised CLEAR_AIR_MARGIN_CAP from 0.15 → 0.25 to make
    // transition haze visible. Anchor the value in WGSL-compatible
    // form (the constant lives in cell_state.rs but this test guards
    // the Rust-side value via flowplan_world).
    assert_eq!(
        flowplan_world::weather::cell_state::CLEAR_AIR_MARGIN_CAP,
        0.25_f32,
        "CLEAR_AIR_MARGIN_CAP regression — should be 0.25 post-CM-10 tuning"
    );
}

#[test]
fn wgsl_uses_mixed_spacing_worley() {
    // CM-10 revision: replaced per-cell Worley with same-type cache
    // reuse (CM-9) with a single Worley call at mixed spacing.
    // Eliminates the multi-texture interference at boundaries that
    // produced visible seams (`blend_line.png`). Regression guard
    // against accidental revert to per-cell Worley calls.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("mix(spacing1, spacing2, w2)"),
        "volumetric.wgsl missing CM-10 mixed-spacing Worley formula"
    );
    // The shader should NOT contain two distinct worley_3d calls in
    // sample_cloud_density (other than reuse via worley_struct1 = ...
    // which we removed). Check for the pattern of two separate calls
    // with separate result vars.
    let normalized: String = VOLUMETRIC_WGSL_SOURCE
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        !normalized.contains("worley_struct1=worley_struct1"),
        "volumetric.wgsl: same-type cache-reuse pattern should be gone (CM-10 mixed-spacing)"
    );
}

#[test]
fn wgsl_contains_cm10_soft_altitude_profile() {
    // CM-10 fix: replaced parabolic `1 - (2t-1)²` (linear slope=4 at
    // edges → block shape) with smoothstep×smoothstep bell (zero
    // derivative at edges). Regression guard.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("smoothstep(0.0, 0.5, t)")
            && VOLUMETRIC_WGSL_SOURCE.contains("smoothstep(1.0, 0.5, t)"),
        "volumetric.wgsl missing CM-10 soft altitude profile (smoothstep×smoothstep bell)"
    );
}

#[test]
fn wgsl_contains_cm10_domain_warp_formula() {
    // CM-10 domain warping: pre-perturb (x, z) before density-noise
    // lookup to break Worley grid alignment (in-game observation:
    // east-west bands aligned with X axis of Worley grid). Regression
    // guard against accidental revert to direct world_pos passthrough.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("perturb * NOISE_WARP_AMPLITUDE_M"),
        "volumetric.wgsl missing CM-10 domain-warp X formula"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("warp_z_perlin * NOISE_WARP_AMPLITUDE_M"),
        "volumetric.wgsl missing CM-10 domain-warp Z formula"
    );
    // Check that warped coords are passed to BOTH Worley and Perlin.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("warped_x, y_stretched, warped_z"),
        "volumetric.wgsl: density-noise lookups should use warped coords (CM-10)"
    );
}
