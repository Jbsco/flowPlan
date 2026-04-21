//! Water surface sampler.
//!
//! Phase 2 Milestone 2.2.3: pure-Rust mirror of the Gerstner wave math in
//! `crates/render/src/water.wgsl`. Wave parameters must stay synchronized between this
//! module and the shader; any tuning happens in both places together (the shader can
//! hot-reload at dev time, so CPU-side constants are the slower-moving half of the
//! contract). Used by Milestone 2.5 pontoon buoyancy and by any future system that
//! needs the water surface shape without reading back from the GPU.
//!
//! Only the vertical (Y) displacement is returned. The full Gerstner model also
//! displaces X/Z (trochoidal motion); for buoyancy that's a small secondary effect that
//! the per-pontoon force model can ignore, using the Y-only height as the immersion
//! depth.

use core::f32::consts::TAU;

/// Wave parameters for one Gerstner component.
#[derive(Clone, Copy, Debug)]
struct Wave {
    /// Direction in the water plane (x, z); normalized on use.
    dir: (f32, f32),
    /// Wavelength in meters (distance between crests).
    wavelength_m: f32,
    /// Peak height above mean water level, meters.
    amplitude_m: f32,
    /// Phase speed, meters per second.
    speed_mps: f32,
}

/// Wave set. Must match the `wave_dir_and_params` / `wave_speed_and_steep` tables in
/// `crates/render/src/water.wgsl`. Steepness is omitted here because it controls only
/// the trochoidal X/Z displacement, not the Y height.
///
/// Tuning goal: gentle protected-water surface that allows float aircraft to taxi /
/// take off without being slammed by half-meter swells, while still showing visible
/// surface motion and texture. Real Beavers operate routinely in chop up to ~0.3 m;
/// the in-game baseline simulates a quiet morning lake (max combined amplitude
/// ~0.14 m, well below typical operational chop). Wind/chop variation will scale
/// this up at runtime once Phase 3.2 weather + 2.5a.7 chop-intensity hook lands.
///
/// Wave hierarchy:
/// - Long swell (40 m wavelength): minimal amplitude — provides slow large-scale
///   rocking but cannot trigger foam alone, eliminating the long swathe pattern
///   that previously dominated foam appearance.
/// - Medium chop (5 m wavelength): primary surface texture and primary foam source.
/// - Fine chop (2.5 m wavelength): granular surface detail.
///
/// Wave hierarchy (7 components): two long swells, three chop, two fine chop. More
/// components break the plaid pattern that 3 waves produce (3 directions = 3 ridge
/// systems intersecting in regular grids); 7 waves with varied directions,
/// wavelengths, and slow speeds create the natural-looking interference patterns
/// of a real water surface.
///
/// Total max amplitude 0.220 m — still well below the 0.45 m original that was
/// disruptive. Swells slowed for more leisurely visual motion (and reduced
/// pontoon impact in light winds).
const WAVES: [Wave; 7] = [
    // Long swells — the rollers
    Wave {
        dir: (1.00, 0.30),
        wavelength_m: 50.0,
        amplitude_m: 0.040,
        speed_mps: 3.5,
    },
    Wave {
        dir: (0.70, 1.00),
        wavelength_m: 35.0,
        amplitude_m: 0.030,
        speed_mps: 2.8,
    },
    // Chop — primary surface texture
    Wave {
        dir: (0.40, 1.00),
        wavelength_m: 6.0,
        amplitude_m: 0.045,
        speed_mps: 2.0,
    },
    Wave {
        dir: (-0.80, 0.50),
        wavelength_m: 4.0,
        amplitude_m: 0.045,
        speed_mps: 1.7,
    },
    Wave {
        dir: (1.00, -0.40),
        wavelength_m: 3.0,
        amplitude_m: 0.040,
        speed_mps: 1.4,
    },
    // Fine chop — high-frequency surface detail
    Wave {
        dir: (-0.30, 1.00),
        wavelength_m: 2.0,
        amplitude_m: 0.035,
        speed_mps: 1.1,
    },
    Wave {
        dir: (0.50, 0.85),
        wavelength_m: 1.4,
        amplitude_m: 0.025,
        speed_mps: 0.85,
    },
];

/// Water-surface Y height (meters) at world-space `(x, z)` at time `t` (seconds).
/// Mean water level is `y = 0`.
#[must_use]
#[allow(clippy::many_single_char_names)]
pub fn water_height(x: f32, z: f32, t: f32) -> f32 {
    let mut y = 0.0_f32;
    for w in &WAVES {
        let (dx, dz) = w.dir;
        let inv_len = dx.hypot(dz).recip();
        let (nx, nz) = (dx * inv_len, dz * inv_len);
        let k = TAU / w.wavelength_m;
        let omega = k * w.speed_mps;
        let phase = k * (nx * x + nz * z) + omega * t;
        y += w.amplitude_m * phase.sin();
    }
    y
}

/// Upper bound on `|water_height|` anywhere. Used for conservative clearance checks.
///
/// Includes the amp-modulation worst case: each wave's effective amplitude can
/// reach `base_amp × (1 + strength)`, so the bound multiplies by the per-class
/// `(1 + strength)` factor (taking the largest, which is the fine-chop bin's
/// strength = 0.65 → factor 1.65).
#[must_use]
pub fn max_amplitude() -> f32 {
    const AMP_MOD_WORST_CASE: f32 = 1.25; // 1 + 0.25 (max strength after retune)
    let mut sum = 0.0_f32;
    let mut i = 0;
    while i < WAVES.len() {
        sum += WAVES[i].amplitude_m;
        i += 1;
    }
    sum * AMP_MOD_WORST_CASE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_at_origin_t0_is_zero() {
        assert!(water_height(0.0, 0.0, 0.0).abs() < 1e-6);
    }

    #[test]
    fn height_bounded_by_max_amplitude() {
        let max = max_amplitude();
        for ix in -32_i32..=32 {
            #[allow(clippy::cast_precision_loss)]
            let x = ix as f32 * 3.0;
            for iz in -32_i32..=32 {
                #[allow(clippy::cast_precision_loss)]
                let z = iz as f32 * 3.0;
                for it in 0_i32..8 {
                    #[allow(clippy::cast_precision_loss)]
                    let t = it as f32 * 0.7;
                    let h = water_height(x, z, t);
                    assert!(h.abs() <= max + 1e-5, "h={h} max={max} at ({x},{z},{t})");
                }
            }
        }
    }

    #[test]
    fn same_input_same_output() {
        let a = water_height(5.0, 3.0, 0.5);
        let b = water_height(5.0, 3.0, 0.5);
        assert!((a - b).abs() < f32::EPSILON);
    }

    #[test]
    fn changes_over_time() {
        let a = water_height(0.0, 0.0, 0.0);
        let b = water_height(0.0, 0.0, 1.0);
        assert!(
            (a - b).abs() > 1e-4,
            "water height should animate; a={a} b={b}"
        );
    }

    #[test]
    fn max_amplitude_matches_sum() {
        // Post amp-modulation strength reduction: sum × 1.25. Sum:
        // 0.040+0.030+0.045+0.045+0.040+0.035+0.025 = 0.260; × 1.25 = 0.325.
        assert!((max_amplitude() - 0.325).abs() < 1e-3);
    }
}
