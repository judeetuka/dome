//! The registry of chambers a host knows about.
//!
//! This is the piece that makes run and stop real across process boundaries. A chamber is an
//! engine process; the record is what lets a later command find it again.
//!
//! Liveness is decided by asking the kernel about the pid rather than trusting the record. A
//! record that says running while the process is gone is exactly the state a crash leaves
//! behind, and reporting it as running would be a lie.

use dome_core::ChamberId;

use crate::cgroup::SupervisorError;
use crate::restart::{ExitReason, RestartPolicy, RestartState};
use crate::state::{ChamberStore, PersistedChamber};

/// One chamber as the registry knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChamberEntry {
    /// Chamber identifier.
    pub id: ChamberId,
    /// Engine process, once started.
    pub pid: Option<u32>,
    /// The blueprint it was created from.
    pub cell: Option<String>,
    /// When it started, in seconds since the Unix epoch.
    pub started_unix: u64,
    /// Whether the record claims it is running.
    pub was_running: bool,
}

/// Whether a process is alive.
///
/// Signal zero answers "does this process exist", but it answers "yes" for a **zombie** too: the
/// entry survives until the parent reaps it. A chamber that exited cleanly would otherwise be
/// reported as running, and stopping it would wait out the whole grace period and then report a
/// forced kill that never happened. The process state in /proc is what settles it.
pub fn is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    if let Some(state) = process_state(pid) {
        // Z is a zombie, X is dead. Everything else is a live process.
        return state != 'Z' && state != 'X';
    }

    // Without /proc, fall back to the existence check, which may be optimistic about zombies.
    #[allow(unsafe_code)]
    // SAFETY: kill with signal 0 touches no memory and delivers no signal.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }

    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The scheduling state letter from /proc, or None when it cannot be read.
fn process_state(pid: u32) -> Option<char> {
    // /proc/<pid>/stat is "<pid> (<comm>) <state> ...", and comm may itself contain spaces or
    // parentheses, so the state is read after the last closing parenthesis rather than by
    // splitting on whitespace.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = stat.rsplit_once(')')?.1;
    after_name.split_whitespace().next()?.chars().next()
}

/// The registry.
#[derive(Debug, Clone)]
pub struct Registry {
    store: ChamberStore,
}

impl Registry {
    /// Use a store for persistence.
    pub fn new(store: ChamberStore) -> Self {
        Self { store }
    }

    /// A registry under a specific state directory, which is what tests use.
    pub fn at(state_root: impl Into<std::path::PathBuf>) -> Self {
        Self::new(ChamberStore::at(state_root))
    }

    /// The default registry under the user's state directory.
    pub fn open() -> Self {
        Self::new(ChamberStore::at(ChamberStore::default_root()))
    }

    /// The store behind it.
    pub fn store(&self) -> &ChamberStore {
        &self.store
    }

    /// Record that a chamber has started.
    pub fn record_started(&self, entry: &ChamberEntry) -> Result<(), SupervisorError> {
        self.store.save(&PersistedChamber {
            id: entry.id.clone(),
            policy: RestartPolicy::No,
            state: RestartState {
                attempts: 0,
                reason: ExitReason::Clean,
            },
            was_running: true,
            pid: entry.pid,
            cell: entry.cell.clone(),
            started_unix: entry.started_unix,
        })
    }

    /// Record that a chamber stopped, keeping the record for inspection.
    ///
    /// A chamber that stops without a previous start record - a body run by hand, or a record
    /// lost to a crash - still leaves one. Reporting nothing would make a chamber that really
    /// ran invisible, which is worse than a record with no pid.
    pub fn record_stopped(
        &self,
        id: &ChamberId,
        reason: ExitReason,
    ) -> Result<(), SupervisorError> {
        let previous = self.store.load(id)?;

        self.store.save(&PersistedChamber {
            id: id.clone(),
            policy: previous
                .as_ref()
                .map(|p| p.policy)
                .unwrap_or(RestartPolicy::No),
            state: RestartState {
                attempts: previous.as_ref().map(|p| p.state.attempts).unwrap_or(0),
                reason,
            },
            was_running: false,
            pid: previous.as_ref().and_then(|p| p.pid),
            cell: previous.as_ref().and_then(|p| p.cell.clone()),
            started_unix: previous.map(|p| p.started_unix).unwrap_or(0),
        })
    }

    /// Forget a chamber entirely.
    pub fn forget(&self, id: &ChamberId) -> Result<(), SupervisorError> {
        self.store.remove(id)
    }

    /// Every chamber, with the truth about whether it is running.
    ///
    /// The record can be stale - a crash leaves one claiming to run - so liveness comes from
    /// the kernel, not from the file.
    pub fn list(&self) -> Result<Vec<(ChamberEntry, bool)>, SupervisorError> {
        let (records, _rejected) = self.store.load_all()?;

        Ok(records
            .into_iter()
            .map(|record| {
                let running = record.was_running && record.pid.is_some_and(is_alive);
                let entry = ChamberEntry {
                    id: record.id,
                    pid: record.pid,
                    cell: record.cell,
                    started_unix: record.started_unix,
                    was_running: record.was_running,
                };
                (entry, running)
            })
            .collect())
    }

    /// Look one up.
    pub fn get(&self, id: &ChamberId) -> Result<Option<(ChamberEntry, bool)>, SupervisorError> {
        Ok(self.list()?.into_iter().find(|(entry, _)| &entry.id == id))
    }

    /// Send a signal to a running chamber.
    pub fn signal(&self, id: &ChamberId, signal: i32) -> Result<(), SupervisorError> {
        let Some((entry, running)) = self.get(id)? else {
            return Err(SupervisorError::CgroupUnavailable(format!(
                "no chamber named {}",
                id.as_str()
            )));
        };

        if !running {
            return Ok(());
        }

        let Some(pid) = entry.pid else {
            return Ok(());
        };

        #[allow(unsafe_code)]
        // SAFETY: kill is called with a pid from a record this user owns, and a signal number
        // the caller chose. It touches no memory.
        let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(SupervisorError::Io {
                    path: format!("pid {pid}"),
                    source: error,
                });
            }
        }

        Ok(())
    }

    /// Stop a chamber the way Docker does: ask, wait, then insist.
    ///
    /// Returns whether it had to be forced.
    pub fn stop(
        &self,
        id: &ChamberId,
        grace: std::time::Duration,
    ) -> Result<StopReport, SupervisorError> {
        let Some((entry, running)) = self.get(id)? else {
            return Err(SupervisorError::CgroupUnavailable(format!(
                "no chamber named {}",
                id.as_str()
            )));
        };

        if !running {
            self.record_stopped(id, ExitReason::StoppedByOperator)?;
            return Ok(StopReport {
                forced: false,
                already_stopped: true,
            });
        }

        self.signal(id, libc::SIGTERM)?;

        let deadline = std::time::Instant::now() + grace;
        while std::time::Instant::now() < deadline {
            if !entry.pid.is_some_and(is_alive) {
                self.record_stopped(id, ExitReason::StoppedByOperator)?;
                return Ok(StopReport {
                    forced: false,
                    already_stopped: false,
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // It ignored the polite request.
        self.signal(id, libc::SIGKILL)?;
        for _ in 0..100 {
            if !entry.pid.is_some_and(is_alive) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        self.record_stopped(id, ExitReason::StoppedByOperator)?;
        Ok(StopReport {
            forced: true,
            already_stopped: false,
        })
    }
}

/// What stopping did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopReport {
    /// Whether the process had to be killed after the grace period.
    pub forced: bool,
    /// Whether it was already gone.
    pub already_stopped: bool,
}
