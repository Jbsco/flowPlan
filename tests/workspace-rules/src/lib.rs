//! flowPlan workspace architectural enforcement.
//!
//! The tests in this crate enforce the crate dependency DAG declared in
//! [`DESIGN.md § Dependency Rules`](../../../../DESIGN.md#dependency-rules). Without them,
//! architectural drift is invisible: a rogue `use` that routes around the layer boundaries
//! will compile cleanly even as it violates the design.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Named flowPlan crates and their workspace-relative Cargo.toml paths.
#[must_use]
pub fn flowplan_crate_manifests() -> BTreeMap<&'static str, PathBuf> {
    let workspace_root = workspace_root();
    let names = [
        "flowplan-core",
        "flowplan-assets",
        "flowplan-sim",
        "flowplan-progression",
        "flowplan-world",
        "flowplan-render",
        "flowplan-audio",
        "flowplan-app",
    ];
    names
        .iter()
        .map(|n| {
            let short = n.trim_start_matches("flowplan-");
            let path = workspace_root.join("crates").join(short).join("Cargo.toml");
            (*n, path)
        })
        .collect()
}

/// The workspace root, derived from this crate's location. `tests/workspace-rules` is two
/// levels below the root.
#[must_use]
pub fn workspace_root() -> PathBuf {
    let this = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    this.parent() // tests/
        .and_then(std::path::Path::parent) // workspace root
        .unwrap_or(&this)
        .to_path_buf()
}

/// Dependency names declared in a crate's `[dependencies]` and `[dev-dependencies]`.
#[must_use]
pub fn declared_deps(manifest_path: &std::path::Path) -> DeclaredDeps {
    let text = std::fs::read_to_string(manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
    let value: toml::Value =
        toml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", manifest_path.display()));
    let deps = section_keys(&value, "dependencies");
    let dev_deps = section_keys(&value, "dev-dependencies");
    DeclaredDeps { deps, dev_deps }
}

fn section_keys(value: &toml::Value, section: &str) -> Vec<String> {
    value
        .get(section)
        .and_then(toml::Value::as_table)
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

/// Dependencies declared in a crate's manifest, split by section.
pub struct DeclaredDeps {
    pub deps: Vec<String>,
    pub dev_deps: Vec<String>,
}

impl DeclaredDeps {
    /// Returns true if the non-dev dependencies include `name`.
    #[must_use]
    pub fn has_runtime_dep(&self, name: &str) -> bool {
        self.deps.iter().any(|d| d == name)
    }
}
