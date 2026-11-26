//! What a kernel has to do to be usable by the engine.
//!
//! A newer kernel brings a new set of assumptions, and they are not in the version number: the
//! device has to be found, the driver has to bind, the network has to come up and init has to run.
//! Each of those is one assertion here, so "is this kernel supported" is a command rather than an
//! opinion, and a future kernel can be judged before anybody decides to ship it.
//!
//! Run it against any profile:
//!
//! ```text
//! DOME_KERNEL_PROFILE=dome-minimal cargo test -p dome-cli --test kernel_conformance
//! ```

use std::path::PathBuf;
use std::time::Duration;

use dome_core::MemorySize;
use dome_engine::devices::ChamberIo;
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;
use dome_image::initramfs::{Entry, Initramfs};
use dome_supervisor::user_net::{SocketRelay, UserNetBackend};

#[test]
fn a_candidate_kernel_meets_what_the_engine_needs() {
    if !std::path::Path::new("/dev/kvm").exists() {
        eprintln!("skipping: no /dev/kvm");
        return;
    }

    let profile =
        std::env::var("DOME_KERNEL_PROFILE").unwrap_or_else(|_| "dome-minimal".to_owned());
    let root = dome_cli::kernel::kernel_root();
    let kernel = match dome_cli::kernel::resolve(&root, &profile) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("skipping: no kernel profile {profile}: {error}");
            return;
        }
    };

    // The smallest userspace that proves the kernel handed over: our own static init.
    let init = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/x86_64-unknown-linux-musl/release/dome-init");
    if !init.is_file() {
        eprintln!(
            "skipping: build dome-init for musl first ({})",
            init.display()
        );
        return;
    }

    let mut image = Initramfs::new();
    for directory in ["dev", "proc", "sys", "tmp"] {
        image.add(Entry::directory(directory)).expect("dir");
    }
    image
        .add(Entry::executable(
            "init",
            std::fs::read(&init).expect("init"),
        ))
        .expect("init");

    let scratch = std::env::temp_dir().join(format!("dome-conformance-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("scratch");
    let initramfs = scratch.join("initramfs.cpio.gz");
    std::fs::write(&initramfs, image.to_gzip().expect("gzip")).expect("write");

    let mut request = LaunchRequest::new(&kernel, MemorySize::from_mib(256).expect("256 MiB"))
        .with_initramfs(&initramfs);
    request.cmdline =
        "console=ttyS0 earlyprintk=ttyS0 panic=1 ip=10.0.2.15::10.0.2.2:255.255.255.0::eth0:off"
            .to_owned();
    request.timeout = Duration::from_secs(20);

    let engine = LinuxEngine::open().expect("engine");
    let io = ChamberIo::with_console_and_network(UserNetBackend::new(SocketRelay::permitting()));
    let outcome = engine.launch_with(&request, io).expect("launch");
    let console = outcome.console.clone();

    // Each expectation is something a kernel change can break, so a failure names the part of
    // the contract that moved instead of saying the kernel does not work.
    assert!(
        console.contains("Registering device virtio-mmio"),
        "the kernel did not find the virtio device: {}",
        tail(&console)
    );
    assert!(
        console.contains("device=eth0") && console.contains("ipaddr=10.0.2.15"),
        "the kernel did not bring the network up: {}",
        tail(&console)
    );
    assert!(
        console.contains("dome-init"),
        "the kernel did not run init: {}",
        tail(&console)
    );
}

fn tail(console: &str) -> String {
    let lines: Vec<&str> = console.lines().collect();
    lines[lines.len().saturating_sub(8)..].join("\n")
}
