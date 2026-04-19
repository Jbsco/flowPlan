//! Per-aircraft spec sheet.
//!
//! Phase 2 Milestone 2.3 sub-step 3: authored as RON data under `data/aircraft/*.ron`
//! and deserialized at startup. No flight-model integration yet -- that arrives in
//! Milestone 2.4 with the Beaver-tuned airfoil model. Fields match the AIRCRAFT.md
//! MVP Reference entry for the DHC-2 Beaver analogue ("Northlander"), normalized to
//! SI where practical; imperial units from the POH are converted on authoring.

use serde::{Deserialize, Serialize};

/// Top-level aircraft specification.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AircraftSpec {
    /// In-game aircraft name (sanitized; never a trademark).
    pub name: String,
    /// Real-world inspiration, for documentation only.
    pub inspiration: String,
    /// Empty weight, kilograms.
    pub empty_mass_kg: f32,
    /// Max take-off weight on wheels, kilograms.
    pub max_mass_wheels_kg: f32,
    /// Max take-off weight on floats, kilograms.
    pub max_mass_floats_kg: f32,
    /// Reference wing area, square meters.
    pub wing_area_m2: f32,
    /// Aspect ratio (wingspan^2 / wing area).
    pub aspect_ratio: f32,
    /// Airfoil section designation at the wing root.
    pub airfoil_root: String,
    /// Airfoil section designation at the wing tip.
    pub airfoil_tip: String,
    /// Engine characteristics.
    pub engine: EngineSpec,
    /// Total usable fuel, US gallons (kept in the authoring unit because POH figures are).
    pub fuel_capacity_gal: f32,
    /// Characteristic speeds, meters per second.
    pub speeds: SpeedsSpec,
}

/// Engine subset of an [`AircraftSpec`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EngineSpec {
    /// Engine name (e.g., "Pratt & Whitney R-985 Wasp Junior").
    pub name: String,
    /// Rated power at sea level, horsepower (kept as POH unit for recognizability).
    pub power_hp: f32,
    /// Typical cruise fuel burn, US gallons per hour.
    pub cruise_fuel_gph: f32,
}

/// V-speed set in SI units (meters per second).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SpeedsSpec {
    /// Never-exceed airspeed.
    pub vne_mps: f32,
    /// Published cruise airspeed.
    pub cruise_mps: f32,
    /// Stall speed, clean (flaps up).
    pub vs_clean_mps: f32,
    /// Stall speed with flaps deployed / on floats (Vso).
    pub vso_mps: f32,
    /// Best-angle climb speed.
    pub vx_mps: f32,
    /// Best-rate climb speed.
    pub vy_mps: f32,
}

impl AircraftSpec {
    /// Parse from a RON source string.
    pub fn from_ron(source: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(source)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    const NORTHLANDER_MIN: &str = r#"(
        name: "Northlander",
        inspiration: "DHC-2 Beaver",
        empty_mass_kg: 1315.5,
        max_mass_wheels_kg: 2313.3,
        max_mass_floats_kg: 2435.8,
        wing_area_m2: 23.23,
        aspect_ratio: 9.2,
        airfoil_root: "NACA 64A416",
        airfoil_tip: "NACA 64A412",
        engine: (name: "R-985", power_hp: 450.0, cruise_fuel_gph: 27.0),
        fuel_capacity_gal: 95.0,
        speeds: (
            vne_mps: 71.53,
            cruise_mps: 64.31,
            vs_clean_mps: 23.25,
            vso_mps: 21.01,
            vx_mps: 29.06,
            vy_mps: 37.99,
        ),
    )"#;

    #[test]
    fn parses_northlander_minimal() {
        let spec = AircraftSpec::from_ron(NORTHLANDER_MIN).expect("parse ron");
        assert_eq!(spec.name, "Northlander");
        assert!((spec.engine.power_hp - 450.0).abs() < f32::EPSILON);
        assert!((spec.speeds.cruise_mps - 64.31).abs() < 1e-3);
    }

    #[test]
    fn rejects_malformed() {
        assert!(AircraftSpec::from_ron("not ron").is_err());
    }
}
