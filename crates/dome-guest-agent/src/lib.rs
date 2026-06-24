#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SentinelCapability {
    FilesystemFreeze,
    DisplayResize,
    ClipboardSync,
    SeamlessWindowMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentinelContract {
    protocol_version: u16,
    capabilities: Vec<SentinelCapability>,
}

impl SentinelContract {
    pub fn new(protocol_version: u16, capabilities: Vec<SentinelCapability>) -> Self {
        Self {
            protocol_version,
            capabilities,
        }
    }

    pub fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    pub fn capabilities(&self) -> &[SentinelCapability] {
        &self.capabilities
    }
}
