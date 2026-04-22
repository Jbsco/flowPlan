//! Weather state — Layer W1 of the three-layer weather composition.
//!
//! Ships the **atmospheric base field**: continuous, global, deterministic per
//! master seed. Pressure / temperature / humidity fields, altitude-stratified
//! prevailing-wind field, terrain-induced turbulence map, and (eventually) a
//! pre-baked frontal-boundary graph.
//!
//! # Architecture
//!
//! W1 is Layer 1 of the [Three-layer weather composition](../../../../MAPGEN.md#three-layer-weather-composition-second-application-of-the-pattern)
//! — paralleling terrain L1 (topographic base). Consumed by:
//!
//! - **Sim flight model** (wind on ground track; density altitude from
//!   pressure + temperature; turbulence baseline).
//! - **W2 Weather Voronoi partition** (Phase 3.2.2): cells check W1 placement
//!   constraints (pressure range, humidity threshold, temperature range,
//!   wind-shear threshold) before accepting a cell-type assignment.
//! - **Audio spine** (Phase 3.1+): cell-scale + position state for the
//!   `AudioContext` snapshot.
//!
//! # Determinism
//!
//! Every sampler is a pure function of `(master_seed, world_coord, time)`. No
//! hidden state, no wall-clock reads. Uses `libm` for transcendentals per
//! [`CODING_RULES.md` § Determinism Discipline](../../../../CODING_RULES.md#determinism-discipline).
//! Within-platform bit-identical; cross-platform bit-identity is Phase 6.1.
//!
//! # Scope staging
//!
//! Phase 3.2.1a: climate fields — pressure, temperature, humidity. Each a
//! separable low-frequency field plus biome-style regional modulation.
//!
//! Phase 3.2.1b (this milestone): altitude-stratified wind field (geostrophic
//! balance from pressure gradient), terrain-induced turbulence map, and a
//! slim frontal-boundary graph. Frontal graph is v1 slim — proper connected
//! polylines arrive alongside W2 Voronoi cells that consume them.

#![allow(clippy::module_name_repetitions)]

pub mod frontal;
pub mod humidity;
pub mod pressure;
pub mod temperature;
pub mod turbulence;
pub mod wind;

pub use frontal::{frontal_segments, FrontalSegment};
pub use humidity::humidity_at;
pub use pressure::pressure_at;
pub use temperature::temperature_at;
pub use turbulence::turbulence_at;
pub use wind::wind_at;

/// Master-seed derivation for all weather subsystems.
///
/// Uses `hash(master_seed, "weather")` as the weather-subsystem root. Each
/// field (pressure, temperature, humidity, wind, fronts, turbulence) derives
/// its own seed from this root via `hash(weather_seed, "<field>")`.
#[must_use]
pub fn weather_seed(master_seed: u64) -> u64 {
    // SipHash-style mix with a fixed weather constant. Seed derivation needs
    // to be deterministic + platform-independent. Using bit-mix from master +
    // fixed u64 derived from "weather" ASCII (cheap + stable across builds).
    const WEATHER_TAG: u64 = u64::from_le_bytes(*b"weather_");
    mix64(master_seed, WEATHER_TAG)
}

/// Per-field seed derivation from the weather-subsystem root.
#[must_use]
fn field_seed(root: u64, field_tag: u64) -> u64 {
    mix64(root, field_tag)
}

/// Deterministic 64-bit mix (splitmix64-inspired). Platform-independent.
#[must_use]
fn mix64(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_seed_deterministic() {
        assert_eq!(weather_seed(42), weather_seed(42));
        assert_eq!(weather_seed(0), weather_seed(0));
    }

    #[test]
    fn weather_seed_differs_by_master() {
        assert_ne!(weather_seed(42), weather_seed(43));
        assert_ne!(weather_seed(0), weather_seed(1));
    }

    #[test]
    fn field_seed_differs_by_tag() {
        let root = weather_seed(42);
        let s1 = field_seed(root, 1);
        let s2 = field_seed(root, 2);
        assert_ne!(s1, s2);
    }
}
