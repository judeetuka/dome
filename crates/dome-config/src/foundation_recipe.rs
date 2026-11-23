//! Foundation recipes: what an image contains, and what that costs.
//!
//! There is no base image. A foundation is the **dependency closure of what the recipe names**,
//! and a recipe that names nothing produces a kernel plus an init and nothing else.
//!
//! The recipe is sectioned rather than one list, because the sections answer different
//! questions: what the release keeps, what only the build needs, and what the interactive shell
//! is. Each package may be pinned.
//!
//! Static linking is the default and is not a style preference. Any dynamically linked binary
//! needs a C library and the dynamic loader: glibc measured 50.7 MiB on an Arch host, and the
//! kernel's own list of init failures includes missing library dependencies. Turning it off is
//! supported, and does the bookkeeping rather than leaving it to be discovered.

use crate::ConfigError;
use crate::installer::Installer;

/// Which package manager fetches a package.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum PackageSource {
    /// Whatever the recipe's builder uses.
    #[default]
    Default,
    /// apk.
    Apk,
    /// pacman, official repositories.
    Pacman,
    /// yay, the AUR through pacman's interface.
    Yay,
    /// mise, which installs self-contained toolchains with no root and no distribution.
    ///
    /// This is the source that fits Dome's model best: a mise toolchain is a directory, so it
    /// can be copied into an image as-is, and it needs no base system to exist first.
    Mise,
}

impl PackageSource {
    /// Parse a prefix.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "" => Self::Default,
            "apk" => Self::Apk,
            "pacman" => Self::Pacman,
            "yay" | "aur" => Self::Yay,
            "mise" => Self::Mise,
            _ => return None,
        })
    }

    /// The prefix as written.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Apk => "apk",
            Self::Pacman => "pacman",
            Self::Yay => "yay",
            Self::Mise => "mise",
        }
    }
}

/// A package, optionally pinned and optionally sourced from a named manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSpec {
    /// Which package manager fetches it.
    pub source: PackageSource,
    /// Package name.
    pub name: String,
    /// Version to pin to, if any.
    pub version: Option<String>,
}

impl PackageSpec {
    /// An unpinned package with no manager override, for packages the recipe adds itself.
    pub fn named(name: &str) -> Self {
        Self {
            source: PackageSource::Default,
            name: name.to_owned(),
            version: None,
        }
    }

    /// Parse "name", "name@version", or "manager:name@version".
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ConfigError::InvalidValue {
                field: "packages",
                reason: "contains an empty name".to_owned(),
            });
        }

        // A prefix names the manager. Without one, the recipe's builder decides.
        let (source, body) = match trimmed.split_once(':') {
            Some((prefix, rest)) if PackageSource::parse(prefix).is_some() => {
                (PackageSource::parse(prefix).expect("checked"), rest.trim())
            }
            _ => (PackageSource::Default, trimmed),
        };

        if body.is_empty() {
            return Err(ConfigError::InvalidValue {
                field: "packages",
                reason: format!("package {trimmed:?} names a manager but no package"),
            });
        }

        let (name, version) = match body.split_once('@') {
            Some((name, version)) => {
                if version.trim().is_empty() {
                    return Err(ConfigError::InvalidValue {
                        field: "packages",
                        reason: format!("package {name:?} pins an empty version"),
                    });
                }
                (name.trim(), Some(version.trim().to_owned()))
            }
            // The body, not the original: the manager prefix was already stripped.
            None => (body, None),
        };

        if name.is_empty() {
            return Err(ConfigError::InvalidValue {
                field: "packages",
                reason: "contains an empty name".to_owned(),
            });
        }

        Ok(Self {
            source,
            name: name.to_owned(),
            version,
        })
    }

    /// How it is written in a plan or a report.
    pub fn label(&self) -> String {
        let prefix = match self.source {
            PackageSource::Default => String::new(),
            other => format!("{}:", other.as_str()),
        };

        match &self.version {
            Some(version) => format!("{prefix}{}@{version}", self.name),
            None => format!("{prefix}{}", self.name),
        }
    }
}

/// One section of packages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Packages {
    /// The packages in this section.
    pub packages: Vec<PackageSpec>,
}

