# Seal security model

A seal (.seal.dome) is a chamber's state, captured and stored as one artifact. This document
states precisely what that protects, what it does not, and with which primitives. Claims here
are bounded deliberately; see `.rules/security.md` for the project-wide rule that forbids
"undetectable" and "escape-proof" language, and which applies equally to "secure".

## What a seal is

A suite-2 `.dome` container whose payload is a set of named sections:

| Section | Contents |
| --- | --- |
| `chamber` | chamber identifier |
| `created-unix` | capture time, seconds since the epoch |
| `foundation` | foundation the chamber was built from |
| `limits` | resource limits at capture time |
| `memory` | the guest's physical memory |
| `registers` | the vCPU's architectural state |

Metadata travels inside the same envelope as the memory, so one signature covers all of it.
Nothing about the seal is stored outside it, which is what makes it portable.

## What it protects

- **Confidentiality at rest, given the key.** Guest memory is encrypted with AES-256-GCM under
  a per-seal file key. Without that key the memory is not readable.
- **Integrity and authenticity.** The container is signed with Ed25519 **and** ML-DSA-65, and
  both must verify. A modified seal fails. Forgery requires breaking curves and lattices
  together.
- **Post-quantum key agreement.** The file key is wrapped with X25519 together with
  ML-KEM-768, combined per RFC 10024, then HKDF-SHA256. A recorded seal cannot be decrypted
  later merely by building a quantum computer that breaks X25519.

## What it does not protect

- **Runtime confidentiality.** A running chamber's memory is visible to a host root on
  hardware without SEV-SNP or TDX. The seal protects a file, not a live guest. This is the
  same boundary ADR 0002 draws.
- **A compromised key.** If the file key or a signing key is stolen, the seal is readable or
  forgeable. Key custody is out of scope for this crate.
- **Coercion, cold boot and physical access.** No claim is made about an attacker with the
  hardware in hand.
- **Side channels.** Timing, cache and power analysis are not addressed.
- **Traffic analysis of the seal itself.** Its size reveals roughly how much memory the
  chamber had.
- **Downstream behaviour.** A seal round-trips state faithfully; whether that state is safe to
  resume is a question about the guest, not about the format.

## Primitives and why

| Purpose | Primitive | Source | Why |
| --- | --- | --- | --- |
| Payload sealing | AES-256-GCM | aws-lc-rs | AEAD, standard, and Grover only halves 256 bits |
| Chunk hashes | SHA-256 | aws-lc-rs | binds a chunk to its position |
| Key wrapping | X25519 + ML-KEM-768, RFC 10024 | aws-lc-rs | hybrid: survives a break of either family |
| Key schedule | HKDF-SHA256 | aws-lc-rs | standard derivation, no invention |
| Signatures | Ed25519 + ML-DSA-65 | aws-lc-rs | both must verify; curves and lattices |
| Nonces | SystemRandom | aws-lc-rs | no counters, no time-based nonces |

No cipher, mode, KDF or signature scheme here is invented. The composition is recorded in
`docs/adr/0003-post-quantum-crypto.md`.

## Capturing state, honestly

The current capture is **post-mortem**: memory and registers are read after the run loop
stops, so a guest that halted cleanly is paused and one that triple-faulted is in its reset
state. Freezing a live guest mid-flight needs the sentinel, and this crate does not pretend
the flag does that.

## Known limits of the implementation

- Capture allocates the whole guest size in host memory, so sealing a large chamber costs that
  much RAM for the duration. Streaming to the file is the obvious next step.
- The seal records what the format knows; it does not yet record the kernel command line or
  the cell blueprint. Those arrive with export (T27).
