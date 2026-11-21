use dome_core::{CellSpec, ChamberId};

/// Errors raised while preparing or running a chamber on a platform engine.
///
/// `DeviceUnavailable` is kept distinct from the rest on purpose: a host without
/// `/dev/kvm` is a capability gap (CLI exit 4), not an engine fault (exit 5).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("hypervisor device unavailable: {0}")]
    DeviceUnavailable(String),
    #[error("engine setup failed: {0}")]
    Setup(String),
    #[error("guest memory allocation failed: {0}")]
    Memory(String),
    #[error("guest architecture {guest} is not supported by engine {engine}")]
    UnsupportedGuestArch { engine: String, guest: String },
    #[error("launch failed: {0}")]
    Launch(String),
    #[error("host cannot contain a DMA-capable aperture: {0}")]
    IoMmuUnavailable(String),
}

pub mod boot;
pub mod cpuid;
pub mod launch;
pub mod loader;
pub mod memory;
pub mod sandbox;
pub mod vcpu;

#[cfg(target_os = "linux")]
pub mod linux;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    chamber_id: ChamberId,
    cell: CellSpec,
}

impl LaunchPlan {
    pub fn new(chamber_id: ChamberId, cell: CellSpec) -> Self {
        Self { chamber_id, cell }
    }

    pub fn chamber_id(&self) -> &ChamberId {
        &self.chamber_id
    }

    pub fn cell(&self) -> &CellSpec {
        &self.cell
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HostOs {
    Linux,
    Macos,
    Windows,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CpuArch {
    X86_64,
    Aarch64,
}

/// Hardware-backed confidentiality for guest memory.
///
/// Ordered by strength. `Unavailable` means guest RAM sits in the VMM's address space and
/// a host root can read it — the honest default on most consumer hardware. Anything above
/// `Unavailable` encrypts guest memory against the host and is the only basis on which
/// host-to-chamber confidentiality may be claimed (see docs/adr/0002-isolation-model.md).
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfidentialMemory {
    /// No hardware support: host root can read guest memory.
    Unavailable,
    /// AMD Secure Encrypted Virtualization (encrypted, no integrity protection).
    Sev,
    /// SEV with Encrypted State (register state protected too).
    SevEs,
    /// SEV Secure Nested Paging: encrypted, integrity-protected, attestable.
    SevSnp,
    /// Intel Trust Domain Extensions: encrypted, integrity-protected, attestable.
    Tdx,
    /// ARM Confidential Compute Architecture realms.
    Cca,
}

impl ConfidentialMemory {
    /// Stable wire name for reports and blueprints.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Sev => "sev",
            Self::SevEs => "sev-es",
            Self::SevSnp => "sev-snp",
            Self::Tdx => "tdx",
            Self::Cca => "cca",
        }
    }

    /// Whether this mode lets Dome claim confidentiality against a malicious host.
    ///
    /// Deliberately narrow: plain SEV has known weaknesses (no integrity, IV reuse fixed
    /// only in later revisions), so only SEV-ES and above qualify.
    pub fn protects_against_host(self) -> bool {
        matches!(self, Self::SevEs | Self::SevSnp | Self::Tdx | Self::Cca)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AccelerationMode {
    NativeHypervisor,
    Unavailable,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PreparedStorageMode {
    RawSparse,
    PlatformNative,
    MemoryOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineCapabilities {
    pub name: String,
    pub host_os: HostOs,
    pub host_arch: CpuArch,
    pub guest_arches: Vec<CpuArch>,
    pub acceleration: AccelerationMode,
    pub storage_modes: Vec<PreparedStorageMode>,
    pub supports_gpio_apertures: bool,
    pub supports_file_apertures: bool,
    pub supports_display: bool,
    pub supports_cold_seals: bool,
    /// Hardware confidentiality available for chamber memory on this host.
    pub confidential_memory: ConfidentialMemory,
    /// Whether the host enforces an IOMMU, without which no DMA-capable aperture is safe.
    pub enforces_iommu: bool,
}

pub trait Engine {
    type Error;

    fn capabilities(&self) -> EngineCapabilities;
    fn validate(&self, plan: &LaunchPlan) -> Result<(), Self::Error>;
}
