<!-- L0: authoring conventions for game data files (aircraft, hubs, cargo, missions, wildlife) -->
# data/

Game data authored as plain-text `.ron` (Rust Object Notation) files. Content-only; no code. Each subdirectory holds entities of one kind. Files here are loaded by crates in the workspace at startup or on hub arrival.

## Why `.ron`

- Human-readable and diffable in review.
- Supports comments, trailing commas, struct literals.
- Parses to typed Rust structs via `serde` -- schema is enforced at compile time.
- Easier to audit than binary or game-specific formats.

## Conventions

- **One entity per file.** File name matches the entity's internal identifier (kebab-case).
- **Fields follow the spec sheets.** Aircraft follow [AIRCRAFT.md](../AIRCRAFT.md); hubs, cargo, missions, wildlife follow entries in [FEATURES.md](../FEATURES.md).
- **Authoritative source beats copy.** If a field is derived from another file (hub references a surface type), reference by identifier -- never re-encode.
- **Figures from real sources are verified before lock.** Aircraft specs derive from type certificates and POHs per AIRCRAFT.md § References to Chase; note verification status with a `// verified: <source>` comment on the field.
- **Validation runs in tests.** `tests/data_validation.rs` (to be added in Phase 4) parses every file and asserts invariants (MTOW > empty, stall speeds ordered, fuel fits tanks, etc.).
- **No runtime data generation.** The `.ron` files are the source of truth; if derivation is needed (e.g., computing Vx from weight and power), do it at load time in the crate that cares, not in the file.

## Subdirectories

| Directory      | Contains                                        | Primary spec source                                                 |
| -------------- | ----------------------------------------------- | ------------------------------------------------------------------- |
| `aircraft/`    | Per-aircraft spec sheets                        | [AIRCRAFT.md](../AIRCRAFT.md)                                       |
| `hubs/`        | Per-hub feature data (surfaces, services, etc.) | [FEATURES.md § Hubs & Landing Areas](../FEATURES.md#hubs--landing-areas) |
| `cargo/`       | Cargo type catalog                              | [FEATURES.md § Cargo-type lists within Missions/Economy sections](../FEATURES.md)  |
| `missions/`    | Mission template definitions                    | [FEATURES.md § Missions & Tasks](../FEATURES.md#missions--tasks)    |
| `wildlife/`    | Wildlife species spawns, behavior params        | [FEATURES.md § Wildlife](../FEATURES.md#wildlife)                   |

Subdirectories hold `.gitkeep` only until the corresponding ROADMAP milestone authors content.

## Authoring workflow

1. Identify which entity you are authoring and which spec is authoritative.
2. If the spec is incomplete or ambiguous for your authoring need, **propose an amendment to the spec doc first**. Do not invent structure.
3. Add the `.ron` file under the right subdirectory.
4. Run `cargo test -p data` (once the validation test exists in Phase 4) to check invariants.
5. Commit with the usual discipline (one logical change, imperative subject, explain *why*).

## Non-goals here

- Runtime generation of data files.
- Per-player variation in the data tree itself (player state lives in saves, not in `data/`).
- Binary packed formats (that is an asset-pipeline concern, see `assets/`).
