use dome_config::Blueprint;
use dome_test_utils::{
    host_fixture_root, invalid_limits_cell_toml, invalid_zero_memory_cell_toml,
    sample_foundation_manifest, test_fixture_path, valid_cell_toml, verify_round_trip,
};

#[test]
fn fixture_paths_resolve_to_real_files() {
    for name in [
        "valid.cell.toml",
        "invalid-zero-memory.cell.toml",
        "invalid-limits.cell.toml",
    ] {
        assert!(test_fixture_path(name).is_file(), "{name} should exist");
    }

    assert!(host_fixture_root().join("proc/cpuinfo").is_file());
}

#[test]
fn embedded_fixtures_parse() {
    let valid = Blueprint::from_toml(valid_cell_toml()).expect("valid fixture must parse");
    valid.validate().expect("valid fixture must validate");

    assert!(Blueprint::from_toml(invalid_zero_memory_cell_toml()).is_err());

    let limits =
        Blueprint::from_toml(invalid_limits_cell_toml()).expect("parses, but fails validation");
    assert!(limits.validate().is_err());
}

#[test]
fn sample_manifest_uses_canonical_naming() {
    let manifest = sample_foundation_manifest();

    assert_eq!(manifest.name().as_str(), "dome/buildroot:minimal");
    assert_eq!(manifest.format().extension(), ".foundation.dome");
    assert_eq!(manifest.virtual_size().as_mib(), 256);
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Sample {
    name: String,
    count: u32,
}

#[test]
fn round_trip_helper_accepts_equal_values() {
    let value = Sample {
        name: "dome".to_owned(),
        count: 3,
    };

    verify_round_trip(&value).expect("identical values must round-trip");
}

#[test]
fn round_trip_helper_is_usable_from_tests() {
    fn check() -> Result<(), Box<dyn std::error::Error>> {
        verify_round_trip(&Sample {
            name: "dome".to_owned(),
            count: 7,
        })?;
        Ok(())
    }

    check().expect("helper works through the ? operator");
}
