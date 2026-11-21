use dome_config::{Blueprint, ConfigError, MIN_MEMORY_MIB, MIN_STORAGE_MIB};
use dome_core::{
    AperturePolicy, CellSpec, CpuCount, FileAperture, FileApertureMode, FoundationName, MemorySize,
    PortMap, PortProtocol, ResourceLimits, StorageSize,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type BlueprintResult = Result<Blueprint, Box<dyn std::error::Error>>;

fn blueprint(memory_mib: u64, storage_mib: u64) -> BlueprintResult {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(memory_mib)?,
        StorageSize::from_mib(storage_mib)?,
        CpuCount::new(1)?,
    );

    Ok(Blueprint::new(CellSpec::new(
        FoundationName::new("dome/buildroot:minimal")?,
        limits,
    )))
}

fn with_files(apertures: AperturePolicy) -> BlueprintResult {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(512)?,
        StorageSize::from_mib(2048)?,
        CpuCount::new(1)?,
    );

    Ok(Blueprint::new(
        CellSpec::new(FoundationName::new("dome/buildroot:minimal")?, limits)
            .with_apertures(apertures),
    ))
}

fn fields(errors: &[ConfigError]) -> Vec<String> {
    errors
        .iter()
        .map(|error| match error {
            ConfigError::InvalidValue { field, .. } => (*field).to_owned(),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn accepts_the_shared_fixture() -> TestResult {
    // Fixture text and its path both come from dome-test-utils, so this test cannot
    // drift from what the CLI and other crates use.
    let blueprint = Blueprint::from_toml(dome_test_utils::valid_cell_toml())?;
    blueprint
        .validate()
        .expect("the shared fixture must be valid");

    assert!(dome_test_utils::test_fixture_path("valid.cell.toml").is_file());
    Ok(())
}

#[test]
fn accepts_a_minimal_chamber_at_the_boundaries() -> TestResult {
    blueprint(MIN_MEMORY_MIB, MIN_STORAGE_MIB)?
        .validate()
        .expect("boundary values are valid");
    Ok(())
}

#[test]
fn rejects_memory_below_the_minimum() -> TestResult {
    let errors = blueprint(MIN_MEMORY_MIB - 1, 2048)
        .expect("construction of limits still succeeds")
        .validate()
        .unwrap_err();

    assert_eq!(fields(&errors), vec!["limits.memory_mib"]);
    assert!(errors[0].to_string().contains("minimum"));
    Ok(())
}

#[test]
fn rejects_storage_below_the_minimum() -> TestResult {
    let errors = blueprint(512, MIN_STORAGE_MIB - 1)
        .expect("construction of limits still succeeds")
        .validate()
        .unwrap_err();

    assert_eq!(fields(&errors), vec!["limits.storage_mib"]);
    Ok(())
}

#[test]
fn rejects_duplicate_published_ports() -> TestResult {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(512)?,
        StorageSize::from_mib(2048)?,
        CpuCount::new(1)?,
    );

    let cell = CellSpec::new(FoundationName::new("dome/test")?, limits)
        .with_port(PortMap::local_tcp(8080, 80)?)
        .with_port(PortMap::local_tcp(8080, 8080)?);

    let errors = Blueprint::new(cell).validate().unwrap_err();
    assert_eq!(fields(&errors), vec!["perimeter.ports"]);
    assert!(errors[0].to_string().contains("published twice"));
    Ok(())
}

#[test]
fn allows_the_same_port_on_different_protocols() -> TestResult {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(512)?,
        StorageSize::from_mib(2048)?,
        CpuCount::new(1)?,
    );

    // Port 53 on both TCP and UDP is legal and common (DNS).
    let cell = CellSpec::new(FoundationName::new("dome/test")?, limits)
        .with_port(PortMap::new("127.0.0.1", 53, 53, PortProtocol::Tcp)?)
        .with_port(PortMap::new("127.0.0.1", 53, 53, PortProtocol::Udp)?);

    Blueprint::new(cell)
        .validate()
        .expect("tcp+udp on one port is fine");
    Ok(())
}

#[test]
fn rejects_relative_aperture_paths() -> TestResult {
    let apertures = AperturePolicy::closed().with_file(FileAperture::new(
        "relative/host",
        "relative/guest",
        FileApertureMode::ReadOnly,
    )?);

    let errors = with_files(apertures)?.validate().unwrap_err();
    assert_eq!(
        fields(&errors),
        vec!["apertures.files.host", "apertures.files.guest"]
    );
    Ok(())
}

#[test]
fn rejects_blocked_host_roots() -> TestResult {
    for blocked in ["/proc", "/proc/self/environ", "/sys", "/dev", "/dev/null"] {
        let apertures = AperturePolicy::closed().with_file(FileAperture::new(
            blocked,
            "/work",
            FileApertureMode::ReadOnly,
        )?);

        let errors = with_files(apertures)?.validate().unwrap_err();
        assert_eq!(
            fields(&errors),
            vec!["apertures.files.host"],
            "{blocked} should be rejected"
        );
        assert!(errors[0].to_string().contains("blocked"));
    }

    Ok(())
}

#[test]
fn accepts_paths_that_merely_start_with_a_blocked_root() -> TestResult {
    // "/procdata" is not "/proc".
    let apertures = AperturePolicy::closed()
        .with_file(FileAperture::new(
            "/procdata/inputs",
            "/work",
            FileApertureMode::ReadOnly,
        )?)
        .with_file(FileAperture::new(
            "/devices/gpio",
            "/gpio",
            FileApertureMode::ReadOnly,
        )?);

    with_files(apertures)?
        .validate()
        .expect("prefix collisions must not be rejected");
    Ok(())
}

#[test]
fn rejects_path_traversal() -> TestResult {
    let apertures = AperturePolicy::closed().with_file(FileAperture::new(
        "/tmp/../proc/self",
        "/work",
        FileApertureMode::ReadOnly,
    )?);

    let errors = with_files(apertures)?.validate().unwrap_err();
    assert!(errors[0].to_string().contains("canonical"));
    Ok(())
}

#[test]
fn reports_every_problem_at_once() -> TestResult {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(32)?,
        StorageSize::from_mib(64)?,
        CpuCount::new(1)?,
    );

    let apertures = AperturePolicy::closed()
        .with_file(FileAperture::new(
            "/proc/version",
            "relative",
            FileApertureMode::ReadOnly,
        )?)
        .with_gpio(dome_core::GpioAperture::new(
            "led",
            "gpiochip0",
            1,
            "gpiochip1:1",
            dome_core::GpioDirection::Output,
        )?);

    let cell = CellSpec::new(FoundationName::new("dome/test")?, limits)
        .with_port(PortMap::local_tcp(8080, 80)?)
        .with_port(PortMap::local_tcp(8080, 80)?)
        .with_apertures(apertures);

    let errors = Blueprint::new(cell).validate().unwrap_err();

    // memory, storage, duplicate port, blocked host path, relative guest path
    assert_eq!(errors.len(), 5, "got {errors:?}");
    Ok(())
}
