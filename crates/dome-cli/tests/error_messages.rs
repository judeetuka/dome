use dome_cli::CliError;

#[test]
fn cli_errors_render_stable_messages() {
    assert_eq!(
        CliError::MissingValue("--format").to_string(),
        "missing value for --format"
    );
    assert_eq!(
        CliError::UnknownCommand("chamber".to_owned()).to_string(),
        "unknown command: chamber"
    );
    assert_eq!(
        CliError::UnknownFlag("--verbose".to_owned()).to_string(),
        "unknown flag: --verbose"
    );
    assert_eq!(
        CliError::UnknownFormat("yaml".to_owned()).to_string(),
        "unknown output format: yaml"
    );
}
