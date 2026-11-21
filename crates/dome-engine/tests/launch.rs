#![cfg(target_os = "linux")]

//! End-to-end launch tests: real kernel, real KVM, real console output.
//!
//! The cage is off here by design: entering a user namespace would move the test harness
//! into it and corrupt every later test in the same process. Cage behaviour is proven
//! separately in tests/sandbox.rs through a disposable child.

use std::time::Duration;

use dome_core::MemorySize;
use dome_engine::EngineError;
use dome_engine::launch::LaunchRequest;
use dome_engine::linux::LinuxEngine;
use dome_test_utils::test_fixture_path;

fn engine_or_skip() -> Option<LinuxEngine> {
    match LinuxEngine::open() {
        Ok(engine) => Some(engine),
        Err(error) => {
            eprintln!("skipping: {error}");
            None
        }
    }
}

fn kernel_or_skip() -> Option<std::path::PathBuf> {
    let path = test_fixture_path("kernel/vmlinuz-virt");
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("skipping: kernel fixture missing");
        None
    }
}

fn request_for(kernel: std::path::PathBuf) -> LaunchRequest {
    let mut request = LaunchRequest::new(kernel, MemorySize::from_mib(512).expect("512 MiB"));
    // Ten seconds is ample: this kernel reaches a panic or a halt in about one.
    request.timeout = Duration::from_secs(10);
    request
}

#[test]
fn boots_a_real_kernel_and_reports_its_memory_map() {
    let Some(engine) = engine_or_skip() else {
        return;
    };
    let Some(kernel) = kernel_or_skip() else {
        return;
    };

    let outcome = engine
        .launch(&request_for(kernel))
        .expect("launch should succeed");
    let console = &outcome.console;

    assert!(outcome.exits > 0, "guest produced no device exits at all");
    assert!(
        console.contains("Linux version"),
        "kernel never announced itself. console was:\n{console}"
    );
    // Without an e820 map the decompressor emits nothing whatever. This assertion is the
    // one that would have caught the spike's most deceptive failure.
    assert!(
        console.contains("BIOS-e820"),
        "no memory map reported. console was:\n{console}"
    );

    // No root filesystem is supplied here, so the kernel is *expected* to panic and reboot
    // forever. Asserting a clean stop would be asserting a bug.
    assert!(
        console.contains("Kernel panic") || console.contains("Unable to mount root"),
        "expected a panic about the missing root. console was:\n{console}"
    );
}

#[test]
fn boots_the_initramfs_and_reaches_our_userspace_init() {
    let Some(engine) = engine_or_skip() else {
        return;
    };
    let Some(kernel) = kernel_or_skip() else {
        return;
    };

    let initramfs = test_fixture_path("kernel/initramfs.cpio.gz");
    if !initramfs.is_file() {
        eprintln!("skipping: initramfs fixture missing");
        return;
    }

    let outcome = engine
        .launch(&request_for(kernel).with_initramfs(initramfs))
        .expect("launch should succeed");
    let console = &outcome.console;

    // Each of these proves one link in the chain: the initrd was found (zero page), it was
    // unpacked, userspace was entered, and our own binary ran and wrote to the port.
    assert!(
        console.contains("Unpacking initramfs"),
        "initramfs never unpacked; type_of_loader is probably wrong. console:\n{console}"
    );
    assert!(
        console.contains("Run /init as init process"),
        "never reached userspace. console was:\n{console}"
    );
    assert!(
        console.contains("dome-init: direct serial write from userspace"),
        "our init did not run. console was:\n{console}"
    );
}

#[test]
fn a_guest_that_never_stops_is_terminated_by_the_time_limit() {
    let Some(engine) = engine_or_skip() else {
        return;
    };
    let Some(kernel) = kernel_or_skip() else {
        return;
    };

    // Without a root filesystem the guest reboots forever. It does not reliably reach a HLT
    // exit, so the wall-clock limit is the only terminator that can be depended on.
    let mut request = request_for(kernel);
    request.timeout = Duration::from_secs(2);

    let started = std::time::Instant::now();
    let outcome = engine.launch(&request).expect("launch should succeed");
    let elapsed = started.elapsed();

    assert!(outcome.timed_out, "the limit should have fired");
    assert!(
        elapsed < Duration::from_secs(20),
        "the limit took {elapsed:?} to fire, so the watch dog is not working"
    );
}

#[test]
fn too_little_memory_is_a_typed_error() {
    let Some(engine) = engine_or_skip() else {
        return;
    };
    let Some(kernel) = kernel_or_skip() else {
        return;
    };

    let mut request = LaunchRequest::new(kernel, MemorySize::from_mib(16).expect("16 MiB"));
    request.cage = false;

    let error = engine.launch(&request).unwrap_err();
    assert!(matches!(error, EngineError::Memory(_)), "got {error:?}");
}

#[test]
fn a_missing_kernel_is_a_typed_error() {
    let Some(engine) = engine_or_skip() else {
        return;
    };

    let request = LaunchRequest::new(
        "/nonexistent/vmlinuz",
        MemorySize::from_mib(256).expect("256 MiB"),
    );
    let error = engine.launch(&request).unwrap_err();

    assert!(matches!(error, EngineError::Setup(_)), "got {error:?}");
}

#[test]
fn captures_guest_state_when_asked() {
    let Some(engine) = engine_or_skip() else {
        return;
    };
    let Some(kernel) = kernel_or_skip() else {
        return;
    };

    let mut request = request_for(kernel);
    request.timeout = Duration::from_secs(3);
    request.capture_state = true;

    let outcome = engine.launch(&request).expect("launch");
    let captured = outcome.captured.expect("state should have been captured");

    // The whole of guest RAM is captured, at the size the request asked for.
    assert_eq!(captured.memory.len(), 512 * 1024 * 1024);
    assert!(
        captured.memory.iter().any(|byte| *byte != 0),
        "captured memory is empty"
    );

    // The CPU state is captured, but this is a post-mortem snapshot: the guest above ran
    // without a root filesystem, panicked and reset, so CR0.PE is legitimately clear. What can
    // be asserted is that the registers are real values from the kernel's view of the vCPU.
    assert_eq!(
        captured.registers.cr3 & 0xFFF,
        0,
        "CR3 always has zero low bits"
    );
    assert_ne!(
        captured.registers.rflags, 0,
        "the flags register was never captured"
    );
}

#[test]
fn capture_is_off_unless_requested() {
    let Some(engine) = engine_or_skip() else {
        return;
    };
    let Some(kernel) = kernel_or_skip() else {
        return;
    };

    let mut request = request_for(kernel);
    request.timeout = Duration::from_secs(2);

    let outcome = engine.launch(&request).expect("launch");
    assert!(
        outcome.captured.is_none(),
        "capture must not happen by default"
    );
}
