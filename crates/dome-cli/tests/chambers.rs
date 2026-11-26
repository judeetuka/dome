//! The run/stop loop, from the registry side and from the engine side.
//!
//! The registry half uses a stand-in launcher so it tests supervision rather than virtualisation.
//! The engine half boots a real kernel, and skips when KVM is unavailable.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dome_cli::chambers::{RunOptions, boot, list, start, stop};

fn scratch(name: &str) -> PathBuf {
    let unique = format!("dome-chambers-{name}-{}", std::process::id());
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

const CELL: &str = "[cell]\nfoundation = \"dome/fixture:test\"\n\n[limits]\nmemory_mib = 64\nstorage_mib = 256\ncpus = 1\n";

fn write_cell(dir: &Path) -> PathBuf {
    let path = dir.join("example.cell.toml");
    std::fs::write(&path, CELL).expect("cell");
    path
}

fn fixture_kernel() -> Option<PathBuf> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/kernel/vmlinuz-virt");
    path.is_file().then_some(path)
}

fn kvm_available() -> bool {
    Path::new("/dev/kvm").exists()
}

fn stand_in_launcher(dir: &Path) -> PathBuf {
    let path = dir.join("stand-in-chamber.sh");
    std::fs::write(&path, "#!/bin/sh\nsleep 30\n").expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    path
}

#[test]
fn an_empty_registry_says_so() {
    let dir = scratch("empty");
    let report = list(&dir).expect("list");
    assert!(report.contains("no chambers"), "{report}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_chamber_is_started_listed_and_stopped() {
    let dir = scratch("loop");
    let cell = write_cell(&dir);

    let mut options = RunOptions::new(&cell, &dir);
    options.name = Some("worker".to_owned());
    options.launcher = Some(stand_in_launcher(&dir));

    let started = start(&options).expect("start");
    assert!(started.contains("started worker"), "{started}");

    let listed = list(&dir).expect("list");
    assert!(listed.contains("worker"), "{listed}");
    assert!(
        listed.contains("running"),
        "a live chamber must be listed: {listed}"
    );

    let stopped = stop("worker", &dir, Duration::from_secs(5)).expect("stop");
    assert!(stopped.contains("stopped"), "{stopped}");

    let listed = list(&dir).expect("list");
    assert!(
        listed.contains("stopped"),
        "the record must survive the stop: {listed}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn starting_a_chamber_twice_is_refused_while_it_runs() {
    let dir = scratch("twice");
    let cell = write_cell(&dir);

    let mut options = RunOptions::new(&cell, &dir);
    options.name = Some("once".to_owned());
    options.launcher = Some(stand_in_launcher(&dir));

    start(&options).expect("start");

    // The invariant is "a running chamber cannot be started twice". Under a loaded machine the
    // registry write and the liveness check can be observed in either order, so wait for the state
    // the assertion is about instead of assuming it is already true.
    let mut observed = false;
    for _ in 0..100 {
        if list(&dir)
            .map(|report| report.contains("running"))
            .unwrap_or(false)
        {
            observed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(observed, "the started chamber never appeared as running");

    let error = start(&options).unwrap_err();
    assert!(error.contains("already running"), "{error}");

    stop("once", &dir, Duration::from_secs(5)).expect("stop");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn starting_a_chamber_without_a_foundation_fails_before_spawning() {
    let dir = scratch("nofoundation");
    let cell = write_cell(&dir);
    let foundations = dir.join("foundations");

    // No stand-in launcher here: this is about the real engine's pre-flight check, which runs
    // in the command before any child exists.
    let mut options = RunOptions::new(&cell, &dir);
    options.foundation_root = foundations;

    // The check happens in the command, not in the child, so the operator sees the problem.
    let error = start(&options).unwrap_err();
    assert!(error.contains("is not installed"), "{error}");

    // And nothing was recorded, so ps does not show a ghost.
    assert!(list(&dir).expect("list").contains("no chambers"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stopping_an_unknown_chamber_is_an_error() {
    let dir = scratch("unknown");
    let error = stop("ghost", &dir, Duration::from_millis(10)).unwrap_err();
    assert!(error.contains("ghost"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_chamber_body_boots_a_real_kernel() {
    let Some(kernel) = fixture_kernel() else {
        return;
    };
    if !kvm_available() {
        return;
    }

    let dir = scratch("boot");
    let cell = write_cell(&dir);

    let mut options = RunOptions::new(&cell, &dir);
    options.name = Some("booted".to_owned());
    options.kernel = Some(kernel);
    options.memory_mib = 64;
    options.timeout = Duration::from_secs(3);

    let console = boot(&options).expect("the chamber body should boot");

    // The console is what the guest wrote, which is the proof it ran.
    assert!(
        console.contains("Linux version"),
        "the guest did not boot: {console:.200}"
    );
    assert!(console.len() > 100, "the console should carry real output");

    // And the body records its own exit, so ps does not show a ghost.
    let listed = list(&dir).expect("list");
    assert!(listed.contains("booted"), "{listed}");
    assert!(listed.contains("stopped"), "{listed}");

    let _ = std::fs::remove_dir_all(&dir);
}
