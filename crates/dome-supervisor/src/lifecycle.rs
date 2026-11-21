//! Chamber lifecycle, modelled on how Docker actually behaves.
//!
//! Verified against the Docker documentation rather than recalled:
//!
//! - a container runs **until its main process exits**; nothing terminates it for being idle
//!   or unhealthy;
//! - `docker stop` sends a configurable signal (`SIGTERM` by default), waits a grace period,
//!   then sends `SIGKILL`, and `--timeout -1` waits indefinitely;
//! - health is a **status alongside** the normal status, never a terminator: `starting`, then
//!   `healthy` or `unhealthy`, with defaults interval 30s, timeout 30s, retries 3, start
//!   period 0s;
//! - restart policies decide what happens *after* exit, not when to exit.
//!
//! Dome follows that mapping deliberately. The chamber stops when its guest stops, `stop` is
//! signal-then-grace-then-force, and health or resource anomalies are **reported, never
//! acted on**. There is no idle timeout by default, because Docker has none and a kernel
//! sitting quietly is a running chamber, not a broken one.

use std::time::{Duration, Instant};

use dome_core::ChamberId;

use crate::cgroup::{ChamberCgroup, SupervisorError};

/// Signal sent to ask a chamber to stop. `SIGTERM` is what `docker stop` uses.
pub const STOP_SIGNAL: i32 = libc::SIGTERM;

/// How long to wait after asking nicely before forcing.
pub const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(10);

/// Where a chamber is in its life.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ChamberState {
    /// Defined but not started.
    Planned,
    /// The engine process exists but the guest has not taken over yet.
    Starting,
    /// The guest is running.
    Running,
    /// A stop was requested and the grace period is running.
    Stopping,
    /// Gone, cleanly or otherwise.
    Stopped,
    /// Something failed in the engine rather than in the guest.
    Failed,
}

/// Health, reported alongside the state and never used to decide it.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HealthState {
    /// No health check configured.
    Unknown,
    /// Inside the start period; failures are not counted yet, as in Docker.
    Starting,
    /// The last check, or a check during the start period, succeeded.
    Healthy,
    /// Consecutive failures reached the configured limit.
    Unhealthy,
}

/// How to stop a chamber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopPolicy {
    /// Signal to send first.
    pub signal: i32,
    /// Grace period before forcing. `None` waits indefinitely, like `docker stop --timeout -1`.
    pub grace: Option<Duration>,
}

impl Default for StopPolicy {
    fn default() -> Self {
        Self {
            signal: STOP_SIGNAL,
            grace: Some(DEFAULT_STOP_GRACE),
        }
    }
}

/// What happened when a chamber was stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopOutcome {
    /// The exit code, if the child reported one.
    pub code: Option<i32>,
    /// Whether the grace period expired and the cgroup had to be killed.
    pub forced: bool,
}

/// Health check timing and tolerance, with Docker's defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthPolicy {
    /// Time between checks.
    pub interval: Duration,
    /// How long a single check may take.
    pub timeout: Duration,
    /// Consecutive failures before reporting unhealthy.
    pub retries: u32,
    /// Bootstrap window during which failures are not counted.
    pub start_period: Duration,
}

impl Default for HealthPolicy {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(30),
            retries: 3,
            start_period: Duration::from_secs(0),
        }
    }
}

/// Health bookkeeping, kept free of clocks so its rules can be tested directly.
#[derive(Debug, Clone)]
pub struct HealthTracker {
    policy: HealthPolicy,
    failures: u32,
    state: HealthState,
    /// Set once a probe succeeds. Docker's rule: a success during the start period marks the
    /// container started, and from then on failures count even if the clock is still inside
    /// the window. Keying only off elapsed time gets that wrong.
    started: bool,
}

impl HealthTracker {
    /// Start tracking, in the `Starting` state.
    pub fn new(policy: HealthPolicy) -> Self {
        Self {
            policy,
            failures: 0,
            state: HealthState::Starting,
            started: false,
        }
    }

    /// Current health.
    pub fn state(&self) -> HealthState {
        self.state
    }

    /// Record one probe result.
    ///
    /// During the start period a failure is not counted, but a success marks the chamber
    /// started - exactly what the Docker documentation specifies.
    pub fn record(&mut self, succeeded: bool, elapsed: Duration) -> HealthState {
        // A success ends the start period immediately, so later failures count.
        let in_start_period = !self.started && elapsed < self.policy.start_period;

        if succeeded {
            self.failures = 0;
            self.started = true;
            self.state = HealthState::Healthy;
        } else if in_start_period {
            // Not counted yet, and the chamber is still starting.
            if self.state != HealthState::Healthy {
                self.state = HealthState::Starting;
            }
        } else {
            self.failures += 1;
            if self.failures >= self.policy.retries.max(1) {
                self.state = HealthState::Unhealthy;
            }
        }

        self.state
    }
}

/// What the chamber is using right now, read from its cgroup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceUsage {
    /// Total CPU time consumed, in microseconds.
    pub cpu_use_usec: u64,
    /// Current memory usage, in bytes.
    pub memory_bytes: u64,
}

impl ChamberCgroup {
    /// Read current CPU and memory usage.
    ///
    /// This is the monitoring the product wants, and it is deliberately *observation*: a
    /// chamber burning CPU is reported, not declared dead. Docker likewise reports usage and
    /// leaves the decision to the operator.
    pub fn usage(&self) -> Result<ResourceUsage, SupervisorError> {
        let cpu_stat = self.read("cpu.stat")?;
        let cpu_use_usec = cpu_stat
            .lines()
            .find_map(|line| line.strip_prefix("usage_usec "))
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0);

        let memory_bytes = self
            .read("memory.current")?
            .trim()
            .parse::<u64>()
            .unwrap_or(0);

        Ok(ResourceUsage {
            cpu_use_usec,
            memory_bytes,
        })
    }
}

/// A chamber identifier formatted for logs.
pub fn describe(id: &ChamberId) -> String {
    format!("chamber {}", id.as_str())
}

/// Milliseconds remaining until a deadline, saturating at zero.
pub fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}
