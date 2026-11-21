# Aperture rules

An aperture is an explicit mapping from a chamber to a host resource.

Supported aperture families:

- files and directories
- GPIO lines
- USB devices
- serial devices
- display endpoints
- GPU/PCI devices in future high-risk modes

Defaults:

- every aperture is disabled unless declared in TOML
- file apertures are read-only unless explicitly writable
- GPIO apertures map semantic pin operations, not raw SoC register writes
- host paths must be canonicalized before use
- `/proc`, `/sys`, broad `/dev`, private keys, and unrestricted block devices are denied by default

GPIO support is proxying, not board emulation. Dome may expose virtual GPIO devices backed by host GPIO brokers or libgpiod. Dome must not promise Raspberry Pi board emulation unless a real board-emulation layer exists.

## DMA-capable apertures

A DMA-capable aperture (PCI, USB, GPU, or anything that can initiate memory writes) is
refused unless the host enforces an IOMMU with per-device isolation. A DMA-capable device
can read every chamber's memory, which defeats the isolation the product exists to provide.

- No IOMMU groups exposed means no DMA aperture. Refuse with a typed error, do not degrade.
- Passthrough is never a convenience feature. It requires a threat-model entry, explicit
  opt-in, and a host that can actually contain it.
- Background: docs/adr/0002-isolation-model.md.
