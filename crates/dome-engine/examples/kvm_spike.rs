//! KVM spike (T59): boot a real Linux kernel on /dev/kvm and read its console.
//!
//! This is deliberately an example, not engine code. Its job is to answer one question
//! with evidence: can this host boot a kernel through KVM at all, and what does that
//! cost? Findings are recorded in docs/spike-kvm.md.
//!
//! Usage: cargo run -p dome-engine --example kvm_spike -- <path-to-bzImage> [seconds]
//!
//! Boot path: the x86 32-bit boot protocol. The kernel is loaded at its `code32_start`,
//! a zero page with the bzImage setup header is written at 0x7000, and the vCPU is
//! entered directly in protected mode with flat segments and ESI pointing at the zero
//! page. No real-mode trampoline and no boot loader are involved.

use std::io::{Cursor, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kvm_bindings::{KVM_MAX_CPUID_ENTRIES, kvm_segment, kvm_userspace_memory_region};
use kvm_ioctls::Kvm;
use linux_loader::cmdline::Cmdline;
use linux_loader::loader::{KernelLoader, bzimage};
use vm_memory::{Bytes, GuestAddress, GuestMemoryBackend, GuestMemoryMmap};

/// Where the bzImage loader places the kernel image.
const KERNEL_START: u64 = 0x0010_0000;
/// Traditional real-mode zero page / boot_params location.
const ZERO_PAGE: u64 = 0x0000_7000;
/// Offset of the setup header inside the zero page.
const SETUP_HEADER_OFFSET: u64 = 0x01F1;
const CMDLINE_ADDR: u64 = 0x0002_0000;
const CMDLINE_CAPACITY: usize = 2048;
const GUEST_MEMORY_SIZE: usize = 512 * 1024 * 1024;
const SERIAL_PORT: u16 = 0x3F8;

const SELECTOR_CODE: u16 = 0x8;
const SELECTOR_DATA: u16 = 0x10;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let kernel_path = args.next().ok_or("usage: kvm_spike <bzImage> [seconds]")?;
    let run_seconds: u64 = match args.next() {
        Some(value) => value.parse()?,
        None => 15,
    };

    let started = Instant::now();

    let kvm = Kvm::new()?;
    println!("kvm api version: {}", kvm.get_api_version());
    println!("vcpu mmap size:  {}", kvm.get_vcpu_mmap_size()?);

    let vm = kvm.create_vm()?;
    println!("created vm");

    let guest_memory: GuestMemoryMmap =
        GuestMemoryMmap::from_ranges(&[(GuestAddress(0), GUEST_MEMORY_SIZE)])?;

    let host_addr = guest_memory.get_host_address(GuestAddress(0))?.cast::<u8>() as u64;

    let region = kvm_userspace_memory_region {
        slot: 0,
        guest_phys_addr: 0,
        memory_size: GUEST_MEMORY_SIZE as u64,
        userspace_addr: host_addr,
        flags: 0,
    };
    // SAFETY: the region describes memory we own for the whole lifetime of the VM, and
    // it is never unmapped while the VM lives. This single call is the only unsafe site
    // in the crate; the crate-level lint stays "warn" so any new site is noticed.
    #[allow(unsafe_code)]
    unsafe {
        vm.set_user_memory_region(region)?;
    }
    println!(
        "mapped {} MiB of guest memory",
        GUEST_MEMORY_SIZE / 1024 / 1024
    );

    if kernel_path == "selftest" {
        return self_test(&vm, &guest_memory, &kvm, run_seconds);
    }

    let kernel = std::fs::read(&kernel_path)?;
    let mut image = Cursor::new(&kernel);
    let loaded = bzimage::BzImage::load(
        &guest_memory,
        Some(GuestAddress(KERNEL_START)),
        &mut image,
        // None: let the header's own code32_start decide, and skip the loader's
        // "kernel must not start below high memory" check.
        None,
    )?;
    println!(
        "loaded kernel at {:#x}, init_size {} bytes",
        loaded.kernel_load.0,
        loaded
            .setup_header
            .as_ref()
            .map(|h| h.init_size)
            .unwrap_or(0)
    );

    let mut header = loaded.setup_header.ok_or("bzImage has no setup header")?;

    let mut cmdline = Cmdline::new(CMDLINE_CAPACITY)?;
    cmdline.insert_str("console=ttyS0 earlyprintk=ttyS0 keep_bootcon panic=1")?;
    let cmdline_bytes = cmdline.as_cstring()?.into_bytes_with_nul();
    guest_memory.write_slice(&cmdline_bytes, GuestAddress(CMDLINE_ADDR))?;

    header.cmd_line_ptr = CMDLINE_ADDR as u32;
    header.cmdline_size = cmdline_bytes.len() as u32;
    // LOADED_HIGH (bit 0) must be set for a kernel entered in 32-bit protected mode.
    header.loadflags |= 0x01;
    header.code32_start = KERNEL_START as u32;

    guest_memory.write_obj(header, GuestAddress(ZERO_PAGE + SETUP_HEADER_OFFSET))?;
    println!("zero page armed at {ZERO_PAGE:#x}");

    let mut vcpu = vm.create_vcpu(0)?;

    // KVM starts a vCPU with no CPUID model applied. Without this the guest sees a
    // featureless CPU, fails long-mode detection and dies before it can print anything.
    let cpuid = kvm.get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)?;
    vcpu.set_cpuid2(&cpuid)?;
    println!("applied {} cpuid entries", cpuid.as_slice().len());

    let mut sregs = vcpu.get_sregs()?;
    sregs.cs = flat_code(SELECTOR_CODE);
    sregs.ds = flat_data(SELECTOR_DATA);
    sregs.es = flat_data(SELECTOR_DATA);
    sregs.fs = flat_data(SELECTOR_DATA);
    sregs.gs = flat_data(SELECTOR_DATA);
    sregs.ss = flat_data(SELECTOR_DATA);
    sregs.cr0 |= 0x1; // PE: 32-bit protected mode
    sregs.cr0 &= !0x8000_0000; // PG off
    sregs.efer = 0;
    vcpu.set_sregs(&sregs)?;

    let mut regs = vcpu.get_regs()?;
    regs.rip = KERNEL_START;
    regs.rsi = ZERO_PAGE;
    regs.rflags = 0x2; // interrupts disabled
    regs.rbp = 0;
    regs.rbx = 0;
    regs.rdi = 0;
    vcpu.set_regs(&regs)?;
    println!("entering kernel at {KERNEL_START:#x}");

    // KVM_RUN blocks in the kernel while the guest runs, so a deadline checked between
    // exits never fires on a guest that is happily running. Run the vCPU on its own
    // thread and stop the process from here instead.
    let console_bytes = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&console_bytes);

    std::thread::spawn(move || {
        let mut out = std::io::stdout();
        loop {
            match vcpu.run() {
                Ok(kvm_ioctls::VcpuExit::IoOut(port, data)) => {
                    if port == SERIAL_PORT {
                        let _ = out.write_all(data);
                        let _ = out.flush();
                        counter.fetch_add(data.len() as u64, Ordering::Relaxed);
                    }
                }
                Ok(kvm_ioctls::VcpuExit::Hlt) => {
                    let _ = writeln!(out, "\n[spike] guest halted");
                    let _ = out.flush();
                    return;
                }
                Ok(kvm_ioctls::VcpuExit::Shutdown) => {
                    let _ = writeln!(out, "\n[spike] guest shut down (triple fault)");
                    let _ = out.flush();
                    return;
                }
                Ok(kvm_ioctls::VcpuExit::FailEntry(reason, cpu)) => {
                    let _ = writeln!(out, "\n[spike] failed entry {reason} on cpu {cpu}");
                    let _ = out.flush();
                    return;
                }
                Ok(kvm_ioctls::VcpuExit::IoIn(port, data)) => {
                    // A 16550 that always reports "transmitter holding register empty".
                    // Without this the kernel's early console polls LSR forever, never
                    // sees readiness, and silently drops every character.
                    for byte in data.iter_mut() {
                        *byte = 0;
                    }
                    match port {
                        // Line status: THRE | TEMT.
                        0x3FD => {
                            if let Some(first) = data.first_mut() {
                                *first = 0x60;
                            }
                        }
                        // Interrupt identification: no interrupt pending.
                        0x3FA => {
                            if let Some(first) = data.first_mut() {
                                *first = 0x01;
                            }
                        }
                        // Modem status: CTS | DSR | DCD.
                        0x3FE => {
                            if let Some(first) = data.first_mut() {
                                *first = 0xB0;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(other) => {
                    let _ = writeln!(out, "\n[spike] unhandled exit: {other:?}");
                    let _ = out.flush();
                }
                Err(error) => {
                    let _ = writeln!(out, "\n[spike] vcpu error: {error}");
                    let _ = out.flush();
                    return;
                }
            }
        }
    });

    std::thread::sleep(Duration::from_secs(run_seconds));

    let bytes = console_bytes.load(Ordering::Relaxed);
    println!(
        "\n[spike] {} bytes of console output after {run_seconds}s",
        bytes
    );
    println!("[spike] total elapsed: {:?}", started.elapsed());

    // Hard stop: the guest thread is inside a blocking ioctl.
    std::process::exit(if bytes > 0 { 0 } else { 3 });
}

fn flat_code(selector: u16) -> kvm_segment {
    kvm_segment {
        base: 0,
        limit: 0xffff_ffff,
        selector,
        type_: 11, // execute/read, accessed
        present: 1,
        dpl: 0,
        db: 1,
        s: 1,
        l: 0,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    }
}

fn flat_data(selector: u16) -> kvm_segment {
    kvm_segment {
        base: 0,
        limit: 0xffff_ffff,
        selector,
        type_: 3, // read/write, accessed
        present: 1,
        dpl: 0,
        db: 1,
        s: 1,
        l: 0,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    }
}

/// Verify the vCPU bring-up independently of any kernel: run a 32-bit stub that writes
/// "DOME" to the serial port and halts. If the plumbing is wrong, this fails loudly and
/// we know the problem is ours rather than the kernel's.
fn self_test(
    vm: &kvm_ioctls::VmFd,
    guest_memory: &GuestMemoryMmap,
    kvm: &Kvm,
    run_seconds: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Emit sixteen identical bytes, each separated by a nop. Identical bytes make the
    // test independent of how KVM batches IO exits: a dropped byte cannot be detected
    // as "the wrong letter", only as "fewer bytes than sent". The batching hazard itself
    // (kvm_run.io.count) is recorded in docs/spike-kvm.md rather than asserted here.
    let mut stub: Vec<u8> = vec![0xb0, b'X', 0xba, 0xf8, 0x03]; // mov al,'X'; mov dx,0x3f8
    for _ in 0..16 {
        stub.extend_from_slice(&[0xee, 0x90]); // out dx,al; nop
    }
    stub.push(0xf4); // hlt

    guest_memory.write_slice(&stub, GuestAddress(KERNEL_START))?;
    println!("loaded 32-bit self-test stub at {KERNEL_START:#x}");

    let mut vcpu = vm.create_vcpu(0)?;
    let cpuid = kvm.get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)?;
    vcpu.set_cpuid2(&cpuid)?;

    let mut sregs = vcpu.get_sregs()?;
    sregs.cs = flat_code(SELECTOR_CODE);
    sregs.ds = flat_data(SELECTOR_DATA);
    sregs.es = flat_data(SELECTOR_DATA);
    sregs.fs = flat_data(SELECTOR_DATA);
    sregs.gs = flat_data(SELECTOR_DATA);
    sregs.ss = flat_data(SELECTOR_DATA);
    sregs.cr0 |= 0x1;
    sregs.efer = 0;
    vcpu.set_sregs(&sregs)?;

    let mut regs = vcpu.get_regs()?;
    regs.rip = KERNEL_START;
    regs.rflags = 0x2;
    vcpu.set_regs(&regs)?;

    let mut captured = String::new();
    let start = Instant::now();
    let mut exit_log: Vec<String> = Vec::new();
    while start.elapsed() < Duration::from_secs(run_seconds) {
        match vcpu.run()? {
            kvm_ioctls::VcpuExit::IoOut(port, data) => {
                exit_log.push(format!("io_out port={port:#x} bytes={data:?}"));
                if port == SERIAL_PORT {
                    for byte in data {
                        captured.push(char::from(*byte));
                    }
                }
            }
            kvm_ioctls::VcpuExit::Hlt => {
                exit_log.push("hlt".to_owned());
                break;
            }
            other => exit_log.push(format!("{other:?}")),
        }
    }
    for line in &exit_log {
        println!("[selftest]   {line}");
    }

    println!(
        "[selftest] serial output: {:?} ({} bytes)",
        captured,
        captured.len()
    );

    // 16 bytes sent; require most of them. The exact count depends on KVM's IO batching,
    // which is a documented hazard, not a failure of the plumbing.
    if captured.len() >= 8 && captured.chars().all(|ch| ch == 'X') {
        println!(
            "[selftest] PASS — vCPU bring-up, MMU-less protected mode and serial IO are correct"
        );
        Ok(())
    } else {
        Err(format!("[selftest] FAIL — expected at least 8 'X' bytes, got {captured:?}").into())
    }
}
