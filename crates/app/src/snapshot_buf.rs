//! Double-buffered snapshot used for sim -> render handoff.
//!
//! The sim writes to `current` at the end of each tick; render reads `previous` +
//! `current` and interpolates by alpha. Phase 1 runs the sim on the main thread, so the
//! structure is just two `Copy` structs behind a mutex that is held for under a
//! microsecond at tick boundaries. Phase 3 moves sim to a worker thread; this structure
//! is ready for that without changes in its public API.

#![deny(unsafe_code)]

use flowplan_sim::SimSnapshot;
use parking_lot::Mutex;

/// A double-buffered snapshot pair.
pub struct SnapshotBuffer {
    state: Mutex<State>,
}

#[derive(Clone, Copy)]
struct State {
    previous: SimSnapshot,
    current: SimSnapshot,
}

impl SnapshotBuffer {
    /// New buffer seeded with `initial` for both slots.
    pub fn new(initial: SimSnapshot) -> Self {
        Self {
            state: Mutex::new(State {
                previous: initial,
                current: initial,
            }),
        }
    }

    /// Publish a new snapshot; the old `current` becomes `previous`.
    pub fn publish(&self, next: SimSnapshot) {
        let mut s = self.state.lock();
        s.previous = s.current;
        s.current = next;
    }

    /// Read out the pair for rendering. Called on the main thread.
    pub fn read(&self) -> (SimSnapshot, SimSnapshot) {
        let s = self.state.lock();
        (s.previous, s.current)
    }
}
