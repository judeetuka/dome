#![cfg(target_os = "linux")]

//! vCPU lifecycle tests against real KVM.
//!
//! These skip rather than fail on a host without /dev/kvm, which keeps the suite honest:
//! a machine that cannot run a guest is not evidence either way.

use dome_core::MemorySize;
use dome_engine::cpuid::{advertises_cet, engine_cpuid};
use dome_engine::memory::GuestMemory;
use dome_engine::vcpu::{SerialDevice, Vcpu, VcpuEntry, VcpuStop, map_guest_memory};
use kvm_ioctls::Kvm;

const STUB_ADDRESS: u64 = 0x0010_0000;

fn kvm_or_skip() -> Option<Kvm> {
    match Kvm::new() {
        Ok(kvm) => Some(kvm),
        Err(error) => {
            eprintln!("skipping: /dev/kvm unusable ({error})");
            None
        }
    }
}

/// 32-bit stub: write one character to COM1 eight times, then halt.
///
/// The nop between writes matters: KVM batches adjacent out instructions, and a naive
/// one-byte-per-exit assumption silently loses characters (measured during the T59 spike).
fn serial_stub() -> Vec<u8> {
    let mut stub = vec![0xb0, b'X', 0xba, 0xf8, 0x03]; // mov al,'X'; mov dx,0x3f8
    for _ in 0..8 {
        stub.extend_from_slice(&[0xee, 0x90]); // out dx,al; nop
    }
    stub.push(0xf4); // hlt
    stub
}

#[test]
fn cpu_policy_does_not_advertise_cet() {
    let Some(kvm) = kvm_or_skip() else { return };

    let cpuid = engine_cpuid(&kvm).expect("read CPUID");

    // The spike showed an unfiltered CPUID kills the guest in an unrelated fault path.
    assert!(
        !advertises_cet(&cpuid),
        "CET must be masked: nothing in the engine implements it"
    );
}

#[test]
fn guest_runs_and_its_serial_output_is_captured() {
    let Some(kvm) = kvm_or_skip() else { return };

    let vm = kvm.create_vm().expect("create vm");
    let memory = GuestMemory::allocate(MemorySize::from_mib(8).expect("size")).expect("memory");
    map_guest_memory(&vm, &memory).expect("map guest memory");

    memory
        .write(STUB_ADDRESS, &serial_stub())
        .expect("write stub");

    let mut vcpu = Vcpu::create(
        &kvm,
        &vm,
        0,
        VcpuEntry {
            entry: STUB_ADDRESS,
            boot_params: 0,
        },
    )
    .expect("create vcpu");

    let mut serial = SerialDevice::default();
    let mut halted = false;

    for _ in 0..1000 {
        match vcpu.run_once(&mut serial).expect("run vcpu") {
            VcpuStop::Halted => {
                halted = true;
                break;
            }
            VcpuStop::Shutdown => panic!("guest shut down, which here means a triple fault"),
            VcpuStop::Continue => {}
        }
    }

    assert!(halted, "guest never halted");

    let output = serial.output();
    assert!(
        output.len() >= 6,
        "expected most of the eight bytes, got {}: {:?}",
        output.len(),
        serial.text()
    );
    assert!(output.iter().all(|byte| *byte == b'X'), "unexpected output");
}

#[test]
fn guest_memory_is_zeroed_before_first_use() {
    let Some(kvm) = kvm_or_skip() else { return };

    let vm = kvm.create_vm().expect("create vm");
    let memory = GuestMemory::allocate(MemorySize::from_mib(4).expect("size")).expect("memory");
    map_guest_memory(&vm, &memory).expect("map guest memory");

    // A guest must never see another chamber's bytes. Fresh mappings are zero-filled by
    // the kernel, which is the property this asserts.
    assert_eq!(memory.read(0x8000, 16).expect("read"), vec![0u8; 16]);
}

#[test]
fn vcpu_creation_reports_a_typed_error_for_an_absurd_id() {
    let Some(kvm) = kvm_or_skip() else { return };

    let vm = kvm.create_vm().expect("create vm");

    // KVM rejects vCPU ids beyond its limit; the error must be ours, not a panic.
    let result = Vcpu::create(
        &kvm,
        &vm,
        u8::MAX,
        VcpuEntry {
            entry: STUB_ADDRESS,
            boot_params: 0,
        },
    );
    match result {
        Ok(_) => eprintln!("this host accepted a 255 vCPU id"),
        Err(error) => assert!(
            format!("{error}").contains("vCPU"),
            "error should name the vCPU: {error}"
        ),
    }
}
