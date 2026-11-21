//! Foundation recipes: the declaration of what an image contains.
//!
//! There is no base image here. A foundation is the **dependency closure of what the recipe
//! names**, and if it names nothing the image is a kernel plus an init and nothing else.
//! That is the difference between this and every container workflow: nothing arrives because
//! it was a reasonable default.
//!
//! Two facts drive the design, both measured rather than assumed:
//!
//! - A dynamically linked binary needs glibc, which is **50.7 MiB** on an Arch host. A static
//!   binary needs nothing, which is why [`Recipe::static_linking`] exists.
//! - An init system is a choice, not a requirement. systemd costs **40.7 MiB** plus dbus on the
//!   same host; the kernel is happy to exec a single static binary as PID 1.

use crate::ConfigError;

/// What runs as PID 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitChoice {
    /// No separate init: the kernel's init= names the workload binary directly.
    None,
    /// Dome's own minimal init, which reaps and execs the workload.
    Sentinel,
    /// BusyBox's init applet.
    BusyBox,
    /// A full service manager, chosen by the user.
    ServiceManager(ServiceManager),
    /// Any binary at this path inside the image.
    Path(String),
}

/// The service managers a recipe may ask for.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ServiceManager {
    /// systemd.
    Systemd,
    /// OpenRC.
    OpenRc,
    /// runit.
    Runit,
    /// s6.
    S6,
    /// dinit.
    Dinit,
}

impl ServiceManager {
    /// Stable name for a recipe.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Systemd => "systemd",
            Self::OpenRc => "openrc",
            Self::Runit => "runit",
            Self::S6 => "s6",
            Self::Dinit => "dinit",
        }
    }

    /// Parse a name.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "systemd" => Self::Systemd,
            "openrc" => Self::OpenRc,
            "runit" => Self::Runit,
            "s6" => Self::S6,
            "dinit" => Self::Dinit,
            _ => return None,
        })
    }

    /// The measured installed size on an Arch host, in MiB.
    ///
    /// Only systemd was measurable here; the others live in the AUR and were not installed,
    /// so they report None rather than a number nobody checked.
    pub fn measured_mib(self) -> Option<u64> {
        match self {
            Self::Systemd => Some(41),
            _ => None,
        }
    }

    /// Packages this manager drags in beyond itself.
    pub fn extra_packages(self) -> &'static [&'static str] {
        match self {
            Self::Systemd => &["systemd-libs", "dbus"],
            _ => &[],
        }
    }
}

impl InitChoice {
    /// Parse the init field of a recipe.
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        let trimmed = value.trim();

        if let Some(path) = trimmed.strip_prefix("path:") {
            if path.is_empty() || !path.starts_with('/') {
                return Err(ConfigError::InvalidValue {
                    field: "foundation.init",
                    reason: format!("path {path:?} must be absolute"),
                });
            }
            return Ok(Self::Path(path.to_owned()));
        }

        Ok(match trimmed {
            "none" => Self::None,
            "sentinel" => Self::Sentinel,
            "busybox" => Self::BusyBox,
            other => match ServiceManager::parse(other) {
                Some(manager) => Self::ServiceManager(manager),
                None => {
                    return Err(ConfigError::Unknown {
                        kind: "init",
                        value: other.to_owned(),
                    });
                }
            },
        })
    }

    /// A short name for reports.
    pub fn as_str(&self) -> &str {
        match self {
            Self::None => "none",
            Self::Sentinel => "sentinel",
            Self::BusyBox => "busybox",
            Self::ServiceManager(manager) => manager.as_str(),
            Self::Path(path) => path,
        }
    }

    /// Whether this choice needs the kernel to find a binary of its own.
    pub fn is_separate_binary(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// What a foundation should contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// Foundation name, for the store.
    pub name: String,
    /// Target architecture.
    pub arch: String,
    /// What runs as PID 1.
    pub init: InitChoice,
    /// Packages to include. Empty means kernel plus init and nothing else.
    pub packages: Vec<String>,
    /// Whether the images are static binaries, which is what removes glibc entirely.
    pub static_linking: bool,
    /// The workload the image exists to run, if any.
    pub workload: Option<String>,
}

impl Recipe {
    /// A recipe that names nothing: the leanest possible image.
    pub fn bare(name: &str, arch: &str) -> Self {
        Self {
            name: name.to_owned(),
            arch: arch.to_owned(),
            init: InitChoice::Sentinel,
            packages: Vec::new(),
            static_linking: true,
            workload: None,
        }
    }

