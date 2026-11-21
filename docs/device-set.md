# The device set

Every Dome engine implements the same devices, so one foundation boots unchanged on Linux,
macOS and Windows. This document is the contract; T43–T46 reference it.

## Why the set is tiny

A device is an attack surface with a guest on the other side of it. Firecracker's design note
makes the argument in full: a minimal device model is a security boundary, not a shortcut. Dome
pins three:

| Device | Module | Window |
| --- | --- | --- |
| 16550 UART | `devices/uart16550.rs` | `0xc000_0000`, 4 KiB; ports `0x3f8`–`0x3ff` |
| virtio-mmio transport | `devices/virtio_mmio.rs` | `0xd000_0000` + `0x1000` × 8 |
| initramfs loader | `boot.rs` | writes into guest RAM, fills boot params |

There is no BIOS, no PCI hole, no VGA, no PS/2, no ACPI table beyond what the kernel needs, and no
legacy floppy. A guest cannot ask for a device that is not in this table.

## The traits

```rust
pub trait Device {
    fn name(&self) -> &'static str;
    fn mmio_range(&self) -> (u64, u64);
    fn mmio_read(&mut self, offset: u64, data: &mut [u8]);
    fn mmio_write(&mut self, offset: u64, data: &[u8]);
}

pub trait VirtioBackend {
    fn device_id(&self) -> u32;
    fn device_features(&self) -> u64;
    fn read_config(&self, offset: u64, data: &mut [u8]);
    fn write_config(&mut self, offset: u64, data: &[u8]);
    fn notify(&mut self, queue: u16) -> bool;
}
```

**No platform types appear in either.** A `Device` is a register file plus a memory view, which
is exactly why the same foundation can boot on three different hypervisor APIs. Nothing here
mentions KVM, Hypervisor.framework or WHP.

`DeviceSet` routes guest accesses by address and reports whether a device claimed them, so the
engine's exit handler stays a dozen lines.

## virtio-mmio, not virtio-pci

The MMIO transport is a flat register window: no configuration space, no BARs, no bus scan, no
capability list. Every microVM picks it for that reason. The transport implements the
specification's status handshake, feature negotiation and queue setup, and refuses a driver that
asks for a feature the device did not offer — the specification requires refusing rather than
guessing, and a test pins it.

Queue sizes are clamped to what the device accepts rather than silently differing, and writing
zero to the status register resets the device completely.

## What this costs, measured

Measured on this host, not estimated:

| Artifact | Size |
| --- | --- |
| `vmlinuz-virt` (Alpine virtual kernel) | 11.7 MB |
| `initramfs.cpio.gz` (test fixture) | 356 KB |
| **bootable chamber base** | **≈ 12 MB** |
| `postgres:16-alpine` container image | 420 MB |
| `alpine` container image | ≈ 8 MB |

Two conclusions, one of them uncomfortable:

1. **Against a fat image we are far lighter.** 12 MB against 420 MB is not a near miss.
2. **Against a thin image we cannot be lighter on disk, and should not pretend otherwise.** A
   container image omits the kernel because it borrows the host's. A chamber carries its own,
   which is the whole isolation story. For the same Alpine userland, expect roughly 12 MB of
   kernel plus ~4 MB of root filesystem against Docker's ~8 MB.

Where the total *system* footprint is genuinely smaller:

- **No daemon.** There is no dockerd or containerd: no idle RSS, no image store to maintain, no
  background health loop per container.
- **One foundation on disk, many chambers.** Copy-on-write and reflink mean N chambers cost one
  foundation plus their deltas (T64). Docker does this with layers; we do it with reflinks.
- **Shared read-only pages.** A read-only, compressed root filesystem (EROFS or squashfs) mapped
  through the page cache lets every chamber from a foundation share those pages, the same way
  containers share layers.
- **No layered filesystem metadata.** No overlayfs upper/lower bookkeeping per container.

The honest summary: **lighter in system footprint, not lighter per image.** Anyone claiming a VM
image beats a container image of the same userland is either not counting the kernel or is
comparing a fat image to a thin one.
