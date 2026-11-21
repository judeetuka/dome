pub mod cgroup;

use dome_core::ChamberId;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ChamberState {
    Planned,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChamberRecord {
    id: ChamberId,
    state: ChamberState,
}

impl ChamberRecord {
    pub fn new(id: ChamberId) -> Self {
        Self {
            id,
            state: ChamberState::Planned,
        }
    }

    pub fn transition(&mut self, state: ChamberState) {
        self.state = state;
    }

    pub fn id(&self) -> &ChamberId {
        &self.id
    }

    pub fn state(&self) -> ChamberState {
        self.state
    }
}
