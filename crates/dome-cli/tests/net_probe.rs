//! Does the kernel support the things a network needs? Asked through its own log.
//!
//! Two questions, both answered without a userspace stack: does the virtio network device probe,
//! and does the kernel configure an interface from the command line (CONFIG_IP_PNP)? If the second
//! is yes, guest addressing costs a command-line parameter rather than a netlink client in the
//! init.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dome_core::MemorySize;
use dome_engine::devices::ChamberIo;
use dome_engine::devices::virtio_net::{DEVICE_ID, NetworkBackend};
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;
use dome_image::initramfs::{Entry, Initramfs};

use std::sync::{Arc, Mutex};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A network that records what the guest sends and never answers.
#[derive(Default)]
struct Recorder {
    frames: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl NetworkBackend for Recorder {
    fn mac(&self) -> [u8; 6] {
        dome_net::nat::GUEST_MAC.0
    }

    fn transmit(&mut self, frame: &[u8]) {
        self.frames.lock().expect("lock").push(frame.to_vec());
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        None
    }
}

#[test]
#[ignore = "the kernel does not probe virtio-mmio on x86 without a virtio_mmio.device= parameter, and device-to-guest signalling needs the irqfd work in T76"]
fn the_kernel_probes_the_network_and_configures_the_address_from_the_command_line() {
    let root = workspace_root();
    let kernel = root.join("tests/fixtures/kernel/vmlinuz-virt");
    let init = root.join("target/x86_64-unknown-linux-musl/release/dome-init");

    if !kernel.is_file() || !init.is_file() || !Path::new("/dev/kvm").exists() {
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

    let archive = image.to_gzip().expect("gzip");
    let dir = std::env::temp_dir().join(format!("dome-netprobe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    let initramfs = dir.join("initramfs.cpio.gz");
    std::fs::write(&initramfs, archive).expect("write");

    let engine = LinuxEngine::open().expect("engine");
    let mut request = LaunchRequest::new(&kernel, MemorySize::from_mib(256).expect("256 MiB"))
        .with_initramfs(&initramfs);

    // The kernel reads this itself if it was built with CONFIG_IP_PNP.
    request.cmdline =
        "console=ttyS0 earlyprintk=ttyS0 panic=1 ip=10.0.2.15::10.0.2.2:255.255.255.0::eth0:off"
            .to_owned();
    request.timeout = Duration::from_secs(8);

    let recorder = Recorder::default();
    let frames = Arc::clone(&recorder.frames);
    let io = ChamberIo::with_console_and_network(recorder);

    let outcome = engine.launch_with(&request, io).expect("boot");
    let console = outcome.console.clone();

    eprintln!("---- network probe ----");
    for line in console.lines().filter(|line| {
        let lower = line.to_lowercase();
        lower.contains("virtio_net")
            || lower.contains("ip-config")
            || lower.contains("eth0")
            || lower.contains("net device")
            || lower.contains("virtio")
    }) {
        eprintln!("{line}");
    }
    eprintln!("frames from guest: {}", frames.lock().expect("lock").len());
    for frame in frames.lock().expect("lock").iter().take(3) {
        eprintln!("  frame {} bytes", frame.len());
    }
    eprintln!("device id: {}", DEVICE_ID);
    eprintln!("---- console tail ----");
    let tail: Vec<&str> = console.lines().rev().take(12).collect();
    for line in tail.iter().rev() {
        eprintln!("{line}");
    }
    eprintln!("---- end ----");

    // The boot itself must still succeed: a device that breaks the boot is worse than no device.
    assert!(
        console.contains("dome-init"),
        "the kernel booted and ran init"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
