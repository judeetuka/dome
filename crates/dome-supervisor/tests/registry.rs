//! The registry, against a real child process.
//!
//! These spawn an actual process rather than mocking one, because the interesting behaviour -
//! liveness, signalling, forcing - only exists in the kernel.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use dome_core::ChamberId;
use dome_supervisor::registry::{ChamberEntry, Registry, is_alive};
use dome_supervisor::restart::ExitReason;
use dome_supervisor::state::ChamberStore;

fn scratch(name: &str) -> PathBuf {
    let unique = format!("dome-registry-{name}-{}", std::process::id());
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn spawn_sleeper() -> std::process::Child {
    Command::new("sh")
        .arg("-c")
        .arg("sleep 30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn a child")
}

fn entry(id: &str, pid: u32) -> ChamberEntry {
    ChamberEntry {
        id: ChamberId::new(id).expect("id"),
        pid: Some(pid),
        cell: Some("example.cell.toml".to_owned()),
        started_unix: 1,
        was_running: true,
    }
}

#[test]
fn a_recorded_chamber_is_reported_running() {
    let dir = scratch("running");
    let registry = Registry::new(ChamberStore::at(dir.join("chambers")));
    let mut child = spawn_sleeper();

    registry
        .record_started(&entry("web", child.id()))
        .expect("record");

    let listed = registry.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].1, "a live pid must be reported as running");
    assert_eq!(listed[0].0.cell.as_deref(), Some("example.cell.toml"));

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_dead_process_is_not_reported_running() {
    let dir = scratch("stale");
    let registry = Registry::new(ChamberStore::at(dir.join("chambers")));
    let mut child = spawn_sleeper();
    let pid = child.id();

    registry
        .record_started(&entry("gone", pid))
        .expect("record");
    child.kill().expect("kill");
    child.wait().expect("reap");

    // The record still says running; the kernel says otherwise, and the kernel wins.
    let listed = registry.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].0.was_running, "the record is stale");
    assert!(!listed[0].1, "a dead pid must not be reported as running");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stopping_an_idle_chamber_says_it_was_already_stopped() {
    let dir = scratch("idle");
    let registry = Registry::new(ChamberStore::at(dir.join("chambers")));
    let id = ChamberId::new("idle").expect("id");

    registry
        .record_started(&ChamberEntry {
            id: id.clone(),
            pid: None,
            cell: None,
            started_unix: 0,
            was_running: true,
        })
        .expect("record");

    let report = registry.stop(&id, Duration::from_millis(50)).expect("stop");
    assert!(report.already_stopped);
    assert!(!report.forced);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stopping_a_cooperative_process_needs_no_force() {
    let dir = scratch("cooperative");
    let registry = Registry::new(ChamberStore::at(dir.join("chambers")));
    let mut child = spawn_sleeper();
    let id = ChamberId::new("worker").expect("id");

    registry
        .record_started(&entry("worker", child.id()))
        .expect("record");

    let report = registry.stop(&id, Duration::from_secs(5)).expect("stop");
    let _ = child.wait();

    assert!(!report.forced, "SIGTERM should have been enough");
    assert!(!report.already_stopped);

    let listed = registry.list().expect("list");
    assert!(!listed[0].1, "it is no longer running");
    let record = registry.store().load(&id).expect("load").expect("present");
    assert_eq!(record.state.reason, ExitReason::StoppedByOperator);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_process_that_ignores_the_signal_is_forced() {
    let dir = scratch("stubborn");
    let registry = Registry::new(ChamberStore::at(dir.join("chambers")));
    let id = ChamberId::new("stubborn").expect("id");

    // The child announces that the trap is installed before it sleeps. Without that handshake
    // the SIGTERM can arrive while the shell is still starting up, the default action kills it,
    // and the test measures a race instead of the behaviour it claims to test.
    let ready = dir.join("ready");
    let script = format!("trap '' TERM; : > {}; sleep 30", ready.display());

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");

    for _ in 0..200 {
        if ready.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists(), "the child never installed its trap");

    registry
        .record_started(&entry("stubborn", child.id()))
        .expect("record");

    let report = registry
        .stop(&id, Duration::from_millis(150))
        .expect("stop");
    let _ = child.wait();

    assert!(
        report.forced,
        "it should have been killed after the grace period"
    );
    assert!(!is_alive(child.id()), "the process must be gone");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stopping_an_unknown_chamber_is_an_error() {
    let dir = scratch("unknown");
    let registry = Registry::new(ChamberStore::at(dir.join("chambers")));
    let id = ChamberId::new("nope").expect("id");

    assert!(registry.stop(&id, Duration::from_millis(10)).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