    /// Parse a recipe from TOML.
    pub fn from_toml(input: &str) -> Result<Self, ConfigError> {
        #[derive(serde::Deserialize)]
        struct File {
            foundation: Table,
        }

        #[derive(serde::Deserialize)]
        struct Table {
            name: String,
            #[serde(default = "default_arch")]
            arch: String,
            #[serde(default = "default_init")]
            init: String,
            #[serde(default)]
            packages: Vec<String>,
            #[serde(default = "default_static")]
            static_linking: bool,
            workload: Option<String>,
        }

        fn default_arch() -> String {
            "x86_64".to_owned()
        }
        fn default_init() -> String {
            "sentinel".to_owned()
        }
        fn default_static() -> bool {
            true
        }

        let file: File =
            toml::from_str(input).map_err(|error| ConfigError::Parse(error.to_string()))?;

        let recipe = Self {
            name: file.foundation.name,
            arch: file.foundation.arch,
            init: InitChoice::parse(&file.foundation.init)?,
            packages: file.foundation.packages,
            static_linking: file.foundation.static_linking,
            workload: file.foundation.workload,
        };

        recipe.validate()?;
        Ok(recipe)
    }

    /// Check the recipe can produce a bootable image.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.name.trim().is_empty() {
            return Err(ConfigError::InvalidValue {
                field: "foundation.name",
                reason: "must not be empty".to_owned(),
            });
        }

        if !matches!(self.arch.as_str(), "x86_64" | "aarch64") {
            return Err(ConfigError::Unknown {
                kind: "architecture",
                value: self.arch.clone(),
            });
        }

        // A package listed twice is a recipe mistake, not something to silently fold away.
        for (index, package) in self.packages.iter().enumerate() {
            if package.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    field: "foundation.packages",
                    reason: "contains an empty name".to_owned(),
                });
            }
            if self.packages[..index].contains(package) {
                return Err(ConfigError::InvalidValue {
                    field: "foundation.packages",
                    reason: format!("package {package:?} is listed twice"),
                });
            }
        }

        // Something must be PID 1. With init=none the kernel execs the workload directly, so
        // there has to be one.
        if self.init == InitChoice::None && self.workload.is_none() {
            return Err(ConfigError::MissingField(
                "foundation.workload (required when init = \"none\")",
            ));
        }

        Ok(())
    }

    /// What the image will contain, and what it will cost.
    pub fn plan(&self) -> Plan {
        let mut packages = self.packages.clone();
        let mut notes = Vec::new();

        if let InitChoice::ServiceManager(manager) = self.init {
            for extra in manager.extra_packages() {
                if !packages.iter().any(|package| package == extra) {
                    packages.push((*extra).to_owned());
                }
            }

            if let Some(mib) = manager.measured_mib() {
                notes.push(format!(
                    "{} was measured at {mib} MiB installed on this host",
                    manager.as_str()
                ));
            }
        }

        if !self.static_linking && !packages.is_empty() {
            // Measured, not estimated: glibc is 50.7 MiB here, and it is what makes a
            // dynamically linked image stop being a kernel plus a binary.
            notes.push(
                "dynamic linking pulls glibc, measured at 51 MiB on this host; static linking removes it entirely"
                    .to_owned(),
            );
        }

        if packages.is_empty() && self.init == InitChoice::Sentinel {
            notes.push(
                "no packages and the sentinel init: the image is a kernel plus one static binary"
                    .to_owned(),
            );
        }

        Plan {
            kernel_cmdline: self.kernel_cmdline(),
            packages,
            notes,
            rootfs_is_empty: self.packages.is_empty(),
        }
    }

    /// The kernel command line that makes this init run.
    pub fn kernel_cmdline(&self) -> String {
        match &self.init {
            InitChoice::None => match &self.workload {
                Some(workload) => format!("init={workload}"),
                // Validation rejects this, so the fallback only exists to keep the function
                // total rather than to be reached.
                None => "init=/bin/sh".to_owned(),
            },
            InitChoice::Path(path) => format!("init={path}"),
            other => format!("init=/sbin/{}", other.as_str()),
        }
    }
}

/// The resolved shape of an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Kernel command line.
    pub kernel_cmdline: String,
    /// Packages after the init system's own dependencies were added.
    pub packages: Vec<String>,
    /// What the operator should know about the cost of their choices.
    pub notes: Vec<String>,
    /// Whether the root filesystem holds nothing but the init.
    pub rootfs_is_empty: bool,
}
