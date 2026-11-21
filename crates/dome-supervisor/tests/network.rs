//! Perimeter planning and namespace identity.

use dome_core::{ChamberId, EgressPolicy, PortMap, PortProtocol};
use dome_net::PerimeterPolicy;
use dome_supervisor::cgroup::SupervisorError;
use dome_supervisor::network::ChamberNetwork;

fn chamber(name: &str) -> ChamberId {
    ChamberId::new(name).expect("chamber id")
}

#[test]
fn an_isolated_chamber_publishes_nothing_and_reaches_nothing() {
    let network = ChamberNetwork::plan(&chamber("quiet"), PerimeterPolicy::isolated()).expect("plan");

    assert!(network.is_isolated());
    assert!(network.published().is_empty());
    assert_eq!(network.policy().egress(), EgressPolicy::None);
    assert_eq!(network.name(), "dome-quiet");
}

#[test]
fn publishing_a_port_stops_being_isolated() {
    let policy = PerimeterPolicy::isolated().publish(PortMap::local_tcp(8080, 80).expect("port"));
    let network = ChamberNetwork::plan(&chamber("web"), policy).expect("plan");

    assert!(!network.is_isolated());
    assert_eq!(network.published().len(), 1);
}

#[test]
fn a_privileged_host_port_is_refused_with_the_reason() {
    // Binding below 1024 needs a capability the supervisor does not have; failing here
    // explains it, whereas failing later looks like a mystery bind error.
    let policy = PerimeterPolicy::isolated().publish(PortMap::local_tcp(80, 8080).expect("port"));

    match ChamberNetwork::plan(&chamber("privileged"), policy) {
        Err(SupervisorError::PrivilegedPort { port }) => assert_eq!(port, 80),
        other => panic!("expected PrivilegedPort, got {other:?}"),
    }
}

#[test]
fn the_same_host_port_cannot_be_published_twice() {
    let policy = PerimeterPolicy::isolated()
        .publish(PortMap::local_tcp(8080, 80).expect("port"))
        .publish(PortMap::local_tcp(8080, 81).expect("port"));

    match ChamberNetwork::plan(&chamber("twice"), policy) {
        Err(SupervisorError::DuplicatePort { port, .. }) => assert_eq!(port, 8080),
        other => panic!("expected DuplicatePort, got {other:?}"),
    }
}

#[test]
fn a_udp_publication_is_refused_rather_than_silently_dropped() {
    let udp = PortMap::new("127.0.0.1", 5353, 53, PortProtocol::Udp).expect("port");
    let policy = PerimeterPolicy::isolated().publish(udp);

    match ChamberNetwork::plan(&chamber("udp"), policy) {
        Err(SupervisorError::UnsupportedProtocol { .. }) => {}
        other => panic!("expected UnsupportedProtocol, got {other:?}"),
    }
}

#[test]
fn a_chamber_in_its_own_namespace_has_a_different_inode() {
    let ours = match ChamberNetwork::netns_inode(std::process::id()) {
        Ok(inode) => inode,
        Err(error) => {
            eprintln!("skipping: {error}");
            return;
        }
    };

    // A child that unshares a network namespace is the same situation a chamber is in.
    let mut child = match std::process::Command::new("unshare")
        .args(["-Urn", "sleep", "5"])
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("skipping: cannot run unshare ({error})");
            return;
        }
    };

    let pid = child.id();
    std::thread::sleep(std::time::Duration::from_millis(300));

    match ChamberNetwork::netns_inode(pid) {
        Ok(theirs) => assert_ne!(
            ours, theirs,
            "the child shares our network namespace, so the identity check proves nothing"
        ),
        Err(error) => eprintln!("skipping: {error}"),
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn inspecting_a_process_that_does_not_exist_is_a_typed_error() {
    match ChamberNetwork::netns_inode(u32::MAX) {
        Err(SupervisorError::Io { .. }) => {}
        other => panic!("expected an Io error, got {other:?}"),
    }
}

#[test]
fn a_hostile_chamber_id_cannot_shape_a_namespace_name() {
    for hostile in ["../escape", "with space", "a/b"] {
        let Ok(id) = ChamberId::new(hostile) else { continue };
        match ChamberNetwork::plan(&id, PerimeterPolicy::isolated()) {
            Err(SupervisorError::InvalidChamberId(_)) => {}
            other => panic!("{hostile:?} was accepted: {other:?}"),
        }
    }
}
