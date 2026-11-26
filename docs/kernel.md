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

**Latest stable that boots, longterm as the fallback** - and the gap between those two is stated
rather than glossed.
| Line | Builds | Boots on our vCPU | Used now |
| --- | --- | --- | --- |
| 6.12.x (longterm) | yes, 7.34 MB | **yes** - device found, eth0 up, init runs | **yes** |
| 7.3.6 (latest stable) | yes, 11.69 MB | no - stalls before init | no |

The 7.3.6 build stops at the same place every time, right after `kprobes: kprobe
jump-optimization is enabled`, with no panic and no further output. `ibt=off` does not change it,
so CET is not the cause. The two candidates worth testing next are `CONFIG_SMP=n` (which the
6.12 build tolerated and a 7.x kernel may not) and a newer initcall that needs something the
trimmed config drops. Until one of them is proven, the profile ships the kernel that boots -
because a version number is not worth a chamber that cannot start.

Both are built by the same command, so the comparison is one flag:

`dome kernel build --version 7.3.6 --profile dome-test`
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