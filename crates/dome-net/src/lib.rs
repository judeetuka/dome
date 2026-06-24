use dome_core::{EgressPolicy, PortMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerimeterPolicy {
    egress: EgressPolicy,
    published_ports: Vec<PortMap>,
}

impl PerimeterPolicy {
    pub fn isolated() -> Self {
        Self {
            egress: EgressPolicy::None,
            published_ports: Vec::new(),
        }
    }

    pub fn with_egress(mut self, egress: EgressPolicy) -> Self {
        self.egress = egress;
        self
    }

    pub fn publish(mut self, port: PortMap) -> Self {
        self.published_ports.push(port);
        self
    }

    pub fn egress(&self) -> EgressPolicy {
        self.egress
    }

    pub fn published_ports(&self) -> &[PortMap] {
        &self.published_ports
    }
}
