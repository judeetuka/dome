# ADR 0004 — Two release models, and what each one is allowed to claim

Status: **Accepted** (2026). Rewritten after the requirement was stated as two distribution
models rather than one.

## The requirement

> "I can create a container and want to release it but protect my IP. Then I can release a seal,
> although my PC might not be able to run it — users need the right environment."
>
> "I can release a container that can run on any hardware too, but still have the highest level
> of security it can have, with or without TDX or the secure hardware."

Two models, both legitimate, with different contracts.

## Model A — sealed release (protecting IP)

| Act | Allowed |
| --- | --- |
| Create | any device |
| Export | any device |
| **Run** | **only hardware that keeps guest memory confidential** |

A publisher develops on whatever machine they have, seals the result, and ships it. The seal runs
only where the host cannot read guest memory (TDX, SEV-SNP, or a shielded VM), which is what makes
the IP protection real: the user's own machine never holds the decrypted image in a form its
operator can read.

**No override flag.** A device without confidential memory support cannot run it, and the error
names what is missing.

## Model B — portable release (widest reach)

| Act | Allowed |
| --- | --- |
| Create | any device |
| Export | any device |
| Run | any device, at the protection that device can provide |

Distributed broadly. On hardware with confidential memory it runs confidentially; anywhere else it
still gets the cage, the filtered CPUID and the minimal device model. `check_resumable` returns the
**effective** protection tier rather than a bare success, so the runtime reports what it actually
got instead of implying more.

## Why creation is unrestricted in both models

A seal's confidentiality cannot exceed the host that produced it: the plaintext lived in that
host's memory before encryption. Restricting creation would not make anything safer — it would only
stop a device recording state it already holds. Restricting *execution* is what protects the
promise, because running a release puts the plaintext back into a device's memory.

Creation is still recorded honestly. Both models carry, inside the signed payload:

- `distribution`: `sealed` or `portable` — where it may run
- `protection`: the tier of the device that captured it, `unprotected` or `confidential` with the
  mechanism named (`tdx`, `sev-snp`, `shielded-vm`)
- `degraded`: `true` when capture happened without protection, so an auditor sees the plaintext
  was exposed at capture time

## Measured platform facts

| Platform | Confidential guest memory | Requires |
| --- | --- | --- |
| Linux | Intel TDX, AMD SEV-SNP | CPU support and a host kernel that enables it |
| Windows | Shielded VM (vTPM, BitLocker-encrypted VM, HGS attestation) | a guarded fabric with a Host Guardian Service |
| macOS | **none exposed** | Virtualization.framework offers no guest memory confidentiality |

On the hardware available while this was written (an Intel box without TDX, a Ryzen Beelink
without SME/SEV), a sealed release can be **created but not run**. A portable release runs with
`unprotected` as its effective tier, and says so.

## Not claimed

The tier records **what the host reported**, not what attestation proved. A real guarantee needs a
TDX quote, an SEV-SNP report or HGS attestation verified against a known root, with the runner
refusing a self-reported tier. Until then the record is only as good as the machine that wrote it
— the same boundary ADR 0002 draws.
