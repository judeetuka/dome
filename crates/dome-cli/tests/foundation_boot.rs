//! A foundation Dome builds, boots, and proves itself.
//!
//! Everything here is assembled by us: a static PID 1 compiled for musl, wrapped in a cpio
//! archive we wrote, booted by our engine. No distribution, no base image, no busybox.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dome_core::MemorySize;
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;
use dome_image::initramfs::{Entry, Initramfs};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The static init, built with the musl target so the image needs no C library.
fn static_init() -> Option<PathBuf> {
    let path = workspace_root().join("target/x86_64-unknown-linux-musl/release/dome-init");
    path.is_file().then_some(path)
}

fn fixture_kernel() -> Option<PathBuf> {
    let path = workspace_root().join("tests/fixtures/kernel/vmlinuz-virt");
    path.is_file().then_some(path)
}

fn scratch(name: &str) -> PathBuf {
    let unique = format!("dome-foundation-boot-{name}-{}", std::process::id());
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn an_initramfs_we_build_contains_what_we_put_in_it() {
    let mut image = Initramfs::new();
    image.add(Entry::directory("dev")).expect("dir");
    image
        .add(Entry::executable("init", b"pretend binary".to_vec()))
        .expect("init");

    assert_eq!(image.len(), 2);
    assert!(image.contains("init"));
    assert!(image.contains("/init"), "a leading slash is tolerated");

    let cpio = image.to_cpio();
    // The kernel reads records until it sees this name, so it must be present and last.
    assert!(
        cpio.windows(11).any(|window| window == b"TRAILER!!!\0"),
        "the archive must end with a trailer"
    );
    assert_eq!(cpio.len() % 4, 0, "every record is four-byte aligned");
}

#[test]
fn an_escaping_path_is_refused() {
    let mut image = Initramfs::new();
    let error = image
        .add(Entry::file("../../etc/shadow", b"nope".to_vec()))
        .unwrap_err();

    assert!(error.to_string().contains("escapes"), "{error}");
}

#[test]
fn a_foundation_of_our_own_boots() {
    let Some(init) = static_init() else {
        eprintln!(
            "skipping: build it with cargo build --release --target x86_64-unknown-linux-musl -p dome-init"
        );
        return;
    };
    let Some(kernel) = fixture_kernel() else {
        return;
    };
    if !Path::new("/dev/kvm").exists() {
        return;
    }

    let init_bytes = std::fs::read(&init).expect("read the static init");

    let mut image = Initramfs::new();
    for directory in ["dev", "proc", "sys", "tmp"] {
        image.add(Entry::directory(directory)).expect("directory");
    }
    image
        .add(Entry::executable("init", init_bytes))
        .expect("init");

    let archive = image.to_gzip().expect("compress");

    let dir = scratch("boot");
    let initramfs = dir.join("initramfs.cpio.gz");
    std::fs::write(&initramfs, &archive).expect("write");

    let engine = LinuxEngine::open().expect("engine");
    let request = LaunchRequest::new(&kernel, MemorySize::from_mib(128).expect("128 MiB"))
        .with_initramfs(&initramfs);
    let mut request = request;
    request.timeout = Duration::from_secs(5);

    let outcome = engine.launch(&request).expect("boot");

    // The console is the proof: our binary ran as pid 1 inside the guest.
    assert!(
        outcome.console.contains("dome-init: starting as pid 1"),
        "our init did not run: {}",
        &outcome.console[outcome.console.len().saturating_sub(600)..]
    );
    assert!(
        outcome.console.contains("staying alive"),
        "the init did not reach its own code: {}",
        &outcome.console[outcome.console.len().saturating_sub(400)..]
    );

    let _ = std::fs::remove_dir_all(&dir);
}
