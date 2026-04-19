//! Difficulty dials.
//!
//! Phase 1 carries the struct and a few wire-level scalars so the tick signature stabilizes
//! (see `DESIGN.md § Simulation Architecture`). Phase 4 milestone 4.9 populates the full
//! dial set and the in-game adjustment panel.
//!
//! **Invariant:** dials tune *parameters*, never swap physics models. Determinism remains
//! preserved for a given `(seed, input trace, dial state, tick count)`.

/// Gameplay difficulty parameters. Changes take effect at the next tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DifficultyDials {
    /// Multiplier on wind magnitude. Placeholder until Phase 3 weather lands.
    pub wind_magnitude: f32,
    /// Multiplier on gust variance.
    pub gust_variance: f32,
    /// Wind shear intensity scalar.
    pub wind_shear: f32,
    /// Storm frequency / intensity scalar.
    pub storm_intensity: f32,
    /// Hydrodynamics realism scalar.
    pub hydrodynamics_realism: f32,
    /// Stall severity: 0 = gentle g-break, 1 = full wing drop.
    pub stall_severity: f32,
    /// Damage sensitivity multiplier.
    pub damage_sensitivity: f32,
    /// Economy harshness multiplier.
    pub economy_harshness: f32,
}

impl Default for DifficultyDials {
    fn default() -> Self {
        Self {
            wind_magnitude: 1.0,
            gust_variance: 1.0,
            wind_shear: 1.0,
            storm_intensity: 1.0,
            hydrodynamics_realism: 1.0,
            stall_severity: 0.5,
            damage_sensitivity: 1.0,
            economy_harshness: 1.0,
        }
    }
}

impl DifficultyDials {
    /// The "Relaxed" preset: lower wind, gentler stall, softer damage.
    #[must_use]
    pub const fn relaxed() -> Self {
        Self {
            wind_magnitude: 0.5,
            gust_variance: 0.5,
            wind_shear: 0.5,
            storm_intensity: 0.5,
            hydrodynamics_realism: 0.5,
            stall_severity: 0.25,
            damage_sensitivity: 0.5,
            economy_harshness: 0.5,
        }
    }

    /// The "Simulator" preset: full magnitudes.
    #[must_use]
    pub const fn simulator() -> Self {
        Self {
            wind_magnitude: 1.0,
            gust_variance: 1.0,
            wind_shear: 1.0,
            storm_intensity: 1.0,
            hydrodynamics_realism: 1.0,
            stall_severity: 1.0,
            damage_sensitivity: 1.0,
            economy_harshness: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_middle_of_the_road() {
        let d = DifficultyDials::default();
        assert!((d.wind_magnitude - 1.0).abs() < f32::EPSILON);
        assert!((d.stall_severity - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn relaxed_less_harsh_than_simulator() {
        let r = DifficultyDials::relaxed();
        let s = DifficultyDials::simulator();
        assert!(r.wind_magnitude < s.wind_magnitude);
        assert!(r.stall_severity < s.stall_severity);
    }
}
