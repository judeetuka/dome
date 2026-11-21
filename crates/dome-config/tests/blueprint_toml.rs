use dome_config::Blueprint;
use dome_core::{
    AperturePolicy, CellSpec, CpuCount, FileAperture, FileApertureMode, FoundationName, MemorySize,
    PortMap, ResourceLimits, StorageSize,
};

#[test]
fn renders_blueprint_as_toml() -> Result<(), Box<dyn std::error::Error>> {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(512)?,
        StorageSize::from_mib(2048)?,
        CpuCount::new(1)?,
    );
    let apertures = AperturePolicy::closed().with_file(FileAperture::new(
        "/host/project",
        "/work",
        FileApertureMode::ReadOnly,
    )?);
    let cell = CellSpec::new(FoundationName::new("dome/buildroot:minimal")?, limits)
        .with_port(PortMap::local_tcp(8080, 80)?)
        .with_apertures(apertures);
    let output = Blueprint::new(cell).to_toml()?;

    assert!(output.contains("[cell]"));
    assert!(output.contains("foundation = \"dome/buildroot:minimal\""));
    assert!(output.contains("[[perimeter.ports]]"));
    assert!(output.contains("[[apertures.files]]"));

    Ok(())
}
