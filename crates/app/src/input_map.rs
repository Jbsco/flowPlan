//! Map winit keyboard state to `InputEvent`s, pushed into the bounded input queue each
//! frame. Gamepad, mouse-look, and HOTAS support lands in Phase 4 per
//! [`ROADMAP.md § Milestone 4.7b`](../../../../ROADMAP.md#milestone-47b-control-bindings--mapping).
//!
//! Phase 1 bindings (fixed):
//!   W / S               throttle up / down  (continuous while held)
//!   Arrow Up            stick forward: pitch nose DOWN  (dive)
//!   Arrow Down          stick back:    pitch nose UP    (climb)
//!   Arrow Left / Right  roll left / right
//!   Q / E               yaw left / right
//!   1 / 2 / 3           view: first-person / chase / orbit
//!   V                   cycle view
//!   L                   reset POV look (no-op until look input lands)
//!   R                   reset vehicle
//!   F                   flaps extend one notch (UP→CLIMB→TAKEOFF→LANDING)
//!   C                   flaps retract one notch (LANDING→TAKEOFF→CLIMB→UP)
//!   T                   toggle water rudder (float variants only — no-op on wheels)
//!   B                   brakes (momentary while held)
//!   Esc                 quit (handled in `game_app`)
//!
//! # Keyboard axis ramp (Phase 2.4.4a)
//!
//! Keyboard keys are binary (0 or 1); a real flight stick moves analog. To avoid the
//! "mashed full rudder in one frame" behavior that produces nonphysical responses in
//! the 2.4.4 moment model, pitch / roll / yaw axes slew toward the held-key target at
//! a bounded rate (`AXIS_SLEW_RATE`). Full travel takes ~0.67 s; release returns to
//! neutral at the same rate (symmetric, matches the spring-return feel of a typical
//! stick). Throttle keeps its existing per-tick delta model -- it has no "neutral"
//! position, so slewing toward 0 on release would misbehave.
//!
//! Analog sources (gamepad, HOTAS, mouse-as-stick) bypass this ramp when added in
//! Phase 4; they provide already-smooth values.

#![deny(unsafe_code)]

use crossbeam_queue::ArrayQueue;
use flowplan_sim::{InputEvent, ViewKind};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

/// Maximum rate of change for a keyboard-driven axis, in `full-deflection per second`.
/// A value of 1.5 means a fully-held key reaches full deflection in `1 / 1.5 ≈ 0.67 s`,
/// and releasing centers in the same time. Human motor response is ~0.2 s for coarse
/// moves; choosing a rate slightly slower than that prevents "mashed-stick" artifacts
/// while staying responsive.
const AXIS_SLEW_RATE: f32 = 1.5;

