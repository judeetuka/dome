//! Shared helpers for Dome integration tests.
//!
//! Every fixture path and sample value used by more than one crate lives here, so a
//! fixture cannot silently move or change shape behind a test's back.

use std::fmt::Debug;
use std::path::PathBuf;

use dome_core::{FoundationName, StorageSize};
use dome_image::{FoundationFormat, FoundationManifest};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Repository root, resolved at compile time from this crate's location.
///
/// Compile-time rather than inferred from the working directory, so tests behave the
/// same however cargo is invoked.
#[must_use]
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Absolute path to `tests/fixtures/<name>`.
#[must_use]
pub fn test_fixture_path(name: &str) -> PathBuf {
    workspace_root().join("tests").join("fixtures").join(name)
}

/// Absolute path to the fake Linux host root used by `doctor --fixtures`.
#[must_use]
pub fn host_fixture_root() -> PathBuf {
    test_fixture_path("linux-host")
}

/// A valid blueprint, embedded at compile time so it cannot drift from the fixture.
#[must_use]
pub fn valid_cell_toml() -> &'static str {
    include_str!("../../../tests/fixtures/valid.cell.toml")
}

/// A blueprint rejected during construction: zero memory and zero CPUs.
#[must_use]
pub fn invalid_zero_memory_cell_toml() -> &'static str {
    include_str!("../../../tests/fixtures/invalid-zero-memory.cell.toml")
}

/// A blueprint that parses but violates validation rules.
#[must_use]
pub fn invalid_limits_cell_toml() -> &'static str {
    include_str!("../../../tests/fixtures/invalid-limits.cell.toml")
}

/// A reusable foundation manifest.
#[must_use]
pub fn sample_foundation_manifest() -> FoundationManifest {
    FoundationManifest::new(
        FoundationName::new("dome/buildroot:minimal").expect("static foundation name is valid"),
        FoundationFormat::DomeFoundation,
        StorageSize::from_mib(256).expect("static storage size is valid"),
        "foundations/buildroot.foundation.dome",
    )
}

/// Assert a value survives a TOML render/parse cycle unchanged.
///
/// Returns `Result` rather than panicking so callers can use `?` in tests that already
/// return `Result`.
pub fn verify_round_trip<T>(value: &T) -> Result<(), String>
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let rendered =
        toml::to_string_pretty(value).map_err(|error| format!("render failed: {error}"))?;
    let parsed: T = toml::from_str(&rendered).map_err(|error| format!("parse failed: {error}"))?;

    if parsed != *value {
        return Err(format!(
            "round trip changed the value:\n{value:?}\n{parsed:?}"
        ));
    }

    Ok(())
}
