# Rust rules

Rust code should stay small, typed, and boring.

- keep public models in `dome-core`
- keep host probes platform-specific and behind host-report abstractions
- keep command parsing in `dome-cli`
- keep TOML parsing and rendering in `dome-config`
- avoid dependencies until a subsystem proves it needs them
- prefer `Result` over panics in library code
- keep `unsafe` out of milestone 1
- model privileged operations before implementing them
- keep tests in `crates/<crate>/tests/`; inline tests are only for tiny private helpers

Verification baseline:

```sh
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