impl Packages {
    /// Parse a list of package strings.
    pub fn parse(values: &[String]) -> Result<Self, ConfigError> {
        let mut packages = Vec::with_capacity(values.len());
        for value in values {
            packages.push(PackageSpec::parse(value)?);
        }
        Ok(Self { packages })
    }

    /// Labels, for reports.
    pub fn labels(&self) -> Vec<String> {
        self.packages
            .iter()
            .map(|package| package.label())
            .collect()
    }

    /// Whether a package of this name is present, pinned or not.
    pub fn contains(&self, name: &str) -> bool {
        self.packages.iter().any(|package| package.name == name)
    }
}

/// What runs as PID 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitChoice {
    /// No separate init: the kernel's init= names the workload binary directly.
    None,
    /// Dome's own minimal init.
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
    /// Stable name.
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

    /// Measured installed size in MiB, where it was measured.
    pub fn measured_mib(self) -> Option<u64> {
        match self {
            Self::Systemd => Some(41),
            _ => None,
        }
    }

    /// Packages this manager drags in.
    pub fn extra_packages(self) -> &'static [&'static str] {
        match self {
            Self::Systemd => &["systemd-libs", "dbus"],
            _ => &[],
        }
    }
}

impl InitChoice {
    /// Parse the init field.
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        let trimmed = value.trim();

        if let Some(path) = trimmed.strip_prefix("path:") {
            if path.is_empty() || !path.starts_with('/') {
                return Err(ConfigError::InvalidValue {
                    field: "init",
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
}

/// Whether the image carries a package manager.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PackageManager {
    /// None: the builder fetches packages and writes them in.
    None,
    /// apk.
    Apk,
    /// pacman.
    Pacman,
}

impl PackageManager {
    /// Parse a name.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "" | "none" => Self::None,
            "apk" => Self::Apk,
            "pacman" => Self::Pacman,
            _ => return None,
        })
    }

    /// The package this puts in the image.
    pub fn package(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Apk => Some("apk-tools"),
            Self::Pacman => Some("pacman"),
        }
    }
}

/// The C library when linking dynamically.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CcLibrary {
    /// glibc.
    Glibc,
    /// musl.
    Musl,
}

impl CcLibrary {
    /// Parse a name.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "" | "glibc" => Self::Glibc,
            "musl" => Self::Musl,
            _ => return None,
        })
    }

    /// The package that provides it.
    pub fn package(self) -> &'static str {
        match self {
            Self::Glibc => "glibc",
            Self::Musl => "musl",
        }
    }
}

/// Who the chamber runs as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserSpec {
    /// Root, which is the default and the smallest image.
    Root,
    /// A user copied from the host: same name, uid and gid.
    ///
    /// This is what makes a chamber feel like the host on Linux, and what lets Windows and
    /// macOS behave the same way: the engine maps the host identity to a numeric uid inside,
    /// so a bind-mounted project keeps its ownership instead of turning into root-owned files.
    Host,
    /// A user the recipe describes explicitly.
    Custom {
        /// Login name.
        name: String,
        /// Numeric uid.
        uid: u32,
        /// Numeric gid.
        gid: Option<u32>,
        /// Home directory, absolute.
        home: Option<String>,
    },
}

/// Where the project appears inside the chamber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Absolute path inside the image.
    pub path: String,
    /// Optional host path to map there, which is an aperture and subject to its rules.
    pub host: Option<String>,
}

/// What a foundation should contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// Foundation name.
    pub name: String,
    /// Target architecture.
    pub arch: String,
    /// Kernel profile. Dome builds its own; no distribution kernel package is involved.
    pub kernel: String,
    /// What runs as PID 1.
    pub init: InitChoice,
    /// The workload, if the image exists to run one.
    pub workload: Option<String>,
    /// Whether the binaries are static.
    pub static_linking: bool,
    /// Which C library the image needs when it is not static.
    pub libc: CcLibrary,
    /// Whether the image carries a package manager.
    pub package_manager: PackageManager,
    /// Packages the release keeps.
    pub runtime: Packages,
    /// Packages used only while building.
    pub build: Packages,
    /// Files the build produced that belong in the release.
    pub artifacts: Vec<String>,
    /// The interactive shell, if the image has one.
    pub shell: Option<Shell>,
    /// Which package manager the chamber installs for itself.
    pub installer: Installer,
    /// Where the chamber fetches its installer from.
    pub installer_source: Option<String>,
    /// Who the chamber runs as.
    pub user: UserSpec,
    /// Where the project lives inside the chamber.
    pub workspace: Option<Workspace>,
}

