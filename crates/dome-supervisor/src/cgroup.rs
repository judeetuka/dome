//! Per-chamber cgroups v2.
//!
//! ADR 0001 settled the privilege question: Dome runs with **zero privilege** and uses the
//! delegated cgroup subtree that the user service already owns. Nothing here needs root, and
//! nothing here writes outside that subtree.
//!
//! The v2 interface is a directory of text files, so this module writes them directly rather
//! than going through a builder crate. That is a deliberate deviation from the plan: the
//! observable behaviour is identical, the dependency disappears, and an error names the exact
//! file that refused the write.

use std::fs;
use std::path::{Path, PathBuf};

use dome_core::{ChamberId, ResourceLimits};
use thiserror::Error;

/// Byte in a MiB, spelled out so the arithmetic is obvious.
const MIB: u64 = 1024 * 1024;
/// cgroup v2 CPU period in microseconds.
const CPU_PERIOD_US: u64 = 100_000;
/// Floor on the process limit, so a tiny chamber still gets a usable allowance.
const MIN_PIDS: u64 = 64;

/// Why a cgroup operation failed.
#[derive(Debug, Error)]
pub enum SupervisorError {
    /// The host does not expose cgroups v2.
    #[error("cgroups v2 are not available on this host: {0}")]
    CgroupUnavailable(String),

    /// The delegated subtree exists but cannot be written to.
    #[error("the delegated cgroup subtree is not writable: {0}")]
    NotDelegated(String),

    /// A file operation on the cgroup filesystem failed.
    #[error("cgroup write failed at {path}: {source}")]
    Io {
        /// File that failed.
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },

    /// A chamber id would have produced an unsafe directory name.
    #[error("chamber id {0:?} cannot be used as a cgroup name")]
    InvalidChamberId(String),

    /// A published port sits below the privileged boundary.
    #[error(
        "cannot publish port {port}: ports below 1024 need privilege a chamber supervisor does not have"
    )]
    PrivilegedPort {
        /// Port that was requested.
        port: u16,
    },

    /// The relay does not carry this transport.
    #[error("the perimeter relay does not carry {protocol}")]
    UnsupportedProtocol {
        /// Transport that was requested.
        protocol: &'static str,
    },

    /// The same host port was published twice.
    #[error("port {port} on {address} is published twice")]
    DuplicatePort {
        /// Address the port was published on.
        address: String,
        /// Port that collided.
        port: u16,
    },

    /// A host port could not be bound.
    #[error("cannot bind port {port}: {reason}")]
    Bind {
        /// Port that failed.
        port: u16,
        /// Why it failed.
        reason: String,
    },

    /// The engine process could not be started.
    #[error("cannot start {program}: {reason}")]
    Spawn {
        /// Program that failed to start.
        program: String,
        /// Why it failed.
        reason: String,
    },

    /// A namespace link did not have the expected shape.
    #[error("process {pid} has an unreadable network namespace: {value}")]
    UnexpectedNetns {
        /// Process that was inspected.
        pid: u32,
        /// What the link contained.
        value: String,
    },
}

/// A cgroup holding one chamber's processes.
#[derive(Debug)]
pub struct ChamberCgroup {
    path: PathBuf,
}

impl ChamberCgroup {
    /// Create and configure the cgroup for a chamber.
    ///
    /// Sets `memory.max`, `cpu.max` and `pids.max` from the chamber's limits and leaves the
    /// cgroup empty: the engine is attached later, after it exists.
    pub fn create(id: &ChamberId, limits: ResourceLimits) -> Result<Self, SupervisorError> {
        let root = delegated_root()?;
        enable_controllers(&root)?;

        let name = safe_name(id.as_str())?;
        let path = root.join("dome").join("chambers").join(&name);

        fs::create_dir_all(&path).map_err(|error| SupervisorError::Io {
            path: path.display().to_string(),
            source: error,
        })?;

        let cgroup = Self { path };

        // memory.max is a byte count; writing the limit rather than a ratio means a chamber
        // that exceeds it is killed by the kernel, not merely slowed.
        let memory_bytes = limits.memory().as_mib().saturating_mul(MIB);
        cgroup.write("memory.max", &memory_bytes.to_string())?;

        // cpu.max is "quota period": quota equal to period times count gives that many CPUs.
        let quota = limits.cpus().get() as u64 * CPU_PERIOD_US;
        cgroup.write("cpu.max", &format!("{quota} {CPU_PERIOD_US}"))?;

        // A process limit scaled to the CPU allowance, never below a usable floor.
        let pids = (limits.cpus().get() as u64 * 64).max(MIN_PIDS);
        cgroup.write("pids.max", &pids.to_string())?;

        Ok(cgroup)
    }

