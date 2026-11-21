use dome_core::{
    AperturePolicy, CellSpec, CpuCount, DeviceAccess, DisplayMode, DomeError, EgressPolicy,
    FileAperture, FileApertureMode, FoundationName, GpioAperture, GpioDirection, MemorySize,
    PortMap, PortProtocol, ResourceLimits, StorageSize,
};
use serde::{Deserialize, Serialize};

/// Errors raised while turning a `.cell.toml` blueprint into domain types.
///
/// Parsing and semantic validation are deliberately separate: a blueprint can be
/// syntactically valid TOML and still be unacceptable (zero memory, a blocked
/// aperture path, a duplicate host port).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to parse blueprint TOML: {0}")]
    Parse(String),
    #[error("failed to render blueprint TOML: {0}")]
    Render(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("invalid value for {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
    #[error("unknown {kind}: {value}")]
    Unknown { kind: &'static str, value: String },
}

/// A cell blueprint: the TOML file on disk that describes a chamber to build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blueprint {
    cell: CellSpec,
}

impl Blueprint {
    pub fn new(cell: CellSpec) -> Self {
        Self { cell }
    }

    pub fn cell(&self) -> &CellSpec {
        &self.cell
    }

    /// Parse a `.cell.toml` blueprint.
    pub fn from_toml(input: &str) -> Result<Self, ConfigError> {
        let file: BlueprintFile =
            toml::from_str(input).map_err(|error| ConfigError::Parse(error.to_string()))?;
        file.into_blueprint()
    }

    /// Render this blueprint back to `.cell.toml`.
    ///
    /// Rendering and parsing share one schema (BlueprintFile), so the two directions
    /// cannot drift apart.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        let file = BlueprintFile::from_cell(self.cell());
        toml::to_string_pretty(&file).map_err(|error| ConfigError::Render(error.to_string()))
    }
}

fn invalid(field: &'static str, error: DomeError) -> ConfigError {
    ConfigError::InvalidValue {
        field,
        reason: error.to_string(),
    }
}

fn unknown(kind: &'static str, value: &str) -> ConfigError {
    ConfigError::Unknown {
        kind,
        value: value.to_owned(),
    }
}

fn parse_protocol(value: &str) -> Result<PortProtocol, ConfigError> {
    match value {
        "tcp" => Ok(PortProtocol::Tcp),
        "udp" => Ok(PortProtocol::Udp),
        other => Err(unknown("protocol", other)),
    }
}

fn parse_egress(value: &str) -> Result<EgressPolicy, ConfigError> {
    match value {
        "none" => Ok(EgressPolicy::None),
        "nat" => Ok(EgressPolicy::Nat),
        "restricted" => Ok(EgressPolicy::Restricted),
        other => Err(unknown("egress policy", other)),
    }
}

fn parse_display(value: &str) -> Result<DisplayMode, ConfigError> {
    match value {
        "none" => Ok(DisplayMode::None),
        "vnc" => Ok(DisplayMode::Vnc),
        "spice" => Ok(DisplayMode::Spice),
        "wayland_seamless" => Ok(DisplayMode::WaylandSeamless),
        other => Err(unknown("display mode", other)),
    }
}

fn parse_file_mode(value: &str) -> Result<FileApertureMode, ConfigError> {
    match value {
        "read-only" => Ok(FileApertureMode::ReadOnly),
        "read-write" => Ok(FileApertureMode::ReadWrite),
        other => Err(unknown("file aperture mode", other)),
    }
}

fn parse_gpio_direction(value: &str) -> Result<GpioDirection, ConfigError> {
    match value {
        "input" => Ok(GpioDirection::Input),
        "output" => Ok(GpioDirection::Output),
        "bidirectional" => Ok(GpioDirection::Bidirectional),
        other => Err(unknown("gpio direction", other)),
    }
}

fn parse_device_access(value: &str) -> Result<DeviceAccess, ConfigError> {
    match value {
        "disabled" => Ok(DeviceAccess::Disabled),
        "virtio_only" => Ok(DeviceAccess::VirtioOnly),
        "explicit_apertures" => Ok(DeviceAccess::ExplicitApertures),
        other => Err(unknown("device access mode", other)),
    }
}

fn egress_str(value: EgressPolicy) -> &'static str {
    match value {
        EgressPolicy::None => "none",
        EgressPolicy::Nat => "nat",
        EgressPolicy::Restricted => "restricted",
    }
}

fn display_str(value: DisplayMode) -> &'static str {
    match value {
        DisplayMode::None => "none",
        DisplayMode::Vnc => "vnc",
        DisplayMode::Spice => "spice",
        DisplayMode::WaylandSeamless => "wayland_seamless",
    }
}

fn device_access_str(value: DeviceAccess) -> &'static str {
    match value {
        DeviceAccess::Disabled => "disabled",
        DeviceAccess::VirtioOnly => "virtio_only",
        DeviceAccess::ExplicitApertures => "explicit_apertures",
    }
}

/// Kept for callers that only need a yes/no answer about a protocol string.
pub fn protocol_from_str(value: &str) -> Option<PortProtocol> {
    parse_protocol(value).ok()
}

