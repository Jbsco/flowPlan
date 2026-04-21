//! Vehicle configuration enum.
//!
//! `VehicleConfig` selects the contact-physics model applied to the aircraft: a
//! floatplane uses `buoyancy::compute_buoyancy`; a wheeled / taildragger uses
//! `ground_contact::compute_ground_contact`. The two are mutually exclusive — a
//! single airframe is flown in one configuration at a time (swapping floats for
//! wheels is a ground-crew operation, not a mid-flight toggle).
//!
//! Default is `Floats` so existing Phase 2.5a behavior — and every bench scenario
//! authored before 2.5b — keeps working unchanged. App-side selection lands in
//! Milestone 2.5b.7 (runtime toggle + mesh swap); until then, wheeled physics is
//! exercised exclusively via tests / benches that explicitly set this field.
//!
//! Folded into `state_hash` so determinism covers config changes across the trace
//! (a scenario that flips mid-run produces a distinct hash from one that doesn't —
//! important once Phase 3+ world-swap scenarios allow config transitions).

use serde::{Deserialize, Serialize};

/// Selects which contact-physics model applies to the aircraft this tick.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VehicleConfig {
    /// Float configuration: `buoyancy::compute_buoyancy` is active, ground contact
    /// is not applied. Matches the `northlander.glb` mesh and the 2.5a-shipped
    /// buoyancy model with the Edo 4930 pontoon geometry.
    #[default]
    Floats,
    /// Wheeled / taildragger configuration: `ground_contact::compute_ground_contact`
    /// is active, buoyancy is not applied. Matches the `northlander_land.glb` mesh
    /// (landed in 2.5b prep) and the Beaver wheels MTOW `max_mass_wheels_kg`.
    Wheels,
}

impl VehicleConfig {
    /// True when buoyancy should be applied this tick.
    #[must_use]
    pub const fn uses_buoyancy(self) -> bool {
        matches!(self, Self::Floats)
    }

    /// True when ground contact (wheels / tailwheel) should be applied this tick.
    #[must_use]
    pub const fn uses_ground_contact(self) -> bool {
        matches!(self, Self::Wheels)
    }

    /// Stable u32 representation for `state_hash`. Never reorder — existing hashes
    /// would drift. New variants append at the next unused value.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Floats => 0,
            Self::Wheels => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_floats() {
        assert_eq!(VehicleConfig::default(), VehicleConfig::Floats);
    }

    #[test]
    fn floats_uses_buoyancy_only() {
        assert!(VehicleConfig::Floats.uses_buoyancy());
        assert!(!VehicleConfig::Floats.uses_ground_contact());
    }

    #[test]
    fn wheels_uses_ground_contact_only() {
        assert!(!VehicleConfig::Wheels.uses_buoyancy());
        assert!(VehicleConfig::Wheels.uses_ground_contact());
    }

    #[test]
    fn hash_discriminants_are_stable() {
        assert_eq!(VehicleConfig::Floats.as_u32(), 0);
        assert_eq!(VehicleConfig::Wheels.as_u32(), 1);
    }
}
