//! Deterministic hashing of `SimState` for property tests.
//!
//! Folds the state's scalar fields into a 64-bit FNV-1a hash. Equal states produce equal
//! hashes; different states overwhelmingly produce different hashes. Used only for
//! testing -- runtime paths do not hash the sim state.

use crate::state::SimState;

/// Deterministic 64-bit hash of `SimState`.
#[must_use]
pub fn state_hash(s: &SimState) -> u64 {
    let mut h = FNV_OFFSET;
    fold_u64(&mut h, s.tick.0);
    fold_u64(&mut h, s.seed);
    fold_vec3(&mut h, s.vehicle.position);
    fold_vec3(&mut h, s.vehicle.velocity);
    fold_quat(&mut h, s.vehicle.orientation);
    fold_vec3(&mut h, s.vehicle.angular_velocity);
    fold_f32(&mut h, s.vehicle.controls.throttle);
    fold_f32(&mut h, s.vehicle.controls.pitch);
    fold_f32(&mut h, s.vehicle.controls.yaw);
    fold_f32(&mut h, s.vehicle.controls.roll);
    fold_u32(&mut h, s.vehicle.config.as_u32());
    fold_vec3(&mut h, s.camera.position);
    fold_quat(&mut h, s.camera.orientation);
    fold_f32(&mut h, s.camera.fov_y_radians);
    fold_u32(&mut h, view_kind_as_u32(s.camera.view_kind));
    fold_f32(&mut h, s.time_of_day);
    fold_f32(&mut h, s.wave_time);
    fold_f32(&mut h, s.chop_intensity);
    h
}

fn view_kind_as_u32(v: crate::camera::ViewKind) -> u32 {
    match v {
        crate::camera::ViewKind::FirstPerson => 0,
        crate::camera::ViewKind::ChaseThirdPerson => 1,
        crate::camera::ViewKind::ExteriorOrbit => 2,
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x100_0000_01b3;

fn fold_bytes(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(FNV_PRIME);
    }
}

fn fold_u64(h: &mut u64, v: u64) {
    fold_bytes(h, &v.to_le_bytes());
}

fn fold_u32(h: &mut u64, v: u32) {
    fold_bytes(h, &v.to_le_bytes());
}

fn fold_f32(h: &mut u64, v: f32) {
    fold_bytes(h, &v.to_le_bytes());
}

fn fold_vec3(h: &mut u64, v: glam::Vec3) {
    fold_f32(h, v.x);
    fold_f32(h, v.y);
    fold_f32(h, v.z);
}

fn fold_quat(h: &mut u64, q: glam::Quat) {
    fold_f32(h, q.x);
    fold_f32(h, q.y);
    fold_f32(h, q.z);
    fold_f32(h, q.w);
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::camera::ViewKind;
    use crate::difficulty::DifficultyDials;
    use crate::input::{InputEvent, InputEvents};
    use crate::state::{SimState, FIXED_DT};

    #[test]
    fn same_seed_same_hash_fresh() {
        let a = SimState::new(42);
        let b = SimState::new(42);
        assert_eq!(state_hash(&a), state_hash(&b));
    }

    #[test]
    fn different_seed_different_hash() {
        let a = SimState::new(1);
        let b = SimState::new(2);
        assert_ne!(state_hash(&a), state_hash(&b));
    }

    #[test]
    fn ticks_change_hash() {
        let mut s = SimState::new(1);
        let h0 = state_hash(&s);
        s.tick(
            &InputEvents::empty(),
            &DifficultyDials::default(),
            FIXED_DT,
            0.5,
        );
        let h1 = state_hash(&s);
        assert_ne!(h0, h1);
    }

    #[test]
    fn same_inputs_same_trace_same_hash() {
        let mut a = SimState::new(7);
        let mut b = SimState::new(7);
        let d = DifficultyDials::default();
        let mut events = InputEvents::default();
        events.push(InputEvent::SetThrottle(0.5));
        events.push(InputEvent::SetPitch(0.1));
        for _ in 0..128 {
            a.tick(&events, &d, FIXED_DT, 0.5);
            b.tick(&events, &d, FIXED_DT, 0.5);
        }
        assert_eq!(state_hash(&a), state_hash(&b));
    }

    // Property-driven sweep over varied input traces.
    // Enforces the determinism invariant from DESIGN.md.

    fn event_strategy() -> impl Strategy<Value = InputEvent> {
        prop_oneof![
            (0.0_f32..1.0).prop_map(InputEvent::SetThrottle),
            (-1.0_f32..1.0).prop_map(InputEvent::SetPitch),
            (-1.0_f32..1.0).prop_map(InputEvent::SetYaw),
            (-1.0_f32..1.0).prop_map(InputEvent::SetRoll),
            (-0.1_f32..0.1).prop_map(InputEvent::ThrottleDelta),
            Just(InputEvent::CycleView),
            Just(InputEvent::SetView(ViewKind::FirstPerson)),
            Just(InputEvent::SetView(ViewKind::ChaseThirdPerson)),
            Just(InputEvent::SetView(ViewKind::ExteriorOrbit)),
            Just(InputEvent::ResetLook),
        ]
    }

    fn batch_strategy() -> impl Strategy<Value = Vec<InputEvent>> {
        prop::collection::vec(event_strategy(), 0..4)
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 32, .. ProptestConfig::default() })]

        #[test]
        fn prop_same_seed_same_trace_same_hash(
            seed in 0u64..10_000,
            batches in prop::collection::vec(batch_strategy(), 1..64),
        ) {
            let mut a = SimState::new(seed);
            let mut b = SimState::new(seed);
            let d = DifficultyDials::default();
            for batch in &batches {
                let ev = InputEvents::from_slice(batch);
                a.tick(&ev, &d, FIXED_DT, 0.5);
                b.tick(&ev, &d, FIXED_DT, 0.5);
            }
            prop_assert_eq!(state_hash(&a), state_hash(&b));
        }

        #[test]
        fn prop_different_seed_different_hash(
            seed_a in 0u64..5_000,
            seed_b in 5_001u64..10_000,
            batches in prop::collection::vec(batch_strategy(), 1..32),
        ) {
            prop_assume!(seed_a != seed_b);
            let mut a = SimState::new(seed_a);
            let mut b = SimState::new(seed_b);
            let d = DifficultyDials::default();
            for batch in &batches {
                let ev = InputEvents::from_slice(batch);
                a.tick(&ev, &d, FIXED_DT, 0.5);
                b.tick(&ev, &d, FIXED_DT, 0.5);
            }
            // Seed is folded into state_hash; divergence is guaranteed.
            prop_assert_ne!(state_hash(&a), state_hash(&b));
        }
    }
}
