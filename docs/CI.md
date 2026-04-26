# CI workflows + local-runner discipline

Three GitHub Actions workflows guard the codebase. This doc explains
what each does, when it runs, and how to reproduce locally with
[`nektos/act`](https://github.com/nektos/act) so private-repo Actions
minutes stay metered.

## Workflows

### `quality-gate.yml` — main gate

**Triggers:** every push to `main`, every PR, manual `workflow_dispatch`.

**Jobs:**

| Job | What it runs | Time |
| --- | --- | --- |
| `fmt` | `cargo fmt --all -- --check` | <1 min |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` | ~5 min |
| `test` | `cargo test --workspace` + `cargo test -p flowplan-sim --test flight_bench --release` | ~10 min |
| `bench` | `cargo bench -p flowplan-benches --bench frame_budget -- --test` (smoke; not full bench run) | ~5 min |

The `test` job already covers the non-ignored perf budget
(`perf_budget_terrain_height_l1`) — see TEST_PLAN.md § Tier 4b.

### `determinism.yml` — cross-platform bit-identity check

**Triggers:** same as quality gate.

Runs world-bake / sim-tick determinism tests on **Ubuntu + Windows + macOS** matrix. Asserts byte-identical output across all three platforms — catches transcendental drift (glam internals, libm vs std, etc.) per CODING_RULES § Determinism Discipline.

### `perf-budgets.yml` — nightly perf-budget regression net (PINNED 2026-04-25)

**Triggers:** `schedule: cron "0 4 * * *"` (nightly 04:00 UTC), manual `workflow_dispatch`. **Not on push** — saves per-push Actions minutes on the private repo.

Runs the `#[ignore]`'d perf-budget tests in `crates/world/tests/perf_budgets.rs` with `--release`. Each test needs a 30-60 s world-bake; running them on every push would burn ~5-10 min per CI run.

Failures = perf regressions. See TEST_PLAN.md § Tier 4b for the runbook (reproduce → profile → revert/optimize → only relax budget as a last resort, with PROGRESS.md note).

## Running CI locally with `act`

[`act`](https://github.com/nektos/act) runs GitHub Actions workflows locally via Docker. Useful for:

- **Verifying CI changes** before push (especially perf-budget yaml edits)
- **Running quality-gate locally** without burning Actions minutes (private-repo metering)
- **Debugging CI failures** — `act -v` is more legible than CI log scrolling

### Install

The repo at `~/cs/act` is the upstream source. Install via either:

```bash
# Option 1 — prebuilt binary
~/cs/act/install.sh --to ~/.local/bin

# Option 2 — build from source (requires Go)
cd ~/cs/act && make build && cp dist/local/act ~/.local/bin/
```

Verify: `act --version`. **Pin to 0.2.86 or later** to avoid CVE-2026-34041 / -34042.

### One-time configuration (`~/.config/act/actrc`)

`act` defaults aren't quite right for this repo. On first invocation it
prompts interactively for an image size; pre-configure to skip that.
Docker Desktop on Linux also uses a non-default socket path:

```ini
# ~/.config/act/actrc
# Medium image — covers cargo/Rust workflows; smaller than Large.
-P ubuntu-latest=catthehacker/ubuntu:act-latest
# Docker Desktop on Linux puts its socket in the user dir, not /var/run.
# Adjust path if your Docker Desktop install differs.
--container-daemon-socket unix:///home/$USER/.docker/desktop/docker.sock
```

Verify with a dry-run:

```bash
act push -W .github/workflows/quality-gate.yml -j fmt --dryrun
# Expect: 🏁 Job succeeded (no actual cargo runs, just step traversal)
```

### Common invocations

```bash
# Run the full quality-gate workflow locally:
act push -W .github/workflows/quality-gate.yml

# Run a specific job (e.g. just clippy):
act push -W .github/workflows/quality-gate.yml -j clippy

# Run the perf-budget workflow locally (matches the nightly run):
act workflow_dispatch -W .github/workflows/perf-budgets.yml

# Verbose — see every step + intermediate output:
act push -W .github/workflows/quality-gate.yml -v
```

### Limitations

- **Linux only.** `act` runs jobs in Docker containers, so it can't execute `runs-on: macos-latest` or `windows-latest`. The determinism workflow's cross-platform matrix needs real GitHub runners.
- **First-run download.** `act` pulls a Docker image (~few GB) the first time. Pin the image to keep subsequent runs fast.
- **Caching mismatch.** `Swatinem/rust-cache@v2` caches under GitHub-Actions semantics; locally you may want a separate cargo cache (e.g. `CARGO_TARGET_DIR=$HOME/.cache/act-target`) so local act runs don't fight your normal builds for incremental state.

### When local-run differs from real CI

If `act` passes but real CI fails:

1. Check the runner image — `act` defaults to a smaller image than GitHub's; some packages may be missing. Use `act -P ubuntu-latest=catthehacker/ubuntu:full-latest` for a fuller match.
2. Cross-platform determinism — only real CI catches Windows/macOS divergence.
3. Cargo lock file — `act` uses your local `Cargo.lock`; CI does too on a fresh checkout, so the only difference is incremental cache state.

## Discipline summary

- **Cheap perf budgets** run on every push (in `quality-gate.yml::test`).
- **Heavy perf budgets** run nightly (in `perf-budgets.yml`).
- **Cross-platform determinism** runs on push (`determinism.yml`).
- **Pre-push verification**: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` covers what `act push` would catch locally.
- **CI-change verification**: `act` runs the workflow locally. Recommended before pushing yaml edits.