    /// Where the cgroup lives, for diagnostics and tests.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Value currently recorded in a cgroup file.
    pub fn read(&self, file: &str) -> Result<String, SupervisorError> {
        let path = self.path.join(file);
        fs::read_to_string(&path)
            .map(|text| text.trim().to_owned())
            .map_err(|error| SupervisorError::Io {
                path: path.display().to_string(),
                source: error,
            })
    }

    /// Put a process into this cgroup.
    pub fn attach_process(&self, pid: u32) -> Result<(), SupervisorError> {
        self.write("cgroup.procs", &pid.to_string())
    }

    /// Kill every process in the cgroup, the analogue of \`SIGKILL\` for the whole tree.
    pub fn kill_all(&self) -> Result<(), SupervisorError> {
        self.write("cgroup.kill", "1")
    }

    /// Kill everything still inside, then remove the cgroup.
    ///
    /// `cgroup.kill` is written first because a cgroup with live processes cannot be
    /// removed, and killing them is what destroying a chamber means.
    pub fn destroy(self) -> Result<(), SupervisorError> {
        // Best effort: kernels before 5.14 lack cgroup.kill, and an empty cgroup does not
        // need it.
        let _ = self.write("cgroup.kill", "1");

        fs::remove_dir(&self.path).map_err(|error| SupervisorError::Io {
            path: self.path.display().to_string(),
            source: error,
        })
    }

    fn write(&self, file: &str, value: &str) -> Result<(), SupervisorError> {
        let path = self.path.join(file);
        fs::write(&path, value).map_err(|error| SupervisorError::Io {
            path: path.display().to_string(),
            source: error,
        })
    }
}

/// Find the cgroup subtree this process is allowed to create children in.
///
/// The user service cgroup is the delegation boundary systemd sets up for
/// `Delegate=yes` services. If the process is not inside one, fall back to the immediate
/// parent of its own cgroup, which is the next best candidate and still never outside the
/// hierarchy the user already controls.
pub fn delegated_root() -> Result<PathBuf, SupervisorError> {
    let hierarchy = Path::new("/sys/fs/cgroup");
    if !hierarchy.is_dir() {
        return Err(SupervisorError::CgroupUnavailable(
            "/sys/fs/cgroup is missing".to_owned(),
        ));
    }

    let contents = fs::read_to_string("/proc/self/cgroup").map_err(|error| {
        SupervisorError::CgroupUnavailable(format!("cannot read /proc/self/cgroup: {error}"))
    })?;

    let relative = contents
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(|| {
            SupervisorError::CgroupUnavailable("no cgroups v2 unified entry".to_owned())
        })?;

    let mut components: Vec<&str> = relative
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();

    // Prefer the deepest .service ancestor: that is the delegated unit. Otherwise step up
    // one level, because a process cannot create siblings inside its own leaf cgroup.
    match components
        .iter()
        .rposition(|part| part.ends_with(".service"))
    {
        Some(index) => components.truncate(index + 1),
        None => {
            components.pop();
        }
    }

    if components.is_empty() {
        return Err(SupervisorError::CgroupUnavailable(
            "cannot determine a delegated subtree".to_owned(),
        ));
    }

    let root = components
        .iter()
        .fold(hierarchy.to_path_buf(), |path, part| path.join(part));

    if !root.is_dir() {
        return Err(SupervisorError::NotDelegated(format!(
            "{} does not exist",
            root.display()
        )));
    }

    Ok(root)
}

/// Ask the parent to delegate cpu, memory and pids to its children.
///
/// Failure is tolerated: the controllers may already be enabled, in which case the write
/// returns EBUSY, and a read-only parent means the subtree was set up by someone else.
fn enable_controllers(root: &Path) -> Result<(), SupervisorError> {
    let path = root.join("cgroup.subtree_control");
    match fs::write(&path, "+cpu +memory +pids") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
        Err(error) => {
            // EBUSY (already enabled) surfaces as Other; treat it as success.
            if error.raw_os_error() == Some(16) {
                Ok(())
            } else {
                Err(SupervisorError::Io {
                    path: path.display().to_string(),
                    source: error,
                })
            }
        }
    }
}

/// Turn a chamber id into a single safe path component.
///
/// Shared with the network module so a name is either safe everywhere or nowhere.
///
/// Chamber ids are user input. Interpolating one into a path would let `../..` escape the
/// delegated subtree, so anything outside a conservative alphabet is refused rather than
/// silently rewritten.
pub(crate) fn safe_name(id: &str) -> Result<String, SupervisorError> {
    let safe = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !id.starts_with('.');

    if safe {
        Ok(id.to_owned())
    } else {
        Err(SupervisorError::InvalidChamberId(id.to_owned()))
    }
}
