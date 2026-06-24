use dome_core::{
    AperturePolicy, CpuCount, DomeError, FileAperture, FileApertureMode, GpioAperture,
    GpioDirection, MemorySize, PortMap, ResourceLimits, StorageSize,
};

#[test]
fn rejects_zero_memory() {
    assert_eq!(MemorySize::from_mib(0), Err(DomeError::ZeroMemory));
}

#[test]
fn builds_resource_limits() -> Result<(), DomeError> {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(2048)?,
        StorageSize::from_mib(20 * 1024)?,
        CpuCount::new(2)?,
    );

    assert_eq!(limits.memory().as_mib(), 2048);
    assert_eq!(limits.storage().as_mib(), 20 * 1024);
    assert_eq!(limits.cpus().get(), 2);

    Ok(())
}

#[test]
fn local_tcp_port_map_is_explicit() -> Result<(), DomeError> {
    let port = PortMap::local_tcp(8080, 80)?;

    assert_eq!(port.host_addr(), "127.0.0.1");
    assert_eq!(port.host_port(), 8080);
    assert_eq!(port.guest_port(), 80);
    assert_eq!(port.protocol().as_str(), "tcp");

    Ok(())
}

#[test]
fn apertures_are_explicit() -> Result<(), DomeError> {
    let policy = AperturePolicy::closed()
        .with_file(FileAperture::new(
            "/host/project",
            "/work",
            FileApertureMode::ReadOnly,
        )?)
        .with_gpio(GpioAperture::new(
            "status-led",
            "gpiochip0",
            17,
            "gpiochip1:22",
            GpioDirection::Output,
        )?);

    assert_eq!(policy.files().len(), 1);
    assert_eq!(policy.gpio().len(), 1);
    assert_eq!(policy.files()[0].mode().as_str(), "read-only");
    assert_eq!(policy.gpio()[0].direction().as_str(), "output");

    Ok(())
}
