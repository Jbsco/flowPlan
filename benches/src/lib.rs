//! flowPlan benches crate root.
//!
//! The real benchmarks live in `frame_budget.rs` (and future `audio_callback.rs`,
//! `sim_tick.rs`). This library exists only to anchor the workspace member and host
//! shared fixture helpers as they appear.

#![forbid(unsafe_code)]

// Phase 0: the lib body is empty. We route `flowplan-core` through the crate root with
// an anonymous marker import so cargo recognizes it as used. Bench targets bring in
// `criterion` directly; the `#[cfg(test)]` marker keeps the lib-test compilation unit
// quiet about the inherited dev-dep.
use flowplan_core as _;

#[cfg(test)]
use criterion as _;
