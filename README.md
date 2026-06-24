# Dome

Dome is a no-QEMU, no-WSL sandbox engine. Linux is the first implementation target, but the product shape is platform-native: Linux uses Linux-native engine APIs, macOS uses Apple-native engine APIs, and Windows uses Windows-native engine APIs.

The first milestone is intentionally conservative. It builds the Rust workspace, models the main concepts, and provides a read-only `doctor` command that reports host capability hints as TOML. It does not create chambers, change firewall rules, bind devices, or require root.

## Architecture baseline

```text
host Linux
  dome supervisor
    per-sandbox cgroup
    per-sandbox network namespace
    hardened engine process
      guest Linux kernel
        stripped distro/rootfs
        optional sentinel
```

A chamber should believe it owns its OS environment operationally: its own kernel, disk, process tree, network stack, hostname, and devices. It may still detect virtualization, so the project must not claim to be undetectable.

Dome uses these terms:

- `foundation`: bootable base OS artifact, stored as `.foundation.dome`
- `cell`: TOML blueprint for a chamber
- `chamber`: running instance
- `perimeter`: network boundary
- `aperture`: explicit host resource mapping, such as files or GPIO
- `seal`: saved chamber state, stored as `.seal.dome`
- `sentinel`: guest-side helper

## Workspace layout

```text
crates/
  dome-core/          shared models and validation
  dome-config/        TOML blueprint rendering and parsing
  dome-host-linux/    read-only Linux capability probes
  dome-cli/           command-line entry point
  dome-engine/        native engine traits and launch plans
  dome-image/         foundation manifests and seal/export model
  dome-net/           explicit perimeter policy model
  dome-plugin/        local and remote plugin contract
  dome-device/        hardware passthrough policy model
  dome-display/       display aperture model
  dome-supervisor/    lifecycle state model
  dome-guest-agent/   future sentinel integration contract
  dome-asm/           isolated assembly assets and ABI planning
```

## Current command

```sh
cargo run -p dome-cli -- doctor --format toml
```

The command reports host hints such as native engine availability, CPU virtualization flags, IOMMU groups, VFIO visibility, cgroups v2, seccomp, Landlock, network namespaces, and assembler availability.

## Foundation format

Dome's canonical artifacts are custom `.dome` files, not tar, zip, qcow2, or VHDX:

```text
alpine-3.21.foundation.dome
api-dev.seal.dome
api.cell.toml
```

Engine-specific disk formats may exist as internal cache files, but users should exchange Dome-native artifacts. The format may be custom; the cryptography must use reviewed primitives.

## Apertures

Every host mapping is explicit. File and directory mappings are read-only by default. GPIO support is proxying through a virtual GPIO or broker aperture, not full Raspberry Pi board emulation.

## Plugins

Dome plugins extend telemetry, policy decisions, enterprise defense, and audit export. They can run as local signed plugins or remote service plugins. Plugins must preserve host transparency: chamber identity, perimeter decisions, aperture access, sentinel status, license state, and plugin verdicts remain host-reportable.

## Verification

```sh
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Roadmap

1. Read-only host capability detection.
2. TOML cell blueprint validation.
3. Dome foundation and seal formats.
4. Native engine abstraction.
5. Linux native engine prototype.
6. Explicit perimeters and file/GPIO apertures.
7. Sentinel protocol for freeze/save/resize hooks.
8. Plugin contracts for telemetry, enterprise defense, and audit export.
9. Hardware apertures, guarded by an explicit threat model.
10. macOS and Windows native engines.
