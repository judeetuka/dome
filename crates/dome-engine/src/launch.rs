//! Bringing the pieces together: memory, kernel, zero page, vCPU, cage.
//!
//! The ordering here is not stylistic. It is forced by two measured facts from T20:
//! `unshare(CLONE_NEWUSER)` fails with EINVAL in a multithreaded process, and seccomp
//! filters are only inherited by threads created after they are installed. So everything
//! is prepared while single-threaded, the cage goes on, and only then may threads start.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dome_core::MemorySize;

use crate::EngineError;
use crate::boot::{PlacedInitrd, write_zero_page};
use crate::devices::ChamberIo;
use crate::linux::LinuxEngine;
use crate::loader::{MIN_KERNEL_ADDRESS, load_bzimage};
use crate::memory::GuestMemory;
use crate::sandbox;
use crate::vcpu::{Vcpu, VcpuEntry, VcpuSnapshot, VcpuStop, map_guest_memory};

/// Upper bound on device exits before a launch is abandoned.
///
/// Booting a kernel to a shell costs tens of thousands of single-byte serial exits, so this
/// is generous; it exists so a guest that spins on a device cannot hang the process.
const MAX_EXITS: u64 = 5_000_000;

/// What to boot.
#[derive(Debug, Clone)]
pub struct LaunchRequest {
    /// Path to a bzImage.
    pub kernel: PathBuf,
    /// Guest RAM.
    pub memory: MemorySize,
    /// Optional initramfs image.
    pub initramfs: Option<PathBuf>,
    /// Kernel command line.
    pub cmdline: String,
    /// Apply the process cage before running the guest.
    ///
    /// DEFAULT OFF, and that is a safety property rather than caution: entering a user
    /// namespace moves the *calling process* into it permanently. A library must not
    /// silently re-namespace its caller. Only a dedicated, disposable child process should
    /// set this.
    pub cage: bool,
    /// Wall-clock limit for the whole boot.
    pub timeout: Duration,
    /// Capture guest memory and CPU state when the run ends, for a seal.
    ///
    /// This is a **post-mortem** capture: the snapshot is taken after the run loop stops, so
    /// the CPU reflects the terminal state. A guest that halted cleanly is paused; one that
    /// triple-faulted is in its reset state, where CR0.PE is clear. Freezing a live guest
    /// mid-flight needs the sentinel (T33), and this flag does not pretend otherwise.
    pub capture_state: bool,
    /// Trap on denied syscalls instead of killing, so the number is reported.
    pub diagnose_cage: bool,
}

impl LaunchRequest {
    /// A minimal request for the given kernel and memory.
    pub fn new(kernel: impl Into<PathBuf>, memory: MemorySize) -> Self {
        Self {
            kernel: kernel.into(),
            memory,
            initramfs: None,
            cmdline: "console=ttyS0 earlyprintk=ttyS0 panic=1".to_owned(),
            cage: false,
            timeout: Duration::from_secs(60),
            diagnose_cage: false,
            capture_state: false,
        }
    }

    /// Attach an initramfs.
    pub fn with_initramfs(mut self, path: impl Into<PathBuf>) -> Self {
        self.initramfs = Some(path.into());
        self
    }

    /// Replace the command line.
    pub fn with_cmdline(mut self, cmdline: impl Into<String>) -> Self {
        self.cmdline = cmdline.into();
        self
    }

    /// Enable the cage. Only for a disposable child process.
    pub fn with_cage(mut self) -> Self {
        self.cage = true;
        self
    }
}

/// Guest state captured at the end of a run.
#[derive(Debug, Clone)]
pub struct CapturedState {
    /// The whole of guest RAM, as a seal would store it.
    pub memory: Vec<u8>,
    /// The vCPU's architectural state.
    pub registers: VcpuSnapshot,
}

/// How a launch ended.
#[derive(Debug, Clone)]
pub struct BootOutcome {
    /// Everything the guest wrote to the serial port.
    pub console: String,
    /// Device exits serviced.
    pub exits: u64,
    /// The guest halted or shut down on its own.
    pub stopped: bool,
    /// The wall-clock limit expired first.
    pub timed_out: bool,
    /// Captured state, present only when the request asked for it.
    pub captured: Option<CapturedState>,
}