/// Live per-frame keyboard held-state for continuous axes plus the slewed axis values.
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
    /// Current slewed pitch stick position in `[-1, 1]`. Updated each tick.
    pub current_pitch: f32,
    /// Current slewed roll stick position in `[-1, 1]`. Updated each tick.
    pub current_roll: f32,
    /// Current slewed yaw pedal position in `[-1, 1]`. Updated each tick.
    pub current_yaw: f32,
    /// Latched water-rudder deployment state. `T` toggles this; each toggle
    /// emits a `SetWaterRudder(new_state)` event. Starts retracted to match
    /// `Controls::default()`; if a scenario or save overrides the sim-side
    /// state, the two may drift until the user presses `T` again. Gamepad /
    /// remap UI in Phase 4.7b resolves this properly.
    pub water_rudder_deployed: bool,
    // ------------------------------------------------------------------
    // Debug-camera state (3.6.3c.5a).
    //
    // Routed through KeyState because the input dispatcher (main loop)
    // already holds this struct; a second parallel state would duplicate
    // the on-key/off-key tracking. When `debug_active` is true, the
    // aircraft-control keys (W/S throttle + pitch / roll / yaw) are
    // *also* routed to debug-camera motion — W = forward, S = back, etc.
    // Emission of aircraft `InputEvent`s is suppressed while debug mode
    // is active so the background aircraft stays in its current state.
    // ------------------------------------------------------------------
    /// True when the debug-fly camera is active. Toggled by backtick.
    pub debug_active: bool,
    /// Debug-camera forward/back (W/S) when `debug_active`.
    pub debug_fwd: bool,
    pub debug_back: bool,
    /// Debug-camera strafe (A/D).
    pub debug_left: bool,
    pub debug_right: bool,
    /// Debug-camera vertical (`LShift` / `LCtrl`).
    pub debug_up: bool,
    pub debug_down: bool,
    /// Edge-triggered: backtick was just pressed this frame. Consumer
    /// clears after handling.
    pub debug_toggle_pending: bool,
    /// Edge-triggered: Tab was just pressed (cycle speed).
    pub debug_cycle_speed_pending: bool,
    /// Edge-triggered: ESC was just pressed in debug mode (release mouse).
    pub debug_release_mouse_pending: bool,
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

        // ---------------- Debug-camera toggle + debug-mode keys ----------
        // Backtick is the global toggle — works in both modes.
        if code == KeyCode::Backquote && pressed && !ev.repeat {
            self.debug_toggle_pending = true;
            return;
        }

        // When debug mode is active, WASD + Shift/Ctrl drive the debug
        // camera; Tab cycles speed; ESC releases mouse. Aircraft controls
        // are intentionally suppressed — sim keeps running but the
        // aircraft receives no input (its current state persists).
        if self.debug_active {
            match code {
                KeyCode::KeyW => self.debug_fwd = pressed,
                KeyCode::KeyS => self.debug_back = pressed,
                KeyCode::KeyA => self.debug_left = pressed,
                KeyCode::KeyD => self.debug_right = pressed,
                KeyCode::ShiftLeft => self.debug_up = pressed,
                KeyCode::ControlLeft => self.debug_down = pressed,
                KeyCode::Tab if pressed && !ev.repeat => {
                    self.debug_cycle_speed_pending = true;
                },
                KeyCode::Escape if pressed && !ev.repeat => {
                    self.debug_release_mouse_pending = true;
                },
                _ => {},
            }
            return; // Suppress aircraft events while debug is active.
        }

        // ---------------- Aircraft controls (normal mode) ----------------
        match code {
            KeyCode::KeyW => self.throttle_up = pressed,
            KeyCode::KeyS => self.throttle_down = pressed,
            // Stick convention: arrow up = stick forward = nose DOWN; arrow down = nose UP.
            // The held-state fields document pitch *intent* (nose-up vs nose-down), so
            // the arrow-to-field mapping is deliberately inverted from the key symbols.
            // Phase 4's full remapping UI will make this user-configurable.
            KeyCode::ArrowUp => self.pitch_down = pressed,
            KeyCode::ArrowDown => self.pitch_up = pressed,
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
            KeyCode::KeyF if pressed && !ev.repeat => {
                let _ = queue.push(InputEvent::ExtendFlaps);
            },
            KeyCode::KeyC if pressed && !ev.repeat => {
                let _ = queue.push(InputEvent::RetractFlaps);
            },
            // Water rudder toggle (float variants). Starts retracted; `T`
            // toggles deploy ↔ retract, emitting the corresponding
            // `SetWaterRudder` event. Wheels variants have no water rudder,
            // so the event is a no-op there — harmless to leave bound.
            KeyCode::KeyT if pressed && !ev.repeat => {
                self.water_rudder_deployed = !self.water_rudder_deployed;
                let _ = queue.push(InputEvent::SetWaterRudder(self.water_rudder_deployed));
            },
            // Momentary brakes: hold B = brakes applied; release = off. Ignore
            // auto-repeat so the release edge isn't swallowed by repeated
            // keydowns when the pilot holds through typematic delay.
            KeyCode::KeyB if !ev.repeat => {
                let _ = queue.push(InputEvent::SetBrakes(pressed));
            },
            _ => {},
        }
    }

    /// Clear all held aircraft-control keys. Called when entering debug
    /// mode so the aircraft doesn't keep receiving stale inputs.
    pub fn clear_aircraft_inputs(&mut self) {
        self.throttle_up = false;
        self.throttle_down = false;
        self.pitch_up = false;
        self.pitch_down = false;
        self.roll_left = false;
        self.roll_right = false;
        self.yaw_left = false;
        self.yaw_right = false;
    }

    /// Clear all held debug-camera keys. Called when exiting debug mode.
    pub fn clear_debug_inputs(&mut self) {
        self.debug_fwd = false;
        self.debug_back = false;
        self.debug_left = false;
        self.debug_right = false;
        self.debug_up = false;
        self.debug_down = false;
    }

    /// Current debug-camera motion axes as signed unit vector
    /// `(forward_back, right_left, up_down)` for feeding `DebugCamera::integrate`.
    #[must_use]
    pub fn debug_motion(&self) -> (f32, f32, f32) {
        let fb = f32::from(i8::from(self.debug_fwd) - i8::from(self.debug_back));
        let rl = f32::from(i8::from(self.debug_right) - i8::from(self.debug_left));
        let ud = f32::from(i8::from(self.debug_up) - i8::from(self.debug_down));
        (fb, rl, ud)
    }

    /// Push the continuous-axis events for this tick based on current held-state.
    /// Caller invokes once per tick. `dt` is in seconds (fixed timestep).
    ///
    /// Pitch / roll / yaw axes slew toward their held-key target at `AXIS_SLEW_RATE`;
    /// throttle keeps its existing per-tick delta model.
    pub fn push_axis_events(&mut self, queue: &ArrayQueue<InputEvent>, dt: f32) {
        let throttle_delta =
            (f32::from(self.throttle_up) - f32::from(self.throttle_down)) * 0.5 * dt;
        if throttle_delta.abs() > 0.0 {
            let _ = queue.push(InputEvent::ThrottleDelta(throttle_delta));
        }

        let pitch_target = f32::from(self.pitch_up) - f32::from(self.pitch_down);
        let roll_target = f32::from(self.roll_right) - f32::from(self.roll_left);
        let yaw_target = f32::from(self.yaw_right) - f32::from(self.yaw_left);

        let max_step = AXIS_SLEW_RATE * dt;
        self.current_pitch = slew_toward(self.current_pitch, pitch_target, max_step);
        self.current_roll = slew_toward(self.current_roll, roll_target, max_step);
        self.current_yaw = slew_toward(self.current_yaw, yaw_target, max_step);

        let _ = queue.push(InputEvent::SetPitch(self.current_pitch));
        let _ = queue.push(InputEvent::SetRoll(self.current_roll));
        let _ = queue.push(InputEvent::SetYaw(self.current_yaw));
    }
}

