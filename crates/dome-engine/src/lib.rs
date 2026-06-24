use dome_core::{CellSpec, ChamberId};

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
