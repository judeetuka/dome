//! Boot the fixture kernel and print what the guest said.
//!
//! A diagnostic binary rather than a test: when a boot misbehaves the console output is the
//! only evidence, and the test harness hides it behind a pass/fail.

use std::time::Duration;

use dome_core::MemorySize;
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = LinuxEngine::open().map_err(|error| format!("no engine: {error}"))?;

    let kernel = std::path::Path::new("tests/fixtures/kernel/vmlinuz-virt");
    let mut request = LaunchRequest::new(kernel, MemorySize::from_mib(512)?);
    request.timeout = Duration::from_secs(20);

    let initramfs = std::path::Path::new("tests/fixtures/kernel/initramfs.cpio.gz");
    if initramfs.is_file() && std::env::args().any(|arg| arg == "--initrd") {
        request.initramfs = Some(initramfs.to_path_buf());
    }

    println!("booting {} ...", kernel.display());
    let started = std::time::Instant::now();
    let outcome = engine.launch(&request)?;

    println!(
        "elapsed {:?}, exits {}, stopped {}, timed_out {}",
        started.elapsed(),
        outcome.exits,
        outcome.stopped,
        outcome.timed_out
    );
    println!("--- console ({} bytes) ---", outcome.console.len());
    println!("{}", outcome.console);
    println!("--- end ---");

    Ok(())
}
