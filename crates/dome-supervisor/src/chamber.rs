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
use crate::lifecycle::{StopOutcome, StopPolicy};
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

    /// Stop the engine the way `docker stop` stops a container.
    ///
    /// Ask with a signal, wait out the grace period, then force by killing the whole cgroup.
    /// With `grace: None` it waits indefinitely, which is what `docker stop --timeout -1`
    /// does.
    pub fn stop(
        &self,
        child: &mut Child,
        policy: StopPolicy,
    ) -> Result<StopOutcome, SupervisorError> {
        let pid = child.id() as i32;

        // SAFETY: the pid belongs to a child this process spawned.
        #[allow(unsafe_code)]
        let sent = unsafe { libc::kill(pid, policy.signal) };
        if sent != 0 {
            let error = io::Error::last_os_error();
            // ESRCH means it already exited, which is a fine outcome for a stop request.
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(SupervisorError::Spawn {
                    program: format!("pid {pid}"),
                    reason: error.to_string(),
                });
            }
        }

        let Some(grace) = policy.grace else {
            let status = child.wait().map_err(|error| SupervisorError::Spawn {
                program: format!("pid {pid}"),
                reason: error.to_string(),
            })?;
            return Ok(StopOutcome {
                code: status.code(),
                forced: false,
            });
        };

        let deadline = std::time::Instant::now() + grace;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Ok(StopOutcome {
                        code: status.code(),
                        forced: false,
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    return Err(SupervisorError::Spawn {
                        program: format!("pid {pid}"),
                        reason: error.to_string(),
                    });
                }
            }

            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // The grace period expired: force it, which also catches children the signal missed.
        self.cgroup.kill_all()?;
        let status = child.wait().map_err(|error| SupervisorError::Spawn {
            program: format!("pid {pid}"),
            reason: error.to_string(),
        })?;

        Ok(StopOutcome {
            code: status.code(),
            forced: true,
        })
    }

    /// Remove the cgroup. The namespace disappears with the last process in it.
    pub fn destroy(self) -> Result<(), SupervisorError> {
        self.cgroup.destroy()
    }
}
