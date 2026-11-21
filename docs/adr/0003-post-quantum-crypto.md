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

## Suite 2 — hybrid, dual-algorithm

- **Key wrapping**: X-Wing (`X25519 + ML-KEM-768`, CFRG draft, with test vectors). The file
  key survives unless both X25519 *and* ML-KEM break.
- **Key schedule**: HKDF-SHA256. Standard.
- **Content**: AES-256-GCM, unchanged.
- **Signatures**: Ed25519 **and** ML-DSA-65 (FIPS 204), verified independently, accepted only
  if both pass. Forgery then requires breaking curves and lattices together.
- **Suite identifier**: a suite byte is in the header from now on (25-byte prefix). Suite 1 is
  classical; unknown suites are refused with `FormatError::UnsupportedSuite` rather than
  parsed on a guess.

Primitives are already available in aws-lc-rs 1.19 (`kem` module, `ML_DSA_44/65/87`). `age`
has no PQ recipient, so suite 2 defines its own wrapping rather than extending age.

## What is still not claimed

Suite 1 remains the default until suite 2 ships and its vectors pass. Until then Dome must
not claim post-quantum confidentiality or authenticity anywhere: the same honesty rule that
forbids "escape-proof" applies to "quantum-safe".
