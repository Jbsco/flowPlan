//! flowPlan audio: Glicol graph, cpal stream, lock-free SPSC reader.
//!
//! Phase 0 placeholder. Full implementation lands in Phase 3 per [`ROADMAP.md`](../../../ROADMAP.md).
//! See [`DESIGN.md § Audio Architecture`](../../../DESIGN.md#audio-architecture).
//!
//! `unsafe` is permitted here only for cpal callback trampolining (framework-required);
//! every `unsafe` block must carry a `// SAFETY: ...` comment per `CODING_RULES.md`.
