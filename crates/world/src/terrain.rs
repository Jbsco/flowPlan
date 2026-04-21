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
    /// Constant offset applied to every height query. Zero for `from_seed()`
    /// (terrain centered around y=0); negative for `flat()` to put the visible
    /// terrain below the water surface (mean water at y=0) and prevent z-fighting
    /// with the water mesh in scenario playback.
    base_height: f32,
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
        // Alaska fjord and alpine-lake imagery). Regional amplitude halved from the
        // original 200 m to 100 m to soften large-scale gradients so shores can
        // actually spread horizontally; the tradeoff is less dramatic mountain peaks.
        //
        // The fundamental limitation: sum-of-sines terrain cannot independently tune
        // amplitude and gradient, so genuinely gentle beach flats next to steep
        // mountain buttresses require a different representation. Proper solution
        // involves real hydrology (rivers carving valleys, lake basins, erosion-aware
        // coastlines) -- tracked under [FEATURES.md § Terrain & Static Landscape]
        // for a Phase 3+ terrain rebuild.
        //
        // - Regional: ~1 km wavelength, ±100 m -- broad mountain ranges vs. valley floors.
        // - Coarse: ~419 m, ±120 m -- major ridges and buttresses.
        // - Mid: ~157 m, ±40 m -- foothill rolls, side ridges.
        // - Fine: ~52 m, ±12 m -- local hillocks.
        // - Detail: ~21 m, ±3 m -- surface texture.
        // Total ±275 m max.
        let octaves = [
            make_octave(&mut rng, 0.006, 100.0), // regional (halved from 200)
            make_octave(&mut rng, 0.015, 120.0),
            make_octave(&mut rng, 0.04, 40.0),
            make_octave(&mut rng, 0.12, 12.0),
            make_octave(&mut rng, 0.3, 3.0),
        ];
        Self {
            seed,
            octaves,
            base_amplitude: 1.0,
            base_height: 0.0,
        }
    }

    /// Flat terrain at a constant depth below the water surface. Used for scenario-
    /// bench visual playback (`cargo run -- --scenario <name>`) so the visual is a
    /// clean "open water with a deep bottom" — water surface at y=0, terrain mesh
    /// 10 m below. Without the depth offset the terrain would z-fight the water
    /// mesh (both at y=0).
    ///
    /// Determinism note: scenario playback overrides `min_altitude = 0.0` in the
    /// game loop (matching `crates/sim/tests/flight_bench.rs`), so the terrain
    /// returned here is purely cosmetic — the simulation never queries it for
    /// physics. The visual `Terrain::flat()` and the headless bench's flat-y=0
    /// world remain consistent in their physics behavior.
    #[must_use]
    pub fn flat() -> Self {
        Self {
            seed: 0,
            octaves: [Octave {
                frequency: 0.0,
                amplitude: 0.0,
                phase_x: 0.0,
                phase_z: 0.0,
            }; 5],
            base_amplitude: 0.0,
            base_height: -10.0,
        }
    }

    /// The seed this terrain was built from.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// The maximum height achievable (base offset + sum of octave amplitudes).
    #[must_use]
    pub fn max_height(&self) -> f32 {
        self.base_height
            + self.base_amplitude * self.octaves.iter().map(|o| o.amplitude).sum::<f32>()
    }

    /// Iterator over the octaves (for inspection / mesh tooling).
    pub fn octaves(&self) -> impl Iterator<Item = (f32, f32, f32, f32)> + '_ {
        self.octaves
            .iter()
            .map(|o| (o.frequency, o.amplitude, o.phase_x, o.phase_z))
    }
}

/// Width of the beach-flattening zone, in meters. Terrain heights within +/- this
/// distance from mean sea level get compressed toward zero so shores rise gently out of
/// the water before the steeper octaves take over. Tuned against the Phase 2.2.4
/// mountain-scale amplitudes (±375 m): 120 m means the beach ramp extends ~120 m
/// vertical before full-amplitude terrain takes over, producing shores that rise at
/// shallow angles over hundreds of horizontal meters even when the underlying octave
/// gradients are steep.
const BEACH_WIDTH_M: f32 = 120.0;

/// `raw * smoothstep(0, width, |raw|) ^ k`. Below `width` heights are compressed with
/// a cubic-ish curve; at or above `width` the identity function applies. Sign preserved.
fn beach_flatten(raw: f32, width: f32) -> f32 {
    let a = raw.abs();
    let t = (a / width).clamp(0.0, 1.0);
    // Stronger flattening near zero than smoothstep alone: square the smoothstep so
    // very-near-zero heights stay essentially at sea level.
    let smooth = t * t * (3.0 - 2.0 * t);
    raw * smooth * smooth
}

impl HeightSampler for Terrain {
    fn height(&self, x: f32, z: f32) -> f32 {
        let raw = self.octaves.iter().fold(0.0, |acc, o| {
            acc + o.amplitude
                * ((x * o.frequency + o.phase_x).sin())
                * ((z * o.frequency + o.phase_z).cos())
        }) * self.base_amplitude;
        beach_flatten(raw, BEACH_WIDTH_M) + self.base_height
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
