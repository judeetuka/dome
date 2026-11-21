# KVM spike (T59) — findings

**Status: SUCCESS.** A real Linux kernel boots on this host through KVM with no QEMU
process anywhere. Full console log: [`../evidence/task-59-console.log`](../evidence/task-59-console.log).

Host: bare metal (`systemd-detect-virt` = none), 14 cores, 15 GiB, uid 1000 in group `kvm`.
Guest: Alpine `vmlinuz-virt` 6.12.115, bzImage, boot protocol 2.15, 512 MiB RAM.
Source: `crates/dome-engine/examples/kvm_spike.rs`.

## The result

```text
[    0.000000] Linux version 6.12.115-0-virt (buildozer@build-3-21-x86_64) ...
[    0.000000] Command line: console=ttyS0 earlyprintk=ttyS0 keep_bootcon panic=1
[    0.000005] clocksource: kvm-clock: mask: 0xffffffffffffffff ...
[    0.019662] Allowing 1 present CPUs plus 0 hotplug CPUs
[    1.127540] Kernel panic - not syncing: VFS: Unable to mount root fs on "" or unknown-block(0,0)
```

The kernel initialises memory, timers, topology, clocksource and the console, then panics
because there is no root filesystem attached. **That is the correct end state for a boot
with no initramfs** — the boot path itself is proven. 13,588 bytes of console output,
whole run in ~12 s wall clock including a deliberate deadline wait.

## What it took, in order

Each step was a real failure with a real fix. This is the actual cost of "boot a kernel",
which is what M3 has to budget for.

| # | Symptom | Cause | Fix |
|---|---|---|---|
| 1 | `InvalidKernelStartAddress` | `BzImage::load` rejects `kernel_offset < highmem_start_address` | pass `None`, let the header decide |
| 2 | Guest halts after one exit, no output | **A vCPU with no CPUID applied cannot run a modern kernel** | `set_cpuid2(get_supported_cpuid(..))` |
| 3 | Runs, but silent forever | No e820 map, so the decompressor has no memory to plan with | write an e820 table into boot_params |
| 4 | Runs 3.3 s then dies in `exc_control_protection` | Pass-through CPUID advertises **CET** (shadow stack, IBT) which we do not implement | mask CET_SS and CET_IBT in leaf 7 |
| 5 | LAPIC accesses arrive as userspace MMIO | No in-kernel IRQ chip | `vm.create_irq_chip()` |
| 6 | (would have been) bytes silently lost | `KVM_EXIT_IO` **batches**; 16 sent, 15 received | honour `kvm_run.io.count` / `io.size` |
| 7 | (would have been) guest prints nothing even when running | UART without LSR/IIR emulation makes the kernel discard output | emulate LSR = `0x60`, IIR = `0x01` |
| 8 | Deadline never fires | `KVM_RUN` blocks indefinitely inside the kernel | run the vCPU on its own thread |

Steps 2, 4 and 5 are the expensive ones to discover, and none is obvious from the code.

## The lesson worth carrying into M3

**Pass-through CPUID is a lie unless you filter it.** Advertising a CPU feature to the
guest that the VMM does not actually implement produces a guest that faults in code paths
nobody expects, with a stack trace that points at the wrong subsystem. M3 needs an
explicit, reviewed allowlist of CPUID features — not `get_supported_cpuid` verbatim.

## Self-test gate

`kvm_spike selftest` runs a hand-written 32-bit stub that writes to the serial port and
halts. It passes in ~240 µs and fails loudly if vCPU bring-up regresses. It is a
regression guard for plumbing, **not** a substitute for the boot test.

## Impact on the plan's estimates

~300 lines of Rust now boots a kernel. Roughly two thirds of that was discovering the six
hazards above; the remaining third is the happy path, and none of the discovery cost
repeats. Revised shape for M3:

- **T63 (device substrate)** is partly de-risked: UART emulation and initramfs loading are
  understood. It still owns the reusable `DeviceSet` abstraction.
- **A new boot-protocol task** owns: zero page construction, e820, CPUID allowlist, and
  the protected-mode entry. Everything above is its specification.
- The "unhandled exit" arms in the spike (`MmioRead`, `MmioWrite`) are correct to leave
  unhandled for now; they disappear once the irqchip owns the LAPIC.

## Next steps

1. Attach an initramfs with a static `/init` so the boot reaches a shell prompt instead of
   panicking — that becomes the M3 acceptance test.
2. Replace the hard-coded CPUID mask with a reviewed feature allowlist.
3. Turn the example into `LinuxEngine::launch` (T16–T19) with the same behaviours, plus
   proper error types.
