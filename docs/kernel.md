# The kernel

Dome runs **Linux**. Not a bespoke kernel, not a unikernel, not a microkernel: the guest is Linux,
and what Dome owns is the machine around it - the VMM, the device set, the image format, and the
kernel's configuration.

That last one is the part that matters most and looks like the least. A distribution kernel is
built for a machine whose hardware you do not know, so it carries drivers for everything, most of
them as modules to load later. A foundation knows exactly what its chamber has: a virtio-mmio
transport, a network card, a serial console, and a RAM disk. Everything else is attack surface
compiled in and never used.

## Rules

1. **Upstream, unpatched.** A patch is a thing to carry forward, re-apply and re-justify at every
   version. Nothing Dome needs is outside upstream Linux.
2. **No modules.** A module is a file to place, a version to match, and a load step that fails at
   boot instead of at build time. Everything the device set needs is compiled in.
3. **Nothing unused.** No filesystems a foundation cannot mount, no drivers for hardware it cannot
   have, no debugging machinery in a release.
4. **The config is a checked-in fragment**, not a `defconfig` plus tribal knowledge. A kernel that
   differs because someone ran `menuconfig` is a kernel nobody can reproduce.

## What must be built in, and why

Every row came from something that failed during M5 rather than from a specification.

| Option | Why | What went wrong without it |
| --- | --- | --- |
| `CONFIG_VIRTIO_MMIO=y` | the transport every device hangs off | the window is never probed |
| `CONFIG_VIRTIO_NET=y` | the network card | the Alpine fixture ships it as a **module**: device registered, no driver, no eth0 |
| `CONFIG_IP_PNP=y` | static addressing from `ip=` | addressing needs a DHCP client in userspace |
| `CONFIG_DEVTMPFS=y` + `_MOUNT=y` | `/dev` at boot | no `/dev/console`, so init's output goes to `/dev/null` and the boot looks silent |
| `CONFIG_BINFMT_SCRIPT=y` | a `#!` script as init or workload | the kernel refuses to execute it |
| `CONFIG_SERIAL_8250_CONSOLE=y` | the console exists at all | nothing is printed, ever |

## What is deliberately absent

- **Modules** (`CONFIG_MODULES` off): the image cannot load anything, so there is nothing to
  smuggle in and nothing to match against a version.
- **The distribution's keyring**: a foundation trusts what its seal says, not what a distro signed.
- **Filesystems a chamber cannot use**: each one is a parser reachable from guest-controlled bytes.

## Which version

**Latest stable.** 7.3.6, built here, boots a chamber: the device registers, eth0 comes up at
10.0.2.15, and the guest sends ARP for its gateway. No distribution kernel, no modules.

| Line | Boots | Notes |
| --- | --- | --- |
| 7.3.6 (latest stable) | **yes** | the default profile |
| 6.12.119 (longterm) | yes | offered with `--version` for a fixed line |

### The one line that decides whether a kernel boots

`# CONFIG_SMP is not set`

With SMP on, a kernel enumerates CPUs from ACPI or MP tables. The engine provides neither yet, so
the guest never learns its topology and **hangs at the first wait on a kernel thread**. That is
what looked like a version incompatibility: initcall_debug stops at `oom_init`, the vCPU spins
with no device access, and a newer kernel logs enough on the way that it looked like a slow boot
rather than a stuck one.

Two earlier findings were real prerequisites and are fixed: an unowned I/O port returned zero,
which a driver reads as "device present, nothing to report", and there was no CMOS RTC for a kernel
to read the wall clock from.

Until the engine can describe an APIC and a CPU topology to the guest, one CPU is the honest
configuration, and it is stated in the fragment rather than left to a default.
## A kernel from the future

The engine does not know about kernel versions, and it should not: it knows what a guest has to
do to be usable here, and every one of those is a line in a test.

```text
DOME_KERNEL_PROFILE=dome-minimal cargo test -p dome-cli --test kernel_conformance
```

It boots the profile and checks the contract: the virtio device is found, the network comes up at
10.0.2.15, and init runs. A future kernel either passes or names the part that moved, so trying a
new version is a command rather than a support promise. Nothing in the engine encodes a version
number.

Two engine gaps were found by exactly this exercise and are now fixed rather than documented as
requirements: an unowned I/O port has to float high, and a CMOS RTC has to exist. Those are
properties of a PC, not of a kernel version, which is why fixing them removed a class of future
breakage instead of adding a compatibility note.

### What still has to be asked for by hand

One thing is not a version problem and is stated in the fragment: `# CONFIG_SMP is not set`. A
SMP kernel asks ACPI or an MP table which CPUs exist, and the engine describes neither. Until it
does, a kernel built with SMP hangs, whatever its version.

## Reproducing a build

One command does the whole thing: fetch, configure from the checked-in fragment, build, install as
a profile.

```
dome kernel build --version 7.3.6 --profile dome-minimal
```

The long form, for anyone who wants to see the steps:

```
cd linux-6.12
make defconfig
./scripts/config --enable VIRTIO_MMIO --enable VIRTIO_NET --enable IP_PNP \
  --enable DEVTMPFS --enable DEVTMPFS_MOUNT --enable BINFMT_SCRIPT \
  --enable SERIAL_8250_CONSOLE
make olddefconfig && make -j$(nproc) bzImage
```

The result is installed as a kernel profile, so no user ever passes a path:

```
dome kernel import --profile dome-minimal --from arch/x86/boot/bzImage
```