# CLAUDE.md

**If you are resuming mid-project:** read [PROGRESS.md](PROGRESS.md) first for the current phase, tech stack versions, conventions, and the list of bugs that must not return. It is the authoritative snapshot for session-compaction recovery.

Then read the six specification documents in full, in this exact order, before any work in this repository:

1. **[PLAN.md](PLAN.md)** -- creative vision, design pillars, and the "why". The mood, the loop, the thing being built.
2. **[DESIGN.md](DESIGN.md)** -- technical architecture, subsystems, dependency rules, invariants, non-goals. The "what".
3. **[CODING_RULES.md](CODING_RULES.md)** -- Rust + real-time game-dev rules for humans and agents. The "how".
4. **[ROADMAP.md](ROADMAP.md)** -- phased execution plan with binary milestones and per-phase gates. The "order".
5. **[TEST_PLAN.md](TEST_PLAN.md)** -- verification strategy: unit, property, frame-budget, playability. The "verify".
6. **[skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md)** -- the design-first discipline applied to game dev. The "discipline".

**No exceptions. Skim is not read.** These six documents are the specification. Code that does not trace to them is rejected.

When in doubt at any point: stop and ask. Re-open the spec docs rather than guessing.

### Progress snapshot and backlog

- **[PROGRESS.md](PROGRESS.md)** -- current phase, locked tech-stack versions, conventions, bugs-fixed-don't-reintroduce, file map, known warnings, and verification commands. **Read this first on cold-start or after session compaction.** Updated at each phase lock.
- **[FEATURES.md](FEATURES.md)** -- granular feature backlog, authoritative source for design *detail* not yet lifted into the specs. Features promote from `[sketch]` to `[ready]` only after they (a) reconcile with the specs above, (b) gain acceptance criteria, and (c) land in a phase in [ROADMAP.md](ROADMAP.md). **Read it whenever scoping new work so you can see what is already captured, including open questions and deferred ideas.** Do not invent features that could instead be promoted from the backlog.
- **[AIRCRAFT.md](AIRCRAFT.md)** -- living aircraft catalog with design-model spec sheets. Adding an aircraft here is free; implementing one requires a ROADMAP slot. Consult before inventing flight-model data.

### Also in the repo

- **[README.md](README.md)** -- orientation for human visitors. Summarizes the project and points at the same spec docs listed above. Not part of the agent reading order; the specs are the authoritative source. If README and a spec doc ever disagree, the spec is correct.

## Core Principle

> **The simulation holds the invariants. Rendering interprets state at display rate. Audio traces state lock-free. The boundaries are narrow and frozen.**

Every design and implementation decision must honor this one-liner. If you are writing code that blurs these boundaries (render mutating sim, audio taking locks, sim reaching into GPU), stop and re-read DESIGN.md.

## How to work here

- **Design first, always.** If a change is not already described in the specification docs, propose a design amendment (edit the relevant spec doc) *before* writing code.
- **Every line of code traces to a spec section.** No speculative features. No "improvements" beyond what was asked. No refactoring code you were not asked to touch.
- **Tests alongside implementation, not after.** Phase gates are non-negotiable (see ROADMAP.md).
- **Small, scoped commits.** Single logical change each. Imperative subject, capital letter, no end punctuation. Explain *why*, not *what*.
- **No vibe coding.** Exploratory prototyping lives on a scratch branch and does not ship. Production code must trace to the design.
- **Bench tests are dual-purpose.** The flight-test bench (`crates/sim/src/flight_bench.rs`) contains BOTH capability scenarios (aircraft correctly does X) AND envelope-limit scenarios (aircraft correctly fails when pushed past X). Players will push the model past realistic envelopes in normal play — the bench must verify that out-of-envelope behavior stays realistic. **Do not force-fit**: don't widen tolerances to mask envelope failures, don't soften scenario conditions to dodge failures, don't delete scenarios because "they fail." See `skills/flowplan-flight-bench/SKILL.md` Core Rule #9 and `skills/flowplan-flight-bench/references/model-gap-triage.md` for the discipline. When a bench scenario fails: first ask "is the aircraft behaving correctly per real-world physics?" If yes, the scenario needs re-classification (capability → envelope-limit), not a tolerance bump.

## Agent-specific

- Read this file, then the six specs in order. No shortcuts.
- Never hold locks in the audio callback. Never allocate in the render or audio hot path. See [CODING_RULES.md](CODING_RULES.md#real-time-discipline).
- No new dependencies without human approval. Approved dependencies are enumerated in [DESIGN.md](DESIGN.md#tech-stack).
- When a spec doc is ambiguous, do not guess: propose a clarifying amendment and stop.
- No commits unless explicitly asked.
