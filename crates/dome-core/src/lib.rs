use std::fmt::{Display, Formatter};
use std::num::{NonZeroU16, NonZeroU64};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomeError {
    EmptyChamberId,
    EmptyFoundationName,
    EmptyHostPath,
    EmptyGuestPath,
    InvalidPort(u16),
    ZeroCpuCount,
    ZeroGpioLine,
    ZeroMemory,
    ZeroStorage,
}

impl Display for DomeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyChamberId => formatter.write_str("chamber id cannot be empty"),
            Self::EmptyFoundationName => formatter.write_str("foundation name cannot be empty"),
            Self::EmptyHostPath => formatter.write_str("host aperture path cannot be empty"),
            Self::EmptyGuestPath => formatter.write_str("guest aperture path cannot be empty"),
            Self::InvalidPort(port) => write!(formatter, "port {port} is invalid"),
            Self::ZeroCpuCount => formatter.write_str("cpu count must be greater than zero"),
            Self::ZeroGpioLine => formatter.write_str("gpio line must be greater than zero"),
            Self::ZeroMemory => formatter.write_str("memory must be greater than zero"),
            Self::ZeroStorage => formatter.write_str("storage must be greater than zero"),
        }
    }
}

impl std::error::Error for DomeError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FoundationName(String);

impl FoundationName {
    pub fn new(value: impl Into<String>) -> Result<Self, DomeError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DomeError::EmptyFoundationName);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChamberId(String);

impl ChamberId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomeError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DomeError::EmptyChamberId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemorySize {
    mib: NonZeroU64,
}

impl MemorySize {
    pub fn from_mib(mib: u64) -> Result<Self, DomeError> {
        let mib = NonZeroU64::new(mib).ok_or(DomeError::ZeroMemory)?;
        Ok(Self { mib })
    }

