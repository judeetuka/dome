use dome_core::{ChamberId, DomeError};
use dome_supervisor::{ChamberRecord, ChamberState};

#[test]
fn chamber_record_tracks_state() -> Result<(), DomeError> {
    let mut record = ChamberRecord::new(ChamberId::new("dev")?);

    assert_eq!(record.state(), ChamberState::Planned);
    record.transition(ChamberState::Running);
    assert_eq!(record.state(), ChamberState::Running);

    Ok(())
}
