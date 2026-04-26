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
- **[AIRCRAFT.md](AIRCRAFT.md)** -- living aircraft catalog with design-model spec sheets. Adding an aircraft here is free; implementing one requires a ROADMAP slot. Consult before inventing flight-model data. **Placeholder meshes swap to project-owner-authored originals before v1 release** — see the Attribution / licensing gate at the top of AIRCRAFT.md.

### Deeper design documents (consult when the topic is in scope)

- **[MAPGEN.md](MAPGEN.md)** -- procedural map generation, chunk model, seed topology, hydrology sub-system, biome placement, flight-log memory. Design draft; read before any Phase 3.6 terrain work or Phase 4 world-scope decision.
- **[docs/WORLD_GEN.md](docs/WORLD_GEN.md)** -- world-gen *implementation* reference (the *how* — algorithms, function signatures, parameters, performance, octave-offset convention). Companion to MAPGEN's *design* doc; read when modifying terrain, biome, or Voronoi code.
- **[docs/WEATHER.md](docs/WEATHER.md)** -- weather subsystem implementation reference (the *how* — W1/W2/W3 status, cloud raymarching technique, Worley noise, particle system architecture, fog + visibility coupling, sim cloud-fly-through hookups). Companion to MAPGEN's three-layer weather composition design; read before any Phase 3.6.3c.6 / 3.2x volumetric weather work or sim cloud-density consumption work.
- **UI_ARCHITECTURE.md** *(pending)* -- UI framework decision, navigation model, prompt system. Pre-Phase-4.8 design deliverable. Draft not yet committed.
- **LIVERY.md** *(pending — may live as an expanded section of FEATURES.md)* -- paint-editor representation choice, brush model, undo-history scope, save format. Pre-Phase-4.10 design deliverable.

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

## Dev entry point — `./flowplan`

A unified shell script at the project root wraps the common dev tasks. Prefer it over raw `cargo` invocations for actions covered:

| Task | Command | Notes |
| --- | --- | --- |
| Run game | `./flowplan run` | Default `northlander/floats`, release profile. Add `--aircraft catalina --variant wheels --instances` for variations; `--instances` sets `FLOWPLAN_FEATURE_INSTANCES=1` (full bake). |
| Local gate (pre-push) | `./flowplan gate` | fmt + clippy + test |
| Per-step gate | `./flowplan fmt-check` / `clippy` / `test` | Individual steps |
| Perf-budget tests | `./flowplan test-perf` | Includes `#[ignore]` budgets — ~3 min |
| Local CI via act | `./flowplan ci` / `ci-job <name>` / `ci-perf` | Mirrors GitHub Actions workflows |
| Bake cache | `./flowplan cache-info` / `cache-clean` | Inspect / wipe `target/world-bake-cache/` |
| Visual bench | `./flowplan bench <name>` (e.g. `feature_groups`) | Lists via `bench-list` |
| Flight bench | `./flowplan flight-bench [--scenario X]` | Outputs to `target/flight-bench/` |
| Help | `./flowplan help` | Authoritative list of subcommands |

Add new subcommands by editing `flowplan` (the script) — append a `cmd_<name>()` function + register it in the `case` dispatch + add a help line. Bash, no compile cost, easy to extend. See `docs/CI.md` for the act configuration the `ci*` subcommands rely on.

## Agent-specific

- Read this file, then the six specs in order. No shortcuts.
- Never hold locks in the audio callback. Never allocate in the render or audio hot path. See [CODING_RULES.md](CODING_RULES.md#real-time-discipline).
- No new dependencies without human approval. Approved dependencies are enumerated in [DESIGN.md](DESIGN.md#tech-stack).
- When a spec doc is ambiguous, do not guess: propose a clarifying amendment and stop.
- No commits unless explicitly asked.
- Prefer `./flowplan <subcommand>` for common dev tasks; fall back to raw `cargo` for one-offs the script doesn't cover.
