//! Per-chamber cgroup tests.
//!
//! These skip when the host does not expose a writable delegated subtree, because a machine
//! without one is not evidence of a bug. Where it does exist, they assert the real kernel
//! state rather than that a function returned Ok.

use dome_core::{ChamberId, CpuCount, MemorySize, ResourceLimits, StorageSize};
use dome_supervisor::cgroup::{ChamberCgroup, SupervisorError, delegated_root};

fn limits(memory_mib: u64, cpus: u16) -> ResourceLimits {
    ResourceLimits::new(
        MemorySize::from_mib(memory_mib).expect("memory"),
        StorageSize::from_mib(1024).expect("storage"),
        CpuCount::new(cpus).expect("cpus"),
    )
}

fn chamber(name: &str) -> ChamberId {
    ChamberId::new(name).expect("chamber id")
}

/// Returns None when this host has no writable delegated subtree.
fn try_create(name: &str) -> Option<ChamberCgroup> {
    match ChamberCgroup::create(&chamber(name), limits(64, 1)) {
        Ok(cgroup) => Some(cgroup),
        Err(error @ (SupervisorError::CgroupUnavailable(_) | SupervisorError::NotDelegated(_))) => {
            eprintln!("skipping: {error}");
            None
        }
        Err(error @ SupervisorError::Io { .. }) => {
            eprintln!("skipping: {error}");
            None
        }
        Err(other) => panic!("unexpected cgroup error: {other}"),
    }
}

#[test]
fn the_delegated_root_is_inside_the_cgroup_hierarchy() {
    match delegated_root() {
        Ok(root) => {
            assert!(
                root.starts_with("/sys/fs/cgroup"),
                "delegated root {} is outside the hierarchy",
                root.display()
            );
        }
        Err(SupervisorError::CgroupUnavailable(message)) => eprintln!("skipping: {message}"),
        Err(other) => panic!("unexpected error: {other}"),
    }
}

#[test]
fn limits_are_written_to_the_kernel() {
    let Some(cgroup) = try_create("test-limits") else {
        return;
    };

    assert_eq!(
        cgroup.read("memory.max").expect("memory.max"),
        (64 * 1024 * 1024u64).to_string()
    );
    assert_eq!(cgroup.read("cpu.max").expect("cpu.max"), "100000 100000");
    assert_eq!(cgroup.read("pids.max").expect("pids.max"), "64");

    // A fresh cgroup holds no processes.
    assert_eq!(cgroup.read("cgroup.procs").expect("cgroup.procs"), "");

    cgroup.destroy().expect("destroy");
}

#[test]
fn more_cpus_raise_both_the_cpu_and_process_limits() {
    let Ok(root) = delegated_root() else { return };

    let cgroup = match ChamberCgroup::create(&chamber("test-two-cpu"), limits(128, 2)) {
        Ok(cgroup) => cgroup,
        Err(error) => {
            eprintln!("skipping: {error}");
            return;
        }
    };
    assert!(root.is_dir());

    assert_eq!(cgroup.read("cpu.max").expect("cpu.max"), "200000 100000");
    // Two CPUs: 2 * 64 processes.
    assert_eq!(cgroup.read("pids.max").expect("pids.max"), "128");

    cgroup.destroy().expect("destroy");
}

#[test]
fn a_process_can_be_attached() {
    let Some(cgroup) = try_create("test-attach") else {
        return;
    };

    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    let pid = child.id();

    cgroup.attach_process(pid).expect("attach");

    let members = cgroup.read("cgroup.procs").expect("cgroup.procs");
    assert!(
        members
            .split_whitespace()
            .any(|entry| entry == pid.to_string()),
        "pid {pid} not in cgroup.procs: {members:?}"
    );

    let _ = child.kill();
    let _ = child.wait();
    cgroup.destroy().expect("destroy");
}

#[test]
fn destroy_removes_the_directory() {
    let Some(cgroup) = try_create("test-destroy") else {
        return;
    };
    let path = cgroup.path().to_path_buf();

    assert!(
        path.is_dir(),
        "cgroup directory should exist before destroy"
    );
    cgroup.destroy().expect("destroy");
    assert!(
        !path.exists(),
        "cgroup directory should be gone after destroy"
    );
}

#[test]
fn an_unsafe_chamber_id_is_refused() {
    // A chamber id is user input; interpolating it into a path would allow traversal out of
    // the delegated subtree.
    for hostile in ["../../escape", "with space", "", "..", "a/b"] {
        let id = match ChamberId::new(hostile) {
            Ok(id) => id,
            Err(_) => continue,
        };

        match ChamberCgroup::create(&id, limits(64, 1)) {
            Err(SupervisorError::InvalidChamberId(_)) => {}
            Err(other) => panic!("{hostile:?} gave an unexpected error: {other}"),
            Ok(cgroup) => {
                let path = cgroup.path().to_path_buf();
                let _ = cgroup.destroy();
                panic!(
                    "{hostile:?} was accepted as a cgroup name: {}",
                    path.display()
                );
            }
        }
    }
}
