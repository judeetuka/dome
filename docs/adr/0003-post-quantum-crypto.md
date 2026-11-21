# ADR 0003 — Post-quantum posture and the crypto composition rule

Status: **Accepted** (2026). Amends the "no custom cryptography" rule in `.rules/security.md`.

## The finding

Suite 1 is not post-quantum safe, and the failure is concentrated in the two places that
matter most:

| Primitive | Role | Quantum status |
|---|---|---|
| AES-256-GCM | content and manifest | safe — Grover only halves it to ~128-bit |
| SHA-256 | chunk hashes | safe at this usage |
| X25519 (age) | wraps the file key | **broken by Shor** |
| Ed25519 | container signature | **broken by Shor** |

Consequence: **store-now-decrypt-later**. Anyone archiving `.dome` files today can read them
once a cryptographically-relevant quantum computer exists, and can retroactively forge
signatures because recovering the key allows signing anything.

## The rule, corrected

The original rule ("no custom cryptography") was right about primitives and too broad about
composition. Amended:

- **Never** invent a cipher, AEAD, hash, KDF or signature scheme. Not negotiable.
- **Composition is allowed**, including a custom envelope or key schedule, when it (a) follows
  a published specification rather than an invention, (b) has published test vectors the
  implementation is checked against, and (c) is recorded in an ADR with its failure modes.

Layering is not itself a security win: every layer adds parsing surface and key-management
burden. Prefer few, specified layers.

## Standards check (verified against primary sources, 2026)

| Standard | Status | Use |
|---|---|---|
| FIPS 203 — ML-KEM | final, Aug 2024 | key encapsulation |
| FIPS 204 — ML-DSA | final, Aug 2024 | signatures |
| FIPS 205 — SLH-DSA | final, Aug 2024 | hash-only signatures, archival |
| RFC 10024 — PQ/T hybrid key agreement (TLS 1.3) | **Standards Track, Aug 2026** | the hybrid combination |
| `draft-connolly-cfrg-xwing-kem-11` (X-Wing) | **Internet-Draft, Informational** | not used: superseded for our purpose by RFC 10024 |
| Falcon, HQC | selected, **standards still in progress** | not used until final |
| NIST IR 8547 | final | quantum-vulnerable algorithms deprecated and removed by **2035**, high-risk far earlier |

**Correction.** An earlier draft of this ADR proposed X-Wing. X-Wing is a well-reviewed
construction but it is still an Individual Internet-Draft. RFC 10024 puts the same
combination — X25519 together with ML-KEM-768 — on Standards Track and in deployment, so
that is what suite 2 cites.

## Suite 2 — hybrid, dual-algorithm

- **Key wrapping**: X25519 + **ML-KEM-768** (FIPS 203), combined per **RFC 10024**: both
  shared secrets go through the KDF, so the file key survives unless X25519 *and* ML-KEM both
  break. ML-KEM-1024 is the level-5 option if we want the margin.
- **Key schedule**: HKDF-SHA256.
- **Content**: AES-256-GCM, unchanged.
- **Signatures**: Ed25519 **and** ML-DSA-65 (FIPS 204), verified independently, accepted only
  if both pass. SLH-DSA (FIPS 205) is the archival alternative: hash-based, so it rests on a
  different assumption than lattices.
- **Suite identifier**: a suite byte is in the header (25-byte prefix). Suite 1 is classical;
  unknown suites are refused with `FormatError::UnsupportedSuite` rather than parsed on a
  guess.

### Format consequence that the suite byte exists to absorb

Suite 2 is much larger than suite 1, and the current layout assumes suite-1 sizes:

| Item | Suite 1 | Suite 2 |
|---|---|---|
| signature footer | 64 B (Ed25519) | 64 B + ~3309 B (ML-DSA-65), or dual |
| wrapping blob | 32 B (X25519) | 32 B + 1088 B (ML-KEM-768 ciphertext) |
| public key | 32 B | 32 B + 1184 B |

So **the footer length must become suite-dependent**. A reader must derive it from the suite,
never from the constant it uses today, or a suite-2 file would be silently mis-framed.

Primitives are already available in aws-lc-rs 1.19 (`kem` module, `ML_DSA_44/65/87`). `age`
has no PQ recipient, so suite 2 defines its own wrapping rather than extending age.

## Status: suite 2 is implemented

Suite 2 ships as of this ADR's revision 2: X25519 + ML-KEM-768 wrapping per RFC 10024,
HKDF-SHA256, AES-256-GCM, and dual Ed25519 + ML-DSA-65 signatures. 19 tests cover it,
including separate tampering of each signature half and explicit refusal in both downgrade
and upgrade directions. Suite 1 remains the default so existing content keeps its meaning.

## What is still not claimed

Suite 1 remains the default until suite 2 ships and its vectors pass. Until then Dome must
not claim post-quantum confidentiality or authenticity anywhere: the same honesty rule that
forbids "escape-proof" applies to "quantum-safe".
