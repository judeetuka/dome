//! M1 acceptance: the commands PLAN.md lists under "After M1" must work end to end.

use dome_cli::{CliError, run};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn fixture(name: &str) -> String {
    format!("../../tests/fixtures/{name}")
}

#[test]
fn doctor_reports_fixture_capabilities_with_devices() -> TestResult {
    let output = run(argv(&[
        "doctor",
        "--format",
        "toml",
        "--fixtures",
        "../../tests/fixtures/linux-host",
    ]))?;

    for expected in [
        "schema_version = 1",
        "[platform]",
        "native_engine_device_available = true",
        "native_engine_device_readable = true",
        "[cgroups]",
        "v2_available = true",
        "[assembler]",
        "[devices]",
        "[[devices.candidates]]",
        "readiness = \"unsafe-iommu-group\"",
    ] {
        assert!(
            output.contains(expected),
            "missing {expected} in:\n{output}"
        );
    }

    Ok(())
}

#[test]
fn doctor_human_format_covers_the_same_facts() -> TestResult {
    let output = run(argv(&[
        "doctor",
        "--format",
        "human",
        "--fixtures",
        "../../tests/fixtures/linux-host",
    ]))?;

    for expected in [
        "Dome host capability report",
        "native engine device: yes",
        "native engine device readable: yes",
        "cgroups v2: yes",
        "devices: 4",
        "vfio nodes: 1",
    ] {
        assert!(
            output.contains(expected),
            "missing {expected} in:\n{output}"
        );
    }

    Ok(())
}

#[test]
fn validate_accepts_the_valid_fixture_in_both_formats() -> TestResult {
    for format in ["toml", "human"] {
        let output = run(argv(&[
            "validate",
            &fixture("valid.cell.toml"),
            "--format",
            format,
        ]))?;
        assert!(
            output.contains("valid"),
            "{format} output should state validity:\n{output}"
        );
    }

    Ok(())
}

#[test]
fn every_invalid_fixture_is_rejected_with_exit_3() {
    let cases = [
        ("invalid-zero-memory.cell.toml", "limits"),
        ("invalid-no-foundation.cell.toml", "cell"),
        ("invalid-duplicate-ports.cell.toml", "published twice"),
        ("invalid-blocked-aperture.cell.toml", "blocked host path"),
        ("invalid-limits.cell.toml", "problem(s)"),
    ];

    for (name, expected_fragment) in cases {
        let error = run(argv(&["validate", &fixture(name)])).unwrap_err();

        assert_eq!(error.exit_code(), 3, "{name} should exit 3");
        assert!(
            matches!(error, CliError::InvalidBlueprint { .. }),
            "{name} should be an InvalidBlueprint, got {error:?}"
        );
        assert!(
            error.to_string().contains(expected_fragment),
            "{name} message should mention {expected_fragment}: {error}"
        );
    }
}

#[test]
fn validation_does_not_touch_the_host() -> TestResult {
    // M1 is read-only: validating a blueprint must not need KVM, cgroups or root, and
    // must succeed on a host that has none of them.
    let output = run(argv(&["validate", &fixture("valid.cell.toml")]))?;

    assert!(output.contains("valid = true"));
    Ok(())
}
