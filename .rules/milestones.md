# Milestones

M0: Correct the foundation

- no QEMU dependency
- no WSL dependency
- TOML blueprints
- dome-themed names
- platform-native engine model
- tests folder per crate

M1: Blueprint and core model

- parse and render TOML
- model foundations, cells, chambers, perimeters, seals, and apertures
- keep all behavior read-only

M2: Dome foundation format

- define `.foundation.dome`
- define encrypted manifest and chunk structure
- separate portable artifacts from engine cache formats

M3: Linux native engine prototype

- direct Linux-native engine path
- boot minimal foundation
- no QEMU process
- no hardware apertures yet

M4: Chamber lifecycle

- create, run, stop, seal, export
- strict memory, CPU, and storage limits
- explicit perimeters

M5: Minimal OS builder

- Buildroot-style minimal foundations
- Alpine-style developer foundations
- kernel profile fragments

M6: Sentinel and display

- sentinel protocol
- freeze/save hooks
- display path

M7: Apertures

- file and directory apertures
- GPIO broker aperture
- hardware aperture threat model

M8: macOS and Windows engines

- native macOS engine path
- native Windows engine path
- same-architecture native execution only

M9: ARM foundations

- ARM64 foundations
- Raspberry Pi OS userland support
- no full board-emulation promise
