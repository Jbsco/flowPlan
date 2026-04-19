//! Phase 0 smoke bench: validates the criterion + workspace plumbing only.
//!
//! A real frame-budget bench arrives in Phase 2 (see
//! [`ROADMAP.md § Per-Phase Quality Gate`](../ROADMAP.md#per-phase-quality-gate) and
//! [`TEST_PLAN.md § Frame-Budget Tests`](../TEST_PLAN.md#frame-budget-tests)).
//!
//! For now, exercises a simple deterministic RNG and tick-count arithmetic so the bench
//! infrastructure reports something sensible when run.

use criterion::{criterion_group, criterion_main, Criterion};
use flowplan_core::{Rng, TickCount};

// Bench target sees the `flowplan-benches` lib as a dep by virtue of being in the same
// package; acknowledge it so `-W unused-crate-dependencies` stays quiet until the lib
// hosts fixture helpers.
use flowplan_benches as _;

fn rng_sequence_smoke(c: &mut Criterion) {
    c.bench_function("core::rng 1k u64", |b| {
        b.iter(|| {
            let mut r = Rng::from_seed(0xDEAD_BEEF_CAFE_F00D);
            let mut acc = 0u64;
            for _ in 0..1000 {
                acc = acc.wrapping_add(r.next_u64());
            }
            acc
        });
    });
}

fn tick_advance_smoke(c: &mut Criterion) {
    c.bench_function("core::time tick advance 10k", |b| {
        b.iter(|| {
            let mut t = TickCount::ZERO;
            for _ in 0..10_000 {
                t = t.next();
            }
            t
        });
    });
}

criterion_group!(frame_budget_smoke, rng_sequence_smoke, tick_advance_smoke);
criterion_main!(frame_budget_smoke);