// ---------------------------------------------------------------------------
// TOML schema
//
// These types are the on-disk .cell.toml shape, deliberately separate from the
// domain types in dome-core. The blueprint nests ports under `perimeter` and
// apertures under their own tables, which no single CellSpec derive could
// reproduce. Keeping one schema for both directions is what makes round-tripping
// trustworthy instead of hopeful.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintFile {
    cell: CellTable,
    limits: LimitsTable,
    #[serde(default, skip_serializing_if = "PerimeterTable::is_empty")]
    perimeter: PerimeterTable,
    #[serde(default, skip_serializing_if = "AperturesTable::is_empty")]
    apertures: AperturesTable,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellTable {
    foundation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitsTable {
    memory_mib: u64,
    storage_mib: u64,
    cpus: u16,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PerimeterTable {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ports: Vec<PortTable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    egress: Option<String>,
}

impl PerimeterTable {
    fn is_empty(&self) -> bool {
        self.ports.is_empty() && self.egress.is_none()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortTable {
    host_addr: String,
    host_port: u16,
    guest_port: u16,
    protocol: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AperturesTable {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileTable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    gpio: Vec<GpioTable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_access: Option<String>,
}

impl AperturesTable {
    fn is_empty(&self) -> bool {
        self.files.is_empty() && self.gpio.is_empty() && self.device_access.is_none()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTable {
    host: String,
    guest: String,
    mode: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GpioTable {
    name: String,
    guest_chip: String,
    guest_line: u16,
    host: String,
    direction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rate_limit_hz: Option<u16>,
}

impl BlueprintFile {
    fn from_cell(cell: &CellSpec) -> Self {
        let limits = cell.limits();
        let apertures = cell.apertures();

        Self {
            cell: CellTable {
                foundation: cell.foundation().as_str().to_owned(),
                display: match cell.display() {
                    DisplayMode::None => None,
                    mode => Some(display_str(mode).to_owned()),
                },
            },
            limits: LimitsTable {
                memory_mib: limits.memory().as_mib(),
                storage_mib: limits.storage().as_mib(),
                cpus: limits.cpus().get(),
            },
            perimeter: PerimeterTable {
                ports: cell
                    .ports()
                    .iter()
                    .map(|port| PortTable {
                        host_addr: port.host_addr().to_owned(),
                        host_port: port.host_port(),
                        guest_port: port.guest_port(),
                        protocol: port.protocol().as_str().to_owned(),
                    })
                    .collect(),
                egress: match cell.egress() {
                    EgressPolicy::None => None,
                    policy => Some(egress_str(policy).to_owned()),
                },
            },
            apertures: AperturesTable {
                files: apertures
                    .files()
                    .iter()
                    .map(|file| FileTable {
                        host: file.host_path().to_owned(),
                        guest: file.guest_path().to_owned(),
                        mode: file.mode().as_str().to_owned(),
                    })
                    .collect(),
                gpio: apertures
                    .gpio()
                    .iter()
                    .map(|gpio| GpioTable {
                        name: gpio.name().to_owned(),
                        guest_chip: gpio.guest_chip().to_owned(),
                        guest_line: gpio.guest_line(),
                        host: gpio.host_line().to_owned(),
                        direction: gpio.direction().as_str().to_owned(),
                        rate_limit_hz: gpio.rate_limit_hz(),
                    })
                    .collect(),
                device_access: match apertures.device_access() {
                    DeviceAccess::Disabled => None,
                    access => Some(device_access_str(access).to_owned()),
                },
            },
        }
    }

    fn into_blueprint(self) -> Result<Blueprint, ConfigError> {
        let foundation = FoundationName::new(self.cell.foundation)
            .map_err(|error| invalid("cell.foundation", error))?;
        let memory = MemorySize::from_mib(self.limits.memory_mib)
            .map_err(|error| invalid("limits.memory_mib", error))?;
        let storage = StorageSize::from_mib(self.limits.storage_mib)
            .map_err(|error| invalid("limits.storage_mib", error))?;
        let cpus =
            CpuCount::new(self.limits.cpus).map_err(|error| invalid("limits.cpus", error))?;

        let mut cell = CellSpec::new(foundation, ResourceLimits::new(memory, storage, cpus));

        if let Some(display) = self.cell.display {
            cell = cell.with_display(parse_display(&display)?);
        }

        for port in self.perimeter.ports {
            let protocol = parse_protocol(&port.protocol)?;
            let map = PortMap::new(port.host_addr, port.host_port, port.guest_port, protocol)
                .map_err(|error| invalid("perimeter.ports", error))?;
            cell = cell.with_port(map);
        }

        if let Some(egress) = self.perimeter.egress {
            cell = cell.with_egress(parse_egress(&egress)?);
        }

        let mut apertures = AperturePolicy::closed();

        for file in self.apertures.files {
            let mode = parse_file_mode(&file.mode)?;
            let aperture = FileAperture::new(file.host, file.guest, mode)
                .map_err(|error| invalid("apertures.files", error))?;
            apertures = apertures.with_file(aperture);
        }

        for gpio in self.apertures.gpio {
            let direction = parse_gpio_direction(&gpio.direction)?;
            let aperture = GpioAperture::new(
                gpio.name,
                gpio.guest_chip,
                gpio.guest_line,
                gpio.host,
                direction,
            )
            .map_err(|error| invalid("apertures.gpio", error))?;

            let aperture = match gpio.rate_limit_hz {
                Some(rate_limit_hz) => aperture
                    .with_rate_limit_hz(rate_limit_hz)
                    .map_err(|error| invalid("apertures.gpio.rate_limit_hz", error))?,
                None => aperture,
            };

            apertures = apertures.with_gpio(aperture);
        }

        if let Some(access) = self.apertures.device_access {
            apertures = apertures.with_device_access(parse_device_access(&access)?);
        }

        Ok(Blueprint::new(cell.with_apertures(apertures)))
    }
}
