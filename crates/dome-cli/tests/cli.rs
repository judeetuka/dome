use dome_cli::run;

#[test]
fn help_mentions_read_only_milestone() -> Result<(), Box<dyn std::error::Error>> {
    let output = run(Vec::new())?;

    assert!(output.contains("Milestone 1 is read-only"));

    Ok(())
}

#[test]
fn doctor_defaults_to_toml() -> Result<(), Box<dyn std::error::Error>> {
    let output = run(vec![
        "doctor".to_owned(),
        "--fixtures".to_owned(),
        "../../tests/fixtures/linux-host".to_owned(),
    ])?;

    assert!(output.contains("[platform]"));
    assert!(output.contains("native_engine_device_available = true"));

    Ok(())
}
