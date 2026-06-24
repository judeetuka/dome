# Dome project rules

Dome is a no-QEMU, no-WSL sandbox engine written in Rust and assembly.

Linux is the first implementation target, not the product boundary. Each host OS must use its native engine path:

- Linux: direct KVM/rust-vmm-style APIs
- macOS: Apple hypervisor APIs
- Windows: Hyper-V/WHP APIs

Dome terminology:

- `foundation`: bootable base OS artifact
- `cell`: buildable sandbox blueprint
- `chamber`: running instance
- `perimeter`: network boundary
- `aperture`: explicit host resource mapping
- `seal`: saved chamber state
- `sentinel`: guest-side helper

Milestone 1 is read-only:

- detect host capabilities
- model cells, foundations, chambers, perimeters, seals, and apertures
- validate resource and port policies
- use TOML for configuration and structured diagnostic output
- avoid root helpers
- avoid privileged writes
- avoid actual hardware passthrough
- avoid claims that the guest cannot detect virtualization

Dome must not promise strong security until a threat model exists and the implementation has been reviewed against it.
