//! Stacks: several chambers that belong together.
//!
//! A `.stack.toml` names services, each backed by a cell, and the dependencies between them.
//! This is the compose-shaped layer: one command brings up a group, in order, on a network only
//! the group can reach.
//!
//! The security question is the whole design. Chambers in a stack can talk to each other, and
//! nothing else can reach them unless a cell publishes a port. That is the same boundary the
//! perimeter draws for a single chamber, extended to a group: membership is the trust boundary,
//! so a stack is a set of chambers that already trust each other.

use std::collections::{BTreeMap, BTreeSet};

use crate::ConfigError;

/// What a service must wait for before its dependants start.
///
/// Docker's depends_on only waits for the container to start, which is the classic race: the
/// database process is running well before it accepts connections. Naming the condition makes
/// the wait mean what people think it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitCondition {
    /// The chamber exists and its process is running. Docker's behaviour.
    Started,
    /// The chamber's health check reports healthy, which is T25's HealthTracker.
    Healthy,
    /// Something accepts a connection on this port inside the chamber.
    PortReady(u16),
}

impl WaitCondition {
    /// Parse the suffix after a service name.
    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed == "started" {
            return Some(Self::Started);
        }
        if trimmed == "healthy" {
            return Some(Self::Healthy);
        }
        if let Some(port) = trimmed.strip_prefix("port:") {
            return port.trim().parse().ok().map(Self::PortReady);
        }

        None
    }
}

/// A dependency, and what has to be true before starting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// The service depended on.
    pub service: String,
    /// What must be true about it.
    pub condition: WaitCondition,
}

impl Dependency {
    /// Parse "web", "web:healthy" or "web:port:5432".
    pub fn parse(value: &str) -> Option<Self> {
        let (service, rest) = match value.split_once(':') {
            Some((service, rest)) => (service.trim(), rest),
            None => (value.trim(), ""),
        };

        if service.is_empty() {
            return None;
        }

        Some(Self {
            service: service.to_owned(),
            condition: WaitCondition::parse(rest)?,
        })
    }
}

/// One service in a stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// Service name, which is also its hostname on the stack network.
    pub name: String,
    /// The cell blueprint that defines the chamber.
    pub cell: String,
    /// Services that must be ready first, and what ready means for each.
    pub depends_on: Vec<Dependency>,
}

/// A group of chambers that share an isolated network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    /// Stack name.
    pub name: String,
    /// Services, sorted by name so plans are stable.
    pub services: Vec<Service>,
}

impl Stack {
    /// Parse a `.stack.toml`.
    pub fn from_toml(input: &str) -> Result<Self, ConfigError> {
        #[derive(serde::Deserialize)]
        struct File {
            stack: StackTable,
            #[serde(default)]
            services: BTreeMap<String, ServiceTable>,
        }

        #[derive(serde::Deserialize)]
        struct StackTable {
            name: String,
        }

        #[derive(serde::Deserialize)]
        struct ServiceTable {
            cell: String,
            #[serde(default)]
            depends_on: Vec<String>,
        }

        let file: File =
            toml::from_str(input).map_err(|error| ConfigError::Parse(error.to_string()))?;

        let mut services = Vec::with_capacity(file.services.len());
        for (name, service) in file.services {
            let mut depends_on = Vec::with_capacity(service.depends_on.len());
            for raw in &service.depends_on {
                let dependency =
                    Dependency::parse(raw).ok_or_else(|| ConfigError::InvalidValue {
                        field: "services.depends_on",
                        reason: format!(
                            "{raw:?} is not service, service:healthy or service:port:PORT"
                        ),
                    })?;
                depends_on.push(dependency);
            }

            services.push(Service {
                name,
                cell: service.cell,
                depends_on,
            });
        }

        let stack = Self {
            name: file.stack.name,
            services,
        };

        stack.validate()?;
        Ok(stack)
    }

    /// Check the stack can actually be brought up.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.name.trim().is_empty() {
            return Err(ConfigError::InvalidValue {
                field: "stack.name",
                reason: "must not be empty".to_owned(),
            });
        }

        if self.services.is_empty() {
            return Err(ConfigError::MissingField("stack.services"));
        }

        let mut seen = BTreeSet::new();
        for service in &self.services {
            if !seen.insert(service.name.as_str()) {
                return Err(ConfigError::InvalidValue {
                    field: "services",
                    reason: format!("service {:?} is defined twice", service.name),
                });
            }

            if service.cell.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    field: "services.cell",
                    reason: format!("service {:?} has no cell", service.name),
                });
            }

            for dependency in &service.depends_on {
                if !self
                    .services
                    .iter()
                    .any(|other| other.name == dependency.service)
                {
                    return Err(ConfigError::Unknown {
                        kind: "service dependency",
                        value: dependency.service.clone(),
                    });
                }
                if dependency.service == service.name {
                    return Err(ConfigError::InvalidValue {
                        field: "services.depends_on",
                        reason: format!("service {:?} depends on itself", service.name),
                    });
                }
            }
        }

        // A cycle would mean nothing can start first, and the failure would otherwise appear
        // as chambers mysteriously waiting for each other.
        self.start_order()?;
        Ok(())
    }

    /// The order services must start in.
    pub fn start_order(&self) -> Result<Vec<String>, ConfigError> {
        let mut ordered = Vec::new();
        let mut visiting = BTreeSet::new();
        let mut done = BTreeSet::new();

        for service in &self.services {
            self.visit(&service.name, &mut visiting, &mut done, &mut ordered)?;
        }

        Ok(ordered)
    }

    fn visit(
        &self,
        name: &str,
        visiting: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
        ordered: &mut Vec<String>,
    ) -> Result<(), ConfigError> {
        if done.contains(name) {
            return Ok(());
        }

        if !visiting.insert(name.to_owned()) {
            return Err(ConfigError::InvalidValue {
                field: "services.depends_on",
                reason: format!("dependency cycle involving {name:?}"),
            });
        }

        let Some(service) = self.services.iter().find(|service| service.name == name) else {
            return Ok(());
        };

        for dependency in service.depends_on.clone() {
            self.visit(&dependency.service, visiting, done, ordered)?;
        }

        visiting.remove(name);
        done.insert(name.to_owned());
        ordered.push(name.to_owned());
        Ok(())
    }

    /// The hostnames the stack network resolves, which are its service names.
    pub fn hostnames(&self) -> Vec<String> {
        self.services
            .iter()
            .map(|service| service.name.clone())
            .collect()
    }
}
