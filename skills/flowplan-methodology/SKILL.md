---
name: flowplan-methodology
description: Design-first development discipline for flowPlan. Use when working in the flowPlan repo, proposing changes, reviewing PRs, or scoping new work. Encodes invariant-first thinking, non-goals as load-bearing, phased execution with testable gates, hot-path discipline, and the "no vibe coding" rule that every line traces to the specification.
---

# flowPlan Methodology

The engineering discipline that keeps flowPlan on rails. This skill is about **how** to work in the repo, not **what** to build. The what lives in [PLAN.md](../../PLAN.md) + [DESIGN.md](../../DESIGN.md) + [ROADMAP.md](../../ROADMAP.md).

This methodology is derived from two precursor projects:

- **hadlink** -- a formally verified URL shortener. Skeleton-first commit, contract-first design, invariants as structural guarantees, non-goals as scope protection, phased roadmap with testable gates.
- **admt** -- a CLI tool built through multi-agent design convergence. Four foundational spec docs (architecture, coding rules, roadmap, test plan) as the single source of truth. Code that does not trace to the spec is rejected.

Both projects succeeded because the design was done before the code. flowPlan follows the same principle, adapted for real-time game development where the constraint system is frame budget + determinism + hot-path discipline rather than formal proofs.

## Core Rules

1. **Read the specs first.** Read in the order declared by [CLAUDE.md](../../CLAUDE.md). Skim is not read. No code before the specs are understood.

2. **Every line of code traces to a spec section.** If a change is not described in PLAN / DESIGN / CODING_RULES / ROADMAP / TEST_PLAN, either it shouldn't be made or the spec needs an amendment first. No vibe coding.

