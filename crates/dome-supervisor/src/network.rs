//! Per-chamber perimeter: what the chamber may reach, and how to identify its namespace.
//!
//! Measured constraint (T23): an unprivileged process cannot create a network namespace in
//! the host hierarchy, cannot create a veth pair there, and cannot `setns` into another user's
//! namespace. **The chamber therefore owns its namespace; this module plans and validates the
//! perimeter but cannot construct it from outside.**
//!
//! What is checked here is the part a mistake would actually cost: publishing a port the
//! process cannot bind, publishing the same port twice, or asking for a transport the relay
//! does not carry. Each is refused before anything is started, with an error naming the cause.

use std::path::{Path, PathBuf};

use dome_core::{ChamberId, EgressPolicy, PortMap, PortProtocol};
use dome_net::PerimeterPolicy;

use crate::cgroup::{SupervisorError, safe_name};

/// Ports below this cannot be bound without privilege, so publishing one is a planning error
/// rather than a runtime surprise.
const FIRST_UNPRIVILEGED_PORT: u16 = 1024;

/// A chamber's network perimeter.
#[derive(Debug, Clone)]
pub struct ChamberNetwork {
    name: String,
    policy: PerimeterPolicy,
}

impl ChamberNetwork {
    /// Validate a perimeter and name the chamber's namespace.
    pub fn plan(id: &ChamberId, policy: PerimeterPolicy) -> Result<Self, SupervisorError> {
        let name = format!("dome-{}", safe_name(id.as_str())?);

        let mut seen: Vec<(String, u16, PortProtocol)> = Vec::new();

        for port in policy.published_ports() {
            let host_port = port.host_port();

            // Binding below 1024 needs CAP_NET_BIND_SERVICE, which a zero-privilege
            // supervisor does not have. Failing here explains why, later it would look like
            // a mystery bind error.
            if host_port < FIRST_UNPRIVILEGED_PORT {
                return Err(SupervisorError::PrivilegedPort {
                    port: host_port,
                });
            }

            // The relay carries TCP. Accepting UDP and silently dropping it would be worse
            // than refusing.
            if port.protocol() != PortProtocol::Tcp {
                return Err(SupervisorError::UnsupportedProtocol {
                    protocol: "udp",
                });
            }

            let key = (port.host_addr().to_owned(), host_port, port.protocol());
            if seen.contains(&key) {
                return Err(SupervisorError::DuplicatePort {
                    address: key.0.clone(),
                    port: host_port,
                });
            }
            seen.push(key);
        }

        Ok(Self { name, policy })
    }

    /// Name of the chamber's network namespace.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The validated policy.
    pub fn policy(&self) -> &PerimeterPolicy {
        &self.policy
    }

    /// Whether the chamber may reach anything outside itself.
    pub fn is_isolated(&self) -> bool {
        self.policy.egress() == EgressPolicy::None && self.policy.published_ports().is_empty()
    }

    /// Ports the relay must carry.
    pub fn published(&self) -> &[PortMap] {
        self.policy.published_ports()
    }

    /// Path to a process's network namespace, for identity checks.
    pub fn netns_link(pid: u32) -> PathBuf {
        Path::new("/proc").join(pid.to_string()).join("ns").join("net")
    }

    /// Kernel inode identifying the network namespace a process is in.
    ///
    /// Two processes share a namespace exactly when these inodes match, which is how a test
    /// proves a chamber really did get its own.
    pub fn netns_inode(pid: u32) -> Result<u64, SupervisorError> {
        let link = Self::netns_link(pid);
        let target = std::fs::read_link(&link).map_err(|error| SupervisorError::Io {
            path: link.display().to_string(),
            source: error,
        })?;

        // The link reads "net:[4026532508]". A namespace whose name has no inode is not
        // something this process can inspect, so it is reported rather than guessed.
        let text = target.to_string_lossy().into_owned();
        let digits = text
            .strip_prefix("net:[")
            .and_then(|rest| rest.strip_suffix(']'))
            .ok_or_else(|| SupervisorError::UnexpectedNetns {
                pid,
                value: text.clone(),
            })?
            .to_owned();

        digits.parse::<u64>().map_err(|_| SupervisorError::UnexpectedNetns {
            pid,
            value: digits,
        })
    }

    /// Whether the namespace still exists for a live process.
    pub fn is_alive(pid: u32) -> bool {
        Self::netns_inode(pid).is_ok()
    }
}