/// Move `current` toward `target` by at most `max_step`. Linear slew: the returned
/// value is either `target` (if `|target - current| <= max_step`) or
/// `current + sign(diff) * max_step`.
fn slew_toward(current: f32, target: f32, max_step: f32) -> f32 {
    let diff = target - current;
    if diff.abs() <= max_step {
        target
    } else {
        current + diff.signum() * max_step
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    // `KeyEvent` in winit 0.30 has a private `platform_specific` field and cannot be
    // struct-constructed directly, so `apply_key_event` is exercised in the hardware
    // test tier instead (see TEST_PLAN.md). Unit tests here cover the pure axis
    // derivation from `KeyState`.

    const DT: f32 = 1.0 / 120.0;

    /// Drain the queue, returning the last seen pitch/roll/yaw and whether any
    /// throttle-delta event was observed. Panics if any axis event is missing.
    fn drain_axes(queue: &ArrayQueue<InputEvent>) -> (f32, f32, f32, bool) {
        let mut pitch: f32 = f32::NAN;
        let mut roll: f32 = f32::NAN;
        let mut yaw: f32 = f32::NAN;
        let mut saw_throttle = false;
        while let Some(ev) = queue.pop() {
            match ev {
                InputEvent::SetPitch(v) => pitch = v,
                InputEvent::SetRoll(v) => roll = v,
                InputEvent::SetYaw(v) => yaw = v,
                InputEvent::ThrottleDelta(_) => saw_throttle = true,
                _ => {},
            }
        }
        assert!(
            pitch.is_finite() && roll.is_finite() && yaw.is_finite(),
            "push_axis_events must emit all three axes every tick",
        );
        (pitch, roll, yaw, saw_throttle)
    }

    #[test]
    fn axis_events_pushed_with_neutral_when_nothing_held() {
        let mut ks = KeyState::default();
        let queue = ArrayQueue::new(16);
        ks.push_axis_events(&queue, DT);
        let (pitch, roll, yaw, saw_throttle) = drain_axes(&queue);
        assert!((pitch - 0.0).abs() < f32::EPSILON);
        assert!((roll - 0.0).abs() < f32::EPSILON);
        assert!((yaw - 0.0).abs() < f32::EPSILON);
        assert!(!saw_throttle);
    }

    #[test]
    fn held_key_ramps_toward_full_deflection() {
        // With AXIS_SLEW_RATE = 1.5 per second, a single tick at DT = 1/120 advances by
        // 1.5 / 120 = 0.0125 toward the target. Full deflection needs 1/0.0125 = 80
        // ticks (2/3 s).
        let mut ks = KeyState::default();
        ks.pitch_up = true;
        let queue = ArrayQueue::new(16);

        ks.push_axis_events(&queue, DT);
        let (pitch, ..) = drain_axes(&queue);
        assert!(
            (pitch - 0.0125).abs() < 1e-5,
            "after 1 tick, pitch should be 0.0125; got {pitch}",
        );

        // Tick many more times; after 80 ticks total we should reach 1.0 exactly.
        for _ in 0..79 {
            ks.push_axis_events(&queue, DT);
            drain_axes(&queue);
        }
        ks.push_axis_events(&queue, DT);
        let (pitch, ..) = drain_axes(&queue);
        assert!(
            (pitch - 1.0).abs() < 1e-5,
            "after 80 ticks held, pitch should be 1.0; got {pitch}",
        );
    }

    #[test]
    fn released_key_ramps_back_to_neutral() {
        let mut ks = KeyState::default();
        ks.current_pitch = 1.0;
        let queue = ArrayQueue::new(16);
        // 80 ticks of release should bring pitch from 1.0 back to 0.0.
        for _ in 0..80 {
            ks.push_axis_events(&queue, DT);
            drain_axes(&queue);
        }
        ks.push_axis_events(&queue, DT);
        let (pitch, ..) = drain_axes(&queue);
        assert!(
            pitch.abs() < 1e-5,
            "after release + 80 ticks, pitch should be 0; got {pitch}",
        );
    }

    #[test]
    fn direction_reversal_passes_through_zero() {
        // Held right roll to full, then hold left. Should cross zero at some point.
        let mut ks = KeyState::default();
        ks.current_roll = 1.0;
        ks.roll_left = true;
        let queue = ArrayQueue::new(16);
        // From +1.0 toward -1.0, at 0.0125 per tick, takes 160 ticks to reach -1.
        // At tick 80, should be near 0.
        for _ in 0..80 {
            ks.push_axis_events(&queue, DT);
            drain_axes(&queue);
        }
        ks.push_axis_events(&queue, DT);
        let (_, roll, ..) = drain_axes(&queue);
        assert!(
            roll.abs() < 0.02,
            "at tick 80 of reversal, roll should be near 0; got {roll}",
        );
    }

    #[test]
    fn throttle_still_uses_delta_model() {
        let mut ks = KeyState::default();
        ks.throttle_up = true;
        let queue = ArrayQueue::new(16);
        ks.push_axis_events(&queue, DT);
        let (_, _, _, saw_throttle) = drain_axes(&queue);
        assert!(saw_throttle, "throttle up held should emit ThrottleDelta");
    }

    #[test]
    fn slew_toward_converges_in_one_step_when_close() {
        assert!((slew_toward(0.9, 1.0, 0.5) - 1.0).abs() < f32::EPSILON);
        assert!((slew_toward(-0.1, 0.0, 0.5) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn slew_toward_steps_when_far() {
        assert!((slew_toward(0.0, 1.0, 0.25) - 0.25).abs() < f32::EPSILON);
        assert!((slew_toward(0.5, -1.0, 0.25) - 0.25).abs() < f32::EPSILON);
    }
}
