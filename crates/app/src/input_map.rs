//! Map winit keyboard state to `InputEvent`s, pushed into the bounded input queue each
//! frame. Gamepad, mouse-look, and HOTAS support lands in Phase 4 per
//! [`ROADMAP.md § Milestone 4.7b`](../../../../ROADMAP.md#milestone-47b-control-bindings--mapping).
//!
//! Phase 1 bindings (fixed):
//!   W / S               throttle up / down  (continuous while held)
//!   Arrow Up / Down     pitch up / down
//!   Arrow Left / Right  roll left / right
//!   Q / E               yaw left / right
//!   1 / 2 / 3           view: first-person / chase / orbit
//!   V                   cycle view
//!   L                   reset POV look (no-op until look input lands)
//!   R                   reset vehicle
//!   Esc                 quit (handled in `game_app`)

#![deny(unsafe_code)]

use crossbeam_queue::ArrayQueue;
use flowplan_sim::{InputEvent, ViewKind};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

/// Live per-frame keyboard held-state for continuous axes.
///
/// Eight bools by design -- one per primary-axis direction. `struct_excessive_bools` is
/// disabled here because a bitfield would cost readability without saving memory on
/// platforms where bool is a byte.
#[derive(Clone, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct KeyState {
    pub throttle_up: bool,
    pub throttle_down: bool,
    pub pitch_up: bool,
    pub pitch_down: bool,
    pub roll_left: bool,
    pub roll_right: bool,
    pub yaw_left: bool,
    pub yaw_right: bool,
}

impl KeyState {
    /// Update from a single winit key event.
    ///
    /// Held-state for continuous axes is tracked internally; edge-triggered events
    /// (view switches, reset actions) are pushed into the queue directly so each press
    /// fires exactly once.
    pub fn apply_key_event(&mut self, ev: &KeyEvent, queue: &ArrayQueue<InputEvent>) {
        let PhysicalKey::Code(code) = ev.physical_key else {
            return;
        };
        let pressed = ev.state == ElementState::Pressed;
        match code {
            KeyCode::KeyW => self.throttle_up = pressed,
            KeyCode::KeyS => self.throttle_down = pressed,
            KeyCode::ArrowUp => self.pitch_up = pressed,
            KeyCode::ArrowDown => self.pitch_down = pressed,
            KeyCode::ArrowLeft => self.roll_left = pressed,
            KeyCode::ArrowRight => self.roll_right = pressed,
            KeyCode::KeyQ => self.yaw_left = pressed,
            KeyCode::KeyE => self.yaw_right = pressed,
            KeyCode::KeyV if pressed && !ev.repeat => {
                let _ = queue.push(InputEvent::CycleView);
            },
            KeyCode::Digit1 if pressed && !ev.repeat => {
                let _ = queue.push(InputEvent::SetView(ViewKind::FirstPerson));
            },
            KeyCode::Digit2 if pressed && !ev.repeat => {
                let _ = queue.push(InputEvent::SetView(ViewKind::ChaseThirdPerson));
            },
            KeyCode::Digit3 if pressed && !ev.repeat => {
                let _ = queue.push(InputEvent::SetView(ViewKind::ExteriorOrbit));
            },
            KeyCode::KeyL if pressed && !ev.repeat => {
                let _ = queue.push(InputEvent::ResetLook);
            },
            KeyCode::KeyR if pressed && !ev.repeat => {
                let _ = queue.push(InputEvent::ResetVehicle);
            },
            _ => {},
        }
    }

    /// Push the continuous-axis events for this tick based on current held-state.
    /// Caller invokes once per tick. `dt` is in seconds (fixed timestep).
    pub fn push_axis_events(&self, queue: &ArrayQueue<InputEvent>, dt: f32) {
        let throttle_delta =
            (f32::from(self.throttle_up) - f32::from(self.throttle_down)) * 0.5 * dt;
        if throttle_delta.abs() > 0.0 {
            let _ = queue.push(InputEvent::ThrottleDelta(throttle_delta));
        }
        let pitch = f32::from(self.pitch_up) - f32::from(self.pitch_down);
        let _ = queue.push(InputEvent::SetPitch(pitch));
        let roll = f32::from(self.roll_right) - f32::from(self.roll_left);
        let _ = queue.push(InputEvent::SetRoll(roll));
        let yaw = f32::from(self.yaw_right) - f32::from(self.yaw_left);
        let _ = queue.push(InputEvent::SetYaw(yaw));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `KeyEvent` in winit 0.30 has a private `platform_specific` field and cannot be
    // struct-constructed directly, so `apply_key_event` is exercised in the hardware
    // test tier instead (see TEST_PLAN.md). Unit tests here cover the pure axis
    // derivation from `KeyState`.

    #[test]
    fn axis_events_pushed_with_neutral_when_nothing_held() {
        let ks = KeyState::default();
        let queue = ArrayQueue::new(16);
        ks.push_axis_events(&queue, 1.0 / 120.0);
        // SetPitch(0), SetRoll(0), SetYaw(0). No ThrottleDelta when neither held.
        let mut seen_pitch = false;
        let mut seen_roll = false;
        let mut seen_yaw = false;
        let mut seen_throttle = false;
        while let Some(ev) = queue.pop() {
            match ev {
                InputEvent::SetPitch(v) => {
                    assert!((v - 0.0).abs() < f32::EPSILON);
                    seen_pitch = true;
                },
                InputEvent::SetRoll(v) => {
                    assert!((v - 0.0).abs() < f32::EPSILON);
                    seen_roll = true;
                },
                InputEvent::SetYaw(v) => {
                    assert!((v - 0.0).abs() < f32::EPSILON);
                    seen_yaw = true;
                },
                InputEvent::ThrottleDelta(_) => seen_throttle = true,
                _ => {},
            }
        }
        assert!(seen_pitch);
        assert!(seen_roll);
        assert!(seen_yaw);
        assert!(!seen_throttle);
    }
}
