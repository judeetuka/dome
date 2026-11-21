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
}

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
}

pub trait Engine {
    type Error;

    fn capabilities(&self) -> EngineCapabilities;
    fn validate(&self, plan: &LaunchPlan) -> Result<(), Self::Error>;
}
