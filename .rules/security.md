# Security rules

Default posture:

- deny inbound networking unless a port is explicitly mapped
- treat egress as an explicit policy
- keep passthrough disabled unless the user opts in
- isolate every chamber with its own native engine boundary and host resource limits
- treat the engine process as an attack surface that needs its own sandbox
- treat file, directory, GPIO, USB, serial, GPU, PCI, and display mappings as apertures

Milestone 1 restrictions:

- do not mutate `/dev`, `/sys`, `/proc`, nftables, cgroups, or kernel module state
- do not add setuid binaries
- do not add root-only setup scripts
- do not bind host devices to VFIO
- do not mount host paths into guests
- do not expose raw `/dev/mem`, broad `/dev`, unrestricted block devices, or privileged host paths

Security claims must be specific. "Operationally isolated" is acceptable when supported by code. "Undetectable" and "escape-proof" are not acceptable.

Dome must distinguish at-rest image secrecy from live runtime secrecy:

- Dome may claim at-rest confidentiality for locked `.dome` files when reviewed encryption is used.
- Dome must not claim runtime confidentiality against malicious host root in normal native-engine mode.
- Stronger seal modes require hardware-backed keys, measured boot, remote key release, attestation, or confidential-computing support.
- A custom file format is packaging. It must not replace standard, reviewed cryptography.
