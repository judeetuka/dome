#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DeviceClass {
    Gpu,
    UsbController,
    NetworkCard,
    StorageController,
    PciOther,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PassthroughReadiness {
    Unknown,
    MissingIommu,
    UnsafeIommuGroup,
    ReadyForExplicitOptIn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCandidate {
    class: DeviceClass,
    sysfs_path: String,
    readiness: PassthroughReadiness,
}

impl DeviceCandidate {
    pub fn new(
        class: DeviceClass,
        sysfs_path: impl Into<String>,
        readiness: PassthroughReadiness,
    ) -> Self {
        Self {
            class,
            sysfs_path: sysfs_path.into(),
            readiness,
        }
    }

    pub fn class(&self) -> DeviceClass {
        self.class
    }

    pub fn sysfs_path(&self) -> &str {
        &self.sysfs_path
    }

    pub fn readiness(&self) -> PassthroughReadiness {
        self.readiness
    }
}
