//! Restart policy and host-restart reconciliation.
//!
//! Learned from Docker's `--restart` flag, which is the most battle-tested version of this:
//!
//! | Docker | meaning |
//! | --- | --- |
//! | `no` (default) | never restart |
//! | `on-failure[:max]` | restart when the exit was a failure, optionally limited |
//! | `always` | restart whenever it is not running |
//! | `unless-stopped` | like `always`, but a deliberate stop sticks |
//!
//! **The flaw being fixed:** under `always`, a container stopped by hand is restarted anyway
//! when the daemon restarts, which surprises people who thought their stop meant something.
//! Dome records *why* a chamber stopped instead of inferring it from the policy, so the
//! decision is explicit rather than emergent.
//!
//! From QEMU and libvirt comes the other half: a VMM does not supervise itself. The policy is
//! evaluated by the supervisor, never by the engine.

/// Why a chamber is not running.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ExitReason {
    /// The guest exited with a zero status.
    Clean,
    /// The guest exited non-zero or was killed.
    Failed,
    /// An operator asked for it to stop.
    StoppedByOperator,
    /// The host went away underneath it.
    HostLost,
}

impl ExitReason {
    /// Whether this counts as a failure for `on-failure`.
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::HostLost)
    }

    /// Stable name for the on-disk record.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Failed => "failed",
            Self::StoppedByOperator => "stopped-by-operator",
            Self::HostLost => "host-lost",
        }
    }

    /// Parse the on-disk form, returning `None` for anything unrecognised.
    pub fn parse_record(value: &str) -> Option<Self> {
        Some(match value {
            "clean" => Self::Clean,
            "failed" => Self::Failed,
            "stopped-by-operator" => Self::StoppedByOperator,
            "host-lost" => Self::HostLost,
            _ => return None,
        })
    }
}

/// What should happen when a chamber is not running.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Never restart. Docker's default.
    No,
    /// Restart after a failure, optionally at most this many times.
    OnFailure {
        /// Attempt budget; `None` means unlimited.
        max_retries: Option<u32>,
    },
    /// Restart whenever it is not running, except after a deliberate stop.
    Always,
    /// Kept for Docker compatibility, and **identical to [`RestartPolicy::Always`]** here.
    ///
    /// Docker distinguishes them by letting `always` override a manual stop when the daemon
    /// restarts. That is the flaw this module does not copy: an operator who typed stop should
    /// not find the chamber running again after a reboot. With that fixed, the two names mean
    /// the same thing, and saying so is better than inventing a third meaning for a word
    /// Docker users already know.
    UnlessStopped,
}

impl RestartPolicy {
    /// Whether to bring the chamber back, given why it stopped and how many attempts failed.
    ///
    /// A deliberate stop always wins. That is the Docker behaviour of `unless-stopped`, and
    /// here it is applied to *every* policy, because an operator who typed stop should not
    /// find the chamber running again after a reboot.
    pub fn should_restart(self, reason: ExitReason, attempts: u32) -> bool {
        if reason == ExitReason::StoppedByOperator {
            return false;
        }

        match self {
            Self::No => false,
            Self::OnFailure { max_retries } => {
                if !reason.is_failure() {
                    return false;
                }
                match max_retries {
                    Some(limit) => attempts < limit,
                    None => true,
                }
            }
            // Host loss is not the chamber's fault, so it is always worth resuming.
            Self::Always => true,
            Self::UnlessStopped => true,
        }
    }

    /// Stable name for the on-disk record.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::OnFailure { max_retries: None } => "on-failure",
            Self::OnFailure {
                max_retries: Some(_),
            } => "on-failure-limited",
            Self::Always => "always",
            Self::UnlessStopped => "unless-stopped",
        }
    }

    /// Parse the on-disk form, including the retry budget.
    pub fn parse_record(value: &str) -> Option<Self> {
        if let Some(rest) = value.strip_prefix("on-failure:") {
            return rest.parse::<u32>().ok().map(|max_retries| Self::OnFailure {
                max_retries: Some(max_retries),
            });
        }

        Some(match value {
            "no" => Self::No,
            "on-failure" => Self::OnFailure { max_retries: None },
            "always" => Self::Always,
            "unless-stopped" => Self::UnlessStopped,
            _ => return None,
        })
    }

    /// The on-disk form, with the retry budget spelled out.
    pub fn to_record(self) -> String {
        match self {
            Self::OnFailure {
                max_retries: Some(limit),
            } => format!("on-failure:{limit}"),
            other => other.as_str().to_owned(),
        }
    }
}

/// One attempt's worth of restart bookkeeping.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RestartState {
    /// How many restarts have already been attempted.
    pub attempts: u32,
    /// Why the chamber last stopped.
    pub reason: ExitReason,
}

impl RestartState {
    /// Fresh state: never restarted, and not stopped for any reason yet.
    pub fn new(reason: ExitReason) -> Self {
        Self {
            attempts: 0,
            reason,
        }
    }

    /// Whether this chamber should be running right now.
    pub fn wants_running(self, policy: RestartPolicy) -> bool {
        policy.should_restart(self.reason, self.attempts)
    }
}

impl std::str::FromStr for ExitReason {
    type Err = String;

    /// Uses the on-disk name, so a config file and a record file agree.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse_record(value).ok_or_else(|| format!("unknown exit reason {value:?}"))
    }
}

impl std::str::FromStr for RestartPolicy {
    type Err = String;

    /// Uses the on-disk name, so a config file and a record file agree.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse_record(value).ok_or_else(|| format!("unknown restart policy {value:?}"))
    }
}
