//! vCPU lifecycle: create, configure, and run with typed exits.
//!
//! ORDERING CONSTRAINT (measured in T20): a user namespace cannot be created by a
//! multithreaded process, and seccomp filters are inherited by threads created after they
//! are installed. So the sequence is fixed: prepare everything while single-threaded,
//! apply the cage, then start vCPU threads.

use kvm_bindings::{kvm_segment, kvm_sregs, kvm_userspace_memory_region};
use kvm_ioctls::{Kvm, VcpuFd, VmFd};

use crate::EngineError;
use crate::cpuid::engine_cpuid;
use crate::memory::GuestMemory;

/// Serial port base address, the classic 16550 COM1.
pub const SERIAL_PORT: u16 = 0x3F8;

/// Flat 4 GiB code segment selector.
const SELECTOR_CODE: u16 = 0x8;
/// Flat 4 GiB data segment selector.
const SELECTOR_DATA: u16 = 0x10;

/// How a vCPU is entered.
#[derive(Debug, Clone, Copy)]
pub struct VcpuEntry {
    /// Instruction pointer to start at.
    pub entry: u64,
    /// Value placed in RSI, which is the boot_params pointer for a Linux kernel.
    pub boot_params: u64,
}

/// Why `run_once` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuStop {
    /// The guest halted; stop running it.
    Halted,
    /// The guest shut down, usually a triple fault.
    Shutdown,
    /// The guest called for an exit that needs no action; keep running.
    Continue,
}

/// Host-side handling for the guest's device accesses.
///
/// The callbacks run while the vCPU's shared run struct is borrowed, which is the only
/// window in which an `in` can be answered.
pub trait VcpuIo {
    /// Guest read from a port.
    fn io_in(&mut self, port: u16, data: &mut [u8]);
    /// Guest write to a port.
    fn io_out(&mut self, port: u16, data: &[u8]);
    /// Guest read from memory-mapped IO.
    fn mmio_read(&mut self, _address: u64, data: &mut [u8]) {
        data.fill(0);
    }
    /// Guest write to memory-mapped IO.
    fn mmio_write(&mut self, _address: u64, _data: &[u8]) {}
}

/// The vCPU's architectural state at a moment in time.
///
/// Deliberately a small, explicit record rather than a raw `kvm_regs`: a seal has to stay
/// readable by a future version, and a struct laid out by the kernel's ABI is not a stable
/// format to put on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuSnapshot {
    /// Instruction pointer.
    pub rip: u64,
    /// Stack pointer.
    pub rsp: u64,
    /// Flags register.
    pub rflags: u64,
    /// Control register 0, including the protected-mode bit.
    pub cr0: u64,
    /// Control register 3, the page-table base.
    pub cr3: u64,
    /// Code segment selector.
    pub cs: u16,
    /// Stack segment selector.
    pub ss: u16,
}

/// A 16550 UART that captures what the guest writes.
///
/// The status registers matter more than the data register: a guest that polls the line
/// status register and never sees 'transmitter empty' silently discards every character.
/// That was measured in the spike and is why this device reports readiness.
#[derive(Debug, Default)]
pub struct SerialDevice {
    output: Vec<u8>,
}

impl SerialDevice {
    /// Bytes the guest has written so far.
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    /// Captured output as text, lossily.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

impl VcpuIo for SerialDevice {
    fn io_in(&mut self, port: u16, data: &mut [u8]) {
        data.fill(0);

        let value = match port {
            // Line status: transmitter holding register empty + transmitter empty.
            0x3FD => 0x60,
            // Interrupt identification: no interrupt pending.
            0x3FA => 0x01,
            // Modem status: CTS, DSR, DCD asserted.
            0x3FE => 0xB0,
            _ => 0x00,
        };

        if let Some(first) = data.first_mut() {
            *first = value;
        }
    }