3. **Invariants are structural, not aspirational.** See [DESIGN.md § Invariants](../../DESIGN.md#invariants). Violating an invariant is a bug, not a style preference. Design changes that would violate an invariant require explicitly amending the invariant.

4. **Non-goals are load-bearing.** See [DESIGN.md § Non-Goals](../../DESIGN.md#non-goals). They resist feature creep and communicate intent. Requests conflicting with non-goals are closed with explanation, not negotiated. This is scope protection, not personal judgment.

5. **Phases have gates; gates are binary.** See [ROADMAP.md § Per-Phase Quality Gate](../../ROADMAP.md#per-phase-quality-gate). A phase is done when the gate passes. Not when the code "looks done."

6. **Tests alongside implementation, not after.** See [TEST_PLAN.md](../../TEST_PLAN.md). Tests are part of the deliverable.

7. **Hot paths are sacred.** See [CODING_RULES.md § Real-Time Discipline](../../CODING_RULES.md#real-time-discipline). No allocations, no locks, no logging in audio callbacks, render submit, or sim tick. Agents regress this the most; guard it first.

8. **Small, traceable commits.** One logical change per commit. Imperative subject, capital letter. Explain *why*, not *what*.

## When Asked to Start New Work

Before writing anything:

1. **Identify the phase** in [ROADMAP.md](../../ROADMAP.md). If the work does not map to a current phase milestone, stop and propose a phase amendment.
2. **Identify affected subsystems.** Cross-check against [DESIGN.md § Subsystems](../../DESIGN.md#subsystems) and [§ Dependency Rules](../../DESIGN.md#dependency-rules).
3. **Check for invariant impact.** Does the work affect fixed timestep, determinism, snapshot immutability, audio lock-freeness, or frame budget? If yes, call it out up front.
4. **Check for non-goal drift.** Does the work smell like engine-building, configuration-as-feature, or scope expansion? If yes, stop and check.
5. **Plan the test strategy.** Which tier ([TEST_PLAN.md § Test Tiers](../../TEST_PLAN.md#test-tiers))? Which specific tests get added?
6. **Then, and only then, write code.**

## When a Spec Doc is Ambiguous

Do not guess. Propose a clarifying amendment to the relevant spec doc as a separate, small PR (or as the first commit in a larger PR). Wait for the amendment to be merged, or confirm with the human, before implementing.

Guessing produces divergence. Divergence produces rework. Rework defeats the point of front-loaded design.

## When an Invariant Seems to Block You

If an invariant is in the way, the odds are that:

- The design is right, and you are trying to do something the design says should not be done -- step back.
- The design is incomplete, and the invariant needs an explicit amendment with rationale.

It is *very* rare that an invariant is "wrong." More often, the feature you are building is outside flowPlan's scope. See non-goals.

## Scope Discipline Rules

From the hadlink whitepaper ("From Coursework to Craft: Single-Sprint Projects as Engineering Practice") and applied here:

- **The constraint is the feature, not the limitation.** 240 Hz on mid-range GPU with 60% utilization is a design goal, not an optimization afterthought.
- **Define scope honestly: what ships and what does not, decided before work begins.** See [ROADMAP.md](../../ROADMAP.md).
- **Resist expansion once work feels productive.** The moment you feel like "I could also add X" -- write it in the deferrals list ([DESIGN.md § Explicit Deferrals](../../DESIGN.md#explicit-deferrals-may-revisit-after-v1)), not in the code.
- **A finished game with clear boundaries beats an ambitious prototype that never ships.**

## Hot-Path Discipline (Zoom In)

This is where agents most commonly regress. See [CODING_RULES.md § Real-Time Discipline](../../CODING_RULES.md#real-time-discipline) for the authoritative rules. Shortlist:

- **No heap allocations.** No `Box`, no growing `Vec`, no `format!`, no `to_string`, no `collect::<Vec<_>>` on unbounded iterators inside `tick`, render submit, or the audio callback.
- **No locks in the audio callback.** No `Mutex::lock`, no `RwLock`, no `parking_lot::*` acquires. SPSC rings (`rtrb`) are the only cross-thread comm.
- **No syscalls in the audio callback.** No logging, no file I/O, no clock reads.
- **No panics on bad audio params.** Clamp, ignore, default -- never panic the audio thread.
- **No `unwrap()`/`expect()`** outside tests and compile-time invariants.

If you catch yourself writing `Vec::new()` inside a function that is called per-frame or per-tick, stop. Use an arena or a preallocated pool.

## Emergent Design Discipline

flowPlan is an emergent systems game (see [PLAN.md § Design Pillars](../../PLAN.md)). This means:

- **Do not script game events.** Weather, wind, fuel, wildlife, and daylight should *produce* situations. You build the rules, not the events.
- **When you feel tempted to hand-author a moment, ask:** "Can the simulation produce this naturally? What missing rule would let it?"
- **Prefer systems over content.** A new weather state that changes flight feel is better than a cutscene.

Before adding a scripted event trigger, propose a systems-level alternative. If the systems cannot produce it, ask whether it belongs in flowPlan at all.

## Feel-Over-Fidelity Discipline

When a fidelity decision and a feel decision conflict, feel wins.

- Response latency beats visual richness.
- Input responsiveness beats frame-rate convenience.
- A crisp cel outline at 240 Hz beats a realistic PBR frame at 60 Hz with drops.
- Plausible physics beats accurate physics.

If a feature adds fidelity at the cost of feel, it is rejected by default. Amendments require explicit justification and playtesting evidence.

## Reviewer Checklist (Per-PR)

Borrowed from admt's spec-traceability review pattern. Reviewers verify, in this order:

1. **Spec traceability.** Every change traces to a section of PLAN / DESIGN / CODING_RULES / ROADMAP / TEST_PLAN. No speculative features, no unrequested refactors.
2. **Quality gate green.** `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, and the phase's frame-budget bench all pass.
3. **Hot path respected.** No new allocations, no new locks, no new panics in render / audio / sim.
4. **Invariants preserved.** Fixed-timestep, determinism, snapshot immutability, audio lock-freeness, frame budget all still hold.
5. **Tests cover failure paths**, not just happy paths.
6. **No suppression escapes.** No undocumented `#[allow]`, no `unwrap()` in non-test code, no unjustified new deps.
7. **History hygiene.** Single logical commit (or a clean short series), message explains *why*, no merge commits from main.

## Anti-Patterns (Do Not)

These are specifically called out because they are common in agent-generated code:

1. **Engine-ification.** Generalizing before there are three concrete instances. Don't.
2. **Configuration as feature.** Every config option is an untested code path. Prefer opinionated defaults.
3. **Allocation by convenience.** `Vec::new()` in `tick()` because "it's only a few KB." No.
4. **Testing through the renderer.** Test `sim` and `core` directly.
5. **"Rework the architecture."** If you want to change a layer boundary, that is a design amendment, not a refactor PR.
6. **Silent scope expansion.** "While I was in here, I also..." No. One change per PR.
7. **Improvising on ambiguity.** If a spec is unclear, amend the spec first.
8. **Committing before the gate is green.** WIP lives in the working tree, not in git history.

## When In Doubt

- Re-read [DESIGN.md § Core Principle](../../DESIGN.md#core-principle) and [§ Invariants](../../DESIGN.md#invariants).
- Re-read the current phase in [ROADMAP.md](../../ROADMAP.md).
- If still unclear: stop and ask.

## References

- [PLAN.md](../../PLAN.md) -- creative vision
- [DESIGN.md](../../DESIGN.md) -- technical architecture
- [CODING_RULES.md](../../CODING_RULES.md) -- rules for all code
- [ROADMAP.md](../../ROADMAP.md) -- phased execution
- [TEST_PLAN.md](../../TEST_PLAN.md) -- verification strategy
- [CLAUDE.md](../../CLAUDE.md) -- agent entry point
