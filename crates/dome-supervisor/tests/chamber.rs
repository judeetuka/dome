//! Chamber orchestration: ordering, fd handoff, cgroup membership.

use std::os::unix::net::UnixStream;

use dome_core::{ChamberId, CpuCount, MemorySize, ResourceLimits, StorageSize};
use dome_net::PerimeterPolicy;
use dome_supervisor::cgroup::SupervisorError;
use dome_supervisor::chamber::{CONTROL_FD, Chamber};
use dome_supervisor::lifecycle::StopPolicy;

fn limits() -> ResourceLimits {
    ResourceLimits::new(
        MemorySize::from_mib(128).expect("memory"),
        StorageSize::from_mib(1024).expect("storage"),
        CpuCount::new(1).expect("cpus"),
    )
}

fn chamber(name: &str) -> Result<Chamber, SupervisorError> {
    Chamber::create(
        ChamberId::new(name).expect("id"),
        limits(),
        PerimeterPolicy::isolated(),
    )
}

#[test]
fn the_engine_child_receives_the_control_channel_on_fd_three() {
    let Ok(chamber) = chamber("test-fd-handoff") else {
        eprintln!("skipping: no writable delegated cgroup subtree");
        return;
    };

    let (parent_end, child_end) = UnixStream::pair().expect("socketpair");

    // A real child process stands in for the engine: what matters here is that fd 3 arrives.
    let mut child = chamber
        .start_engine(
            std::path::Path::new("/bin/sh"),
            &["-c".to_owned(), "sleep 5".to_owned()],
            child_end,
        )
        .expect("start engine");

    let pid = child.id();
    let fd_link = format!("/proc/{pid}/fd/{CONTROL_FD}");

    let mut found_socket = false;
    for _ in 0..50 {
        if let Ok(target) = std::fs::read_link(&fd_link) {
            // A socket link reads "socket:[inode]".
            found_socket = target.to_string_lossy().starts_with("socket:");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        found_socket,
        "child did not receive a socket on fd {CONTROL_FD}"
    );

    // The parent still holds its end, so the channel is usable from both sides.
    drop(parent_end);

    let _ = child.kill();
    let _ = child.wait();
    chamber.destroy().expect("destroy");
}

#[test]
fn the_engine_child_is_attached_to_the_chamber_cgroup() {
    let Ok(chamber) = chamber("test-attach-on-start") else {
        eprintln!("skipping: no writable delegated cgroup subtree");
        return;
    };

    let (_parent_end, child_end) = UnixStream::pair().expect("socketpair");

    let mut child = chamber
        .start_engine(
            std::path::Path::new("/bin/sh"),
            &["-c".to_owned(), "sleep 5".to_owned()],
            child_end,
        )
        .expect("start engine");

    let pid = child.id();
    let members = chamber.cgroup().read("cgroup.procs").expect("cgroup.procs");

    assert!(
        members
            .split_whitespace()
            .any(|entry| entry == pid.to_string()),
        "pid {pid} was not attached: {members:?}"
    );

    let _ = child.kill();
    let _ = child.wait();
    chamber.destroy().expect("destroy");
}

#[test]
fn an_impossible_perimeter_leaves_no_cgroup_behind() {
    // The perimeter is validated before the cgroup exists, so a chamber with a policy that
    // cannot work never holds resources at all.
    let policy =
        PerimeterPolicy::isolated().publish(dome_core::PortMap::local_tcp(80, 8080).expect("port"));

    let result = Chamber::create(
        ChamberId::new("test-bad-policy").expect("id"),
        limits(),
        policy,
    );

    match result {
        Err(SupervisorError::PrivilegedPort { port }) => assert_eq!(port, 80),
        Err(other) => panic!("expected PrivilegedPort, got {other}"),
        Ok(chamber) => {
            let _ = chamber.destroy();
            panic!("a privileged port was accepted");
        }
    }
}

#[test]
fn destroying_a_chamber_removes_its_cgroup() {
    let Ok(chamber) = chamber("test-destroy-chamber") else {
        eprintln!("skipping: no writable delegated cgroup subtree");
        return;
    };

    let path = chamber.cgroup().path().to_path_buf();
    assert!(path.is_dir());

    chamber.destroy().expect("destroy");
    assert!(!path.exists(), "cgroup should be gone after destroy");
}

#[test]
fn a_cooperative_child_exits_within_the_grace_period() {
    let Ok(chamber) = chamber("test-stop-cooperative") else {
        eprintln!("skipping: no writable delegated cgroup subtree");
        return;
    };

    let (_parent_end, child_end) = UnixStream::pair().expect("socketpair");
    let mut child = chamber
        .start_engine(
            std::path::Path::new("/bin/sh"),
            &["-c".to_owned(), "sleep 30".to_owned()],
            child_end,
        )
        .expect("start engine");

    let policy = StopPolicy {
        grace: Some(std::time::Duration::from_secs(5)),
        ..StopPolicy::default()
    };

    let outcome = chamber.stop(&mut child, policy).expect("stop");
    assert!(
        !outcome.forced,
        "a child that honours SIGTERM should not need forcing"
    );

    chamber.destroy().expect("destroy");
}

#[test]
fn a_child_that_ignores_the_signal_is_forced_after_the_grace_period() {
    let Ok(chamber) = chamber("test-stop-forced") else {
        eprintln!("skipping: no writable delegated cgroup subtree");
        return;
    };

    let (_parent_end, child_end) = UnixStream::pair().expect("socketpair");
    // Trap and ignore SIGTERM: only the forced path can end this one.
    let mut child = chamber
        .start_engine(
            std::path::Path::new("/bin/sh"),
            &["-c".to_owned(), "trap '' TERM; sleep 30".to_owned()],
            child_end,
        )
        .expect("start engine");

    let policy = StopPolicy {
        grace: Some(std::time::Duration::from_millis(200)),
        ..StopPolicy::default()
    };

    let started = std::time::Instant::now();
    let outcome = chamber.stop(&mut child, policy).expect("stop");

    assert!(outcome.forced, "a child ignoring SIGTERM must be forced");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the grace period should be short here"
    );

    chamber.destroy().expect("destroy");
}

