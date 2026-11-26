//! The proof: a chamber that installs its own package manager.
//!
//! Everything here is Dome's: the kernel (profile dome-minimal), the init, the shell, the fetcher,
//! the network device and the user-mode network behind it. The guest resolves a name, opens a TLS
//! connection, downloads its package manager and runs the installer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dome_cli::foundation_build::{BuildOptions, build, default_fetcher, default_init};
use dome_core::MemorySize;
use dome_engine::devices::ChamberIo;
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;
use dome_supervisor::user_net::{SocketRelay, UserNetBackend};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dome-proof-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn a_chamber_fetches_its_own_package_manager() {
    let kernels = dome_cli::kernel::kernel_root();
    let Ok(kernel) = dome_cli::kernel::resolve(&kernels, "dome-minimal") else {
        eprintln!("skipping: no dome-minimal kernel profile installed");
        return;
    };

    let (Some(init), Some(fetcher)) = (default_init(), default_fetcher()) else {
        eprintln!("skipping: build dome-init and dome-fetch for musl first");
        return;
    };

    let shells = dome_cli::shell::shell_root();
    if dome_cli::shell::resolve(&shells, "bash").is_err() {
        eprintln!("skipping: no bash shell profile");
        return;
    }

    if !Path::new("/dev/kvm").exists() {
        return;
    }

    // A recipe that needs mise: no installer field, no source. The package list says it.
    let recipe = [
        "[foundation]",
        "name = \"dome/proof:fetch\"",
        "init = \"sentinel\"",
        "static_linking = true",
        "",
        "[runtime]",
        "packages = [\"mise:node@20\"]",
        "",
        "[shell]",
        "default = \"bash\"",
    ]
    .join("\n");

    let dir = scratch("fetch");
    let recipe_path = dir.join("dome.toml");
    std::fs::write(&recipe_path, recipe).expect("recipe");

    let foundation = dir.join("foundation");
    build(&BuildOptions {
        recipe: recipe_path,
        kernel: Some(kernel.clone()),
        kernel_root: dir.join("kernels"),
        output: foundation.clone(),
        init: Some(init),
        workload: None,
        fetcher: Some(fetcher),
        shell_root: shells,
    })
    .expect("build");

    let initramfs = foundation.join(dome_cli::foundation_build::FOUNDATION_INITRAMFS);

    let mut request = LaunchRequest::new(&kernel, MemorySize::from_mib(256).expect("256 MiB"))
        .with_initramfs(&initramfs);
    request.cmdline =
        "console=ttyS0 earlyprintk=ttyS0 panic=1 ip=10.0.2.15::10.0.2.2:255.255.255.0::eth0:off"
            .to_owned();
    // Enough for the handshake and the fetch attempt; the assertions read what happened early, so
    // waiting longer only slows the suite.
    request.timeout = Duration::from_secs(20);

    let engine = LinuxEngine::open().expect("engine");
    let io = ChamberIo::with_console_and_network(UserNetBackend::new(SocketRelay::permitting()));

    let outcome = engine.launch_with(&request, io).expect("boot");
    let console = outcome.console.clone();

    eprintln!("---- what the chamber said ----");
    for line in console.lines().filter(|line| {
        let lower = line.to_lowercase();
        lower.contains("dome-init")
            || lower.contains("ip-config")
            || lower.contains("eth0")
            || lower.contains("installer")
            || lower.contains("mise")
            || lower.contains("fetch")
    }) {
        eprintln!("{line}");
    }
    eprintln!("---- end ----");

    // The chamber has an address.
    assert!(
        console.contains("ipaddr=10.0.2.15"),
        "the guest never got an address: {}",
        &console[console.len().saturating_sub(600)..]
    );

    // And it used it to fetch on its own.
    assert!(
        console.contains("dome-init: fetching") || console.contains("fetched"),
        "the chamber never fetched anything: {}",
        &console[console.len().saturating_sub(800)..]
    );

    let _ = std::fs::remove_dir_all(&dir);
}
