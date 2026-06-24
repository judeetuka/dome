# String obfuscation policy

Default to plain Rust string literals.

Compile-time string obfuscation is allowed only as anti-`strings(1)` hygiene in release binaries. It is not a security boundary.

Do not describe string obfuscation as encryption, secrecy, anti-reverse-engineering protection, IP protection, anti-tamper protection, credential protection, or a way to hide Dome behavior.

Never store secrets in source code, obfuscated or plain. Credentials, tokens, private keys, license secrets, private endpoints, and cryptographic material must not be committed.

Keep these strings plain:

- CLI commands, flags, help text, usage text, and user-facing errors
- TOML table names, key names, schema names, wire values, enum string values, and serialized output
- product names, artifact names, file extensions, architecture names, engine names, and documented terminology
- Linux probe paths, environment variable names, kernel/sysfs/procfs tokens, tool names, and capability labels
- test fixture strings, assertion strings, sample IDs, and documented examples
- any string used for parsing, serialization, compatibility, support, logs, diagnostics, or public documentation

Obfuscation may be used only for non-contract, non-user-facing, release-only marker strings where the sole goal is reducing casual `strings(1)` output.

Every approved use must have a nearby comment:

```rust
// obfuscation: anti-strings hygiene only; not a security boundary; reason: <specific reason>
```

In open-source code, assume every obfuscated value is recoverable by reading source, building with instrumentation, debugging, tracing, or dumping process memory.
