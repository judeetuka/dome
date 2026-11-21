# KVM spike (T59) — findings

Status: **partial**. The KVM plumbing is proven; booting a real kernel is not yet
achieved. Everything below is measured on this host, not estimated.

Host: bare metal (`systemd-detect-virt` = none), 14 cores, 15 GiB, uid 1000 in group `kvm`.
Kernel under test: Alpine `vmlinuz-virt` 6.12.115, bzImage, boot protocol 2.15.
Spike source: `crates/dome-engine/examples/kvm_spike.rs`.

## What is proven to work

| Step | Evidence |
|---|---|
| Open `/dev/kvm` O_RDWR | `KVM_GET_API_VERSION = 12` |
| vCPU run-struct size | `KVM_GET_VCPU_MMAP_SIZE = 12288` |
| Create VM | `KVM_CREATE_VM` returns a live fd |
| Map guest memory | 512 MiB region at GPA 0 via `set_user_memory_region` |
| Apply CPUID | 57 entries from `get_supported_cpuid` |
| Load a real bzImage | `linux_loader::bzimage::BzImage::load`, kernel at 0x100000, `init_size` 39,604,224 |
| Enter 32-bit protected mode | flat segments, `CR0.PE`, `CR0.PG` clear, `EFER = 0` |
| Execute guest code | a hand-written 32-bit stub runs and halts in ~240 µs |
| Serial IO exits | 15 of 16 bytes reached the host through `KVM_EXIT_IO` on port 0x3F8 |
| HLT exit | `KVM_EXIT_HLT` delivered normally |

The self-test (`kvm_spike selftest`) is a hard gate: it fails if the vCPU cannot execute
a stub and write to the serial port. It passes.

## What does not work yet

**The kernel produces no console output and never exits.** With the Alpine bzImage loaded
at 0x100000 and entered per the 32-bit boot protocol, the guest runs for the full deadline
without emitting a byte and without an unhandled exit.

Observed along the way:

- Before CPUID was applied, the guest halted immediately after one exit. **A vCPU with no
  CPUID model applied cannot run a modern kernel.** Firecracker applies CPUID for exactly
  this reason.
- With CPUID applied, the guest stays resident and issues IO (PCI config reads on
  `0xCFC` were observed), so it is executing real instructions — just not reaching a
  console.
- A 16550 that does not report "transmitter holding register empty" on LSR (`0x3FD`)
  makes the kernel's early console silently drop every character. Emulating LSR = `0x60`
  and IIR = `0x01` is required before any output can appear.

Likely remaining causes, in order of suspicion:

1. **The legacy path may need the 16-bit real-mode entry**, not a direct 32-bit jump.
   Modern kernels set `xloadflags` bits for 64-bit and EFI handover entries; entering the
   32-bit `code32_start` directly is legal but less travelled, and something in the early
   handoff may be rejected silently.
2. **A boot_params field is missing or wrong.** `setup_header` is written, but e820
   entries, `type_of_loader`, and `hardware_subarch` are left zero. The kernel can panic
   on "no memory" before any console driver initialises.
3. **The compressed stub needs `%esi` to survive a mode switch** and needs the
   `boot_flag`/`header` magic verified at the right offset — worth asserting directly.

## Hazards this spike surfaced (each is real work in M3)

1. **`KVM_EXIT_IO` batches.** One exit may carry `io.count` bytes of the same port, and
   naive one-byte-per-exit handling *loses bytes* — measured: 16 sent, 15 received. The
   engine must honour `kvm_run.io.count` and `kvm_run.io.size`.
2. **A UART is not just an output port.** LSR/IIR emulation is mandatory or the guest
   silently discards output. This is device-model work, not plumbing.
3. **CPUID must be applied** before any kernel will run.
4. **`KVM_RUN` blocks indefinitely.** A wall-clock deadline checked between exits never
   fires on a guest that is happily running; the vCPU must run on its own thread (this
   spike does) or use `immediate_exit`.
5. **`BzImage::load` rejects a kernel_offset below `highmem_start_address`** — pass
   `None` and let the header decide.
6. **`unsafe` is genuinely required in `dome-engine`**: `set_user_memory_region` is an
   unsafe call. The Revision 2 section 3 override (`unsafe_code = "warn"`, deleting
   `[lints] workspace = true`) was necessary exactly as documented.

## Impact on the plan's estimates

Rev 1 priced "M3: boot minimal foundation" as part of a 7-task milestone. Measured reality:
roughly 250 lines of Rust produced a vCPU that executes code and handles serial IO, and
that is the *easy* half. Reaching a printed kernel boot additionally needs a correct
boot-protocol handoff (real-mode or EFI path), a boot_params with a usable memory map, and
a UART model — none of which is a one-line change.

Recommendation: keep M3 scope as "kernel console output through an emulated UART", but
budget it as **two** tasks: a boot-protocol task (proven painful above) and a device-model
task (UART + whatever the kernel demands next). Do not merge them.

## Next action

Shortest path to a first boot: try the **16-bit real-mode entry** (`jmp 0xF000:0x1000`
after loading, with `ds`/`es`/`si` per the boot protocol and A20 already on), instead of
jumping straight into 32-bit protected mode at `code32_start`. If that fails, build an
e820 map into the zero page before blaming the entry path.
