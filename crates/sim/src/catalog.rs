//! Aircraft catalog.
//!
//! At startup the app scans `data/aircraft/*.ron` and builds an `AircraftCatalog`
//! that knows about every flyable aircraft + their variants. The catalog is
//! enumerable (for future menu UI), lookups are by stable id, and each entry
//! knows its canonical default variant.
//!
//! The id of an entry is the RON filename without extension: `northlander.ron`
//! → id `"northlander"`. This keeps the filesystem the canonical source of
//! truth — adding an aircraft is "drop a RON + GLB in place", no code change.
//!
//! Variants come from the spec's `variants: Vec<AircraftVariant>` field. The
//! catalog validates that every spec's `default_variant` matches a variant id;
//! malformed specs are reported through `CatalogError` and excluded from the
//! catalog rather than silently accepted.
//!
//! The catalog itself holds owned `AircraftSpec` clones. It is constructed once
//! at startup and held by the app for the lifetime of the process. A future
//! reload path (mid-session hot-reload when a RON file changes) can be added
//! without API change — the type is not frozen once the app borrows from it.

use std::fs;
use std::path::{Path, PathBuf};

use crate::aircraft_spec::{AircraftSpec, AircraftVariant};

/// Errors raised by catalog scanning / lookup. Aggregated so the app can log
/// per-entry problems without aborting the whole scan.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// I/O failure reading the directory or a spec file.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path that caused the error.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// A spec file parsed as RON but violated invariants (empty variants,
    /// unknown default variant id, duplicate variant ids).
    #[error("spec validation failed at {path}: {reason}")]
    Validation {
        /// Path that caused the error.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },
    /// A spec file failed to parse as RON.
    #[error("RON parse error at {path}: {source}")]
    Parse {
        /// Path that caused the error.
        path: PathBuf,
        /// Underlying RON parser error.
        source: ron::error::SpannedError,
    },
}

/// One aircraft in the catalog.
///
/// Carries a stable id (derived from the RON filename) plus the owned
/// `AircraftSpec`. Variant lookups go through `find_variant` /
/// `default_variant` so the caller never has to index the `variants` vec
/// directly.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogEntry {
    /// Stable id for selector matching. Derived from the RON filename (without
    /// the `.ron` extension). Lowercase + single-word is the convention.
    pub id: String,
    /// Full spec loaded from the RON. Owned, so the app can hold the catalog
    /// across the sim lifetime without borrow complications.
    pub spec: AircraftSpec,
}

impl CatalogEntry {
    /// Find a variant by id. Returns `None` if the id doesn't match any
    /// declared variant on this entry.
    #[must_use]
    pub fn find_variant(&self, variant_id: &str) -> Option<&AircraftVariant> {
        self.spec.variants.iter().find(|v| v.id == variant_id)
    }

    /// The default variant for this entry, as declared in the spec. Guaranteed
    /// to succeed because catalog validation rejects entries whose
    /// `default_variant` doesn't match a variant id; `unreachable` is the right
    /// panic shape here (invariant broken = programmer error), but we return
    /// `Option` to keep the API forgiving for callers who prefer propagation.
    #[must_use]
    pub fn default_variant(&self) -> Option<&AircraftVariant> {
        self.find_variant(&self.spec.default_variant)
    }
}

/// Runtime-scanned catalog of flyable aircraft. Cheap to `clone`; expected to be
/// built once per process but small enough that cloning on config change is a
/// non-event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AircraftCatalog {
    entries: Vec<CatalogEntry>,
}

impl AircraftCatalog {
    /// Scan a directory for `*.ron` aircraft specs. Each valid spec becomes a
    /// `CatalogEntry`; invalid ones are returned as `errors` for the caller to
    /// log, allowing the catalog to boot even when a single RON is broken.
    ///
    /// Order of entries follows filesystem iteration order (platform-dependent);
    /// callers that care about a stable display order should sort on the
    /// returned catalog. Does not recurse into subdirectories.
    pub fn scan(dir: &Path) -> (Self, Vec<CatalogError>) {
        let mut entries = Vec::new();
        let mut errors = Vec::new();

        let read_dir = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(source) => {
                errors.push(CatalogError::Io {
                    path: dir.to_path_buf(),
                    source,
                });
                return (Self { entries }, errors);
            },
        };

