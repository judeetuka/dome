//! Durable chamber records, and what to do with them after the host restarts.
//!
//! Cgroups and network namespaces do not survive a reboot, and neither does an engine
//! process. The **record** does, and it is the only thing that can answer "should this
//! chamber be running?" at startup.
//!
//! The file format is deliberately plain text, one `key=value` per line: it can be read with
//! `cat` while debugging a supervisor that will not start, which is exactly when a binary
//! format is least welcome. Writes go to a temporary file and are renamed, so a record is
//! never half-written.
//!
//! A corrupt record does not stop the others loading. It is reported and skipped, because one
//! bad file must not take the whole host's chambers down.

use std::fs;
use std::path::{Path, PathBuf};

use dome_core::ChamberId;

use crate::cgroup::{SupervisorError, safe_name};
use crate::restart::{ExitReason, RestartPolicy, RestartState};

/// One chamber as it is stored on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedChamber {
    /// Chamber identifier.
    pub id: ChamberId,
    /// What to do when it is not running.
    pub policy: RestartPolicy,
    /// Restart bookkeeping.
    pub state: RestartState,
    /// Whether it was running when the record was last written.
    pub was_running: bool,
}

/// A record that could not be read, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedRecord {
    /// File that was rejected.
    pub path: PathBuf,
    /// Why it was rejected.
    pub reason: String,
}

/// Where chamber records live.
#[derive(Debug, Clone)]
pub struct ChamberStore {
    root: PathBuf,
}

impl ChamberStore {
    /// Use a specific directory, which is what tests do.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The default location: `$XDG_STATE_HOME/dome/chambers`, or under the home directory.
    pub fn default_root() -> PathBuf {
        if let Ok(base) = std::env::var("XDG_STATE_HOME") {
            return PathBuf::from(base).join("dome").join("chambers");
        }

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("dome")
            .join("chambers")
    }

    /// The directory in use.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where one chamber's record lives.
    pub fn path_for(&self, id: &ChamberId) -> Result<PathBuf, SupervisorError> {
        Ok(self
            .root
            .join(format!("{}.chamber", safe_name(id.as_str())?)))
    }

    /// Write a record, replacing any previous one atomically.
    pub fn save(&self, record: &PersistedChamber) -> Result<(), SupervisorError> {
        fs::create_dir_all(&self.root).map_err(|error| SupervisorError::Io {
            path: self.root.display().to_string(),
            source: error,
        })?;

        let path = self.path_for(&record.id)?;
        let temporary = path.with_extension("chamber.tmp");

        let body = format!(
            "id={}\npolicy={}\nattempts={}\nreason={}\nrunning={}\n",
            record.id.as_str(),
            record.policy.to_record(),
            record.state.attempts,
            record.state.reason.as_str(),
            record.was_running,
        );

        fs::write(&temporary, body).map_err(|error| SupervisorError::Io {
            path: temporary.display().to_string(),
            source: error,
        })?;

        // Rename is atomic within a directory, so a crash leaves either the old record or the
        // new one, never a partial file.
        fs::rename(&temporary, &path).map_err(|error| SupervisorError::Io {
            path: path.display().to_string(),
            source: error,
        })
    }

    /// Read one record, if it exists.
    pub fn load(&self, id: &ChamberId) -> Result<Option<PersistedChamber>, SupervisorError> {
        let path = self.path_for(id)?;
        if !path.is_file() {
            return Ok(None);
        }

        let text = fs::read_to_string(&path).map_err(|error| SupervisorError::Io {
            path: path.display().to_string(),
            source: error,
        })?;

        match parse(&text) {
            Ok(record) => Ok(Some(record)),
            Err(reason) => Err(SupervisorError::Io {
                path: path.display().to_string(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, reason),
            }),
        }
    }

    /// Read every record, reporting the ones that could not be read.
    ///
    /// A single corrupt file is not allowed to prevent the rest from starting.
    pub fn load_all(
        &self,
    ) -> Result<(Vec<PersistedChamber>, Vec<RejectedRecord>), SupervisorError> {
        let mut records = Vec::new();
        let mut rejected = Vec::new();

        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((records, rejected));
            }
            Err(error) => {
                return Err(SupervisorError::Io {
                    path: self.root.display().to_string(),
                    source: error,
                });
            }
        };

        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();

            if path.extension().and_then(|ext| ext.to_str()) != Some("chamber") {
                continue;
            }

            match fs::read_to_string(&path) {
                Ok(text) => match parse(&text) {
                    Ok(record) => records.push(record),
                    Err(reason) => rejected.push(RejectedRecord { path, reason }),
                },
                Err(error) => rejected.push(RejectedRecord {
                    path,
                    reason: error.to_string(),
                }),
            }
        }

        // Deterministic order, so a caller's decisions do not depend on directory order.
        records.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));

        Ok((records, rejected))
    }

    /// Forget a chamber.
    pub fn remove(&self, id: &ChamberId) -> Result<(), SupervisorError> {
        let path = self.path_for(id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(SupervisorError::Io {
                path: path.display().to_string(),
                source: error,
            }),
        }
    }

    /// Which chambers should be running now, given their policies.
    ///
    /// A record that was running when it was last written did not stop on its own, so it is
    /// treated as [`ExitReason::HostLost`]: the host went away, which is not the chamber's
    /// fault and is exactly what a restart policy is for.
    pub fn reconcile(&self) -> Result<Vec<PersistedChamber>, SupervisorError> {
        let (records, _rejected) = self.load_all()?;

        Ok(records
            .into_iter()
            .filter(|record| {
                let reason = if record.was_running {
                    ExitReason::HostLost
                } else {
                    record.state.reason
                };

                RestartState {
                    attempts: record.state.attempts,
                    reason,
                }
                .wants_running(record.policy)
            })
            .collect())
    }
}

fn parse(text: &str) -> Result<PersistedChamber, String> {
    let mut id = None;
    let mut policy = None;
    let mut attempts = 0u32;
    let mut reason = None;
    let mut was_running = false;

    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "id" => id = Some(value.trim().to_owned()),
            "policy" => {
                policy = Some(
                    RestartPolicy::parse_record(value.trim())
                        .ok_or_else(|| format!("unknown restart policy {:?}", value.trim()))?,
                );
            }
            "attempts" => attempts = value.trim().parse().unwrap_or(0),
            "reason" => {
                reason = Some(
                    ExitReason::parse_record(value.trim())
                        .ok_or_else(|| format!("unknown exit reason {:?}", value.trim()))?,
                );
            }
            "running" => was_running = value.trim() == "true",
            _ => {}
        }
    }

    let id = id.ok_or_else(|| "record has no id".to_owned())?;
    let id = ChamberId::new(id).map_err(|error| format!("bad chamber id: {error}"))?;

    Ok(PersistedChamber {
        id,
        policy: policy.unwrap_or(RestartPolicy::No),
        state: RestartState {
            attempts,
            reason: reason.unwrap_or(ExitReason::Clean),
        },
        was_running,
    })
}
