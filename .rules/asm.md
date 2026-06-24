# Assembly rules

Assembly is allowed only where Rust cannot express the required boundary or where a measured hot path justifies it.

Accepted uses:

- boot or entry stubs
- CPU feature probes
- syscall or interrupt boundaries
- context save/restore code
- measured memory or pixel-format hot paths

Assembly must be isolated under `dome-asm` or an architecture-specific `asm/` directory. Rust code must expose typed wrappers around assembly entrypoints before other crates can use them.

Milestone 1 may include assembly source files as skeleton assets, but it must not link or execute assembly routines until the ABI contract is written down.