        for entry in read_dir {
            let path = match entry {
                Ok(e) => e.path(),
                Err(source) => {
                    errors.push(CatalogError::Io {
                        path: dir.to_path_buf(),
                        source,
                    });
                    continue;
                },
            };
            if path.extension().and_then(|s| s.to_str()) != Some("ron") {
                continue;
            }
            match Self::load_entry(&path) {
                Ok(entry) => entries.push(entry),
                Err(e) => errors.push(e),
            }
        }

        entries.sort_by(|a, b| a.id.cmp(&b.id));
        (Self { entries }, errors)
    }

    fn load_entry(path: &Path) -> Result<CatalogEntry, CatalogError> {
        let source = fs::read_to_string(path).map_err(|source| CatalogError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let spec = AircraftSpec::from_ron(&source).map_err(|source| CatalogError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| CatalogError::Validation {
                path: path.to_path_buf(),
                reason: "could not derive id from filename".to_string(),
            })?
            .to_string();
        validate_spec(path, &spec)?;
        Ok(CatalogEntry { id, spec })
    }

    /// All catalog entries, sorted by id. Stable across processes given the
    /// same directory contents.
    #[must_use]
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// Find an entry by stable id.
    #[must_use]
    pub fn find(&self, aircraft_id: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.id == aircraft_id)
    }

    /// The first entry in id-sorted order, used as the catalog-level default
    /// when the caller didn't supply an aircraft id. Returns `None` only when
    /// the catalog is empty.
    #[must_use]
    pub fn default_entry(&self) -> Option<&CatalogEntry> {
        self.entries.first()
    }

    /// True when the catalog has no flyable aircraft. Caller falls back to the
    /// placeholder cube in this case.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn validate_spec(path: &Path, spec: &AircraftSpec) -> Result<(), CatalogError> {
    if spec.variants.is_empty() {
        return Err(CatalogError::Validation {
            path: path.to_path_buf(),
            reason: "variants must be non-empty".to_string(),
        });
    }
    let mut seen = Vec::new();
    for v in &spec.variants {
        if seen.contains(&v.id.as_str()) {
            return Err(CatalogError::Validation {
                path: path.to_path_buf(),
                reason: format!("duplicate variant id {:?}", v.id),
            });
        }
        seen.push(v.id.as_str());
    }
    if !spec.variants.iter().any(|v| v.id == spec.default_variant) {
        return Err(CatalogError::Validation {
            path: path.to_path_buf(),
            reason: format!(
                "default_variant {:?} does not match any variant id",
                spec.default_variant
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::Write;

    const NORTHLANDER_MIN: &str = r#"(
        name: "Northlander",
        inspiration: "DHC-2 Beaver",
        empty_mass_kg: 1315.5,
        max_mass_wheels_kg: 2313.3,
        max_mass_floats_kg: 2435.8,
        wing_area_m2: 23.23,
        aspect_ratio: 9.2,
        airfoil_root: "NACA 64A416",
        airfoil_tip: "NACA 64A412",
        engine: (name: "R-985", power_hp: 450.0, cruise_fuel_gph: 27.0),
        fuel_capacity_gal: 95.0,
        speeds: (
            vne_mps: 71.53,
            cruise_mps: 64.31,
            vs_clean_mps: 23.25,
            vso_mps: 21.01,
            vx_mps: 29.06,
            vy_mps: 37.99,
        ),
        variants: [
            (id: "floats", display_name: "Floats", config: Floats, mesh_path: "data/aircraft/northlander.glb"),
            (id: "wheels", display_name: "Wheels", config: Wheels, mesh_path: "data/aircraft/northlander_land.glb"),
        ],
        default_variant: "floats",
    )"#;

    fn temp_dir_with(contents: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, body) in contents {
            let mut f =
                fs::File::create(dir.path().join(name)).expect("create ron file in tempdir");
            f.write_all(body.as_bytes()).expect("write ron file body");
        }
        dir
    }

    #[test]
    fn scan_empty_dir_produces_empty_catalog() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (cat, errs) = AircraftCatalog::scan(dir.path());
        assert!(errs.is_empty(), "no errors for empty dir, got {errs:?}");
        assert!(cat.is_empty());
        assert!(cat.default_entry().is_none());
    }

    #[test]
    fn scan_picks_up_ron_files() {
        let dir = temp_dir_with(&[("northlander.ron", NORTHLANDER_MIN)]);
        let (cat, errs) = AircraftCatalog::scan(dir.path());
        assert!(errs.is_empty(), "unexpected errors: {errs:?}");
        assert_eq!(cat.entries().len(), 1);
        let entry = cat.find("northlander").expect("find northlander");
        assert_eq!(entry.id, "northlander");
        assert_eq!(entry.spec.variants.len(), 2);
    }

    #[test]
    fn default_variant_returns_matching_variant() {
        let dir = temp_dir_with(&[("northlander.ron", NORTHLANDER_MIN)]);
        let (cat, _) = AircraftCatalog::scan(dir.path());
        let entry = cat.find("northlander").expect("find");
        let def = entry.default_variant().expect("default variant");
        assert_eq!(def.id, "floats");
    }

    #[test]
    fn entries_are_sorted_by_id() {
        let alpha = NORTHLANDER_MIN.replace("\"Northlander\"", "\"Alpha\"");
        let dir = temp_dir_with(&[
            ("northlander.ron", NORTHLANDER_MIN),
            ("alpha.ron", alpha.as_str()),
        ]);
        let (cat, _) = AircraftCatalog::scan(dir.path());
        let ids: Vec<_> = cat.entries().iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "northlander"]);
    }

    #[test]
    fn missing_default_variant_is_validation_error() {
        let bad =
            NORTHLANDER_MIN.replace("default_variant: \"floats\"", "default_variant: \"nope\"");
        let dir = temp_dir_with(&[("northlander.ron", bad.as_str())]);
        let (cat, errs) = AircraftCatalog::scan(dir.path());
        assert!(cat.is_empty());
        assert_eq!(errs.len(), 1);
        let CatalogError::Validation { reason, .. } = &errs[0] else {
            panic!("expected Validation, got {:?}", errs[0]);
        };
        assert!(reason.contains("default_variant"));
    }

    #[test]
    fn empty_variants_is_validation_error() {
        let bad = NORTHLANDER_MIN.replace(
            "variants: [\n            (id: \"floats\", display_name: \"Floats\", config: Floats, mesh_path: \"data/aircraft/northlander.glb\"),\n            (id: \"wheels\", display_name: \"Wheels\", config: Wheels, mesh_path: \"data/aircraft/northlander_land.glb\"),\n        ]",
            "variants: []",
        );
        let dir = temp_dir_with(&[("bad.ron", bad.as_str())]);
        let (cat, errs) = AircraftCatalog::scan(dir.path());
        assert!(cat.is_empty());
        assert_eq!(errs.len(), 1);
        let CatalogError::Validation { reason, .. } = &errs[0] else {
            panic!("expected Validation");
        };
        assert!(reason.contains("variants"));
    }

    #[test]
    fn bad_ron_yields_parse_error() {
        let dir = temp_dir_with(&[("bad.ron", "not ron")]);
        let (cat, errs) = AircraftCatalog::scan(dir.path());
        assert!(cat.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], CatalogError::Parse { .. }));
    }

    #[test]
    fn find_missing_aircraft_returns_none() {
        let dir = temp_dir_with(&[("northlander.ron", NORTHLANDER_MIN)]);
        let (cat, _) = AircraftCatalog::scan(dir.path());
        assert!(cat.find("no-such-plane").is_none());
    }
}
