//! Enforces the crate-dependency DAG from `DESIGN.md § Dependency Rules`.
//!
//! Each test asserts a specific rule. When a rule fires, the fix is almost always to move
//! the offending use to a layer where the dependency is allowed, not to relax the rule.

use flowplan_workspace_rules::{declared_deps, flowplan_crate_manifests};

// `toml` is used transitively via the workspace-rules lib; the integration-test target
// inherits the dep but doesn't call `toml` directly, so mark it as intentional.
use toml as _;

/// `flowplan-core` may not depend on any other flowPlan crate.
#[test]
fn core_has_no_flowplan_deps() {
    let manifests = flowplan_crate_manifests();
    let core_path = &manifests["flowplan-core"];
    let d = declared_deps(core_path);
    let flowplan_runtime_deps: Vec<&String> = d
        .deps
        .iter()
        .filter(|n| n.starts_with("flowplan-"))
        .collect();
    assert!(
        flowplan_runtime_deps.is_empty(),
        "flowplan-core must not depend on any flowPlan crate, found: {flowplan_runtime_deps:?}"
    );
}

/// `flowplan-sim` must not pull in wgpu, winit, or cpal. It is pure simulation state.
#[test]
fn sim_has_no_gpu_windowing_or_audio_deps() {
    let manifests = flowplan_crate_manifests();
    let sim = declared_deps(&manifests["flowplan-sim"]);
    for forbidden in ["wgpu", "winit", "cpal", "glicol_synth", "pollster"] {
        assert!(
            !sim.has_runtime_dep(forbidden),
            "flowplan-sim must not depend on `{forbidden}`"
        );
    }
}

/// `flowplan-audio` must not pull in wgpu or winit.
#[test]
fn audio_has_no_gpu_or_windowing_deps() {
    let manifests = flowplan_crate_manifests();
    let audio = declared_deps(&manifests["flowplan-audio"]);
    for forbidden in ["wgpu", "winit"] {
        assert!(
            !audio.has_runtime_dep(forbidden),
            "flowplan-audio must not depend on `{forbidden}`"
        );
    }
}

/// `flowplan-render` must not pull in audio or app.
#[test]
fn render_has_no_audio_or_app_deps() {
    let manifests = flowplan_crate_manifests();
    let render = declared_deps(&manifests["flowplan-render"]);
    for forbidden in ["flowplan-audio", "flowplan-app"] {
        assert!(
            !render.has_runtime_dep(forbidden),
            "flowplan-render must not depend on `{forbidden}`"
        );
    }
}

/// `flowplan-world` must not pull in rendering, audio, or app.
#[test]
fn world_has_no_render_audio_or_app_deps() {
    let manifests = flowplan_crate_manifests();
    let world = declared_deps(&manifests["flowplan-world"]);
    for forbidden in ["flowplan-render", "flowplan-audio", "flowplan-app"] {
        assert!(
            !world.has_runtime_dep(forbidden),
            "flowplan-world must not depend on `{forbidden}`"
        );
    }
}

/// `flowplan-assets` must not depend on sim, progression, world, render, audio, or app.
#[test]
fn assets_has_no_higher_layer_deps() {
    let manifests = flowplan_crate_manifests();
    let assets = declared_deps(&manifests["flowplan-assets"]);
    for forbidden in [
        "flowplan-sim",
        "flowplan-progression",
        "flowplan-world",
        "flowplan-render",
        "flowplan-audio",
        "flowplan-app",
    ] {
        assert!(
            !assets.has_runtime_dep(forbidden),
            "flowplan-assets must not depend on `{forbidden}`"
        );
    }
}

/// `flowplan-progression` must not depend on sim, world, render, audio, or app. It is
/// persistent-state only.
#[test]
fn progression_has_no_peer_or_higher_layer_deps() {
    let manifests = flowplan_crate_manifests();
    let prog = declared_deps(&manifests["flowplan-progression"]);
    for forbidden in [
        "flowplan-sim",
        "flowplan-world",
        "flowplan-render",
        "flowplan-audio",
        "flowplan-app",
    ] {
        assert!(
            !prog.has_runtime_dep(forbidden),
            "flowplan-progression must not depend on `{forbidden}`"
        );
    }
}

/// Nothing (besides the app's own `[[bin]]`) may depend on `flowplan-app`.
#[test]
fn nothing_depends_on_app() {
    let manifests = flowplan_crate_manifests();
    for (name, path) in &manifests {
        if *name == "flowplan-app" {
            continue;
        }
        let d = declared_deps(path);
        assert!(
            !d.has_runtime_dep("flowplan-app"),
            "{name} must not depend on flowplan-app"
        );
    }
}

/// Forbidden external crates per the non-goals in `DESIGN.md`. Catches accidental inclusion
/// of engine frameworks, async runtimes, or UI toolkits that Phase 0 has no use for.
#[test]
fn forbidden_external_crates_absent() {
    let manifests = flowplan_crate_manifests();
    // Additional non-flowplan-internal workspace members should also be covered.
    let extras = [
        (
            "flowplan-workspace-rules",
            workspace_root().join("tests/workspace-rules/Cargo.toml"),
        ),
        (
            "flowplan-benches",
            workspace_root().join("benches/Cargo.toml"),
        ),
    ];

    let forbidden = [
        "bevy",
        "bevy_ecs",
        "bevy_render",
        "fyrox",
        "specs",
        "legion",
        "hecs",
        "tokio",
        "async-std",
        "actix",
        "actix-web",
        "async-global-executor",
        "imgui",
        "imgui-winit-support",
    ];

    let all: Vec<(&str, std::path::PathBuf)> = manifests
        .iter()
        .map(|(n, p)| (*n, p.clone()))
        .chain(extras.iter().map(|(n, p)| (*n, p.clone())))
        .collect();

    for (name, path) in &all {
        if !path.exists() {
            continue;
        }
        let d = declared_deps(path);
        for f in &forbidden {
            assert!(
                !d.has_runtime_dep(f) && !d.dev_deps.iter().any(|x| x == f),
                "{name} must not depend on `{f}` (forbidden per DESIGN.md non-goals)"
            );
        }
    }
}

fn workspace_root() -> std::path::PathBuf {
    flowplan_workspace_rules::workspace_root()
}
