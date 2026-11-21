#![cfg(target_os = "linux")]

use dome_core::{
    AperturePolicy, CellSpec, CpuCount, DeviceAccess, FoundationName, MemorySize, ResourceLimits,
    StorageSize,
};
use dome_engine::linux::{LinuxEngine, host_memory_mib};
use dome_engine::{AccelerationMode, ConfidentialMemory, Engine, EngineError, HostOs, LaunchPlan};

fn plan_with(memory_mib: u64, cpus: u16, device_access: DeviceAccess) -> LaunchPlan {
    let limits = ResourceLimits::new(
        MemorySize::from_mib(memory_mib).expect("nonzero memory"),
        StorageSize::from_mib(2048).expect("nonzero storage"),
        CpuCount::new(cpus).expect("nonzero cpus"),
    );

    let cell = CellSpec::new(
        FoundationName::new("dome/buildroot:minimal").expect("name"),
        limits,
    )
    .with_apertures(AperturePolicy::closed().with_device_access(device_access));

    LaunchPlan::new(dome_core::ChamberId::new("test").expect("id"), cell)
}

#[test]
fn reports_capabilities_or_a_typed_error() {
    match LinuxEngine::open() {
        Ok(engine) => {
            let caps = engine.capabilities();
            assert_eq!(caps.host_os, HostOs::Linux);
            assert_eq!(caps.acceleration, AccelerationMode::NativeHypervisor);
            assert_eq!(caps.guest_arches, vec![caps.host_arch]);
            assert_eq!(caps.name, "linux-kvm");
        }
        Err(EngineError::DeviceUnavailable(message)) => {
            // A host without /dev/kvm must produce this, not a panic.
            eprintln!("host has no usable KVM: {message}");
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn validation_accepts_a_sane_chamber() {
    let Ok(engine) = LinuxEngine::open() else {
        eprintln!("skipping: host has no usable KVM");
        return;
    };

    engine
        .validate(&plan_with(256, 1, DeviceAccess::Disabled))
        .expect("a small chamber should validate");
}

#[test]
fn validation_rejects_more_memory_than_the_host_has() {
    let Ok(engine) = LinuxEngine::open() else {
        return;
    };

    let host_mib = host_memory_mib().unwrap_or(0);
    let error = engine
        .validate(&plan_with(
            host_mib.saturating_add(1024),
            1,
            DeviceAccess::Disabled,
        ))
        .unwrap_err();

    assert!(matches!(error, EngineError::Memory(_)), "got {error:?}");
}

#[test]
fn validation_refuses_dma_apertures_without_an_iommu() {
    let Ok(engine) = LinuxEngine::open() else {
        return;
    };

    if engine.iommu_enforced() {
        eprintln!("skipping: this host does enforce an IOMMU");
        return;
    }

    // ADR 0002 (F): refuse rather than pretend a DMA-capable device can be contained.
    let error = engine
        .validate(&plan_with(256, 1, DeviceAccess::ExplicitApertures))
        .unwrap_err();

    assert!(
        matches!(error, EngineError::IoMmuUnavailable(_)),
        "got {error:?}"
    );
}

#[test]
fn confidential_memory_is_reported_honestly() {
    let Ok(engine) = LinuxEngine::open() else {
        return;
    };

    let mode = engine.confidential_memory();

    // Whatever the host offers, the claim must match the hardware.
    if mode.protects_against_host() {
        assert!(matches!(
            mode,
            ConfidentialMemory::SevEs | ConfidentialMemory::SevSnp | ConfidentialMemory::Tdx
        ));
    }
}