    fn io_out(&mut self, port: u16, data: &[u8]) {
        if port == SERIAL_PORT {
            self.output.extend_from_slice(data);
        }
    }
}

/// One guest CPU.
pub struct Vcpu {
    fd: VcpuFd,
}

impl Vcpu {
    /// Create a vCPU and put it in 32-bit protected mode at `entry`.
    ///
    /// Paging is off and EFER is zero, which is the state the Linux 32-bit boot protocol
    /// expects. Nothing here writes to guest memory except through the maps passed in.
    pub fn create(kvm: &Kvm, vm: &VmFd, id: u8, entry: VcpuEntry) -> Result<Self, EngineError> {
        let fd = vm
            .create_vcpu(u64::from(id))
            .map_err(|error| EngineError::Setup(format!("cannot create vCPU {id}: {error}")))?;

        let cpuid = engine_cpuid(kvm)?;
        fd.set_cpuid2(&cpuid)
            .map_err(|error| EngineError::Setup(format!("cannot set CPUID: {error}")))?;

        let mut sregs: kvm_sregs = fd
            .get_sregs()
            .map_err(|error| EngineError::Setup(format!("cannot read sregs: {error}")))?;

        sregs.cs = flat_code(SELECTOR_CODE);
        sregs.ds = flat_data(SELECTOR_DATA);
        sregs.es = flat_data(SELECTOR_DATA);
        sregs.fs = flat_data(SELECTOR_DATA);
        sregs.gs = flat_data(SELECTOR_DATA);
        sregs.ss = flat_data(SELECTOR_DATA);
        sregs.cr0 |= 0x1; // PE: protected mode
        sregs.cr0 &= !0x8000_0000; // PG: paging off
        sregs.efer = 0;

        fd.set_sregs(&sregs)
            .map_err(|error| EngineError::Setup(format!("cannot set sregs: {error}")))?;

        let mut regs = fd
            .get_regs()
            .map_err(|error| EngineError::Setup(format!("cannot read regs: {error}")))?;

        regs.rip = entry.entry;
        regs.rsi = entry.boot_params;
        regs.rflags = 0x2; // interrupts disabled
        regs.rbp = 0;
        regs.rbx = 0;
        regs.rdi = 0;

        fd.set_regs(&regs)
            .map_err(|error| EngineError::Setup(format!("cannot set regs: {error}")))?;

        Ok(Self { fd })
    }

    /// Capture the architectural state without disturbing the guest.
    pub fn snapshot(&self) -> Result<VcpuSnapshot, EngineError> {
        let regs = self
            .fd
            .get_regs()
            .map_err(|error| EngineError::Setup(format!("cannot read registers: {error}")))?;
        let sregs = self.fd.get_sregs().map_err(|error| {
            EngineError::Setup(format!("cannot read segment registers: {error}"))
        })?;

        Ok(VcpuSnapshot {
            rip: regs.rip,
            rsp: regs.rsp,
            rflags: regs.rflags,
            cr0: sregs.cr0,
            cr3: sregs.cr3,
            cs: sregs.cs.selector,
            ss: sregs.ss.selector,
        })
    }

    /// Run until the guest touches a device, halts, or shuts down.
    pub fn run_once<H: VcpuIo>(&mut self, io: &mut H) -> Result<VcpuStop, EngineError> {
        let exit = self
            .fd
            .run()
            .map_err(|error| EngineError::Launch(format!("KVM_RUN failed: {error}")))?;

        Ok(match exit {
            kvm_ioctls::VcpuExit::IoIn(port, data) => {
                io.io_in(port, data);
                VcpuStop::Continue
            }
            kvm_ioctls::VcpuExit::IoOut(port, data) => {
                io.io_out(port, data);
                VcpuStop::Continue
            }
            kvm_ioctls::VcpuExit::MmioRead(address, data) => {
                io.mmio_read(address, data);
                VcpuStop::Continue
            }
            kvm_ioctls::VcpuExit::MmioWrite(address, data) => {
                io.mmio_write(address, data);
                VcpuStop::Continue
            }
            kvm_ioctls::VcpuExit::Hlt => VcpuStop::Halted,
            kvm_ioctls::VcpuExit::Shutdown => VcpuStop::Shutdown,
            _ => VcpuStop::Continue,
        })
    }
}

/// Map guest memory into a VM.
pub fn map_guest_memory(vm: &VmFd, memory: &GuestMemory) -> Result<(), EngineError> {
    let region = kvm_userspace_memory_region {
        slot: 0,
        guest_phys_addr: 0,
        memory_size: memory.size_bytes(),
        userspace_addr: memory.host_address()?,
        flags: 0,
    };

    // SAFETY: the region describes memory owned by `memory`, which outlives the VM in every
    // caller. This is the only unsafe site in the crate.
    #[allow(unsafe_code)]
    unsafe {
        vm.set_user_memory_region(region)
            .map_err(|error| EngineError::Setup(format!("cannot map guest memory: {error}")))?;
    }

    Ok(())
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
