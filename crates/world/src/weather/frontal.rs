//! Frontal-boundary graph (slim v1).
//!
//! Pre-baked line-segment approximation of the pressure-gradient-maxima graph.
//! Analog of the L1 hydrology river graph: global structure, immutable across
//! W2/W3. Consumed by [Phase 3.2.2](../../../../ROADMAP.md#milestone-322-layer-w2--weather-voronoi-partition)
//! to bias weather-Voronoi cells along fronts toward frontal-passage types.
//!
//! # V1 scope (slim)
//!
//! - Discretize a world-space bounding box into cells of size
//!   [`DEFAULT_SAMPLE_SPACING_M`].
//! - At each cell center, sample `|∇p|` on the sea-level pressure field.
//! - Cells whose gradient exceeds [`FRONTAL_THRESHOLD_HPA_PER_M`] emit one line
//!   segment perpendicular to the gradient direction, spanning the cell.
//! - Return a [`Vec<FrontalSegment>`] — the caller iterates.
//!
//! This is a *line-structure sketch*, not a connected graph: adjacent frontal
//! cells emit parallel segments that visually form a front but aren't linked
//! by shared endpoints. A proper graph (connected polylines, branch points)
//! arrives with W2 once there's a downstream consumer whose requirements can
//! pin the shape precisely.
//!
//! Computation cost: `O(N²)` in samples-per-axis. A 500 km × 500 km world at
//! 20 km spacing = 25² = 625 samples; each is 4 pressure evaluations
//! (finite-difference) — 2500 ops per bake. Cheap.

use glam::Vec2;

use super::pressure;

/// Default discretization spacing (m) for the frontal-cell grid.
///
/// 20 km matches the expected W2 weather-cell spacing so the segment density
/// naturally aligns with cell density.
pub const DEFAULT_SAMPLE_SPACING_M: f32 = 20_000.0;

/// Gradient threshold above which a cell is tagged frontal (hPa/m).
///
/// Typical synoptic pressure gradients are 0.0001–0.001 hPa/m. Strong fronts
/// are 2–3× stronger. Threshold picks the upper quartile so a few percent of
/// cells across a mid-latitude grid get tagged — matches the observed density
/// of fronts on a real weather map.
pub const FRONTAL_THRESHOLD_HPA_PER_M: f32 = 0.0002;

/// Finite-difference step for pressure-gradient sampling (m).
const FD_STEP_M: f32 = 2_000.0;

/// A single frontal-boundary line segment, in world XZ coordinates.
///
/// `start` and `end` are the two endpoints; `gradient_magnitude` records the
/// pressure-gradient strength at the segment's origin cell for downstream
/// consumers that want to weight by front intensity.
#[derive(Copy, Clone, Debug)]
pub struct FrontalSegment {
    pub start: Vec2,
    pub end: Vec2,
    pub gradient_magnitude: f32,
}

/// Emit the frontal segments intersecting the AABB
/// `[-half_extent, +half_extent]² + center` at `time_s`.
///
/// Deterministic per `(master_seed, center, half_extent, spacing, time_s)`.
#[must_use]
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_precision_loss)]
pub fn frontal_segments(
    master_seed: u64,
    center_x_m: f32,
    center_z_m: f32,
    half_extent_m: f32,
    spacing_m: f32,
    time_s: f32,
) -> Vec<FrontalSegment> {
    let steps_per_axis = ((2.0 * half_extent_m) / spacing_m).round() as i32;
    let steps = steps_per_axis.max(1);
    let mut out = Vec::new();

    for ix in 0..steps {
        for iz in 0..steps {
            let x = center_x_m - half_extent_m + (ix as f32 + 0.5) * spacing_m;
            let z = center_z_m - half_extent_m + (iz as f32 + 0.5) * spacing_m;

            let (dp_dx, dp_dz) = pressure_gradient(master_seed, x, z, time_s);
            let grad_mag = libm::sqrtf(dp_dx * dp_dx + dp_dz * dp_dz);

            if grad_mag >= FRONTAL_THRESHOLD_HPA_PER_M {
                // Segment direction is perpendicular to gradient (rotate 90°).
                let inv_mag = 1.0 / grad_mag;
                let dir_x = -dp_dz * inv_mag;
                let dir_z = dp_dx * inv_mag;
                let half = 0.5 * spacing_m;
                out.push(FrontalSegment {
                    start: Vec2::new(x - dir_x * half, z - dir_z * half),
                    end: Vec2::new(x + dir_x * half, z + dir_z * half),
                    gradient_magnitude: grad_mag,
                });
            }
        }
    }

    out
}

