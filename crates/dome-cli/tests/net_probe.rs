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
fn the_kernel_probes_the_network_and_configures_the_address_from_the_command_line() {
    let root = workspace_root();
    // The kernel Dome built, if it is installed; otherwise the fixture. A profile can be named so
    // a candidate kernel can be investigated without editing this test.
    let kernels = dome_cli::kernel::kernel_root();
    let profile =
        std::env::var("DOME_KERNEL_PROFILE").unwrap_or_else(|_| "dome-minimal".to_owned());
    let kernel = dome_cli::kernel::resolve(&kernels, &profile)
        .unwrap_or_else(|_| root.join("tests/fixtures/kernel/vmlinuz-virt"));
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
    // The command line is plain: no ip= parameter, so the only new thing in this boot is the
    // device itself.
    // Asking the kernel to configure the interface itself: with IP_PNP compiled in, this costs a
    // command-line parameter rather than a DHCP client in userspace.
    let extra = std::env::var("DOME_EXTRA_CMDLINE").unwrap_or_default();
    request.cmdline = format!(
        "console=ttyS0 earlyprintk=ttyS0 panic=1 ip=10.0.2.15::10.0.2.2:255.255.255.0::eth0:off {extra}"
    );
    // A working kernel reaches init in about a second, but a candidate with more logging needs
    // longer, and "it hung" and "it was still booting" look identical without this knob.
    let seconds = std::env::var("DOME_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    request.timeout = Duration::from_secs(seconds);

    let recorder = Recorder::default();
    let frames = Arc::clone(&recorder.frames);
    let io = ChamberIo::with_console_and_network(recorder);
    // A modern kernel assumes a real platform bus; the probe can ask for that behaviour.
    let io = if std::env::var("DOME_FLOATING_BUS").is_ok() {
        io.with_floating_bus()
    } else {
        io
    };

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
