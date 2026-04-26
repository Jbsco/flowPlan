//! Bake cache (Phase 3.6.3c.3c.2 — 3.7-precursor).
//!
//! Serializes `BakedAssignments` + `FeatureGroupBundle` to disk so
//! subsequent game launches load in seconds instead of re-running the
//! 6+ minute world bake. Deterministic per `master_seed`, so caching
//! is safe — same seed → same bytes.
//!
//! # Cache invalidation
//!
//! [`BAKE_VERSION`] is a const bumped manually whenever the bake-time
//! algorithms or their inputs change in a way that would invalidate
//! previously-cached output. Examples:
//!
//! - Adding a new road kind / changing per-kind cost constants
//! - Changing Bridson Poisson parameters
//! - Modifying placement-rule evaluation
//! - Adding new sub-biome features to the RON catalog
//!
//! When in doubt, bump it. A stale cache loads silently and can ship
//! invisible content drift; a bumped cache costs one re-bake (~6 min).
//!
//! # Cache location
//!
//! Currently `target/world-bake-cache/<seed>-<bake_version>.bin`. This
//! is per-checkout: a fresh `cargo clean` wipes the cache. The 3.7
//! save format will move it to the user-data dir alongside save files.
//!
//! # Format
//!
//! `bincode` (compact binary) over a flat representation. We don't
//! cache the post-bake `SelfSpacingState` (used only in diagnostics) —
//! that field defaults to empty on load.

#![allow(clippy::doc_markdown)]
#![allow(clippy::doc_lazy_continuation)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::baked_assignments::{BakeStats, BakedAssignments};
use crate::feature_groups::FeatureGroupBundle;
use crate::feature_instances::FeatureInstance;
use crate::sub_biome::BiomeSource;
use crate::voronoi_cells::CellId;

/// Bake-cache schema version.
///
/// Bump when bake logic / artifacts change in a way that invalidates
/// previously-cached output. A version mismatch loads nothing and
/// re-bakes — the cache is a pure perf optimization, never a
/// correctness contract.
pub const BAKE_VERSION: u32 = 1;

/// Magic number — `flowplan-bake-cache-v1`. Catches accidentally-
/// truncated / wrong-format files before the deserializer panics.
const MAGIC: u64 = 0x_46_50_42_43_56_30_30_31; // "FPBCV001"

/// Root cache file payload. One file per `(seed, BAKE_VERSION)` tuple.
#[derive(Serialize, Deserialize)]
struct CacheFile {
    magic: u64,
    bake_version: u32,
    master_seed: u64,
    baked: BakedAssignmentsCache,
    groups: FeatureGroupBundle,
}

/// Serializable representation of `BakedAssignments`. Excludes
/// `spatial_hashes` (post-bake diagnostic state, defaults to empty
/// on load).
#[derive(Serialize, Deserialize)]
struct BakedAssignmentsCache {
    per_cell: Vec<u16>,
    per_cell_source: Vec<BiomeSource>,
    feature_ids: Vec<String>,
    feature_id_index: HashMap<String, u16>,
    per_cell_instances: HashMap<CellId, Vec<FeatureInstance>>,
    stats: BakeStats,
}

/// Compute the cache file path for a given seed.
#[must_use]
pub fn cache_path(master_seed: u64) -> PathBuf {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    workspace_root
        .join("target")
        .join("world-bake-cache")
        .join(format!("{master_seed:016x}-v{BAKE_VERSION}.bin"))
}

/// Result of a `try_load` attempt — `Some` if the cache existed, was
/// valid for the version + seed, and decoded cleanly.
#[must_use]
pub struct CachedBake {
    pub baked: BakedAssignments,
    pub groups: FeatureGroupBundle,
}

/// Attempt to load a cached bake for `master_seed`.
///
/// Returns `None` on any failure (missing file, version mismatch,
/// decode error) — caller runs the full bake instead. Errors are
/// logged via `tracing::info` but never propagated; the cache is
/// best-effort.
#[must_use]
pub fn try_load(master_seed: u64) -> Option<CachedBake> {
    let path = cache_path(master_seed);
    if !path.exists() {
        return None;
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            tracing::info!(
                target: "bake_cache",
                error = %e,
                path = %path.display(),
                "cache read failed; will re-bake"
            );
            return None;
        },
    };
    let cache: CacheFile = match bincode::deserialize(&bytes) {
        Ok(c) => c,
        Err(e) => {
            tracing::info!(
                target: "bake_cache",
                error = %e,
                "cache decode failed; will re-bake"
            );
            return None;
        },
    };
    if cache.magic != MAGIC {
        tracing::info!(
            target: "bake_cache",
            "cache magic mismatch ({:#x}); will re-bake",
            cache.magic
        );
        return None;
    }
    if cache.bake_version != BAKE_VERSION {
        tracing::info!(
            target: "bake_cache",
            cached = cache.bake_version,
            current = BAKE_VERSION,
            "cache version mismatch; will re-bake"
        );
        return None;
    }
    if cache.master_seed != master_seed {
        // Cache filename includes the seed; this should never trigger
        // unless a hash-collision-or-misnamed file slipped in.
        tracing::info!(
            target: "bake_cache",
            "cache master_seed mismatch; will re-bake"
        );
        return None;
    }
    let baked = BakedAssignments::from_cache_repr(cache.baked.into_parts());
    Some(CachedBake {
        baked,
        groups: cache.groups,
    })
}

/// Save a baked world to disk. Errors are logged but not propagated —
/// the cache is best-effort, never load-bearing.
pub fn save(master_seed: u64, baked: &BakedAssignments, groups: &FeatureGroupBundle) {
    let path = cache_path(master_seed);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::info!(
                target: "bake_cache",
                error = %e,
                "create cache dir failed; cache write skipped"
            );
            return;
        }
    }
    let cache = CacheFile {
        magic: MAGIC,
        bake_version: BAKE_VERSION,
        master_seed,
        baked: BakedAssignmentsCache::from(baked),
        groups: groups.clone(),
    };
    let bytes = match bincode::serialize(&cache) {
        Ok(b) => b,
        Err(e) => {
            tracing::info!(
                target: "bake_cache",
                error = %e,
                "cache serialize failed; cache write skipped"
            );
            return;
        },
    };
    if let Err(e) = std::fs::write(&path, &bytes) {
        tracing::info!(
            target: "bake_cache",
            error = %e,
            path = %path.display(),
            "cache write failed; will re-bake next time"
        );
        return;
    }
    tracing::info!(
        target: "bake_cache",
        path = %path.display(),
        bytes = bytes.len(),
        "bake cache saved"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Cache <-> live-struct conversions
// ═══════════════════════════════════════════════════════════════════════

impl BakedAssignmentsCache {
    /// Disassemble into `CacheReprParts` for
    /// `BakedAssignments::from_cache_repr` to reassemble. Doing the
    /// disassembly here keeps `BakedAssignments`'s field privacy
    /// intact — the conversion functions are the only callers.
    fn into_parts(self) -> crate::baked_assignments::CacheReprParts {
        (
            self.per_cell,
            self.per_cell_source,
            self.feature_ids,
            self.feature_id_index,
            self.per_cell_instances,
            self.stats,
        )
    }
}

impl From<&BakedAssignments> for BakedAssignmentsCache {
    fn from(b: &BakedAssignments) -> Self {
        let parts = b.cache_repr_parts();
        Self {
            per_cell: parts.0,
            per_cell_source: parts.1,
            feature_ids: parts.2,
            feature_id_index: parts.3,
            per_cell_instances: parts.4,
            stats: parts.5,
        }
    }
}
