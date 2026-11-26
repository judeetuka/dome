//! Why does the fetcher die inside the chamber? Ask it from a shell.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dome_core::MemorySize;
use dome_engine::devices::ChamberIo;
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;
use dome_image::initramfs::{Entry, Initramfs};
use dome_supervisor::user_net::{SocketRelay, UserNetBackend};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "a diagnostic tool, not a passing test: it prints a boot and asserts nothing"]
fn the_fetcher_explains_itself_inside_the_chamber() {
    let root = workspace_root();
    let init = root.join("target/x86_64-unknown-linux-musl/release/dome-init");
    let fetcher = root.join("target/x86_64-unknown-linux-musl/release/dome-fetch");
    let home = std::env::var("HOME").unwrap_or_default();
    let bash = PathBuf::from(&home).join(".local/share/dome/shells/bash/sh");
    let Ok(kernel) = dome_cli::kernel::resolve(&dome_cli::kernel::kernel_root(), "dome-minimal")
    else {
        eprintln!("skipping: no kernel profile");
        return;
    };

    if !init.is_file() || !fetcher.is_file() || !bash.is_file() {
        eprintln!("skipping: missing init, fetcher or shell");
        return;
    }

    let mut image = Initramfs::new();
    for directory in ["dev", "proc", "sys", "tmp", "bin", "etc", "usr", "usr/bin"] {
        image.add(Entry::directory(directory)).expect("dir");
    }
    image
        .add(Entry::executable(
            "init",
            std::fs::read(&init).expect("init"),
        ))
        .expect("init");
    image
        .add(Entry::executable(
            "bin/bash",
            std::fs::read(&bash).expect("bash"),
        ))
        .expect("bash");
    image.add(Entry::symlink("bin/sh", "bash")).expect("sh");
    image
        .add(Entry::executable(
            "usr/bin/dome-fetch",
            std::fs::read(&fetcher).expect("fetch"),
        ))
        .expect("fetch");
    image
        .add(Entry::executable(
            "workload",
            std::fs::read(&bash).expect("bash"),
        ))
        .expect("workload");
    image
        .add(Entry::file(
            "etc/resolv.conf",
            b"nameserver 10.0.2.3\n".to_vec(),
        ))
        .expect("resolv");

    let archive = image.to_gzip().expect("gzip");
    let dir = std::env::temp_dir().join(format!("dome-diag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    let initramfs = dir.join("initramfs.cpio.gz");
    std::fs::write(&initramfs, archive).expect("write");

    let engine = LinuxEngine::open().expect("engine");
    let mut request = LaunchRequest::new(&kernel, MemorySize::from_mib(256).expect("256 MiB"))
        .with_initramfs(&initramfs);
    request.cmdline =
        "console=ttyS0 earlyprintk=ttyS0 panic=1 ip=10.0.2.15::10.0.2.2:255.255.255.0::eth0:off"
            .to_owned();
    request.timeout = Duration::from_secs(30);

    let io = ChamberIo::with_console_and_network(UserNetBackend::new(SocketRelay::permitting()));
    let outcome = engine.launch_with(&request, io).expect("boot");
    let console = outcome.console.clone();

    eprintln!("---- diagnostics ----");
    for line in console.lines().filter(|line| {
        let lower = line.to_lowercase();
        lower.contains("diag") || lower.contains("dome-fetch") || lower.contains("dome-init")
    }) {
        eprintln!("{line}");
    }
    eprintln!("---- raw tail ----");
    for line in console
        .lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .iter()
        .rev()
    {
        eprintln!("{line}");
    }
    eprintln!("---- end ----");

    let _ = std::fs::remove_dir_all(&dir);
}
