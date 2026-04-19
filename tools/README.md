<!-- L0: build-time tools (asset bakers, validators, data generators); not part of the shipped runtime -->
# tools/

Build-time and author-time tools. These are not part of the shipped runtime; they run during `cargo build` (via `build.rs` where trivial, or via `cargo xtask`-style runners for anything non-trivial) or on-demand from the developer's shell.

## Planned tools (populated per phase)

| Tool                 | Purpose                                                               | Phase  |
| -------------------- | --------------------------------------------------------------------- | ------ |
| `bake_shaders`       | WGSL source -> compiled/validated SPIR-V or WGSL blob for shipping    | 2      |
| `bake_models`        | glTF -> engine-native mesh + material tables                          | 2      |
| `bake_textures`      | PNG / EXR -> KTX2 with appropriate compression per platform           | 5      |
| `bake_audio`         | Glicol graph pre-validation; optional sample decoding                 | 3      |
| `validate_data`      | `.ron` files schema + invariant check (runs in `tests/` too)          | 4      |
| `gen_type_cert_stub` | Fetch / template a type-certificate-derived spec sheet from input     | 2      |
| `gen_scene`          | Synthesize `benches/scenes/*.flow` fixtures from parameters           | 2-3    |

Each tool's source lives under `tools/<name>/` with its own `Cargo.toml` (workspace member or standalone, decide during Phase 0).

## Conventions

- **Tools are hermetic.** Input paths and output paths are explicit; no ambient filesystem scanning.
- **Tools report.** Every invocation prints what it did (input -> output) so the build log is self-documenting.
- **Tools are deterministic.** Same input -> same output, bit-identical. Required for caching.
- **Tools are not shipped.** They never appear in the runtime binary. `[dev-dependencies]` or a separate workspace section.
- **No network calls.** All tooling is offline.

## Non-goals here

- Ad-hoc shell scripts that do build logic outside of this directory or `build.rs`.
- Bash/PowerShell-specific tools (everything should work on Linux, macOS, and eventually Windows).
- Tools that replace `cargo`, `rustfmt`, `clippy`, or `cargo test` -- those are primary.
