//! Structured input events for the simulation.
//!
//! `InputEvent` is the cross-boundary type. The main thread translates raw winit events
//! (and, post-v1, gamepad/joystick events) into `InputEvent` and pushes them into a
//! bounded queue; the sim drains the queue at the start of each tick.

use glam::Vec2;

use crate::camera::ViewKind;

/// A single intent-level input event, independent of the source device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    /// Set throttle to an absolute position in `[0.0, 1.0]`.
    SetThrottle(f32),
    /// Set pitch control to `[-1.0, 1.0]` (positive = nose up).
    SetPitch(f32),
    /// Set yaw control to `[-1.0, 1.0]` (positive = yaw right).
    SetYaw(f32),
    /// Set roll control to `[-1.0, 1.0]` (positive = roll right).
    SetRoll(f32),
    /// Apply a delta to throttle, scaled by tick dt on the caller side.
    ThrottleDelta(f32),
    /// Cycle to the next view kind.
    CycleView,
    /// Switch directly to the given view kind.
    SetView(ViewKind),
    /// POV-hat / look-around delta in `(yaw, pitch)` radians, applied per-tick.
    LookDelta(Vec2),
    /// Center-snap the POV-hat back to the view's natural orientation.
    ResetLook,
    /// Reset the vehicle to its starting state (debug).
    ResetVehicle,
    /// Advance flap handle one detent toward LANDING. No-op if already at LANDING.
    ExtendFlaps,
    /// Advance flap handle one detent toward UP. No-op if already at UP.
    RetractFlaps,
    /// Deploy or retract the water rudder (float aircraft only). Deployed gives
    /// strong yaw authority at low water speed; must be retracted for takeoff.
    SetWaterRudder(bool),
}

/// A per-tick batch of input events drained from the input queue.
///
/// Constructed by the app layer each tick, passed by reference to `SimState::tick`.
/// Deliberately `Vec`-backed for simplicity; Phase 3 switches to an arena-backed slice
/// when per-tick allocation becomes a hot-path concern.
#[derive(Clone, Debug, Default)]
pub struct InputEvents {
    events: Vec<InputEvent>,
}

impl InputEvents {
    /// An empty batch.
    #[must_use]
    pub const fn empty() -> Self {
        Self { events: Vec::new() }
    }

    /// Construct from a slice of events (copies).
    #[must_use]
    pub fn from_slice(events: &[InputEvent]) -> Self {
        Self {
            events: events.to_vec(),
        }
    }

    /// Append a single event.
    pub fn push(&mut self, event: InputEvent) {
        self.events.push(event);
    }

    /// Iterate the events in insertion order.
    pub fn iter(&self) -> core::slice::Iter<'_, InputEvent> {
        self.events.iter()
    }
}

impl<'a> IntoIterator for &'a InputEvents {
    type Item = &'a InputEvent;
    type IntoIter = core::slice::Iter<'a, InputEvent>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// Keep InputEvents' remaining methods in a second impl block for clarity.
impl InputEvents {
    /// Number of events in this batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the batch is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Remove all events from this batch.
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_empty() {
        let e = InputEvents::empty();
        assert!(e.is_empty());
        assert_eq!(e.len(), 0);
    }

    #[test]
    fn push_and_iter() {
        let mut e = InputEvents::default();
        e.push(InputEvent::SetThrottle(0.5));
        e.push(InputEvent::CycleView);
        assert_eq!(e.len(), 2);
        let collected: Vec<_> = e.iter().copied().collect();
        assert_eq!(
            collected,
            vec![InputEvent::SetThrottle(0.5), InputEvent::CycleView]
        );
    }

    #[test]
    fn from_slice_copies() {
        let events = [InputEvent::SetPitch(0.25), InputEvent::ResetLook];
        let e = InputEvents::from_slice(&events);
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn clear_resets() {
        let mut e = InputEvents::default();
        e.push(InputEvent::CycleView);
        e.clear();
        assert!(e.is_empty());
    }
}
