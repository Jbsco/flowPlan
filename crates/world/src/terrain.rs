//! Procedural heightmap.
//!
//! Phase 1 terrain is the sum of four sinusoids: two coarse, two fine, with seeded phases.
//! Good enough to fly over with visible hills and valleys; deterministic for a given seed.
//! Phase 2 replaces this with a real biome-aware heightmap backed by a 16-bit PNG.

use flowplan_core::{splitmix64, Rng};

/// A sampleable height function.
pub trait HeightSampler: Send + Sync {
    /// World-space height at `(x, z)`.
    fn height(&self, x: f32, z: f32) -> f32;
}

/// Procedural heightmap from a seed. See module docs.
#[derive(Clone, Debug)]
pub struct Terrain {
    seed: u64,
    octaves: [Octave; 5],
    base_amplitude: f32,
}

#[derive(Clone, Copy, Debug)]
struct Octave {
    frequency: f32,
    amplitude: f32,
    phase_x: f32,
    phase_z: f32,
}

impl Terrain {
    /// Build a seeded procedural terrain.
    ///
    /// Five octaves. The coarsest (octave 0) acts as a regional bias -- a ~1 km
    /// wavelength shift that creates broad highland and lowland zones, giving the map
    /// occasional tall plateaus and low basins among the rolling mid-scale detail.
    /// The four finer octaves match the Phase 2.2.5 tuning: halved amplitudes so peaks
    /// stay plausible at aircraft scale. Phase 2.2.4 polish (regional term) pushes max
    /// amplitude to ±6.4 m.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let mut rng = Rng::from_seed(splitmix64(seed));
        // Phase 2.2.4 mountain-scale tuning (per ref1/2/3 reference photos: coastal
        // Alaska fjord and alpine-lake imagery with 300-500 m peaks).
        // - Regional: ~1 km wavelength, ±200 m -- broad mountain ranges vs. valley floors.
        // - Coarse: ~419 m, ±120 m -- major ridges and buttresses.
        // - Mid: ~157 m, ±40 m -- foothill rolls, side ridges.
        // - Fine: ~52 m, ±12 m -- local hillocks.
        // - Detail: ~21 m, ±3 m -- surface texture.
        // Total ±375 m max; sea level cuts the lowlands, forests climb the middle,
        // rock and snow crown the ridges. Plane's 4 m height is now small relative to
        // the terrain, matching real-world floatplane photography scale.
        let octaves = [
            make_octave(&mut rng, 0.006, 200.0), // regional
            make_octave(&mut rng, 0.015, 120.0),
            make_octave(&mut rng, 0.04, 40.0),
            make_octave(&mut rng, 0.12, 12.0),
            make_octave(&mut rng, 0.3, 3.0),
        ];
        Self {
            seed,
            octaves,
            base_amplitude: 1.0,
        }
    }

    /// The seed this terrain was built from.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// The maximum height achievable (sum of octave amplitudes).
    #[must_use]
    pub fn max_height(&self) -> f32 {
        self.base_amplitude * self.octaves.iter().map(|o| o.amplitude).sum::<f32>()
    }

    /// Iterator over the octaves (for inspection / mesh tooling).
    pub fn octaves(&self) -> impl Iterator<Item = (f32, f32, f32, f32)> + '_ {
        self.octaves
            .iter()
            .map(|o| (o.frequency, o.amplitude, o.phase_x, o.phase_z))
    }
}

impl HeightSampler for Terrain {
    fn height(&self, x: f32, z: f32) -> f32 {
        self.octaves.iter().fold(0.0, |acc, o| {
            acc + o.amplitude
                * ((x * o.frequency + o.phase_x).sin())
                * ((z * o.frequency + o.phase_z).cos())
        }) * self.base_amplitude
    }
}

fn make_octave(rng: &mut Rng, frequency: f32, amplitude: f32) -> Octave {
    use core::f32::consts::TAU;
    Octave {
        frequency,
        amplitude,
        phase_x: rng.next_f32() * TAU,
        phase_z: rng.next_f32() * TAU,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_height() {
        let a = Terrain::from_seed(42);
        let b = Terrain::from_seed(42);
        let samples: [(f32, f32); 4] = [(0.0, 0.0), (10.0, -5.0), (-25.0, 25.0), (100.0, -100.0)];
        for (x, z) in samples {
            assert!((a.height(x, z) - b.height(x, z)).abs() < 1e-6);
        }
    }

    #[test]
    fn different_seed_different_height_at_origin() {
        let a = Terrain::from_seed(1);
        let b = Terrain::from_seed(2);
        // Vanishingly unlikely to land on an identical value for two decorrelated seeds.
        assert!((a.height(0.0, 0.0) - b.height(0.0, 0.0)).abs() > 1e-6);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn max_height_bounds_samples() {
        let t = Terrain::from_seed(0xA1B2_C3D4);
        let max = t.max_height();
        // Sample a grid; every sample must lie within [-max, max].
        for ix in -10_i32..=10 {
            for iz in -10_i32..=10 {
                let h = t.height(ix as f32 * 5.0, iz as f32 * 5.0);
                assert!(h.abs() <= max + 1e-5, "h={h} max={max} at ({ix},{iz})");
            }
        }
    }

    #[test]
    fn octaves_enumerable() {
        let t = Terrain::from_seed(7);
        let count = t.octaves().count();
        assert_eq!(count, 5);
    }

    #[test]
    fn seed_accessor_round_trips() {
        let t = Terrain::from_seed(0xDEAD);
        assert_eq!(t.seed(), 0xDEAD);
    }
}