#[test]
fn with_no_grace_period_it_waits_instead_of_forcing() {
    let Ok(chamber) = chamber("test-stop-indefinite") else {
        eprintln!("skipping: no writable delegated cgroup subtree");
        return;
    };

    let (_parent_end, child_end) = UnixStream::pair().expect("socketpair");
    let mut child = chamber
        .start_engine(
            std::path::Path::new("/bin/sh"),
            &["-c".to_owned(), "sleep 30".to_owned()],
            child_end,
        )
        .expect("start engine");

    let policy = StopPolicy {
        grace: None,
        ..StopPolicy::default()
    };

    let outcome = chamber.stop(&mut child, policy).expect("stop");
    assert!(
        !outcome.forced,
        "no grace period means the force path is unreachable"
    );

    chamber.destroy().expect("destroy");
}

#[test]
fn usage_is_read_from_the_cgroup() {
    let Ok(chamber) = chamber("test-usage") else {
        eprintln!("skipping: no writable delegated cgroup subtree");
        return;
    };

    let (_parent_end, child_end) = UnixStream::pair().expect("socketpair");
    let mut child = chamber
        .start_engine(
            std::path::Path::new("/bin/sh"),
            &["-c".to_owned(), "sleep 5".to_owned()],
            child_end,
        )
        .expect("start engine");

    std::thread::sleep(std::time::Duration::from_millis(150));
    let usage = chamber.cgroup().usage().expect("usage");

    // The child is in this cgroup, so memory must be accounted to it.
    assert!(
        usage.memory_bytes > 0,
        "expected memory usage, got {usage:?}"
    );

    let _ = child.kill();
    let _ = child.wait();
    chamber.destroy().expect("destroy");
}
