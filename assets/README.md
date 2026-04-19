<!-- L0: source assets (shaders, models, textures, audio); bakes into engine-native forms at build time -->
# assets/

Source assets in their authoring formats. These are processed at build time (Phase 1+ once the `tools/` baking step exists) into engine-native forms that the runtime loads via `bytemuck::cast_slice`.

## What lives here

| Directory       | Contents                                                        | Build step                                     |
| --------------- | --------------------------------------------------------------- | ---------------------------------------------- |
| `shaders/`      | WGSL shader source, one file per logical shader unit            | WGSL is loaded as text in dev; compiled in release |
| `models/`       | glTF 2.0 source (one `.gltf` or `.glb` per model)               | Pre-processed into engine-native mesh + material |
| `textures/`     | PNG, KTX2, EXR sources                                          | Compressed and packed into GPU-ready formats   |
| `audio/`        | Glicol graph source files (`.glicol`), optional sample wav/ogg  | Graphs loaded at runtime; samples decoded once |

Subdirectories hold `.gitkeep` only; they populate during Phase 2 (shaders + first models) and Phase 5 (full content pass).

## Conventions

- **Source in, engine-native out.** The runtime does not parse glTF at launch; build tooling in `tools/` emits binary-ready form.
- **Tracked source, ignored build output.** `.gltf`/`.png`/`.wgsl` are tracked; `target/build-artifacts/` is `.gitignore`d.
- **File naming matches handle identifiers.** `models/northlander-wheels.gltf` becomes handle `models::northlander_wheels` at runtime.
- **No runtime asset import.** Hot-reload in dev re-runs the baker for the changed file only, swaps the slot, bumps the handle generation.

## Shaders

- **One file per logical pipeline stage** (vertex + fragment can share a file if tightly coupled; otherwise split).
- **Uniform layouts match `#[repr(C)]` Rust structs** in `render`. When you change one, change the other in the same commit.
- **Hot reload in dev builds only** (`#[cfg(debug_assertions)]` gated, plus a feature flag).
- **No cargo-cult complexity.** Prefer clear multi-pass over clever monoshader tricks.

## Models

- **Low-poly, cel-shade-compatible.** See [PLAN.md § Rendering Approach](../PLAN.md#rendering-approach) for the art target.
- **glTF 2.0 authoring source** from Blender; textures embedded or referenced.
- **Naming matches spec entries.** A model file maps to its in-game aircraft name (sanitized per [AIRCRAFT.md](../AIRCRAFT.md)).
- **LODs** are a Phase 5 concern; author models with that in mind but do not build LOD chains until profiling demands.

## Textures

- **PNG** for authoring; **KTX2** for shipping once a bake pipeline exists.
- **Cel-shading palette constraints** recommended but not enforced -- the banded shading does the visual work, not the texture detail.

## Audio

- **Glicol graph source** is the primary audio authoring surface -- patches for engine sound, wind, water, music, wildlife.
- **Sampled audio** (radio voices, rare sample-based events) kept to a minimum; processed through Glicol at runtime for consistency.

## Non-goals here

- Runtime asset downloads.
- Proprietary authoring formats that the build tool cannot ingest.
- Committed build outputs (always regenerate from source).
