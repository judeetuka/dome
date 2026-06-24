use dome_device::{DeviceCandidate, DeviceClass, PassthroughReadiness};

#[test]
fn device_candidate_reports_readiness() {
    let candidate = DeviceCandidate::new(
        DeviceClass::Gpu,
        "/sys/bus/pci/devices/0000:01:00.0",
        PassthroughReadiness::ReadyForExplicitOptIn,
    );

    assert_eq!(candidate.class(), DeviceClass::Gpu);
    assert_eq!(
        candidate.readiness(),
        PassthroughReadiness::ReadyForExplicitOptIn
    );
}
