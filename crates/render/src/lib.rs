//! flowPlan renderer: wgpu pipelines, passes, HUD, camera.
//!
//! Phase 0 placeholder. See [`DESIGN.md § Rendering Architecture`](../../../DESIGN.md#rendering-architecture).
//!
//! `unsafe` is permitted here only at documented wgpu lifetime-management sites; every
//! `unsafe` block must carry a `// SAFETY: ...` comment per `CODING_RULES.md`.
