use dome_cli::{CliError, run};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

const VALID: &str = "../../tests/fixtures/valid.cell.toml";
const INVALID_LIMITS: &str = "../../tests/fixtures/invalid-limits.cell.toml";

#[test]
fn version_reports_the_crate_version() -> TestResult {
    let output = run(argv(&["--version"]))?;

    assert!(output.contains("dome"), "got {output}");
    assert!(output.contains("0.1.0"), "got {output}");
    Ok(())
}

#[test]
fn validate_accepts_the_shared_fixture() -> TestResult {
    let output = run(argv(&["validate", VALID]))?;

    assert!(output.contains("[validation]"));
    assert!(output.contains("valid = true"));
    assert!(output.contains("foundation = \"dome/buildroot:minimal\""));
    assert!(output.contains("published_ports = 1"));
    Ok(())
}

#[test]
fn validate_human_format_is_readable() -> TestResult {
    let output = run(argv(&["validate", VALID, "--format", "human"]))?;

    assert!(output.contains("valid"));
    assert!(output.contains("foundation: dome/buildroot:minimal"));
    assert!(output.contains("512 MiB memory"));
    Ok(())
}

#[test]
fn validate_reports_every_rule_violation_at_once() {
    let error = run(argv(&["validate", INVALID_LIMITS])).unwrap_err();

    match error {
        CliError::InvalidBlueprint {
            count, messages, ..
        } => {
            assert_eq!(count, 4, "expected four problems, got {messages}");
            assert!(messages.contains("memory_mib"));
            assert!(messages.contains("storage_mib"));
            assert!(messages.contains("published twice"));
            assert!(messages.contains("blocked host path"));
        }
        other => panic!("expected InvalidBlueprint, got {other:?}"),
    }
}

#[test]
fn validation_failure_exits_3() {
    let error = run(argv(&["validate", INVALID_LIMITS])).unwrap_err();
    assert_eq!(error.exit_code(), 3);
}

#[test]
fn unreadable_blueprint_is_a_validation_failure() {
    let error = run(argv(&["validate", "definitely-not-here.cell.toml"])).unwrap_err();

    assert!(matches!(error, CliError::UnreadableFile { .. }));
    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("definitely-not-here"));
}

#[test]
fn unimplemented_commands_exit_2_and_say_so() {
    let cases: [&[&str]; 4] = [
        &["run", VALID],
        &["stop", "chamber-1"],
        &["seal", "chamber-1"],
        &["build", "--list"],
    ];

    for case in cases {
        let error = run(argv(case)).unwrap_err();
        assert!(
            matches!(error, CliError::NotYetImplemented(_)),
            "{case:?} gave {error:?}"
        );
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("not implemented"));
    }
}

#[test]
fn export_requires_an_output_path() {
    let error = run(argv(&["export", "chamber-1"])).unwrap_err();

    assert!(matches!(error, CliError::Usage(_)));
    assert!(error.to_string().contains("--output"));
}

#[test]
fn unknown_subcommand_is_a_usage_error() {
    let error = run(argv(&["bogus"])).unwrap_err();

    assert!(matches!(error, CliError::Usage(_)));
    assert!(error.to_string().contains("bogus"));
    assert_eq!(error.exit_code(), 2);
}

#[test]
fn unknown_format_is_a_usage_error() {
    let error = run(argv(&["doctor", "--format", "yaml"])).unwrap_err();

    assert!(matches!(error, CliError::Usage(_)));
    assert!(error.to_string().contains("yaml"));
}

#[test]
fn completions_are_generated_for_every_shell() -> TestResult {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let output = run(argv(&["completions", shell]))?;
        assert!(!output.trim().is_empty(), "{shell} produced nothing");
    }

    let bash = run(argv(&["completions", "bash"]))?;
    assert!(
        bash.contains("dome"),
        "completion should mention the binary"
    );
    Ok(())
}

#[test]
fn build_rejects_recipe_and_list_together() {
    let error = run(argv(&["build", "--recipe", "x", "--list"])).unwrap_err();
    assert!(matches!(error, CliError::Usage(_)));
}

#[test]
fn doctor_toml_declares_a_schema_version() -> TestResult {
    // docs/cli-contract.md section 4: --format toml is a stable contract and must
    // carry schema_version so consumers can detect a break.
    let output = run(argv(&["doctor"]))?;

    assert!(
        output.starts_with("schema_version = 1"),
        "schema_version must come first, got: {}",
        output.lines().next().unwrap_or_default()
    );
    Ok(())
}
