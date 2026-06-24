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
