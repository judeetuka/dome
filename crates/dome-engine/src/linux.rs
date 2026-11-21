//! The Linux native engine: KVM through kvm-ioctls.
//!
//! This module owns the host-side facts M3 depends on: whether KVM is usable, what
//! confidentiality the hardware offers, and whether an IOMMU can contain a DMA-capable
//! aperture. Per ADR 0002 nothing here assumes; every value is probed.

use std::fs;

use dome_core::{CellSpec, DeviceAccess};
use kvm_ioctls::Kvm;

use crate::{
    AccelerationMode, ConfidentialMemory, CpuArch, Engine, EngineCapabilities, EngineError, HostOs,
    LaunchPlan, PreparedStorageMode,
};

/// Directory the kernel exposes IOMMU groups in.
const IOMMU_GROUPS: &str = "/sys/kernel/iommu_groups";

/// A KVM-backed engine on the host this process is running on.
pub struct LinuxEngine {
    kvm: Kvm,
    confidential: ConfidentialMemory,
    iommu_enforced: bool,
}

impl LinuxEngine {
    /// Open /dev/kvm and probe what this host can do.
    pub fn open() -> Result<Self, EngineError> {
        let kvm = Kvm::new().map_err(|error| {
            EngineError::DeviceUnavailable(format!("/dev/kvm unusable: {error}"))
        })?;

        Ok(Self {
            kvm,
            confidential: detect_confidential_memory(),
            iommu_enforced: iommu_enforced(),
        })
    }

    /// Confidentiality available for guest memory on this host.
    pub fn confidential_memory(&self) -> ConfidentialMemory {
        self.confidential
    }

    /// Whether an IOMMU is enforced here.
    pub fn iommu_enforced(&self) -> bool {
        self.iommu_enforced
    }

    /// Access to the KVM handle, for building VMs.
    pub fn kvm(&self) -> &Kvm {
        &self.kvm
    }

    /// Build the capability report for this host.
    pub fn capabilities(&self) -> EngineCapabilities {
        let arch = host_arch();

        EngineCapabilities {
            name: "linux-kvm".to_owned(),
            host_os: HostOs::Linux,
            host_arch: arch,
            // Same-architecture execution only: no emulation, ever.
            guest_arches: vec![arch],
            acceleration: AccelerationMode::NativeHypervisor,
            storage_modes: vec![PreparedStorageMode::RawSparse],
            supports_gpio_apertures: true,
            supports_file_apertures: true,
            supports_display: false,
            supports_cold_seals: false,
            confidential_memory: self.confidential,
            enforces_iommu: self.iommu_enforced,
        }
    }

    /// Check a launch plan against this host.
    pub fn validate_plan(&self, plan: &LaunchPlan) -> Result<(), EngineError> {
        let cell: &CellSpec = plan.cell();

        // ADR 0002 (F): a DMA-capable aperture without an IOMMU can read every chamber's
        // memory. Refuse rather than degrade.
        if cell.apertures().device_access() == DeviceAccess::ExplicitApertures
            && !self.iommu_enforced
        {
            return Err(EngineError::IoMmuUnavailable(
                "this host exposes no IOMMU groups, so device passthrough cannot be contained"
                    .to_owned(),
            ));
        }

        let requested_mib = cell.limits().memory().as_mib();
        let host_mib = host_memory_mib().unwrap_or(u64::MAX);

        if requested_mib > host_mib {
            return Err(EngineError::Memory(format!(
                "chamber requests {requested_mib} MiB but the host has {host_mib} MiB"
            )));
        }

        let host_cpus = std::thread::available_parallelism()
            .map(|count| count.get() as u16)
            .unwrap_or(1);

        if cell.limits().cpus().get() > host_cpus {
            return Err(EngineError::Setup(format!(
                "chamber requests {} vCPUs but the host has {host_cpus}",
                cell.limits().cpus().get()
            )));
        }

        Ok(())
    }
}

impl Engine for LinuxEngine {
    type Error = EngineError;

    fn capabilities(&self) -> EngineCapabilities {
        Self::capabilities(self)
    }

    fn validate(&self, plan: &LaunchPlan) -> Result<(), Self::Error> {
        self.validate_plan(plan)
    }
}

/// The architecture this binary was built for.
pub fn host_arch() -> CpuArch {
    #[cfg(target_arch = "x86_64")]
    {
        CpuArch::X86_64
    }

    #[cfg(target_arch = "aarch64")]
    {
        CpuArch::Aarch64
    }
}

/// Best confidentiality the CPU and its devices actually offer.
///
/// Flags and device nodes are both required: firmware regularly ships a CPU that lists SEV
/// while leaving it disabled, so advertising a flag is not proof of availability.
pub fn detect_confidential_memory() -> ConfidentialMemory {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let has_flag = |flag: &str| cpuinfo.split_whitespace().any(|token| token == flag);

    if has_flag("tdx") && (path_exists("/dev/tdx_guest") || path_exists("/dev/tdx-guest")) {
        return ConfidentialMemory::Tdx;
    }

    let sev_device = path_exists("/dev/sev") || path_exists("/dev/sev-guest");

    if sev_device && has_flag("sev_snp") {
        return ConfidentialMemory::SevSnp;
    }

    if sev_device && has_flag("sev_es") {
        return ConfidentialMemory::SevEs;
    }

    if sev_device && has_flag("sev") {
        return ConfidentialMemory::Sev;
    }

    ConfidentialMemory::Unavailable
}

/// Whether the host exposes IOMMU groups, which is what makes DMA containment real.
pub fn iommu_enforced() -> bool {
    fs::read_dir(IOMMU_GROUPS)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

fn path_exists(path: &str) -> bool {
    fs::metadata(path).is_ok()
}

/// Total RAM in MiB, from /proc/meminfo.
pub fn host_memory_mib() -> Option<u64> {
    let meminfo = fs::read_to_string("/proc/meminfo").ok()?;

    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kib / 1024);
        }
    }

    None
}
