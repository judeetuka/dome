use dome_config::Blueprint;
use dome_core::{
    AperturePolicy, CellSpec, CpuCount, DeviceAccess, DisplayMode, EgressPolicy, FileAperture,
    FileApertureMode, FoundationName, GpioAperture, GpioDirection, MemorySize, PortMap,
    PortProtocol, ResourceLimits, StorageSize,
};

fn rich_cell() -> Result<CellSpec, Box<dyn std::error::Error>> {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(1024)?,
        StorageSize::from_mib(8192)?,
        CpuCount::new(2)?,
    );

    let apertures = AperturePolicy::closed()
        .with_file(FileAperture::new(
            "/host/src",
            "/work",
            FileApertureMode::ReadWrite,
        )?)
        .with_file(FileAperture::new(
            "/host/docs",
            "/docs",
            FileApertureMode::ReadOnly,
        )?)
        .with_gpio(
            GpioAperture::new(
                "status-led",
                "gpiochip0",
                17,
                "gpiochip1:22",
                GpioDirection::Output,
            )?
            .with_rate_limit_hz(50)?,
        )
        .with_device_access(DeviceAccess::ExplicitApertures);

    Ok(
        CellSpec::new(FoundationName::new("dome/alpine:edge")?, limits)
            .with_port(PortMap::new("127.0.0.1", 8080, 80, PortProtocol::Tcp)?)
            .with_port(PortMap::new("0.0.0.0", 5353, 53, PortProtocol::Udp)?)
            .with_egress(EgressPolicy::Restricted)
            .with_display(DisplayMode::Vnc)
            .with_apertures(apertures),
    )
}

fn minimal_cell() -> Result<CellSpec, Box<dyn std::error::Error>> {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(512)?,
        StorageSize::from_mib(2048)?,
        CpuCount::new(1)?,
    );

    Ok(CellSpec::new(
        FoundationName::new("dome/buildroot:minimal")?,
        limits,
    ))
}

#[test]
fn round_trips_a_fully_populated_blueprint() -> Result<(), Box<dyn std::error::Error>> {
    let blueprint = Blueprint::new(rich_cell()?);
    let rendered = blueprint.to_toml()?;

    let parsed = Blueprint::from_toml(&rendered)?;
    assert_eq!(parsed, blueprint, "render then parse must be lossless");

    Ok(())
}

#[test]
fn rendering_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let rendered = Blueprint::new(rich_cell()?).to_toml()?;
    let parsed = Blueprint::from_toml(&rendered)?;
    let re_rendered = parsed.to_toml()?;

    assert_eq!(
        rendered, re_rendered,
        "a second render must be byte-identical"
    );
    Ok(())
}

#[test]
fn round_trips_a_minimal_blueprint() -> Result<(), Box<dyn std::error::Error>> {
    let blueprint = Blueprint::new(minimal_cell()?);
    let rendered = blueprint.to_toml()?;

    assert_eq!(Blueprint::from_toml(&rendered)?, blueprint);

    // Optional tables are omitted entirely rather than written empty.
    assert!(!rendered.contains("[perimeter]"));
    assert!(!rendered.contains("[apertures]"));
    assert!(!rendered.contains("egress"));
    assert!(!rendered.contains("display"));

    Ok(())
}

#[test]
fn renders_the_documented_blueprint_shape() -> Result<(), Box<dyn std::error::Error>> {
    let rendered = Blueprint::new(rich_cell()?).to_toml()?;

    assert!(rendered.contains("[cell]"));
    assert!(rendered.contains("foundation = \"dome/alpine:edge\""));
    assert!(rendered.contains("display = \"vnc\""));
    assert!(rendered.contains("[limits]"));
    assert!(rendered.contains("memory_mib = 1024"));
    assert!(rendered.contains("storage_mib = 8192"));
    assert!(rendered.contains("cpus = 2"));
    assert!(rendered.contains("[[perimeter.ports]]"));
    assert!(rendered.contains("egress = \"restricted\""));
    assert!(rendered.contains("protocol = \"udp\""));
    assert!(rendered.contains("[[apertures.files]]"));
    assert!(rendered.contains("mode = \"read-write\""));
    assert!(rendered.contains("[[apertures.gpio]]"));
    assert!(rendered.contains("direction = \"output\""));
    assert!(rendered.contains("rate_limit_hz = 50"));
    assert!(rendered.contains("device_access = \"explicit_apertures\""));

    Ok(())
}
