# flowPlan Coding Rules

These rules apply to **all** code contributed to flowPlan, whether written by a human or an AI agent. They are requirements, not suggestions. Code that violates them must not be merged.

The rules exist because flowPlan has strict architectural constraints (see [DESIGN.md](DESIGN.md#invariants)) and tight performance targets (see [DESIGN.md](DESIGN.md#performance-budget)). Agents are fast and capable but accumulate technical debt without guardrails; the rules here are the guardrails.

## Contents

- [Language and Toolchain](#language-and-toolchain)
- [Architecture Rules](#architecture-rules)
- [Type Discipline](#type-discipline)
- [Code Style](#code-style)
- [Real-Time Discipline](#real-time-discipline)
- [Determinism Discipline](#determinism-discipline)
- [Unsafe and FFI](#unsafe-and-ffi)
- [Error Handling](#error-handling)
- [Logging and Tracing](#logging-and-tracing)
- [Dependencies](#dependencies)
- [Testing Requirements](#testing-requirements)
- [Git and Commit Discipline](#git-and-commit-discipline)
- [What Not to Do](#what-not-to-do)
- [Agent-Specific Rules](#agent-specific-rules)

---

## Language and Toolchain

- **Rust, stable channel.** MSRV is the channel specified in `rust-toolchain.toml`. Do not require nightly features.
- **Target platforms:** Linux first, Windows and macOS second, Steam Deck verified before v1. Do not introduce platform-specific code paths that preclude any of these.
- **Cargo workspace.** One crate per subsystem per [DESIGN.md](DESIGN.md#directory-structure). Dependencies are declared at the crate level, not the workspace root (workspace only for version unification).
- **No `build.rs` tricks** beyond trivial asset-baking invocations. If a task seems to need a complex `build.rs`, propose an amendment first.

### Lints and formatting

Configured in `Cargo.toml` / `.clippy.toml` / `.rustfmt.toml`. Run before every commit:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Required clippy settings (lints promoted to error):

- `clippy::pedantic` enabled except specific allow-list below
- `clippy::cargo` enabled
- `clippy::nursery` enabled
- `clippy::alloc_instead_of_core`, `clippy::unwrap_used`, `clippy::expect_used` in non-test code
- `clippy::float_cmp`, `clippy::cast_*_loss` error-level (opt out per-line with justification only)

Allowed exceptions (configured, not inline):

- `clippy::module_name_repetitions` (cosmetic)
- `clippy::missing_errors_doc`, `clippy::missing_panics_doc` (too noisy in game code)
- `clippy::similar_names` (vector/matrix math uses similar names legitimately)

### Lint suppression policy

**When a lint fires, fix it. Do not suppress it.**

- Inline `#[allow(...)]` requires an adjacent comment explaining why, and reviewer agreement.
- Crate-level `#![allow(...)]` requires an entry in this document and a justifying comment.
- `#[cfg_attr(not(test), deny(...))]` is the preferred way to enforce stricter rules outside tests.

---

## Architecture Rules

These correspond to the dependency rules in [DESIGN.md](DESIGN.md#dependency-rules). They are enforced by automated tests (see [TEST_PLAN.md](TEST_PLAN.md)).

1. **Crate dependencies follow the DAG:** `app -> {render, audio, world, sim, assets, core}`, `render -> {world, sim, assets, core}`, `audio -> {sim, core}`, `world -> {sim, core}`, `sim -> core` (and `assets` for handle types only), `assets -> core`. `core` depends on no flowPlan crate.
2. **`render` never imports from `audio`, and vice versa.**
3. **`sim` never imports wgpu, winit, or cpal.**
4. **`audio` never imports wgpu or winit.**
5. **`core` never imports any other flowPlan crate.**
6. **No crate re-exports types from another flowPlan crate** just to shorten imports. Explicit is better.
7. **No catch-all "util" crate.** If a utility fits in `core` (math, handles, time) it goes there; otherwise it lives in the crate that uses it.

### Module structure within a crate

- One public module per top-level responsibility.
- Private helpers go in `pub(crate)` modules.
- `lib.rs` contains only re-exports and crate-level docs.
- No `prelude` modules in flowPlan crates; users of a crate import what they need.

---

## Type Discipline

All code is fully typed. Rust type inference is welcome inside function bodies; function signatures are explicit.

- **No `impl Trait` returns in public APIs** unless there is no other option. Named types are easier to mock, test, and document.
- **Newtype wrappers over bare primitives** for anything with semantic meaning: `struct TickCount(u64)`, `struct Meters(f32)`, `struct RadiansPerSecond(f32)`. No `f32` parameters called `time`.
- **Handles, not raw pointers.** Asset references are `Handle<T>` (see [DESIGN.md](DESIGN.md#asset-pipeline)).
- **`#[repr(C)]` or `#[repr(transparent)]` required on any struct that crosses to the GPU** or through `bytemuck`. Document why.
- **Option<T> is the only acceptable nullability.** No magic sentinel values (`u32::MAX` for "none", `-1.0` for "unset") without a documented reason.
- **`Result` carries errors, `Option` carries optionality.** They are not interchangeable.
- **No `Any`, no trait objects in hot paths.** Trait objects in cold/setup code are fine; add a comment if one appears anywhere touched per frame.

### Parse, don't validate

Follow the "parse, don't validate" pattern for data coming in from outside the trust boundary: untrusted input becomes a *different type* once validated, so downstream code receives evidence of validation by construction.

```rust
struct RawUrl(String);
struct ValidUrl(String);  // Only constructible via `validate`
fn validate(raw: RawUrl) -> Result<ValidUrl, Error>;
```

flowPlan analogs: `RawInput -> InputEvent`, `RawGltf -> MeshData`, `RawShaderSource -> CompiledPipeline`.

---

## Code Style

### Formatting

- 100-column soft limit (enforced via `rustfmt`).
- One statement per line.
- Trailing commas in multi-line lists.
- No manual column-alignment of `=` or field values.

### Naming

- `snake_case` functions, variables, modules.
- `CamelCase` types and traits.
- `SCREAMING_SNAKE_CASE` constants and statics.
- Acronyms are treated as words (`Gpu`, `Http`, not `GPU`, `HTTP`). Exception: domain acronyms where SI or industry convention demands capitals (`LODLevel` is acceptable; prefer avoiding).

### File and function size

- Files should fit comfortably on one screen scroll. Long enum matches and data tables are exempt; prose logic is not.
- Functions target <=40 lines of body. Longer is acceptable for straight-line code (shader pipeline setup, draw calls); refactor when logic forks.
- Cyclomatic complexity target 10. Measured via clippy or a dedicated tool.

### Comments

Default to writing no comments. Write one only when the WHY is non-obvious:

- Subtle invariants (`// Must be called before audio init; otherwise cpal loses the device.`)
- Workarounds for specific bugs (`// wgpu #1234: this validation is stricter than spec requires.`)
- Math or domain references (`// Gerstner wave form per GPU Gems 2, Ch. 1.`)

**Do not** write comments that restate what the code does (`// increment counter`). Do not reference the current task, PR, or callers in code comments -- those belong in commit messages or PR descriptions.

### Docstrings

- Public items have `///` docs describing purpose and any non-obvious semantics.
- Crate-level `//!` docs describe the crate's role in the layer architecture and link to [DESIGN.md](DESIGN.md).

---

## Real-Time Discipline

This is the most load-bearing section for flowPlan's design goals. Violations here can silently break 240 Hz targets.

### Hot paths

The **render hot path** = everything from `accumulator update` through `queue submit`.
The **audio hot path** = everything inside the cpal callback.
The **sim hot path** = everything inside `SimState::tick()`.

### Absolute prohibitions in hot paths

- **No heap allocations.** That means no `Box`, no `Vec::push` that can grow, no `format!`, no `to_string`, no `collect::<Vec<_>>` on an unbounded iterator, no `HashMap::insert` that can grow, no `String` creation.
- **No locks** (audio and render paths only; sim tick is single-threaded but still no locks inside).
- **No syscalls** beyond what the framework itself makes (winit/wgpu/cpal).
- **No logging** in audio. `tracing::trace!/debug!` is permitted in render and sim but must be gated by level so shipping builds strip them.
- **No `unwrap()` / `expect()` on anything that can actually fail at runtime.** Compile-time invariants (like integer cast from a constant) are fine.
- **No RNG reseeding.** Seeds are established at startup or on sim reset.
- **No clock reads** in sim (sim uses its own fixed-step clock, not wall time).

### Preallocation patterns

- **Arenas** (via `bumpalo` or hand-rolled) for per-frame data: draw lists, particle instances, debug line segments.
- **Fixed-capacity pools** for entities: planes, cargo, birds, water splashes. Use a free-list; when the pool is full, new allocations are dropped (with a counter increment and a dev-only warning path).
- **Growable only at phase boundaries.** A collection that might need to grow does so during level load or sim reset, not during tick/render/callback.

### Alloc enforcement

In debug builds, use `assert_no_alloc` (or a hand-rolled allocator wrapper) around hot paths. Any allocation during a guarded block is a panic; the panic locates the offender. Release builds are unaffected.

### Audio callback rules (stricter than render)

Inside the `cpal` callback, additionally:

- No panics on malformed parameter updates. Clamp, ignore, or reset to a safe default.
- No atomic stores that could race with the sim side in ways that produce inaudible-but-undefined behavior; use `rtrb` or equivalent SPSC primitives which publish via `Release`/`Acquire` ordering.
- No trait-object dispatch for the DSP graph in the callback hot loop. The Glicol graph is built at setup; the callback walks it with static dispatch where possible.

### Frame budget checkpoints

- Every crate adds its own `tracing` spans at subsystem entry points.
- Tracy is integrated in dev builds; profile early, profile often.
- Per-phase CI runs a benchmark scene against the reference hardware budget (see [TEST_PLAN.md](TEST_PLAN.md)).

---

## Determinism Discipline

Determinism is a first-class invariant ([DESIGN.md Invariant #8](DESIGN.md#invariants)). The flight bench, world-gen visual bench, save-format replay, and Phase 6 stretch-goal multiplayer all depend on it. Violations are **bugs**, not style preferences.

### Scope: what determinism flowPlan actually guarantees today

**Within-platform determinism (VERIFIED):** for a given `(seed, inputs, dials, tick, min_altitude)` on a single platform, flowPlan produces bit-identical state across runs. The flight bench enforces this; every PR runs it. Violations are caught at CI.

**Cross-platform bit-identity (NOT YET ACHIEVED):** running the same scenario on Linux, Windows, and macOS produces **numerically close but byte-different** CSV output. Root cause: glam's internal transcendentals (`Vec3::normalize`, `Quat::from_rotation_x`, `.to_euler()`, etc.) route through each platform's stdlib (MSVC CRT vs glibc vs Apple libsystem), which differ at single-precision rounding. Glam 0.29's `libm` feature only adds the dep; it doesn't re-route internals under `std`.

Full cross-platform bit-identity is **Phase 6.1 determinism hardening** work — it requires patching glam (or a soft-float math layer) to force libm everywhere. Blocked until multiplayer lockstep is actually load-bearing (Phase 6 stretch-goal).

The [cross-platform determinism CI workflow](../.github/workflows/determinism.yml) runs the bench on Linux / Windows / macOS-aarch64 each commit; the diff job is marked `continue-on-error` so divergence is reported as a warning, not a failure, until Phase 6.1 closes it.

### Divergence sources (documented; partially mitigated)

- **Transcendental stdlib drift** across MSVC CRT / glibc / Apple libsystem / musl. Our direct code uses `libm` (mitigated). glam internals don't (unmitigated; Phase 6.1).
- **SIMD architecture variance** (SSE vs NEON vs AVX). glam's `scalar-math` feature is an option if needed in 6.1.
- **FMA contraction** — `a*b+c` → single fused op. Rust defaults `fp-contract=off` at language level; `-Ctarget-feature=-fma` on x86 targets would disable ISA-level FMA but was not observed to affect output in our benches (dead code). Re-investigate in 6.1 if cross-arch is truly gated on this.
- **`-ffast-math` / `/fp:fast`** — allows reassociation, FMA contraction, skipped NaN handling; breaks determinism hard. **Never use.** (We don't, and wouldn't.)

### Rules (apply today)

1. **Use the `libm` crate for transcendentals in sim hot paths + world-gen paths.** Pure Rust, platform-independent implementations of `sin`, `cos`, `atan2`, `exp`, `log`, `sqrt`, `hypot`, `powf`. In use: `flight.rs`, `terrain.rs`, `water.rs`, `sky.rs`, `flight_bench.rs`. Modest perf cost; zero risk.

    ```rust
    libm::sinf(x)   // not x.sin() on f32
    libm::atan2f(y, x)  // not y.atan2(x)
    ```

    This is partial mitigation — it doesn't reach glam internals — but it's correct discipline for our own code, and it prevents our direct calls from contributing to divergence.

2. **Pin `codegen-units = 1` + `lto = "thin"` in `[profile.release]`** (already done in flowPlan's Cargo.toml). Prevents cross-unit optimization variation between builds.

3. **CPU-GPU shared samplers must use identical math both sides.** Where a world-gen sampler (e.g. `world::cloud_density`) is consumed on both CPU (sim) and GPU (shader), the WGSL implementation must match the Rust implementation bit-for-bit. Use the same noise formulas, same constants, same transcendentals (libm on CPU; hand-rolled WGSL equivalent on GPU — WGSL's built-in trig is not guaranteed bit-identical across drivers).

4. **No wall-clock reads in sim or world-gen hot paths.** Use `SimState.wave_time` / `SimState.tick` / explicit function parameters. No `Instant::now()`, `SystemTime::now()`, or thread-local timing. (Presentation-side smoothing in render or HUD is fine — just not in deterministic paths.)

5. **Seeded RNG only.** PCG or xorshift via flowPlan's `core::rng`; seed derivation via `hash(master_seed, subsystem_name)`. No `thread_rng()`, no `rand::random()`, no OS entropy.

6. **Hashing function for seed derivation must be platform-independent.** `SipHash` or `wyhash`; reject anything using the Rust stdlib `DefaultHasher` (not portable across stdlib versions).

### Verification

- **Flight bench** ([TEST_PLAN.md § Flight Test Bench](TEST_PLAN.md#flight-test-bench)): within-platform bit-identity enforced on every push via CI.
- **World-gen visual bench** ([TEST_PLAN.md § World-Gen Visual Bench](TEST_PLAN.md#world-gen-visual-bench)): within-platform bit-identity for the world-bake pipeline. Lands with Phase 3.6.1.
- **Cross-platform bit-identity** (Linux == Windows == macOS): **deferred to Phase 6.1**. The CI workflow reports divergence today as a warning (non-blocking).

### What investigations in Phase 3 Step 1 revealed (documented for Phase 6.1)

This section retains the forensic findings from the Phase 3 Step 1 cross-platform determinism investigation, so Phase 6.1 starts with full context rather than repeating the exploration.

- `-Ctarget-feature=-fma`, `-Cllvm-args=-fp-contract=off`, `-Cllvm-args=-vectorize-loops=false`, `-Cllvm-args=-vectorize-slp=false` — all had zero effect on bench output. Rust release builds already disable fp-contract at the language level; LLVM vectorization appears not to be the source of divergence.
- glam's `libm` feature + `scalar-math` feature **changed** Linux output (from pre-libm baseline), but did **not** bring Windows x86_64 in line with Linux x86_64. Indicates glam still calls stdlib transcendentals under `std`; the `libm` feature only adds the dep.
- Root cause for x86_64-cross-OS divergence: stdlib transcendentals inside glam (MSVC CRT on Windows ≠ glibc on Linux). glam needs patching or replacement.
- Root cause for aarch64 additional divergence: likely the same plus NEON vs SSE when glam has SIMD enabled. `scalar-math` mitigates SIMD; stdlib transcendentals remain.

Phase 6.1 plan: patch glam's scalar-math paths to force libm, via `[patch.crates-io]` fork; or introduce a thin math-abstraction layer over glam that the hot paths go through; or adopt a different math crate that provides libm routing under `std`.

### What about presentation-side non-determinism?

**Allowed** in render / HUD / audio feedback paths:
- Wall-clock-driven animations (cel outline shimmer, debug HUD readout smoothing).
- Rolling averages / needle lag for instrument readouts.
- Non-deterministic render timing (frame interpolation alpha from a wall-clock accumulator is expected).

These must not feed back into sim state. If a presentation smoother's output is ever fed back into `SimState.tick()`, you've created a determinism violation — redirect it through a deterministic intermediary first.

---

## Unsafe and FFI

`unsafe` is permitted only at documented sites enumerated in [DESIGN.md](DESIGN.md#approved-unsafe-boundaries). Every `unsafe` block has:

```rust
// SAFETY: <why the memory/thread/aliasing invariants hold>
unsafe { /* ... */ }
```

- `unsafe fn` declarations get a doc comment specifying what the caller must uphold.
- New `unsafe` sites require the same review attention as a new dependency: proposed in a PR, justified, with reviewer agreement.
- `#[allow(unsafe_code)]` is never applied crate-wide. `#![forbid(unsafe_code)]` is the default at the crate level; opt-in only where necessary (`render/` and maybe `audio/`).

### `bytemuck` patterns

- Use `bytemuck::Pod + Zeroable` derives where possible.
- Manual `#[repr(C)]` with cast via `bytemuck::cast_slice` is acceptable for GPU upload; comment on alignment if the struct has nontrivial packing.

---

## Error Handling

- **`Result<T, E>` with explicit error types per crate.** Use `thiserror` for deriving. No `Box<dyn std::error::Error>` in internal APIs.
- **`anyhow` is permitted at the `app` layer only**, for top-level error reporting. No `anyhow` inside library crates.
- **Panics are bugs.** The only acceptable panic sites are:
    - Compile-time invariants violated at runtime (e.g., integer constant out of range -- vanishingly rare).
    - Startup assertions that flowPlan cannot operate without (e.g., no GPU adapter).
    - Test code.
- **No silent failures.** If an operation fails in a way the user should know about, `tracing::error!` in addition to returning the error.
- **Recovery paths are explicit.** Where flowPlan can recover (asset load fails -> use placeholder, device lost -> reinit), the recovery is documented at the error site.

### Panic vs. return on audio callback?

Audio callback never panics on malformed input. Clamp or ignore. Panics there crash the audio thread and, depending on platform, the process.

---

## Logging and Tracing

- **`tracing`** is the logging framework. No `println!` / `eprintln!` outside debug-only scaffolding (and even then, prefer `tracing::debug!`).
- **Structured fields**, not interpolated strings, for anything an operator might want to grep.
- **Span discipline:** one span per subsystem entry point (tick, frame, audio callback is *not* spanned in release builds -- too hot). Spans are `tracing::info_span!` or `trace_span!` depending on level.
- **Tracy integration via `tracing-tracy`** is dev-only.
- **No logging in the audio callback.** Ever. Use an atomic counter and a once-per-second drain in the main thread if you need audio-side instrumentation.

---

## Dependencies

Every dependency is a maintenance burden and a potential vector for churn.

- **Approved dependencies** are enumerated in [DESIGN.md](DESIGN.md#tech-stack). Adding a new one requires an amendment to that doc and a justification in the PR.
- **"It would save a few lines"** is not justification. A dependency must solve a problem that is hard to solve correctly in-house.
- **Pin exact versions in `Cargo.toml`** (`= "1.2.3"` or `^1.2.3` with good reason). Use `cargo update` deliberately, not casually.
- **License audit on first add.** Track every dependency's license. MIT/BSD/Apache-2.0 compatible only; GPL/AGPL are incompatible with v1's shipping plans unless we decide otherwise.
- **No `*` dependencies, no git dependencies** in shipping builds. Git dependencies are acceptable temporarily during local prototyping with a TODO to switch to a crates.io release.

---

## Testing Requirements

See [TEST_PLAN.md](TEST_PLAN.md) for the full strategy. The non-negotiable minimums here:

1. **Every new public function in `core`, `sim`, and `world` gets at least one unit test.**
2. **Every bug fix includes a regression test.** Test fails before the fix, passes after.
3. **Property tests** for pure functions where generative coverage helps (math, canonicalization, deterministic simulation round-trips).
4. **Frame-budget tests** for any render change that touches a pass structure.
5. **Allocation assertions** wrap hot paths in debug builds. A new hot-path function that can allocate is rejected at review.
6. **Tests run clean:** `cargo test --workspace` is a pass criterion.
7. **Coverage target:** 80% for `core`, `sim`, `world` (the deterministic, testable layers). Lower expectations for `render`, `audio`, `app` which need real hardware.

---

## Git and Commit Discipline

- **One logical change per commit.** No mixing refactor + feature. No mixing formatting + bugfix.
- **Commit subject:** imperative mood, capital letter, no trailing period, <=72 characters. Examples: `Add Gerstner water vertex shader`, `Fix pontoon buoyancy sampling during pitch`.
- **Commit body:** explains *why*, not *what*. The diff shows what changed.
- **Linear history within branches.** Rebase rather than merge from `main` into a feature branch.
- **Do not commit:** `target/`, build artifacts, shader binaries checked in by mistake, `.tracy` captures, asset caches, personal editor configs.
- **Do not commit secrets.** flowPlan has no secrets in v1; if something looks like a secret (API key, cloud token), do not add it.

### Commit granularity for agents

Agents produce iterative WIP. That iteration lives in the working tree, not git history.

- **Do not commit until the quality gate passes** (cargo fmt / clippy / test). Use `git stash` if you need to context-switch with dirty state.
- **Prefer one polished commit per phase sub-step** over a noisy WIP trail.
- **Amending before push is acceptable.** Amending after push is not.

### TODO policy

- **No `TODO`, `FIXME`, `XXX`, or `HACK` markers in committed code** except the one allowed form below.
- **Allowed:** `TODO(phase-N): <one-line description>` referencing an item in [ROADMAP.md](ROADMAP.md). For capabilities the current phase defers, not for unfinished work in the current change.
- **No commented-out code.** Delete it. Git remembers.

### What to review per PR

Each PR implements one phase step or one logical sub-step. Reviewers verify, in this order:

1. **Spec traceability.** Every change traces to a section of PLAN/DESIGN/CODING_RULES/ROADMAP/TEST_PLAN. No speculative features.
2. **Quality gate green.** `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` all pass.
3. **Hot path respected.** No new allocations, no new locks, no new panics in render/audio/sim.
4. **Tests cover failure paths**, not just happy paths.
5. **No suppression escapes.** No undocumented `#[allow]`, no `unwrap()` in non-test code, no unjustified new deps.
6. **History hygiene.** Single logical commit (or a clean short series), message explains *why*, no merge commits from main.

---

## What Not to Do

These anti-patterns are common in agent-generated game code and must be avoided:

1. **Do not build an engine.** flowPlan is a game, not a framework. Generalization requires three concrete instances first.
2. **Do not add abstractions for hypothetical future requirements.** Concrete, then general. Three similar lines is better than a premature trait.
3. **Do not add configuration knobs that aren't needed.** Every config option is an untested code path. Prefer opinionated defaults.
4. **Do not allocate in hot paths.** See [Real-Time Discipline](#real-time-discipline). This is the most common cold-start agent mistake.
5. **Do not hold locks in the audio callback.** Ever. Under any circumstance.
6. **Do not reach across layer boundaries.** `render` does not mutate `sim`. `audio` does not read `wgpu`. `sim` does not invoke the renderer.
7. **Do not add features beyond what was asked.** A bug fix does not need surrounding cleanup. A new shader does not need a surrounding system.
8. **Do not leave commented-out code.** Delete it.
9. **Do not add backwards-compatibility shims.** If code is unused, delete it. No `_old_function` aliases.
10. **Do not test through the renderer.** Test `sim` and `core` directly. Use render-free integration tests.
11. **Do not use `unwrap`/`expect` outside tests and compile-time invariants.**
12. **Do not skip writing tests** because "it's simple." Write the test.

---

## Agent-Specific Rules

These supplement, not replace, all rules above.

1. **Read the specs before writing any code.** [CLAUDE.md](CLAUDE.md) lists the order. "Skim is not read."
2. **No speculative features.** Every line traces to the design documents.
3. **Write tests alongside implementation, not after.** Tests are part of the deliverable, not a follow-up.
4. **Run the full gate locally before reporting a task complete.** `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.
5. **No new dependencies without human approval.** If a task seems to need one, stop and propose an amendment.
6. **Stay within the established crate structure.** Do not create new top-level crates or reorganize existing structure without approval.
7. **When in doubt: smaller, simpler, fewer dependencies, no engine-ification.** The right answer is almost always less code.
8. **If a spec doc is ambiguous, propose a clarifying amendment and stop.** Do not guess.
9. **No shell, git, or remote operations unless explicitly requested.** No commits, no pushes, no PR creation.
10. **Report honestly.** If you did not finish, say so. If tests fail, say which ones. Partial work with an accurate report is more valuable than "done" with hidden gaps.
