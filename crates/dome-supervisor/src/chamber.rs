//! A chamber: its cgroup, its perimeter, and the engine process that runs inside them.
//!
//! The ordering here is the one T20 measured and T23 forced:
//!
//! 1. create the cgroup, so limits exist before any process does;
//! 2. create the relay socketpair, so the chamber side has something to inherit;
//! 3. spawn the engine child with that end of the socketpair handed across `exec`;
//! 4. attach the child to the cgroup;
//! 5. the child unshares its namespaces and runs the far end of the relay inside them.

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};

use dome_core::{ChamberId, ResourceLimits};
use dome_net::PerimeterPolicy;

use crate::cgroup::{ChamberCgroup, SupervisorError};
use crate::network::ChamberNetwork;

/// File descriptor the relay control channel arrives on inside the engine process.
pub const CONTROL_FD: i32 = 3;

/// A chamber that has been created but not necessarily started.
pub struct Chamber {
    id: ChamberId,
    cgroup: ChamberCgroup,
    network: ChamberNetwork,
}

impl Chamber {
    /// Create the cgroup and validate the perimeter.
    ///
    /// Both happen before anything runs, so a chamber with an impossible policy never gets
    /// as far as holding resources.
    pub fn create(
        id: ChamberId,
        limits: ResourceLimits,
        policy: PerimeterPolicy,
    ) -> Result<Self, SupervisorError> {
        let network = ChamberNetwork::plan(&id, policy)?;
        let cgroup = ChamberCgroup::create(&id, limits)?;

        Ok(Self {
            id,
            cgroup,
            network,
        })
    }

    /// The chamber's identifier.
    pub fn id(&self) -> &ChamberId {
        &self.id
    }

    /// Its cgroup, for limits and process membership.
    pub fn cgroup(&self) -> &ChamberCgroup {
        &self.cgroup
    }

    /// Its perimeter.
    pub fn network(&self) -> &ChamberNetwork {
        &self.network
    }

    /// Start the engine process, handing it the relay control channel on fd 3.
    ///
    /// The child is attached to the cgroup *after* spawning because its pid does not exist
    /// before then. The window between `exec` and the attach is real, which is why the
    /// attach happens before any work is signalled to the child.
    pub fn start_engine(
        &self,
        program: &Path,
        args: &[String],
        control: UnixStream,
    ) -> Result<Child, SupervisorError> {
        let fd = control.as_raw_fd();

        let mut command = Command::new(program);
        command.args(args);

        // SAFETY: `pre_exec` runs between fork and exec, where only async-signal-safe calls
        // are allowed. `dup2` and `close` are both on that list, and the closure captures
        // only an integer.
        #[allow(unsafe_code)]
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(fd, CONTROL_FD) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command.spawn().map_err(|error| SupervisorError::Spawn {
            program: program.display().to_string(),
            reason: error.to_string(),
        })?;

        self.cgroup.attach_process(child.id())?;

        // The parent's copy is not needed once the child has its own.
        drop(control);

        Ok(child)
    }

    /// Remove the cgroup. The namespace disappears with the last process in it.
    pub fn destroy(self) -> Result<(), SupervisorError> {
        self.cgroup.destroy()
    }
}