    pub fn as_mib(self) -> u64 {
        self.mib.get()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StorageSize {
    mib: NonZeroU64,
}

impl StorageSize {
    pub fn from_mib(mib: u64) -> Result<Self, DomeError> {
        let mib = NonZeroU64::new(mib).ok_or(DomeError::ZeroStorage)?;
        Ok(Self { mib })
    }

    pub fn as_mib(self) -> u64 {
        self.mib.get()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CpuCount {
    count: NonZeroU16,
}

impl CpuCount {
    pub fn new(count: u16) -> Result<Self, DomeError> {
        let count = NonZeroU16::new(count).ok_or(DomeError::ZeroCpuCount)?;
        Ok(Self { count })
    }

    pub fn get(self) -> u16 {
        self.count.get()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ResourceLimits {
    memory: MemorySize,
    storage: StorageSize,
    cpus: CpuCount,
}

impl ResourceLimits {
    pub fn new(memory: MemorySize, storage: StorageSize, cpus: CpuCount) -> Self {
        Self {
            memory,
            storage,
            cpus,
        }
    }

    pub fn memory(self) -> MemorySize {
        self.memory
    }

    pub fn storage(self) -> StorageSize {
        self.storage
    }

    pub fn cpus(self) -> CpuCount {
        self.cpus
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PortProtocol {
    Tcp,
    Udp,
}

impl PortProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortMap {
    host_addr: String,
    host_port: NonZeroU16,
    guest_port: NonZeroU16,
    protocol: PortProtocol,
}

impl PortMap {
    pub fn new(
        host_addr: impl Into<String>,
        host_port: u16,
        guest_port: u16,
        protocol: PortProtocol,
    ) -> Result<Self, DomeError> {
        let host_port = NonZeroU16::new(host_port).ok_or(DomeError::InvalidPort(host_port))?;
        let guest_port = NonZeroU16::new(guest_port).ok_or(DomeError::InvalidPort(guest_port))?;

        Ok(Self {
            host_addr: host_addr.into(),
            host_port,
            guest_port,
            protocol,
        })
    }

    pub fn local_tcp(host_port: u16, guest_port: u16) -> Result<Self, DomeError> {
        Self::new("127.0.0.1", host_port, guest_port, PortProtocol::Tcp)
    }

    pub fn host_addr(&self) -> &str {
        &self.host_addr
    }

    pub fn host_port(&self) -> u16 {
        self.host_port.get()
    }

    pub fn guest_port(&self) -> u16 {
        self.guest_port.get()
    }

    pub fn protocol(&self) -> PortProtocol {
        self.protocol
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EgressPolicy {
    None,
    Nat,
    Restricted,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DisplayMode {
    None,
    Vnc,
    Spice,
    WaylandSeamless,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DeviceAccess {
    Disabled,
    VirtioOnly,
    ExplicitApertures,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSpec {
    foundation: FoundationName,
    limits: ResourceLimits,
    ports: Vec<PortMap>,
    egress: EgressPolicy,
    display: DisplayMode,
    apertures: AperturePolicy,
}

impl CellSpec {
    pub fn new(foundation: FoundationName, limits: ResourceLimits) -> Self {
        Self {
            foundation,
            limits,
            ports: Vec::new(),
            egress: EgressPolicy::None,
            display: DisplayMode::None,
            apertures: AperturePolicy::closed(),
        }
    }

    pub fn with_port(mut self, port: PortMap) -> Self {
        self.ports.push(port);
        self
    }

    pub fn with_egress(mut self, egress: EgressPolicy) -> Self {
        self.egress = egress;
        self
    }

    pub fn with_display(mut self, display: DisplayMode) -> Self {
        self.display = display;
        self
    }

    pub fn with_apertures(mut self, apertures: AperturePolicy) -> Self {
        self.apertures = apertures;
        self
    }

    pub fn foundation(&self) -> &FoundationName {
        &self.foundation
    }

    pub fn limits(&self) -> ResourceLimits {
        self.limits
    }

    pub fn ports(&self) -> &[PortMap] {
        &self.ports
    }

    pub fn egress(&self) -> EgressPolicy {
        self.egress
    }

    pub fn display(&self) -> DisplayMode {
        self.display
    }

    pub fn apertures(&self) -> &AperturePolicy {
        &self.apertures
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FileApertureMode {
    ReadOnly,
    ReadWrite,
}

impl FileApertureMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ReadWrite => "read-write",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAperture {
    host_path: String,
    guest_path: String,
    mode: FileApertureMode,
}

impl FileAperture {
    pub fn new(
        host_path: impl Into<String>,
        guest_path: impl Into<String>,
        mode: FileApertureMode,
    ) -> Result<Self, DomeError> {
        let host_path = host_path.into();
        let guest_path = guest_path.into();

        if host_path.trim().is_empty() {
            return Err(DomeError::EmptyHostPath);
        }

        if guest_path.trim().is_empty() {
            return Err(DomeError::EmptyGuestPath);
        }

        Ok(Self {
            host_path,
            guest_path,
            mode,
        })
    }

    pub fn host_path(&self) -> &str {
        &self.host_path
    }

    pub fn guest_path(&self) -> &str {
        &self.guest_path
    }

    pub fn mode(&self) -> FileApertureMode {
        self.mode
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GpioDirection {
    Input,
    Output,
    Bidirectional,
}

impl GpioDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::Bidirectional => "bidirectional",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpioAperture {
    name: String,
    guest_chip: String,
    guest_line: NonZeroU16,
    host_line: String,
    direction: GpioDirection,
    rate_limit_hz: Option<NonZeroU16>,
}

impl GpioAperture {
    pub fn new(
        name: impl Into<String>,
        guest_chip: impl Into<String>,
        guest_line: u16,
        host_line: impl Into<String>,
        direction: GpioDirection,
    ) -> Result<Self, DomeError> {
        let guest_line = NonZeroU16::new(guest_line).ok_or(DomeError::ZeroGpioLine)?;

        Ok(Self {
            name: name.into(),
            guest_chip: guest_chip.into(),
            guest_line,
            host_line: host_line.into(),
            direction,
            rate_limit_hz: None,
        })
    }

    pub fn with_rate_limit_hz(mut self, rate_limit_hz: u16) -> Result<Self, DomeError> {
        let rate_limit_hz = NonZeroU16::new(rate_limit_hz).ok_or(DomeError::ZeroGpioLine)?;
        self.rate_limit_hz = Some(rate_limit_hz);
        Ok(self)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn guest_chip(&self) -> &str {
        &self.guest_chip
    }

    pub fn guest_line(&self) -> u16 {
        self.guest_line.get()
    }

    pub fn host_line(&self) -> &str {
        &self.host_line
    }

    pub fn direction(&self) -> GpioDirection {
        self.direction
    }

    pub fn rate_limit_hz(&self) -> Option<u16> {
        self.rate_limit_hz.map(NonZeroU16::get)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AperturePolicy {
    files: Vec<FileAperture>,
    gpio: Vec<GpioAperture>,
    device_access: DeviceAccess,
}

impl AperturePolicy {
    pub fn closed() -> Self {
        Self {
            files: Vec::new(),
            gpio: Vec::new(),
            device_access: DeviceAccess::Disabled,
        }
    }

    pub fn with_file(mut self, aperture: FileAperture) -> Self {
        self.files.push(aperture);
        self
    }

    pub fn with_gpio(mut self, aperture: GpioAperture) -> Self {
        self.gpio.push(aperture);
        self
    }

    pub fn with_device_access(mut self, access: DeviceAccess) -> Self {
        self.device_access = access;
        self
    }

    pub fn files(&self) -> &[FileAperture] {
        &self.files
    }

    pub fn gpio(&self) -> &[GpioAperture] {
        &self.gpio
    }

    pub fn device_access(&self) -> DeviceAccess {
        self.device_access
    }
}
