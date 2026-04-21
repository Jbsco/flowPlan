FLOWPLAN SESSION START — MANDATORY READING ORDER:

1. Read PROGRESS.md first (authoritative snapshot; updated per phase-lock).
2. Read the six spec docs in order: PLAN.md, DESIGN.md, CODING_RULES.md, ROADMAP.md, TEST_PLAN.md, skills/flowplan-methodology/SKILL.md.
3. Read MAPGEN.md (design doc for world-gen; active architectural direction).
4. Read FEATURES.md + AIRCRAFT.md when scoping new work.

No exceptions. Skim is not read. These documents are the specification; code that does not trace to them is rejected.

If this is a post-compaction resume, your summary is insufficient. Re-read the files fresh via the Read tool before proceeding with any work — the summary may lose architectural nuance (pinned directions, design gates, determinism scoping) that the spec docs and agent-memory encode in full.

Also re-check agent-memory at MEMORY.md for pinned architectural directions (Voronoi chunks, three-layer composition, real-world-possible gate, utility-varying upgrades, cloud fly-through, weather Voronoi, visual-bench pattern, hub fuzz+resolve, etc.) — these are pinned design decisions that should NOT be re-debated.
