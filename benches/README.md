<!-- L0: frame-budget and subsystem benchmarks; authoritative on per-phase performance gates -->
# benches/

`criterion`-driven benchmarks. These are the performance gate for every phase per [TEST_PLAN.md § Frame-Budget Tests](../TEST_PLAN.md#frame-budget-tests).

## Structure

```
benches/
  frame_budget.rs           # Top-level frame-budget runner
  scenes/                   # Standardized scene fixtures
    scene_minimal.flow      # Flat terrain + 1 plane, baseline
    scene_standard.flow     # Curated archipelago segment, typical
    scene_stress.flow       # Dense particles, full water tessellation, 10+ planes
  audio_callback.rs         # Synthetic audio callback timing
  sim_tick.rs               # Sim tick cost across representative states
```

Scenes hold `.gitkeep` until the Phase 2-3 milestones author them. The `*.rs` files are created alongside the corresponding phase work.

## Running

- Interactive: `cargo bench --bench frame_budget`
- CI / gate mode: `cargo bench --bench frame_budget -- --test` (runs each scene once, fails on budget breach)

## Gates per phase

See [ROADMAP.md § Per-Phase Quality Gate](../ROADMAP.md#per-phase-quality-gate). Summary of bench expectations:

| Phase | Scenes to pass            | Hardware tier      |
| ----- | ------------------------- | ------------------ |
| 0-1   | scene_minimal             | minimum            |
| 2     | scene_minimal + standard  | reference          |
| 3+    | all three                 | reference          |
| 5     | all three                 | min + reference + high, plus Steam Deck |

## Regression detection

Baseline results live in `benches/baseline.json` (committed). A 10% regression fails the PR gate. Intentional regressions require an explicit baseline amendment in the same PR with justification.

## Authoring a new bench

1. Check if an existing bench can be extended instead of adding a new one.
2. Fixtures go in `scenes/`; one `.flow` file per scene. Keep scenes deterministic.
3. Use `criterion::Criterion::bench_function` for single measurements; `bench_with_input` for parameter sweeps.
4. Frame-budget benches must use the same deterministic seed every run.
5. Audio callback and sim tick benches use synthetic state; no cpal or wgpu device required.

## Non-goals here

- Microbenchmarks of individual math operations (premature; if glam is the bottleneck we will hear it).
- GPU benchmarks without a real device (Tier 3 territory per [TEST_PLAN.md](../TEST_PLAN.md)).
- Profiling that lives in `tracing` spans -- those are observability, not gate criteria.