/// The shell an image offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    /// Which shell starts by default.
    pub default: String,
    /// Shells to install.
    pub packages: Packages,
}

impl Recipe {
    /// A recipe that names nothing.
    pub fn bare(name: &str, arch: &str) -> Self {
        Self {
            name: name.to_owned(),
            arch: arch.to_owned(),
            kernel: "dome-minimal".to_owned(),
            init: InitChoice::Sentinel,
            workload: None,
            static_linking: true,
            libc: CcLibrary::Glibc,
            package_manager: PackageManager::None,
            runtime: Packages::default(),
            build: Packages::default(),
            artifacts: Vec::new(),
            shell: None,
            installer: Installer::None,
            installer_source: None,
            user: UserSpec::Root,
            workspace: None,
        }
    }

    /// Parse a recipe from TOML.
    pub fn from_toml(input: &str) -> Result<Self, ConfigError> {
        #[derive(serde::Deserialize)]
        struct File {
            foundation: FoundationTable,
            #[serde(default)]
            runtime: PackageTable,
            #[serde(default)]
            build: BuildTable,
            shell: Option<ShellTable>,
            user: Option<UserTable>,
            workspace: Option<WorkspaceTable>,
        }

        #[derive(serde::Deserialize)]
        struct FoundationTable {
            name: String,
            #[serde(default = "default_arch")]
            arch: String,
            #[serde(default = "default_kernel")]
            kernel: String,
            #[serde(default = "default_init")]
            init: String,
            #[serde(default = "default_static")]
            static_linking: bool,
            #[serde(default)]
            libc: String,
            workload: Option<String>,
            #[serde(default)]
            package_manager: String,
            #[serde(default)]
            installer: String,
            installer_source: Option<String>,
            /// Shorthand for the runtime section.
            #[serde(default)]
            packages: Vec<String>,
        }

        #[derive(serde::Deserialize, Default)]
        struct PackageTable {
            #[serde(default)]
            packages: Vec<String>,
        }

        #[derive(serde::Deserialize, Default)]
        struct BuildTable {
            #[serde(default)]
            packages: Vec<String>,
            #[serde(default)]
            artifacts: Vec<String>,
        }

        #[derive(serde::Deserialize)]
        struct UserTable {
            #[serde(default = "default_mode")]
            mode: String,
            #[serde(default)]
            name: String,
            uid: Option<u32>,
            gid: Option<u32>,
            home: Option<String>,
        }

        #[derive(serde::Deserialize)]
        struct WorkspaceTable {
            path: String,
            host: Option<String>,
        }

        fn default_mode() -> String {
            "none".to_owned()
        }

        #[derive(serde::Deserialize)]
        struct ShellTable {
            default: String,
            #[serde(default)]
            packages: Vec<String>,
        }

        fn default_arch() -> String {
            "x86_64".to_owned()
        }
        fn default_kernel() -> String {
            "dome-minimal".to_owned()
        }
        fn default_init() -> String {
            "sentinel".to_owned()
        }
        fn default_static() -> bool {
            true
        }

        let file: File =
            toml::from_str(input).map_err(|error| ConfigError::Parse(error.to_string()))?;

        let mut runtime = Packages::parse(&file.runtime.packages)?;
        // The shorthand merges into the runtime section rather than being a separate thing.
        runtime
            .packages
            .extend(Packages::parse(&file.foundation.packages)?.packages);

        let recipe = Self {
            name: file.foundation.name,
            arch: file.foundation.arch,
            kernel: file.foundation.kernel,
            init: InitChoice::parse(&file.foundation.init)?,
            workload: file.foundation.workload,
            static_linking: file.foundation.static_linking,
            libc: CcLibrary::parse(&file.foundation.libc).ok_or_else(|| ConfigError::Unknown {
                kind: "C library",
                value: file.foundation.libc.clone(),
            })?,
            package_manager: PackageManager::parse(&file.foundation.package_manager).ok_or_else(
                || ConfigError::Unknown {
                    kind: "package manager",
                    value: file.foundation.package_manager.clone(),
                },
            )?,
            runtime,
            build: Packages::parse(&file.build.packages)?,
            artifacts: file.build.artifacts,
            user: match file.user {
                None => UserSpec::Root,
                Some(user) => match user.mode.as_str() {
                    "none" => UserSpec::Root,
                    "host" => UserSpec::Host,
                    "custom" => UserSpec::Custom {
                        name: if user.name.is_empty() {
                            "dev".to_owned()
                        } else {
                            user.name
                        },
                        uid: user.uid.unwrap_or(1000),
                        gid: user.gid,
                        home: user.home,
                    },
                    other => {
                        return Err(ConfigError::Unknown {
                            kind: "user mode",
                            value: other.to_owned(),
                        });
                    }
                },
            },
            workspace: file.workspace.map(|workspace| Workspace {
                path: workspace.path,
                host: workspace.host,
            }),
            installer: Installer::parse(&file.foundation.installer).ok_or_else(|| {
                ConfigError::Unknown {
                    kind: "installer",
                    value: file.foundation.installer.clone(),
                }
            })?,
            installer_source: file.foundation.installer_source,
            shell: match file.shell {
                Some(shell) => Some(Shell {
                    default: shell.default,
                    packages: Packages::parse(&shell.packages)?,
                }),
                None => None,
            },
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

        if self.kernel.trim().is_empty() {
            return Err(ConfigError::InvalidValue {
                field: "foundation.kernel",
                reason: "must name a kernel profile".to_owned(),
            });
        }

        for section in [&self.runtime, &self.build] {
            for (index, package) in section.packages.iter().enumerate() {
                if section.packages[..index]
                    .iter()
                    .any(|earlier| earlier.name == package.name)
                {
                    return Err(ConfigError::InvalidValue {
                        field: "packages",
                        reason: format!("package {:?} is listed twice", package.name),
                    });
                }
            }
        }

        // A package in both stages is a manifest mistake: nobody could tell which copy the
        // image ended up with.
        for package in &self.build.packages {
            if self.runtime.contains(&package.name) {
                return Err(ConfigError::InvalidValue {
                    field: "build.packages",
                    reason: format!(
                        "package {:?} is listed as both a runtime and a build package",
                        package.name
                    ),
                });
            }
        }

        for artifact in &self.artifacts {
            if !artifact.starts_with('/') {
                return Err(ConfigError::InvalidValue {
                    field: "build.artifacts",
                    reason: format!("artifact {artifact:?} must be an absolute path"),
                });
            }
        }

        if let Some(shell) = &self.shell
            && !shell.packages.contains(&shell.default)
        {
            return Err(ConfigError::InvalidValue {
                field: "shell.default",
                reason: format!("{:?} is not among the shell packages", shell.default),
            });
        }

        if let UserSpec::Custom { home, .. } = &self.user
            && let Some(home) = home
            && !home.starts_with('/')
        {
            return Err(ConfigError::InvalidValue {
                field: "user.home",
                reason: "must be an absolute path".to_owned(),
            });
        }

        if let Some(workspace) = &self.workspace
            && !workspace.path.starts_with('/')
        {
            return Err(ConfigError::InvalidValue {
                field: "workspace.path",
                reason: "must be an absolute path inside the image".to_owned(),
            });
        }

        if self.installer.needs_binary() && self.installer_source.is_none() {
            return Err(ConfigError::MissingField("foundation.installer_source"));
        }

        if self.init == InitChoice::None && self.workload.is_none() {
            return Err(ConfigError::MissingField(
                "foundation.workload (required when init = none)",
            ));
        }

        Ok(())
    }

    /// What the image will contain, and what it will cost.
    pub fn plan(&self) -> Plan {
        let mut runtime: Vec<PackageSpec> = self.runtime.packages.clone();
        let mut notes = vec![format!(
            "kernel {} built by Dome, not a distribution package",
            self.kernel
        )];

        // Dynamic linking is not a flag that only affects the build: it decides whether the
        // release needs a C library at all. Adding it here means nobody has to remember.
        if !self.static_linking {
            let libc = self.libc.package();
            if !runtime.iter().any(|package| package.name == libc) {
                runtime.push(PackageSpec::named(libc));
            }
            notes.push(format!(
                "dynamic linking: {libc} is added to the release; glibc measured 50.7 MiB on this host"
            ));
        }

        // The interactive shell is part of the release by definition.
        if let Some(shell) = &self.shell {
            for package in &shell.packages.packages {
                if !runtime.iter().any(|existing| existing.name == package.name) {
                    runtime.push(package.clone());
                }
            }
            notes.push(format!("default shell: {}", shell.default));
        }

        if let InitChoice::ServiceManager(manager) = self.init {
            for extra in manager.extra_packages() {
                if !runtime.iter().any(|package| package.name == *extra) {
                    runtime.push(PackageSpec::named(extra));
                }
            }
            if let Some(mib) = manager.measured_mib() {
                notes.push(format!(
                    "{} was measured at {mib} MiB installed on this host",
                    manager.as_str()
                ));
            }
        }

        if !self.installer.needs_binary()
            && (!self.runtime.packages.is_empty() || !self.build.packages.is_empty())
        {
            // Parsing stays permissive because a plan is useful without an installer; producing an
            // image is not, and the builder refuses there.
            notes.push(
                "packages are named but no installer is: nothing will install them inside the chamber"
                    .to_owned(),
            );
        }

        if self.installer.needs_binary() {
            notes.push(format!(
                "{} is installed by the chamber itself from {}",
                self.installer.as_str(),
                self.installer_source.as_deref().unwrap_or("(no source)")
            ));
        }

        if let Some(package) = self.package_manager.package() {
            if !runtime.iter().any(|existing| existing.name == package) {
                runtime.push(PackageSpec::named(package));
            }
            notes.push(format!(
                "the image carries {package} and can install at runtime"
            ));
        } else {
            notes.push("no package manager in the image".to_owned());
        }

        if !self.build.packages.is_empty() {
            notes.push(format!(
                "{} build package(s) are used in a separate stage and never reach the release",
                self.build.packages.len()
            ));
        }

        if !self.artifacts.is_empty() {
            notes.push(format!(
                "{} artifact(s) are copied from the build into the release",
                self.artifacts.len()
            ));
        }

        match &self.user {
            UserSpec::Root => {}
            UserSpec::Host => {
                notes.push(
                    "runs as the host user: uid and gid are mapped, so shared files keep ownership"
                        .to_owned(),
                );
            }
            UserSpec::Custom { name, uid, .. } => {
                notes.push(format!("runs as {name} (uid {uid})"));
            }
        }

        if let Some(workspace) = &self.workspace {
            match &workspace.host {
                Some(host) => notes.push(format!(
                    "workspace {} is mapped from {host}",
                    workspace.path
                )),
                None => notes.push(format!("workspace at {}", workspace.path)),
            }
        }

        let pinned = runtime
            .iter()
            .filter(|package| package.version.is_some())
            .count();
        if pinned > 0 {
            notes.push(format!("{pinned} package(s) are pinned to a version"));
        }

        Plan {
            kernel_cmdline: self.kernel_cmdline(),
            runtime_packages: runtime.iter().map(|package| package.label()).collect(),
            build_packages: self.build.labels(),
            artifacts: self.artifacts.clone(),
            notes,
            rootfs_is_empty: runtime.is_empty() && self.artifacts.is_empty(),
            image_has_package_manager: self.package_manager.package().is_some(),
            packages_dropped_after_build: self
                .build
                .packages
                .iter()
                .map(|package| package.label())
                .collect(),
        }
    }

    /// The kernel command line that makes this init run.
    pub fn kernel_cmdline(&self) -> String {
        match &self.init {
            InitChoice::None => match &self.workload {
                Some(workload) => format!("init={workload}"),
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
    /// Packages the release keeps.
    pub runtime_packages: Vec<String>,
    /// Packages used only while building.
    pub build_packages: Vec<String>,
    /// Files copied from the build into the release.
    pub artifacts: Vec<String>,
    /// What the operator should know.
    pub notes: Vec<String>,
    /// Whether the root filesystem holds nothing but the init.
    pub rootfs_is_empty: bool,
    /// Whether the release can install packages at runtime.
    pub image_has_package_manager: bool,
    /// What the build stage installs and the release therefore never contains.
    pub packages_dropped_after_build: Vec<String>,
}
