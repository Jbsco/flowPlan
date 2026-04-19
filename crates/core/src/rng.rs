//! Seeded deterministic RNG wrapper.
//!
//! Wraps `rand_pcg::Pcg64Mcg` with a fixed-seed constructor and stream-separation helpers.
//! Determinism is a first-class invariant per [`DESIGN.md § Invariants`](../../../DESIGN.md#invariants):
//! for a given `(seed, input trace, tick count)` the sim produces bit-identical state. This
//! wrapper exists so that every subsystem can derive its own sub-stream from a master seed
//! without interfering with unrelated subsystems' RNG consumption.

use rand::{Rng as _, RngCore, SeedableRng};
use rand_pcg::Pcg64Mcg;

/// A seeded, deterministic PCG random number generator scoped to one stream.
///
/// Use `Rng::from_seed` at startup with the world seed; derive per-subsystem streams via
/// `Rng::stream`. Do not share a single `Rng` instance across unrelated subsystems: it makes
/// feature toggles (e.g., disabling weather) perturb unrelated RNG consumption and breaks
/// reproducibility of wildlife, missions, etc.
#[derive(Debug, Clone)]
pub struct Rng {
    inner: Pcg64Mcg,
}

impl Rng {
    /// Construct an RNG from a 64-bit seed. The same seed always yields the same sequence.
    #[inline]
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        // Extend the 64-bit seed to Pcg64Mcg's 16-byte seed via repetition. This is stable
        // across builds and not cryptographically sensitive (flowPlan uses RNG for gameplay,
        // not security).
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&seed.to_le_bytes());
        bytes[8..].copy_from_slice(&seed.to_le_bytes());
        Self {
            inner: Pcg64Mcg::from_seed(bytes),
        }
    }

    /// Derive a per-subsystem stream from a parent seed and a stream id. The returned RNG
    /// is independent of the parent and of other streams derived with different ids.
    ///
    /// Conventional stream ids (reserved): 0 = weather, 1 = wildlife, 2 = mission-parameterization,
    /// 3 = wear-accretion, 4 = soundtrack, 5 = hub-gen.
    #[inline]
    #[must_use]
    pub fn stream(seed: u64, stream_id: u64) -> Self {
        // Mix seed and stream_id deterministically via SplitMix64-style hash to decorrelate.
        let mixed = splitmix64(seed ^ rotate_left_u64(stream_id, 17));
        Self::from_seed(mixed)
    }

    /// Generate a random `f32` in the half-open range `[0.0, 1.0)`.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        self.inner.gen::<f32>()
    }

    /// Generate a random `f64` in the half-open range `[0.0, 1.0)`.
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        self.inner.gen::<f64>()
    }

    /// Generate a random `u32`.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        self.inner.next_u32()
    }

    /// Generate a random `u64`.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    /// Generate a random integer in the half-open range `[low, high)`.
    /// Panics in debug if `low >= high`.
    #[inline]
    pub fn range_i32(&mut self, low: i32, high: i32) -> i32 {
        debug_assert!(low < high, "range_i32 requires low < high");
        self.inner.gen_range(low..high)
    }
}

/// `SplitMix64` mixer, used to decorrelate a seed across streams. Public because it is
/// useful for other deterministic derivations too.
#[inline]
#[must_use]
pub const fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
const fn rotate_left_u64(x: u64, n: u32) -> u64 {
    x.rotate_left(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::from_seed(0xDEAD_BEEF_CAFE_F00D);
        let mut b = Rng::from_seed(0xDEAD_BEEF_CAFE_F00D);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seed_different_sequence() {
        let mut a = Rng::from_seed(1);
        let mut b = Rng::from_seed(2);
        // Vanishingly unlikely to match on all 16 samples if they are truly decorrelated.
        let any_diff = (0..16).any(|_| a.next_u64() != b.next_u64());
        assert!(any_diff);
    }

    #[test]
    fn streams_decorrelate() {
        let mut a = Rng::stream(42, 0);
        let mut b = Rng::stream(42, 1);
        let any_diff = (0..16).any(|_| a.next_u64() != b.next_u64());
        assert!(
            any_diff,
            "streams with same seed but different id must decorrelate"
        );
    }

    #[test]
    fn streams_same_id_same_sequence() {
        let mut a = Rng::stream(42, 7);
        let mut b = Rng::stream(42, 7);
        for _ in 0..32 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn f32_in_unit_range() {
        let mut r = Rng::from_seed(1);
        for _ in 0..1024 {
            let x = r.next_f32();
            assert!((0.0..1.0).contains(&x), "f32 out of [0,1): {x}");
        }
    }

    #[test]
    fn range_i32_respects_bounds() {
        let mut r = Rng::from_seed(1);
        for _ in 0..1024 {
            let x = r.range_i32(-5, 10);
            assert!((-5..10).contains(&x), "i32 out of [-5,10): {x}");
        }
    }

    #[test]
    fn splitmix_is_deterministic() {
        assert_eq!(splitmix64(0), splitmix64(0));
        assert_ne!(splitmix64(0), splitmix64(1));
    }
}