fn pressure_gradient(master_seed: u64, x_m: f32, z_m: f32, time_s: f32) -> (f32, f32) {
    let p_xp = pressure::sea_level_pressure(master_seed, x_m + FD_STEP_M, z_m, time_s);
    let p_xm = pressure::sea_level_pressure(master_seed, x_m - FD_STEP_M, z_m, time_s);
    let p_zp = pressure::sea_level_pressure(master_seed, x_m, z_m + FD_STEP_M, time_s);
    let p_zm = pressure::sea_level_pressure(master_seed, x_m, z_m - FD_STEP_M, time_s);
    let dp_dx = (p_xp - p_xm) / (2.0 * FD_STEP_M);
    let dp_dz = (p_zp - p_zm) / (2.0 * FD_STEP_M);
    (dp_dx, dp_dz)
}

/// Unit-vector tangent of the local frontal band at world `(x, z)`.
///
/// Perpendicular to the pressure gradient → aligned with the front line.
/// Returns `Vec2::X` (default `(1, 0)`) for points where the gradient is
/// weak (no meaningful frontal direction). Used by Phase C banded-cloud
/// rendering: for `Frontal` weather cells, the cloud's noise sampling
/// stretches along this tangent so cloud features form streaks along
/// the front instead of isotropic puffs.
///
/// Cost: 4 pressure evaluations (same as gradient sampling). Called
/// once per Frontal cell at chunk-bake time, not per-frame or per-pixel.
#[must_use]
pub fn band_tangent_at(master_seed: u64, x_m: f32, z_m: f32, time_s: f32) -> Vec2 {
    let (dp_dx, dp_dz) = pressure_gradient(master_seed, x_m, z_m, time_s);
    let mag_sq = dp_dx * dp_dx + dp_dz * dp_dz;
    let weak_threshold = FRONTAL_THRESHOLD_HPA_PER_M * 0.5;
    if mag_sq < weak_threshold * weak_threshold {
        return Vec2::X;
    }
    let inv_mag = 1.0 / libm::sqrtf(mag_sq);
    Vec2::new(-dp_dz * inv_mag, dp_dx * inv_mag)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn frontal_segments_deterministic() {
        let a = frontal_segments(42, 0.0, 0.0, 250_000.0, 20_000.0, 0.0);
        let b = frontal_segments(42, 0.0, 0.0, 250_000.0, 20_000.0, 0.0);
        assert_eq!(a.len(), b.len());
        for (sa, sb) in a.iter().zip(b.iter()) {
            assert_eq!(sa.start.x.to_bits(), sb.start.x.to_bits());
            assert_eq!(sa.start.y.to_bits(), sb.start.y.to_bits());
            assert_eq!(sa.end.x.to_bits(), sb.end.x.to_bits());
            assert_eq!(sa.end.y.to_bits(), sb.end.y.to_bits());
        }
    }

    #[test]
    fn different_seeds_produce_different_graphs() {
        let g1 = frontal_segments(1, 0.0, 0.0, 250_000.0, 20_000.0, 0.0);
        let g2 = frontal_segments(2, 0.0, 0.0, 250_000.0, 20_000.0, 0.0);
        // Segment lists should differ — either length or positions.
        let diff = g1.len() != g2.len()
            || g1
                .iter()
                .zip(g2.iter())
                .any(|(a, b)| a.start != b.start || a.end != b.end);
        assert!(diff, "seeds 1 vs 2 produced identical frontal graphs");
    }

    #[test]
    fn gradient_magnitude_exceeds_threshold() {
        let segs = frontal_segments(42, 0.0, 0.0, 250_000.0, 20_000.0, 0.0);
        for s in &segs {
            assert!(
                s.gradient_magnitude >= FRONTAL_THRESHOLD_HPA_PER_M,
                "segment gradient {} below threshold",
                s.gradient_magnitude
            );
        }
    }

    #[test]
    fn segments_within_bounding_box() {
        let half = 250_000.0;
        let segs = frontal_segments(42, 0.0, 0.0, half, 20_000.0, 0.0);
        // Endpoints can extend ±half-spacing past the box (each segment is
        // centered on a cell and extends half the cell each way). Allow
        // spacing/2 margin.
        let margin = half + 20_000.0 / 2.0 + 1.0;
        for s in &segs {
            assert!(s.start.x.abs() <= margin);
            assert!(s.start.y.abs() <= margin);
            assert!(s.end.x.abs() <= margin);
            assert!(s.end.y.abs() <= margin);
        }
    }

    #[test]
    fn synoptic_scale_produces_some_segments() {
        // At 500 km extent + 20 km spacing, we expect *some* gradient cells
        // to exceed the threshold. Exact count is seed-dependent.
        let segs = frontal_segments(42, 0.0, 0.0, 250_000.0, 20_000.0, 0.0);
        assert!(
            !segs.is_empty(),
            "expected at least one frontal segment at synoptic scale"
        );
    }

    #[test]
    fn empty_box_produces_no_segments() {
        let segs = frontal_segments(42, 0.0, 0.0, 0.0, 20_000.0, 0.0);
        assert!(segs.is_empty());
    }
}
