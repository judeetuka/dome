use dome_core::{DomeError, EgressPolicy, PortMap};
use dome_net::PerimeterPolicy;

#[test]
fn perimeter_is_closed_by_default() {
    let policy = PerimeterPolicy::isolated();

    assert_eq!(policy.egress(), EgressPolicy::None);
    assert!(policy.published_ports().is_empty());
}

#[test]
fn publish_adds_explicit_port() -> Result<(), DomeError> {
    let policy = PerimeterPolicy::isolated().publish(PortMap::local_tcp(8080, 80)?);

    assert_eq!(policy.published_ports().len(), 1);

    Ok(())
}