impl LinuxEngine {
    /// Boot a kernel in a fresh VM and return what it printed.
    ///
    /// This is a synchronous path with a wall-clock limit, not a chamber lifecycle: it
    /// exists so the pieces can be proven to work together before M4 adds scheduling.
    pub fn launch(&self, request: &LaunchRequest) -> Result<BootOutcome, EngineError> {
        let vm = self
            .kvm()
            .create_vm()
            .map_err(|error| EngineError::Setup(format!("cannot create VM: {error}")))?;

        // Without an in-kernel irqchip the guest never receives a timer interrupt and
        // appears to hang with no output at all (spike finding).
        vm.create_irq_chip()
            .map_err(|error| EngineError::Setup(format!("cannot create irqchip: {error}")))?;

        let memory = GuestMemory::allocate(request.memory)?;
        map_guest_memory(&vm, &memory)?;

        let loaded = load_bzimage(&memory, &request.kernel, MIN_KERNEL_ADDRESS)?;
        let header = loaded
            .setup_header
            .ok_or_else(|| EngineError::Setup("kernel image carries no setup header".to_owned()))?;

        let initrd = match &request.initramfs {
            Some(path) => {
                let bytes = std::fs::read(path).map_err(|error| {
                    EngineError::Setup(format!("cannot read initramfs {}: {error}", path.display()))
                })?;

                if bytes.is_empty() {
                    return Err(EngineError::Setup(format!(
                        "initramfs {} is empty",
                        path.display()
                    )));
                }

                // Placement depends on the chamber's size: a fixed high address would put the
                // archive outside guest RAM in any chamber smaller than that address.
                let address = crate::boot::initrd_address(memory.size_bytes(), bytes.len())?;
                memory.write(address, &bytes)?;

                Some(PlacedInitrd {
                    address,
                    size: bytes.len(),
                })
            }
            None => None,
        };

        let zero_page = write_zero_page(
            &memory,
            header,
            &request.cmdline,
            initrd,
            MIN_KERNEL_ADDRESS,
        )?;

        let mut vcpu = Vcpu::create(
            self.kvm(),
            &vm,
            0,
            VcpuEntry {
                entry: loaded.entry.0,
                boot_params: zero_page.0,
            },
        )?;

        // The cage goes on after everything is prepared and before anything runs, so the
        // guest executes inside it rather than having it applied retroactively.
        if request.cage {
            // Order is fixed by sandbox and is not renegotiable here. The diagnostic variant
            // differs only in trapping instead of killing, so a missing allowlist entry can
            // report its own number.
            if request.diagnose_cage {
                sandbox::apply_engine_cage_diagnostic(Some(&request.kernel))?;
            } else {
                sandbox::apply_engine_cage(Some(&request.kernel))?;
            }
        }

        // The watch dog is started AFTER the cage: creating a thread before entering a user
        // namespace would make it impossible (EINVAL), so ordering is not negotiable here.
        install_alarm_handler()?;

        let finished = Arc::new(AtomicBool::new(false));
        let watchdog = start_watchdog(current_thread_id(), Arc::clone(&finished), request.timeout);

        // One object for both paths: a legacy port write and a memory-mapped write reach the
        // same devices, so the console cannot disagree with itself.
        let mut io = ChamberIo::with_console();
        let mut exits = 0u64;
        let mut stopped = false;
        let mut timed_out = false;

        while exits < MAX_EXITS {
            match vcpu.run_once(&mut io) {
                Ok(VcpuStop::Halted | VcpuStop::Shutdown) => {
                    stopped = true;
                    break;
                }
                Ok(VcpuStop::Continue) => {
                    // A write may have rung a doorbell; only this loop has the memory a device
                    // needs to act on it.
                    io.service_pending(&memory);
                }
                Err(EngineError::Launch(message)) if message.contains("Interrupted") => {
                    // KVM_RUN returned EINTR: the watch dog interrupted this exact thread.
                    timed_out = true;
                    break;
                }
                Err(error) => {
                    finished.store(true, Ordering::Relaxed);
                    return Err(error);
                }
            }

            exits += 1;
        }

        finished.store(true, Ordering::Relaxed);
        let _ = watchdog.join();

        // Capture after the loop: the guest is quiescent here, which is the coldest point a
        // synchronous launch can offer without a sentinel to freeze it.
        let captured = if request.capture_state {
            Some(CapturedState {
                memory: memory.capture()?,
                registers: vcpu.snapshot()?,
            })
        } else {
            None
        };

        Ok(BootOutcome {
            console: io.console_text(),
            exits,
            stopped,
            timed_out,
            captured,
        })
    }
}

/// Install a no-op handler so the watch dog's signal does not terminate the process.
///
/// The handler does nothing on purpose: the point is not the signal, it is that the thread
/// blocked in KVM_RUN returns EINTR and leaves the guest.
fn install_alarm_handler() -> Result<(), EngineError> {
    // SAFETY: sigaction writes only into a local, and the handler is a plain extern "C" fn.
    #[allow(unsafe_code)]
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_alarm as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);

        if libc::sigaction(libc::SIGALRM, &action, std::ptr::null_mut()) != 0 {
            return Err(EngineError::Setup(format!(
                "cannot install alarm handler: {}",
                std::io::Error::last_os_error()
            )));
        }
    }

    Ok(())
}

/// No-op signal handler.
extern "C" fn on_alarm(_signal: libc::c_int) {}

/// Identifier of the calling thread, for `pthread_kill`.
///
/// Must be `pthread_self()`: a `std::thread::Thread` is a pointer to an Arc, not a pthread
/// identifier, and reading it as one yields a value that `pthread_kill` rejects.
fn current_thread_id() -> libc::pthread_t {
    // SAFETY: pthread_self takes no arguments and always succeeds.
    #[allow(unsafe_code)]
    unsafe {
        libc::pthread_self()
    }
}

/// Interrupt `KVM_RUN` if the guest overruns its wall-clock limit.
///
/// Uses `pthread_kill` against the exact thread inside KVM_RUN rather than a signal sent to
/// the process: a process-directed signal is delivered to an arbitrary unblocked thread,
/// which is not the one blocked in the hypervisor, so the run never returns. Both the signal
/// design and KVM's `immediate_exit` were tried first and neither can reach a spinning guest
/// from a different thread.
fn start_watchdog(
    thread: libc::pthread_t,
    finished: Arc<AtomicBool>,
    limit: Duration,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let step = Duration::from_millis(50);
        let mut waited = Duration::ZERO;

        while waited < limit {
            if finished.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(step);
            waited += step;
        }

        if !finished.load(Ordering::Relaxed) {
            // SAFETY: the thread handle came from a live thread in this process.
            #[allow(unsafe_code)]
            unsafe {
                libc::pthread_kill(thread, libc::SIGALRM);
            }
        }
    })
}
