use dome_core::{
    CellSpec, ChamberId, CpuCount, DomeError, FoundationName, MemorySize, ResourceLimits,
    StorageSize,
};
use dome_engine::{
    AccelerationMode, CpuArch, EngineCapabilities, HostOs, LaunchPlan, PreparedStorageMode,
};

#[test]
fn launch_plan_uses_chamber_and_cell_names() -> Result<(), DomeError> {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(1024)?,
        StorageSize::from_mib(4096)?,
        CpuCount::new(2)?,
    );
    let cell = CellSpec::new(FoundationName::new("dome/alpine:edge")?, limits);
    let plan = LaunchPlan::new(ChamberId::new("api-dev")?, cell);

    assert_eq!(plan.chamber_id().as_str(), "api-dev");
    assert_eq!(plan.cell().foundation().as_str(), "dome/alpine:edge");

    Ok(())
}

#[test]
fn engine_capabilities_are_platform_neutral() {
    let capabilities = EngineCapabilities {
        name: "linux-native".to_owned(),
        host_os: HostOs::Linux,
        host_arch: CpuArch::X86_64,
        guest_arches: vec![CpuArch::X86_64],
        acceleration: AccelerationMode::NativeHypervisor,
        storage_modes: vec![PreparedStorageMode::RawSparse],
        supports_gpio_apertures: true,
        supports_file_apertures: true,
        supports_display: false,
        supports_cold_seals: true,
    };

    assert_eq!(capabilities.host_os, HostOs::Linux);
    assert_eq!(
        capabilities.acceleration,
        AccelerationMode::NativeHypervisor
    );
}
