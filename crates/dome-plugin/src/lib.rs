use dome_core::{ChamberId, PortProtocol};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(value: impl Into<String>) -> Result<Self, PluginError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(PluginError::EmptyPluginId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PluginVersion(String);

impl PluginVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, PluginError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(PluginError::EmptyPluginVersion);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PluginKind {
    Telemetry,
    PerimeterGuard,
    ApertureGuard,
    EnterpriseDefense,
    AuditExport,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PluginExecutionMode {
    LocalSigned,
    RemoteService,
    AttestedLocal,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LicenseTier {
    Community,
    Enterprise,
    EnterpriseService,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LicenseState {
    NotRequired,
    Active,
    GracePeriod,
    Expired,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PluginCapability {
    ObserveChamberLifecycle,
    ObserveGuestProcesses,
    ObserveNetworkFlows,
    ObserveApertureAccess,
    DecidePerimeterTraffic,
    DecideApertureAccess,
    RequestChamberQuarantine,
    RequestForensicSeal,
    ExportAuditEvents,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PluginFailureMode {
    FailClosed,
    FailOpen,
    AlertOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    id: PluginId,
    version: PluginVersion,
    kind: PluginKind,
    execution: PluginExecutionMode,
    license_tier: LicenseTier,
    capabilities: Vec<PluginCapability>,
    failure_mode: PluginFailureMode,
}

impl PluginManifest {
    pub fn new(
        id: PluginId,
        version: PluginVersion,
        kind: PluginKind,
        execution: PluginExecutionMode,
        license_tier: LicenseTier,
        failure_mode: PluginFailureMode,
    ) -> Self {
        Self {
            id,
            version,
            kind,
            execution,
            license_tier,
            capabilities: Vec::new(),
            failure_mode,
        }
    }

    pub fn with_capability(mut self, capability: PluginCapability) -> Self {
        self.capabilities.push(capability);
        self
    }

    pub fn id(&self) -> &PluginId {
        &self.id
    }

    pub fn version(&self) -> &PluginVersion {
        &self.version
    }

    pub fn kind(&self) -> PluginKind {
        self.kind
    }

    pub fn execution(&self) -> PluginExecutionMode {
        self.execution
    }

    pub fn license_tier(&self) -> LicenseTier {
        self.license_tier
    }

    pub fn capabilities(&self) -> &[PluginCapability] {
        &self.capabilities
    }

    pub fn failure_mode(&self) -> PluginFailureMode {
        self.failure_mode
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelemetryEvent {
    ChamberStarted {
        chamber_id: ChamberId,
    },
    ChamberStopped {
        chamber_id: ChamberId,
    },
    GuestProcessReported {
        chamber_id: ChamberId,
        pid: u32,
        command: String,
    },
    NetworkFlowObserved {
        chamber_id: ChamberId,
        protocol: PortProtocol,
        remote: String,
        guest_port: u16,
    },
    ApertureAccessRequested {
        chamber_id: ChamberId,
        aperture: String,
        operation: String,
    },
    SentinelHeartbeat {
        chamber_id: ChamberId,
    },
}

impl TelemetryEvent {
    pub fn chamber_id(&self) -> &ChamberId {
        match self {
            Self::ChamberStarted { chamber_id }
            | Self::ChamberStopped { chamber_id }
            | Self::GuestProcessReported { chamber_id, .. }
            | Self::NetworkFlowObserved { chamber_id, .. }
            | Self::ApertureAccessRequested { chamber_id, .. }
            | Self::SentinelHeartbeat { chamber_id } => chamber_id,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PluginDecision {
    Allow,
    Deny,
    RateLimit,
    AlertOnly,
    QuarantineChamber,
    SealForForensics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginVerdict {
    decision: PluginDecision,
    reason_code: String,
}

impl PluginVerdict {
    pub fn new(decision: PluginDecision, reason_code: impl Into<String>) -> Self {
        Self {
            decision,
            reason_code: reason_code.into(),
        }
    }

    pub fn allow(reason_code: impl Into<String>) -> Self {
        Self::new(PluginDecision::Allow, reason_code)
    }

    pub fn deny(reason_code: impl Into<String>) -> Self {
        Self::new(PluginDecision::Deny, reason_code)
    }

    pub fn decision(&self) -> PluginDecision {
        self.decision
    }

    pub fn reason_code(&self) -> &str {
        &self.reason_code
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInvocation {
    plugin_id: PluginId,
    license_state: LicenseState,
    event: TelemetryEvent,
}

impl PluginInvocation {
    pub fn new(plugin_id: PluginId, license_state: LicenseState, event: TelemetryEvent) -> Self {
        Self {
            plugin_id,
            license_state,
            event,
        }
    }

    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub fn license_state(&self) -> LicenseState {
        self.license_state
    }

    pub fn event(&self) -> &TelemetryEvent {
        &self.event
    }
}

pub trait PluginHost {
    type Error;

    fn manifest(&self) -> &PluginManifest;
    fn evaluate(&self, invocation: &PluginInvocation) -> Result<PluginVerdict, Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    EmptyPluginId,
    EmptyPluginVersion,
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPluginId => formatter.write_str("plugin id cannot be empty"),
            Self::EmptyPluginVersion => formatter.write_str("plugin version cannot be empty"),
        }
    }
}

impl std::error::Error for PluginError {}
