# ADR 0002 — Isolation model: maximum isolation in both directions

Status: **Accepted** (2026), revised after research. Supersedes the first draft.

Requirement from the product owner: *"very very isolated — any attack on the host should
have little or no effect on the container, and a runaway attack vector in the container
should have little or no effect on the host."*

This ADR answers it against the literature rather than intuition. Sources are cited inline
and listed at the end.

## What the research says

**1. The dominant lever is attack-surface minimisation, not the hypervisor itself.**

gVisor's security model defines the attack surface as *all* the interactions a workload has
with the rest of the system, and states two design principles: intercept the host API, and
**minimise the host API that the sandboxed component can reach**, specifically "to minimize
indirect exploitability, which is the exploitation by an exploited or buggy Sentry (e.g.
chaining an exploit)" (gVisor, *Security Model*). Applied to a VMM: the virtual hardware
*is* that API, so the device set is the escape surface.

**2. Production microVMs already embody this, and the device list is tiny.**

Firecracker's design exposes: VirtIO Net, VirtIO Block, a serial console, and a *partial*
i8042 used only so a guest can request a reboot — plus the PIC/IOAPIC/PIT that KVM itself
provides (Firecracker, *design.md*). No VGA, no IDE, no USB, no PCI hole. The NSDI'20 paper
frames the whole point as refusing the historical "strong security **or** low overhead"
trade (Firecracker, NSDI'20).

**3. Defence in depth assumes the virtualization boundary fails.**

Firecracker: "To assure defense in depth, Firecracker should only run constrained at the
process level... seccomp filters for disallowing unwanted system calls, cgroups and
namespaces for resource isolation, and dropping privileges by jailing the process"
(*design.md*). That is the same shape as our engine cage, and it exists because escapes
happen.

**4. A chamber is not isolated from availability attacks by default.**

Kata's threat model states it plainly: all KVM guests share the host kernel, so "one VM
could potentially impact the performance or stability of other VMs, including the
possibility of a Denial of Service attack" (Kata, *threat-model*). A runaway chamber that
starves the host is a failed isolation property even if it never escapes.

**5. The host→chamber direction has exactly one real answer: confidential computing.**

Linux's TDX documentation: Intel TDX "protect[s] confidential guest VMs **from the host**
and physical attacks by isolating the guest register state and by encrypting the guest
memory" (kernel, *tdx.rst*). AMD's SEV-SNP equivalent adds the RMP and per-page private/
shared selection (kernel, *amd-memory-encryption.rst*). Without one of these, guest RAM
sits in the VMM's address space and host root can read it. There is no userspace mitigation.

## Hardware reality on this machine (probed)

| Property | Value | Consequence |
|---|---|---|
| CPU | Intel, `vmx ept vpid pcid` | hardware virtualization with second-level translation |
| Confidential computing | **no SME/SEV/SNP/TDX flags, no devices** | host-root → chamber secrecy is **impossible here** |
| IOMMU groups | **0 exposed** | DMA-capable apertures must be refused, not merely discouraged |
| `ptrace_scope` / `dmesg_restrict` / unprivileged BPF | 1 / 1 / disabled | useful, but not ours to rely on |

## Decision: a two-tier isolation model

### Baseline tier — available on this hardware, and mandatory

**A. A pinned, minimal device set.** A chamber sees exactly:
UART (16550) · virtio-mmio transport · virtio-blk. Nothing else.
No VGA, no IDE/ATA, no USB, no i8042 unless reboot signalling is actually required, no PCI
hole devices, no SMM, no nested virtualization. **A new device is a security decision, not
a feature decision**, and requires a threat-model entry plus fuzz coverage before it ships.

**B. Deny-by-default CPUID.** Not passthrough. We learned this the expensive way: unfiltered
CPUID advertised CET and killed the guest inside an unrelated fault path (spike finding).
Firecracker solves it with CPU templates; Dome needs the equivalent allowlist.

**C. I/O rate limiting per device.** A malicious guest can make the single-threaded device
loop do unbounded work. Firecracker rate-limits VirtIO I/O; Dome must too, because this is a
host-availability attack that never touches the escape boundary.

**D. A cage around the engine process** (extends T20 beyond seccomp + Landlock):
own user+mount+pid+net namespace · minimal read-only filesystem view · no capabilities ·
`PR_SET_NO_NEW_PRIVS` · `PR_SET_DUMPABLE = 0` · `RLIMIT_CORE = 0` · Landlock path
restriction · seccomp allowlist derived from a measured trace.

**E. Host-availability caps** per chamber: `memory.max`, `pids.max`, `cpu.max` (ADR 0001),
plus a vCPU cap and a wall-clock/boot timeout. A chamber that hangs must be killable.

**F. No DMA-capable apertures.** With zero IOMMU groups, PCI/USB/GPU passthrough would let a
device DMA over every chamber's memory. Refused outright on this class of host; permitted
only after an IOMMU with enforced isolation appears, and then only behind the M7 threat
model.

**G. File apertures use virtio-fs, not 9p.** 9p is a legacy debug protocol; virtio-fs keeps
the host-side server in its own process with a smaller protocol surface. (This corrects the
earlier revision of this plan, which preferred 9p for simplicity.)

### Confidential tier — requires hardware this machine does not have

SEV-SNP or TDX with attestation and encrypted guest memory. Only in this tier may Dome
claim that the host cannot read a running chamber. The engine must **detect and report
unavailability**, never silently pretend. This is the same rule
`.rules/security.md:26` already states.

## What each direction can honestly claim

**Chamber → host (the direction we can engineer):** a chamber runs its own kernel on
hardware virtualization, sees a three-device machine, and if it escapes the VMM it lands in
a non-dumpable, namespaced, seccomp-filtered, unprivileged process with an empty filesystem
view and cgroup ceilings. This is roughly the rootless-microVM position and is the strongest
achievable configuration here.

**Host → chamber:** a malicious host root can read chamber memory on this CPU. Non-root
same-uid processes cannot (dumpable=0, core dumps off, `ptrace_scope=1`). Claim the latter,
never the former.

## Verification is part of the design

Each layer gets a test that fails if the layer is removed: dumpable/NoNewPrivs asserted from
inside the child; a denied syscall dying with SIGSYS; a Landlock write refused outside the
chamber directory; CET bits asserted clear in the CPUID handed to the vCPU; a missing
`/dev/kvm` returning `EngineError::DeviceUnavailable` rather than panicking; and device
parsers fuzzed (T56) rather than trusted.

## Sources

- Firecracker `docs/design.md` — <https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md>
- Agache et al., *Firecracker: Lightweight Virtualization for Serverless Applications*, NSDI'20 — <https://www.usenix.org/conference/nsdi20/presentation/agache>
- gVisor, *Security Model* — <https://gvisor.dev/docs/architecture_guide/security/>
- Kata Containers, *Threat Model* — <https://github.com/kata-containers/kata-containers/blob/main/docs/threat-model/threat-model.md>
- Linux kernel, *Intel Trust Domain Extensions (TDX)* — <https://docs.kernel.org/arch/x86/tdx.html>
- Linux kernel, *AMD Memory Encryption* — <https://docs.kernel.org/arch/x86/amd-memory-encryption.html>
