<!-- L0: cross-crate integration tests; hardware-gated tier lives under tests/hardware -->
# tests/

Cross-crate integration tests at the workspace level. Unit tests live inside each crate (`#[cfg(test)] mod tests`); this directory holds tests that exercise interactions between crates.

## Organization (populated per phase)

```
tests/
  workspace_rules.rs        # Dependency-DAG enforcement (Phase 0)
  data_validation.rs        # .ron schema + invariant checks (Phase 4)
  input_queue_drain.rs      # Input -> sim integration (Phase 1)
  sim_audio_ring.rs         # SPSC ring producer/consumer integration (Phase 3)
  save_roundtrip.rs         # Serialize -> deserialize -> identical state (Phase 4)
  hardware/                 # GPU / audio / window dependent, feature-gated
    pipeline_creation.rs
    audio_stream_open.rs
    visual_snapshots.rs
```

Test files get created alongside the phase that introduces the capability they cover.

## Tier alignment

These correspond to tiers in [TEST_PLAN.md § Test Tiers](../TEST_PLAN.md#test-tiers):

- **Tier 1 (unit)** -- lives inside each crate, not here.
- **Tier 2 (integration, no hardware)** -- most of this directory.
- **Tier 3 (hardware)** -- `hardware/` subdirectory; gated by `#[cfg(feature = "hardware-tests")]` or by a skip on missing device.
- **Tier 4 (bench)** -- lives in [benches/](../benches/).

## Architectural enforcement tests

The `workspace_rules.rs` test is load-bearing -- it parses each crate's `Cargo.toml` and enforces the dependency DAG from [DESIGN.md § Dependency Rules](../DESIGN.md#dependency-rules). Without it, architectural drift is invisible.

Forbidden patterns this test catches:

- `core` depends on any other flowPlan crate.
- `sim` depends on wgpu / winit / cpal.
- `audio` depends on wgpu / winit.
- Any crate depends on `app`.
- Any back-edges in the DAG.
- Unpinned dependency versions (`*`) or git dependencies in shipping builds.
- Forbidden crate imports (bevy, fyrox, tokio, async-std, etc.) per the non-goals.

## Running

- `cargo test --workspace` -- Tier 1 + Tier 2.
- `cargo test --workspace --features hardware-tests` -- adds Tier 3.
- `cargo bench --bench frame_budget -- --test` -- Tier 4 gate mode.

## Non-goals here

- Duplicating per-crate unit tests.
- Flaky timing tests; use determinism tests instead.
- Tests that depend on specific paths on disk (hermetic fixtures only, per [TEST_PLAN.md § Test Fixture Strategy](../TEST_PLAN.md#test-fixture-strategy)).
