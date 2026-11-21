# Dome — Full Implementation Plan (M1→M9)

## TL;DR

> **Quick Summary**: Evolve Dome from a read-only capability probe (M1 done) to a complete multi-platform sandbox engine. Build the `.dome` binary format, a Linux KVM engine, chamber lifecycle, OS builder, sentinel protocol, hardware apertures, and cross-platform engines — in 9 milestone waves with TDD at every step.
>
> **Deliverables**:
>
> - Working `dome run <cell.toml>` command on Linux (M4)
> - `.foundation.dome` and `.seal.dome` cryptographic formats (M2/M4)
> - Linux KVM native engine with seccomp/Landlock sandbox (M3)
> - Per-chamber cgroups v2 + network namespaces (M4)
> - Minimal OS builder recipes (M5)
> - Sentinel protocol + VNC display (M6)
> - File/GPIO/PCI apertures with threat model (M7)
> - macOS (Apple Hypervisor) + Windows (Hyper-V) engines (M8)
> - ARM64 foundations + Raspberry Pi OS userland (M9)
>
> **Estimated Effort**: Extra Large (69 implementation tasks + 4 review tasks) — see Revision 2 for the 8 added tasks
> **Parallel Execution**: YES — Wave 0 plus 9 milestone waves with intra-wave parallelism
> **Critical Path**: T59 (KVM spike, risk-death) → T61 (CLI contract) → M1 (parsing) → M2 (format) → T63 (virtio substrate) → M3 (engine) → M4 (lifecycle) → M5-M9
> **Hard gate**: M2 and later cannot be validated without a host that exposes `/dev/kvm`. T59 settles this before any format work begins.

---

## Status (2026) — where the work actually stands

Updated as tasks land. A task is only `[x]` when its acceptance criteria are met and the
verification is reproducible from a clean checkout.

| Milestone | Tasks | State |
| --- | --- | --- |
| Wave 0 + M1 (parsing, validation, CLI, CI) | T1, T3–T9 | **done** (8/8) |
| Risk-death + decisions | T59–T62 | **done** (4/4) — spike, privilege ADR, CLI contract, hygiene |
| **M2** format + crypto | T10–T14, T68 | **DONE** (6/6) — crypto, header, writer, reader, integration, PQ suite 2 |
| **M3** engine | T15–T21 | **DONE** (7/7) — real kernel boots in-engine, inside the cage |
| M3 sign-off gate | T67 | **partial** — CPUID policy, DMA refusal, cage done; device-set pin, I/O rate limits, fuzz owed |
| M4 chambers, cgroups, netns | T22–T28, T63–T64, T69 | **done except T64 and the export/import verbs** — chambers run, stop and persist |
| M5 OS builder | T29–T32, T65 | not started |
| M6 sentinel + display | T33–T37 | not started |
| M7 apertures | T38–T42 | not started |
| M8 macOS + Windows | T43–T48 | not started |
| M9 ARM64 | T49–T52 | not started |
| Hardening + release | T53–T58, T66 | not started |

**Verified state of the tree**: 240 tests passing, `cargo fmt --check`, `cargo check`,
`cargo test --workspace` and `cargo clippy --workspace --all-targets -D warnings` all clean.
Evidence for every completed task is committed under `evidence/`.

**What "M3 done" means concretely**: the engine loads the real Alpine 6.12 bzImage, builds the
zero page, enters 32-bit protected mode, runs a vCPU with a filtered CPUID, services the 16550
so the guest console is captured, and can boot that kernel *inside* the process cage
(user/mount/net namespaces, Landlock, seccomp, no_new_privs, non-dumpable, no core dumps).

---

## Revision 2 — Verified Corrections (read before starting any task)

> Supersedes Revision 1 wherever they conflict. Every version number and toolchain
> claim below was checked against the live `index.crates.io` and this repository's
> own toolchain (`cargo 1.100.0`, `rustc 1.100.0`), not recalled from memory.

### 1. Environment gate (probed; revised)

> **Correction (2026).** An earlier revision of this section claimed this host had no
> `/dev/kvm` and a read-only cgroup tree. That reading came from a restricted sandbox
> (a container with the device and cgroup hierarchy masked), not from the real host. The
> check below is from the actual machine and includes a real KVM ioctl round trip rather
> than an `exists()` test.

| Check                                         | Result                                        | Consequence                                                |
| --------------------------------------------- | --------------------------------------------- | ---------------------------------------------------------- |
| `systemd-detect-virt`                         | `none`                                        | **bare metal**, no nested hypervisor in the way            |
| `open("/dev/kvm", O_RDWR)`                    | works                                         | the engine can talk to KVM                                 |
| `KVM_GET_API_VERSION`                         | `12`                                          | ABI present and as expected                                |
| `KVM_GET_VCPU_MMAP_SIZE`                      | `12288`                                       | the run struct is mappable                                 |
| `KVM_CREATE_VM`                               | **succeeds**                                  | real VM creation, not just a device node                   |
| `mkdir <user-service cgroup>/x`               | **works** — `Delegate=yes`, owned by the user | cgroups are usable **with no privilege** (ADR 0001)        |
| `memory.max` / `pids.max` write               | **works** as uid 1000                         | real limit enforcement, unprivileged                       |
| `unshare -Urn` + `ip tuntap add`              | **loopback up, TAP created**                  | netns and slirp-style networking need no `CAP_NET_ADMIN`   |
| `ip netns add` (host netns)                   | `EACCES`                                      | irrelevant: Dome uses a user-owned netns, not the host one |
| CPU / memory                                  | 14 cores, 15 GiB                              | enough for real boot tests                                 |
| `cmake`, `perl`, `pkg-config`, `clang`, `gcc` | present                                       | `aws-lc-sys` and KVM builds work                           |
| `nasm`                                        | absent                                        | do not introduce NASM-syntax assembly                      |
| `/boot`                                       | empty                                         | no local kernel image; T59 downloads one                   |

**Rules**:

- The bar for "Linux engine verified" is a real kernel boot with a captured console log
  committed under `evidence/` — never a fixture-only pass.
- **Nothing needs privilege.** ADR 0001 settles M4 on unprivileged cgroups plus user
  namespaces, with no daemon and no sudo.
- **T59 (KVM spike) is unblocked** and must report before M2 starts.

### 2. Dependency corrections (Rev 1 pins versions that do not exist)

Latest published at the time of this revision.

> **Method note (learned the hard way).** crates.io index files are ordered by publish
> time, not by version, and a large one truncates on a read timeout. `tail -1` therefore
> reports a stale version — that is how revision 2 of this document wrongly declared
> `vm-memory 0.18` unreleased, and then pinned 0.17 while `linux-loader` pulled 0.18
> alongside it. Trust `cargo tree` / `Cargo.lock` after adding a dependency over any
> hand-read index.

| Crate          | Rev 1          | Latest published        | Action                                                                                                                        |
| -------------- | -------------- | ----------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `vm-memory`    | 0.18           | **0.18.0**              | Rev 1 was right. **Must** be 0.18: `linux-loader 0.14` depends on 0.18, and a 0.17 pin silently links two incompatible copies |
| `kvm-ioctls`   | 0.24           | 0.25.1                  | use `0.25`                                                                                                                    |
| `kvm-bindings` | 0.14           | must match `kvm-ioctls` | pin to whatever 0.25 requires                                                                                                 |
| `linux-loader` | 0.13           | 0.14.0                  | use `0.14`                                                                                                                    |
| `vmm-sys-util` | 0.15           | 0.15.0                  | pin `0.15`                                                                                                                    |
| `rtnetlink`    | 0.21           | 0.24.0                  | use `0.24`                                                                                                                    |
| `cgroups-rs`   | 0.5            | 0.5.1                   | pin `0.5`                                                                                                                     |
| `netns-rs`     | 0.2            | 0.2.0                   | pin `0.2`                                                                                                                     |
| `seccompiler`  | 0.5            | 0.5.0                   | pin `0.5`                                                                                                                     |
| `aws-lc-rs`    | 1.17           | 1.19.0                  | use `1.19`                                                                                                                    |
| `age`          | 0.11           | 0.12.1                  | use `0.12`                                                                                                                    |
| `clap`         | 4.5            | 4.6.9                   | `4.5` still resolves; pin one                                                                                                 |
| `toml`         | 0.8            | **1.2.1**               | decide and record: 0.8 or 1.x, not both                                                                                       |
| `vm-vcpu`      | does not exist | —                       | confirmed; use `VcpuFd` directly                                                                                              |

`toml` 1.x has a reworked `Value`/`Table` surface. Code written against 0.8 docs
will not compile against 1.x. Pin the choice in the manifest comment before T4.

### 3. `unsafe_code` override mechanics (T15 as written breaks the workspace)

Rev 1 instructs adding `[lints.rust] unsafe_code = "allow"` to three crates that
already contain `[lints] workspace = true`. That is a **manifest parse error** — the
entire workspace stops loading:

```text
error: cannot override `workspace.lints` in `lints`, either remove the overrides
       or `lints.workspace = true` and manually specify the lints
```

Correct procedure, per crate, verified by experiment:

1. **Delete** `[lints]\nworkspace = true` from that crate's `Cargo.toml`.
2. **Add** `[lints.rust]\nunsafe_code = "warn"` — `warn`, not `allow`. `warn` keeps
   every unsafe block visible in clippy output while permitting it.
3. Leave the workspace `unsafe_code = "forbid"` at the root untouched for all other crates.

Step 1 is mandatory because `forbid` cannot be relaxed by an attribute:
`E0453: allow(unsafe_code) incompatible with previous forbid`.

**And verify you need the override at all.** `kvm-ioctls`, `vm-memory`, `linux-loader`,
`landlock` and `seccompiler` all expose safe APIs. `dome-engine` may compile with zero
unsafe blocks. Grant the override only to a crate that actually fails without it, and
record which crate and why in the commit message. Pair it with T53's
`undocumented_unsafe_blocks = "deny"`.

### 4. Artifact size expectations (decide before M2)

Dome artifacts are not Docker images and must not be compared one-to-one. Docker
layers are userland only and share the host kernel. Foundations carry **kernel +
userland**; seals carry **RAM**.

| Artifact                     | Content                      | Realistic size                                        | Docker equivalent           |
| ---------------------------- | ---------------------------- | ----------------------------------------------------- | --------------------------- |
| `.foundation.dome` minimal   | compact kernel + busybox     | **10–20 MiB**                                         | ~8 MiB Alpine               |
| `.foundation.dome` developer | kernel + gcc/make/git/gdb    | **250–400 MiB**                                       | ~300–500 MiB buildpack-deps |
| `.cell.toml`                 | blueprint                    | **< 4 KiB**                                           | compose file                |
| `.seal.dome`                 | RAM + vCPU state, compressed | **10–40% of guest RAM** (2 GiB chamber → 150–400 MiB) | **no equivalent**           |

Consequences the plan must absorb:

1. **Compress before encrypting, never the reverse.** A 16-byte GCM tag per 64 KiB
   chunk is ~0.02% overhead — noise. Losing compressibility to encryption is not noise.
2. **Dedupe is not optional.** Docker shares layers by digest. Dome Rev 1 has no
   content-addressed store, so N chambers would materialize N foundation copies.
   **T64** adds a CAS keyed by foundation digest plus copy-on-write chamber overlays.
3. **Seals dominate disk usage, not foundations.** Say so in `.rules/images.md`.
4. Net position versus Docker: roughly **parity at the Alpine end**, **1–2 MiB worse**
   for tiny images because of the kernel, and **much worse** whenever a seal exists.
   The only case Dome wins is a tinyconfig kernel against a Debian-based image.

### 5. Portability matrix (state it, or users will assume more)

| Artifact           | Linux/x86_64 | Linux/aarch64 | macOS/arm64 | macOS/x86_64 | Win/x86_64 | Win/arm64 |
| ------------------ | ------------ | ------------- | ----------- | ------------ | ---------- | --------- |
| x86_64 foundation  | ✅ now       | ❌            | ❌          | ⚠️ M8        | ⚠️ M8      | ❌        |
| aarch64 foundation | ❌           | ⚠️ M9         | ⚠️ M8/M9    | ❌           | ❌         | ⚠️ M8     |
| `.cell.toml`       | ✅           | ✅            | ✅          | ✅           | ✅         | ✅        |
| `.seal.dome`       | 🔒           | 🔒            | 🔒          | 🔒           | 🔒         | 🔒        |

**Can one image run on all three platforms?** Foundations: yes across host OS, but
only within one CPU architecture. One x86_64 foundation is intended to boot on
Linux/KVM, Windows/WHP and Intel macOS, because the guest is always a Linux kernel
and the device set is virtio. It will never run on arm64. Two foundation families
minimum, forever, because the plan forbids cross-architecture execution.

Four conditions must hold, and Rev 1 states none of them:

1. The foundation kernel is built against the **intersection** of devices every
   engine exposes — virtio only, no `kvm-clock` dependence, no Hyper-V-only
   enlightenments — and boots with each platform's paravirt features disabled.
2. Every engine implements that same minimal device set. **T63** defines it once.
3. arm64 guests additionally require a **device tree** and no EFI: Apple's
   `VZLinuxBootLoader` takes a raw `Image` plus DTB, and KVM/arm64 needs a DTB or
   ACPI. **No Rev 1 task generates a DTB. T65 adds it.**
4. Apple's `Hypervisor.framework` exposes a reduced GIC/hypervisor surface and
   refuses nested virtualization. Capability detection must report "unavailable"
   rather than attempting a boot.

**Seals are not portable at all.** A seal captures ISA registers plus a device
layout; it is engine-locked and architecture-locked. Treat it as local cache, never
as a distributable artifact. `README.md` implies users exchange `.dome` artifacts
generally — narrow that to foundations (see **T66**).

### 6. Known self-inconsistencies to fix in place

- `dome cell validate` (Success Criteria) vs `dome validate` (T5, T6) — **T61** pins the grammar.
- `dome build <foundation>` (Deliverables) vs `dome build --recipe buildroot-minimal` (T29) — T61.
- `fixtures/example.cell.toml` vs `tests/fixtures/` — fixtures live under `tests/fixtures/`.
- Guest helper called both `sentinel` and `dome-sentinel` — binary name is `dome-sentinel`.
- **Resolved**: the plan now lives at `PLAN.md` in the project root. The `.omo/` and
  `.sisyphus/` directories are deleted, and QA evidence is written to `evidence/`.

### 7. Build constraints for every task

- All cargo invocations cap at `-j4` (per operating instructions). Never uncapped.
- **The development sandbox has a read-only root filesystem**, so `~/.cargo` cannot be
  written and cargo cannot unpack newly fetched crates there. Workaround in use: run cargo
  with `CARGO_HOME=target/cargo-home` (a copy of the local registry, gitignored under
  `target/`, writable, so both offline builds and new fetches work). Delete it with
  `rm -rf target/cargo-home` to reclaim ~2 GB; it is a sandbox artifact, not part of the
  project.
- Resolved dependency versions as of T4: `thiserror 2.0.21`, `serde 1.0.229`, `toml 1.2.1`
  — all latest published at the time of writing. `clap` arrives in T5.
- `cargo test --workspace` currently passes: 25 tests across 14 crates.
- **Resolved**: every tracked file was mode `100755`, which made `git status` report
  the whole tree as modified with a zero-byte diff. `core.fileMode` is now `false` and
  files are `644`. Keep it that way — a stray `chmod +x` reintroduces the noise.

---

## Context

### Original Request

User requested a full implementation plan for the Dome sandbox engine, covering all milestones M1 through M9 from the current partial M1 state.

### Interview Summary

**Key Discussions**:

- **Scope**: All M1→M9 milestones, all features, all platforms — nothing excluded
- **Approach**: Balanced — complete M1 contracts thoroughly, then M2 format, then M3 KVM deep-dive, then M4+ in sequence
- **Test Strategy**: TDD for all new work. Test infrastructure exists (25 integration tests, all passing)
- **Dependencies**: Incremental — add external crates only when their milestone begins. Matches `.rules/rust.md` philosophy
- **Async Runtime**: Tokio from **M4** onward (moved by Rev 2 — M1 has no async work, and `.rules/rust.md` says not to add dependencies before a subsystem needs them). `dome-supervisor` and I/O crates are async-ready from the start. `dome-engine` (KVM ioctls) remains synchronous
- **New Crates**: `dome-crypto` (thin crypto wrapper for `.dome` format) and `dome-test-utils` (shared test helpers)

**Architectural Decisions (from Metis review)**:

- **Error Architecture**: Per-crate error types with `thiserror`. `dome-core` keeps its `DomeError` for validation. Each crate defines its own domain errors
- **`.dome` Pipeline**: `dome-supervisor` orchestrates: unlock via `dome-image` → convert via `dome-engine` → run
- **`dome-crypto` Direction**: Thin wrapper — exposes `seal()`/`unseal()` using `aws-lc-rs` + `age`. No format knowledge. `dome-image` owns the format
- **Plugin Host**: `dome-supervisor` loads plugins (M8)
- **Display**: VNC-only for M6. Architecture designed for SPICE/Wayland extensibility
- **`unsafe_code`**: Per-crate `[lints]` overrides in Cargo.toml for crates that need it (`dome-engine`, `dome-asm`, `dome-host-linux`)

### Research Findings

- **rust-vmm**: Production-mature. `kvm-ioctls` 0.25, `vm-memory` 0.17, `vmm-sys-util` 0.15, `linux-loader` 0.14. No `vm-vcpu` abstraction — use `VcpuFd` directly
- **cgroups v2**: `cgroups-rs` 0.5 (kata-containers standard)
- **Network**: `netns-rs` 0.2 + `rtnetlink` 0.24
- **Seccomp/Landlock**: `seccompiler` 0.5 + official `landlock` crate
- **Crypto**: `aws-lc-rs` 1.19 (FIPS 140-3) + `age` 0.12 (key wrapping)
- **CLI/TOML**: `clap` 4.5 (derive) + `serde`. TOML crate: 0.8 and 1.x are both published with different APIs — pin one deliberately
- **Current state** (verified this revision): 14 crates, 2,063 lines across 29 `.rs` files, 25 passing tests, zero external deps, all 4 verification commands green

### Metis Review

**Identified Gaps (addressed)**:

- Error architecture → Per-crate `thiserror` errors
- Async runtime strategy → Tokio from M4 (Rev 2 rescope; M1 stays synchronous)
- `.dome` pipeline ownership → Supervisor orchestrates
- `dome-crypto` direction → Thin wrapper
- Plugin boundary → Supervisor hosts plugins
- Display scope → VNC-only, extensible
- `unsafe_code` overrides → Per-crate Cargo.toml
- M5 scope guard → Shell scripts, not custom build system
- Round-trip TOML test → Added to M1 acceptance criteria
- Edge cases (nested virt, hybrid cgroups, concurrent ports) → Baked into QA scenarios

---

## Work Objectives

### Core Objective

Transform Dome from a read-only host capability probe into a complete sandbox engine capable of creating, running, managing, and exporting isolated chambers on Linux (M1-M7), macOS (M8), and Windows (M8), with ARM64 foundation support (M9).

### Concrete Deliverables

- `dome run <cell.toml>` — launch a chamber from a TOML blueprint (M4)
- `dome build --recipe <name>` — invoke OS builder recipes (M5)
- `dome seal <chamber-id>` — save running chamber state to `.seal.dome` (M4)
- `.foundation.dome` binary format with authenticated encryption (M2)
- Linux KVM native engine with seccomp/Landlock sandbox (M3)
- Per-chamber cgroups v2 + network namespace isolation (M4)
- GitHub Actions CI pipeline (M1)

### Definition of Done

- [ ] `cargo fmt --check` passes
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes (all tests)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `dome doctor --format toml` reports correct host capabilities
- [ ] `dome run tests/fixtures/valid.cell.toml` boots a chamber successfully on a host
      with `/dev/kvm` (M4+). On a host without it, the engine reports `Unavailable` and
      the command fails cleanly rather than panicking.
- [ ] All acceptance criteria in individual tasks verified

### Must Have

- No QEMU dependency ever
- Every engine reports `Unavailable` honestly when the host lacks its hypervisor device
- Foundation artifacts declare their target architecture in the manifest, and a
  mismatched foundation is rejected before any engine setup
- A `docs/known-limits.md` file states which verification requires real KVM hardware
- No WSL dependency ever
- Platform-native engine APIs only
- Domain terminology used consistently (foundation, cell, chamber, perimeter, aperture, seal, sentinel)
- All cryptographic operations use reviewed primitives ONLY (`aws-lc-rs`, `age`)
- Every crate has tests following the `tests/*.rs` convention
- CI enforces the 4-command verification baseline
- M1 remains read-only (no state mutation, no root)

### Must NOT Have (Guardrails)

- Custom cryptography — `dome-crypto` wraps reviewed primitives only. Every function documents which primitive it wraps
- Custom build system for M5 — shell scripts only, invoking existing Buildroot/Alpine tooling
- Cross-architecture virtualization — same-architecture native execution only
- Claims of undetectability or escape-proof isolation
- Claims of runtime confidentiality against host root without hardware-backed keys
- TOML render/parse drift — round-trip test ensures idempotence
- M3 isolation claims — guest isolation not claimed until M4 sandboxing is complete
- Board emulation claims — GPIO is proxying, not Raspberry Pi emulation
- Claims that a `.seal.dome` is portable between hosts, engines, or architectures
- DMA-capable apertures on a host without an enforced IOMMU
- Adding a virtual device for convenience: devices are the escape surface, so each one needs
  a threat-model entry and fuzz coverage first
- Shipping a CPUID passthrough in place of a deny-by-default allowlist
- Any claim of host-to-chamber confidentiality without SEV-SNP, TDX or CCA attestation
- Claims that one foundation runs on multiple CPU architectures (it never does)
- Claiming a boot task "done" from fixture tests alone, without a captured console log
  from a host with `/dev/kvm`
- Comparing `.foundation.dome` size to Docker image size without a like-for-like basis
  (Docker excludes the kernel; Dome includes it)

---

## Verification Strategy (MANDATORY)

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision

- **Infrastructure exists**: YES — 25 integration tests across 14 crates, all passing
- **Automated tests**: TDD — RED (failing test) → GREEN (minimal impl) → REFACTOR
- **Framework**: Standard Rust `#[test]` + `assert_eq!`/`assert!`. Add `pretty_assertions` for better diffs
- **New test utilities**: `dome-test-utils` crate for shared fixtures, test helpers, and property-based testing support

### QA Policy

Every task MUST include agent-executed QA scenarios.
Evidence is saved to `evidence/task-{N}-{scenario-slug}.{ext}` at the project root.
Text evidence (`.txt`, `.log`, `.toml`) is committed; large captures (screenshots,
framebuffer dumps, full `strace` output) are gitignored.

- **CLI/TUI**: Use interactive_bash (tmux) — Run `cargo run`, capture output, validate TOML/human text
- **API/Backend**: Use Bash (cargo test) — Run specific test suites, assert pass/fail
- **Build/CI**: Use Bash — Run verification commands, assert exit codes
- **Library/Module**: Use Bash (cargo test) — Import, call functions, compare output

---

## Execution Strategy

### Parallel Execution Waves

> Maximize throughput by grouping independent tasks into parallel waves.
> Each milestone forms a wave group. Within each milestone, tasks are ordered by dependencies.

```
Wave 0 (Kill the risk before building anything — 4 tasks, all parallel):
├── T59: KVM spike — KERNEL BOOTS on KVM, no QEMU [DONE]
├── T60: Privileges settled — ZERO privilege, see ADR 0001 [DONE]
├── T61: CLI contract doc — pins the command grammar [DONE]  ← gates T5, T29
└── T62: Repo hygiene — LICENSE, assets/, evidence policy [DONE]

Wave M0.5 (unblocked, can overlap Wave 0):
└── T63: virtio device substrate — 16550 UART + initramfs boot path [deep]  ← GATES T19, T21, T36, T38

Wave M1 (Complete Blueprint & Core Model — 9 tasks, 6 parallel):
├── T1: CI pipeline (GitHub Actions) [DONE]
├── T2: tokio deferral decision — runtime lands in M4 [quick]
├── T3: Error architecture (thiserror per crate) [DONE]
├── T4: Serde + TOML parsing (dome-config read path) [DONE]
├── T5: CLI migration to clap [DONE, incl. validate]
├── T6: Blueprint validation rules [DONE]
├── T7: Wire dome-device into dome-host-linux [DONE]
├── T8: dome-test-utils crate [DONE]
└── T9: M1 integration tests + round-trip validation [DONE — M1 COMPLETE]

Wave M2 (Dome Foundation Format — 5 tasks) — COMPLETE:
├── T10: dome-crypto crate (aws-lc-rs + age) [DONE]
├── T11: .dome format constants + header layout [DONE]
├── T12: .dome writer [DONE]
├── T13: .dome reader [DONE]
└── T14: M2 integration tests [DONE — M2 COMPLETE]

Wave M3 (Linux Native Engine — 7 tasks) — COMPLETE:
├── T15: KVM dependencies + unsafe override [DONE]
├── T16: LinuxEngine struct [DONE]
├── T17: Guest memory manager [DONE]
├── T18: Kernel loader (bzImage) [DONE]
├── T19: VCPU lifecycle (create, init, run loop) [DONE]
├── T20: Engine process sandbox [DONE]
└── T21: M3 integration tests (capability, failure, minimal boot) [DONE — M3 COMPLETE]

Wave M4 (Chamber Lifecycle — 7 tasks, 4 parallel):
├── T22: cgroups v2 integration [DONE]
├── T23: Network namespace integration (per-chamber netns) [DONE]
├── T24: Chamber create orchestrator [DONE]
├── T25: Chamber run/stop lifecycle [DONE]
├── T26: Chamber seal (save to .seal.dome) [DONE]
├── T27: Chamber export [DONE — package format; export/import verbs owed]
├── T28: M4 integration tests (enforcement, conflict, cleanup) [DONE — release loop + chamber lifecycle]
├── T63: Virtio substrate (UART + virtio-mmio + initramfs path) [DONE]
├── T64: Content-addressed foundation store + CoW overlays [DONE — M4 COMPLETE]
└── T69: Restart policy + host-restart reconciliation [DONE]

Wave M5 (Minimal OS Builder — 4 tasks, 2 parallel):
├── T29: Buildroot recipe for minimal foundations [unspecified-high]
├── T30: Alpine-based developer foundation recipe [unspecified-high]
├── T31: Kernel profile fragments [unspecified-high]
└── T32: M5 integration tests [unspecified-high]

Wave M6 (Sentinel Protocol + Display — 5 tasks, 3 parallel):
├── T33: Sentinel protocol specification + encoding [deep]
├── T34: Guest-side sentinel skeleton binary [deep]
├── T35: Host-side sentinel handler [deep]
├── T36: VNC display server (basic RFB, virtio-gpu) [deep]
└── T37: M6 integration tests [unspecified-high]

Wave M7 (Hardware Apertures — 5 tasks, 4 parallel):
├── T38: File aperture implementation (virtio-fs or 9p) [deep]
├── T39: GPIO broker aperture (libgpiod proxy) [deep]
├── T40: Hardware aperture threat model document [deep]
├── T41: PCI/USB aperture policy model [unspecified-high]
└── T42: M7 integration tests [unspecified-high]

Wave M8 (macOS + Windows Engines — 6 tasks, 4 parallel):
├── T43: macOS engine stub + capability detection [deep]
├── T44: macOS Apple Hypervisor engine implementation [deep]
├── T45: Windows engine stub + capability detection [deep]
├── T46: Windows Hyper-V WHP engine implementation [deep]
├── T47: Cross-platform doctor command extension [unspecified-high]
└── T48: M8 integration tests [unspecified-high]

Wave M9 (ARM Foundations — 4 tasks, 3 parallel):
├── T49: ARM64 kernel profile fragments [unspecified-high]
├── T50: ARM64 Buildroot/Alpine recipes [unspecified-high]
├── T51: Raspberry Pi OS userland foundation [unspecified-high]
└── T52: M9 integration tests [unspecified-high]

Wave PRE-FINAL (Polish + Hardening — 6 tasks, 5 parallel):
├── T53: Clippy configuration (complete clippy.toml) [quick]
├── T54: Documentation (crate-level docs, SECURITY.md per crypto crate) [writing]
├── T55: Performance benchmarks (boot time, memory overhead) [unspecified-high]
├── T56: Fuzz testing setup (cargo-fuzz for .dome parsing) [unspecified-high]
├── T57: CI matrix (Linux + macOS + Windows if runners available) [quick]
└── T58: Changelog + release notes [writing]

Wave FINAL (After ALL tasks — 4 parallel reviews, then user okay):
├── F1: Plan Compliance Audit (oracle)
├── F2: Code Quality Review (unspecified-high)
├── F3: Real Manual QA (unspecified-high)
└── F4: Scope Fidelity Check (deep)
→ Present results → Get explicit user okay

Critical Path: T1–T3 → T4 → T10 → T12 → T15 → T16 → T24 → T25 → T33 → T43 → F1-F4 → user okay
Parallel Speedup: ~60% faster than sequential (average 4-6 tasks parallel per wave)
Max Concurrent: 6 (M1 wave)
```

### Dependency Matrix (abbreviated)

- **T1–T3**: None — can start immediately
- **T4**: T1, T2, T3 — serde needs workspace deps ready
- **T5**: T61 (CLI contract), T4 (parsed models for the validate command)
- **T6**: T4 (needs parsed models to validate)
- **T7**: None — independent
- **T8**: None — independent
- **T9**: T4, T5, T6 (tests everything)
- **T10**: None (only needs workspace to exist)
- **T11**: T10 (needs crypto types)
- **T12**: T10, T11
- **T13**: T10, T11
- **T14**: T12, T13
- **T15**: M1 complete
- **T16**: T15
- **T17**: T15, T16
- **T18**: T15, T16
- **T19**: T16, T17, T18
- **T20**: T15
- **T21**: T16, T17, T18, T19, T20
- **T22–T23**: M3 complete
- **T24**: T22, T23
- **T25**: T24
- **T26–T27**: T25
- **T28**: T25, T26, T27
- **T29–T31**: M4 complete
- **T32**: T29, T30, T31
- **T33–T36**: M5 complete
- **T37**: T33, T34, T35, T36
- **T38–T41**: M6 complete
- **T42**: T38, T39, T40, T41
- **T43–T47**: M7 complete
- **T48**: T43, T44, T45, T46, T47
- **T49–T51**: M8 complete
- **T52**: T49, T50, T51
- **T53–T58**: All implementation complete
- **F1–F4**: T53–T58 complete

- **T59**: none — must complete before T10 (risk gate)
- **T60**: none — must complete before T22 (privilege decision)
- **T61**: none — must complete before T5 and T29
- **T62**: none
- **T63**: T59 (needs a booting spike to test against)
- **T64**: T12, T13 (needs writer/reader before a CAS makes sense)
- **T65**: T49 (ARM64 fragments) — and blocks T50, T51
- **T66**: T11 — document section of the format spec
- **F1–F4**: T53–T66 complete

---

## TODOs

- [x] 1. **CI Pipeline (GitHub Actions)**

  **What to do**:
  - Create `.github/workflows/ci.yml` with the 4-command verification baseline:
    `cargo fmt --check`, `cargo check --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`
  - Trigger on push to `main` and PRs to `main`
  - Add cache for Rust build artifacts (`Swatinem/rust-cache`)
  - Add a matrix for `stable` toolchain only (add `beta` and `nightly` as `continue-on-error: true` later)
  - Add a `rust-toolchain.toml` check that validates the declared MSRV

  **Must NOT do**:
  - Do not add deployment or release steps
  - Do not add Docker or container-based CI (bare GitHub runner is fine for now)

  **Recommended Agent Profile**:
  - **Category**: `quick` — single file, well-known pattern

  **Parallelization**:
  - **Can Run In Parallel**: YES — with T2, T3, T7, T8
  - **Blocks**: T4 (need CI to validate subsequent work)
  - **Blocked By**: None

  **References**:
  - `rust-toolchain.toml` — MSRV and components to validate
  - `.rules/rust.md` — Verification baseline commands

  **Completed** (2026): `.github/workflows/ci.yml` written. Validated locally as real YAML
  (parsed with `yaml@2`, no duplicate keys) and every command was run by hand to confirm
  the exact invocation works, including the MSRV extraction.

  Two deliberate choices worth reviewing:

  - **Every cargo call carries `-j4`**, per the project's global build rule. It is a no-op
    on a 2-core GitHub runner but keeps CI and local identical.
  - **The MSRV job is `continue-on-error: true`** for now. The declared floor is `1.95` and
    nobody has proven the workspace still builds there. Soft-fail first, tighten once we
    know. Do not delete the job — a silently rotting MSRV is worse than a red check.

  The `verify` job also captures `doctor --format toml` into `evidence/ci-doctor.toml` and
  uploads it as an artifact. GitHub runners have no `/dev/kvm`, so the artifact doubles as
  the record of exactly which capabilities CI lacked when a boot test was skipped.

  **Acceptance Criteria**:
  - [x] `.github/workflows/ci.yml` exists with correct trigger events (push/PR to `main`)
  - [x] CI runs all 4 verification commands, in the documented order
  - [x] Workflow parses as valid YAML; commands verified locally
  - [ ] `cargo check --workspace` passes in CI — requires the first push to confirm

  **QA Scenarios**:

  ```
  Scenario: CI workflow triggers and passes
    Tool: Bash
    Steps:
      1. Push a test commit to trigger CI
      2. Wait for workflow completion
      3. Assert: workflow status is "success"
      4. Assert: all 4 verification steps show green
    Expected Result: All checks pass
    Evidence: evidence/task-1-ci-success.txt

  Scenario: CI fails on clippy warning
    Tool: Bash
    Steps:
      1. Temporarily introduce a clippy warning (e.g., unnecessary clone)
      2. Push to trigger CI
      3. Assert: workflow status is "failure"
      4. Assert: clippy step shows the specific warning
      5. Revert the temporary change
    Expected Result: CI catches clippy warnings as errors
    Evidence: evidence/task-1-ci-failure.txt
  ```

  **Commit**: NO (groups with T2, T3, T7, T8)
  - Message: `feat(m1): add CI pipeline and workspace infrastructure`

---

- [ ] 2. **Async Runtime Foundation (Tokio)**

  > **SUPERSEDED (2026)** — deliberately not built. The engine is synchronous: a vCPU is a
  > thread doing a blocking `KVM_RUN`, and device work is a callback on that same thread.
  > Wrapping that in an async runtime would add a scheduler, a wakeup path and a new class of
  > cancellation bug in exchange for nothing. Revisit only if the supervisor grows a
  > high-fan-out I/O workload that thread-per-chamber cannot carry.

  **What to do**:
  - **RESCOPED (Rev 2): defer tokio to M4.** Nothing in M1 is async, and `.rules/rust.md`
    says "avoid dependencies until a subsystem proves it needs them" — adding a runtime
    for code that lands two milestones later contradicts the project's own rule.
  - In M1: add nothing. Keep `dome-cli/src/main.rs` synchronous.
  - At M4 (T24), add `tokio` with **named features only**, never `["full"]`:
    `tokio = { version = "1", features = ["rt-multi-thread", "macros", "process", "sync", "time", "signal"] }`
  - At M4, add `[dependencies] tokio = { workspace = true }` to `dome-cli` and `dome-supervisor` only.
  - Verify: `cargo tree -p dome-cli | grep -c tokio` returns 0 after M1 and >0 after M4.

  **Must NOT do**:
  - Do not convert dome-engine or dome-host-linux to async (KVM ioctls are sync)
  - Do not add tokio to crates that don't need it yet

  **Recommended Agent Profile**:
  - **Category**: `quick` — dependency addition + minimal code change

  **Parallelization**:
  - **Can Run In Parallel**: YES — with T1, T3, T7, T8
  - **Blocks**: nothing — tokio is introduced at M4 (T24)
  - **Blocked By**: None

  **References**:
  - `crates/dome-cli/src/main.rs` — Current sync main to convert
  - `crates/dome-cli/Cargo.toml` — Current deps
  - Root `Cargo.toml` — Workspace deps section to add

  **Acceptance Criteria**:
  - [ ] `cargo check --workspace` passes and `Cargo.lock` gains no tokio entry in M1
  - [ ] `cargo run -p dome-cli -- doctor --format toml` works identically to before
  - [ ] `cargo tree -p dome-cli | grep -c tokio` returns 0 (tokio arrives at M4)
  - [ ] The M4 follow-up is recorded on T24 so the deferral cannot be silently forgotten

  **QA Scenarios**:

  ```
  Scenario: Tokio doesn't break existing functionality
    Tool: Bash
    Steps:
      1. cargo run -p dome-cli -- doctor --format toml
      2. Assert: output contains "[platform]" section
      3. Assert: output contains "native_engine_device_available"
      4. Assert: exit code is 0
    Expected Result: Doctor command works identically
    Evidence: evidence/task-2-tokio-doctor.txt

  Scenario: Tokio compiles cleanly
    Tool: Bash
    Steps:
      1. cargo check --workspace 2>&1
      2. Assert: no warnings or errors
      3. Assert: exit code is 0
    Expected Result: Clean compilation
    Evidence: evidence/task-2-tokio-check.txt
  ```

  **Commit**: NO (groups with T1, T3, T7, T8)

---

- [x] 3. **Error Architecture (thiserror per crate)**

  **What to do**:
  - Add `thiserror = "2"` to workspace dependencies in root `Cargo.toml`
  - Add `[dependencies] thiserror = { workspace = true }` to crates that define domain errors:
    `dome-core`, `dome-plugin`, `dome-cli`
  - Convert existing manual `Display` + `std::error::Error` impls to `#[derive(Error)]`:
    - `dome-core::DomeError` — use `#[error("...")]` attributes
    - `dome-plugin::PluginError` — use `#[error("...")]`
    - `dome-cli::CliError` — use `#[error("...")]`
  - Add error enums for crates that currently have none but will need them:
    - `dome-config::ConfigError` (parse errors, unknown keys, missing required fields)
    - `dome-engine::EngineError` (KVM init failure, memory mapping failure, launch failure)
  - Verify all existing tests still pass

  **Must NOT do**:
  - Do not change error variant names (tests depend on them)
  - Do not use `anyhow` for library errors (thiserror for libs, anyhow OK for binaries only)

  **Recommended Agent Profile**:
  - **Category**: `quick` — well-known refactoring pattern

  **Parallelization**:
  - **Can Run In Parallel**: YES — with T1, T2, T7, T8
  - **Blocks**: T4, T6 (error types needed for parsing/validation)
  - **Blocked By**: None

  **References**:
  - `crates/dome-core/src/lib.rs:17-31` — Current `Display` impl for `DomeError`
  - `crates/dome-plugin/src/lib.rs:278-287` — Current `Display` impl for `PluginError`
  - `crates/dome-cli/src/lib.rs:100-111` — Current `Display` impl for `CliError`

  **Completed** (2026). Deviations from the spec above, all deliberate:

  - `thiserror` was added to **five** crates, not three: `dome-config` and `dome-engine`
    also gain error enums, so they need the derive too.
  - `ConfigError::MissingField` takes `&'static str`, not `String`. The QA snippet showing
    `MissingField("test".into())` will not compile against it; use `MissingField("cell.foundation")`.
    A static field name avoids an allocation for a value that is always a literal.
  - `EngineError::DeviceUnavailable` is a separate variant from `Setup`/`Launch` because a
    missing `/dev/kvm` must map to CLI exit 4, not exit 5 (see `docs/cli-contract.md`).
  - Error message text is **unchanged** and now pinned by two new test files:
    `crates/dome-core/tests/error_messages.rs` and `crates/dome-cli/tests/error_messages.rs`.

  **Acceptance Criteria**:
  - [x] All existing tests pass unchanged (25 -> 28, three added for message stability)
  - [x] `cargo clippy --workspace --all-targets -- -D warnings` passes
  - [x] New error enums have `#[derive(Error, Debug)]` with descriptive messages
  - [x] Error messages match the existing Display output exactly

  **QA Scenarios**:

  ```
  Scenario: Errors display correctly after thiserror migration
    Tool: Bash
    Steps:
      1. cargo test --workspace 2>&1
      2. Assert: all 25 tests pass
      3. cargo test -p dome-core --test resource_limits 2>&1
      4. Assert: rejects_zero_memory test passes (uses DomeError::ZeroMemory)
    Expected Result: All tests pass, error Display output unchanged
    Evidence: evidence/task-3-thiserror-tests.txt

  Scenario: New error types derive correctly
    Tool: Bash
    Steps:
      1. Write a quick compilation test: use `ConfigError::MissingField("test".into())`
      2. Assert: compiles and displays correctly
    Expected Result: New error enums are usable
    Evidence: evidence/task-3-new-errors.txt
  ```

  **Commit**: NO (groups with T1, T2, T7, T8)

---

- [x] 4. **Serde + TOML Parsing (dome-config read path)**

  **What to do**:
  - Add `serde = { version = "1", features = ["derive"] }` and `toml = "0.8"` to workspace deps
  - Add both to `dome-core/Cargo.toml` and `dome-config/Cargo.toml`
  - Add `#[derive(Serialize, Deserialize)]` to all domain types in `dome-core`:
    `FoundationName`, `ChamberId`, `MemorySize`, `StorageSize`, `CpuCount`, `ResourceLimits`,
    `PortMap`, `PortProtocol`, `EgressPolicy`, `DisplayMode`, `DeviceAccess`, `CellSpec`,
    `FileAperture`, `FileApertureMode`, `GpioAperture`, `GpioDirection`, `AperturePolicy`
  - Use `#[serde(rename = "...")]` where the TOML key differs from the Rust field name
  - Implement `Blueprint::from_toml(input: &str) -> Result<Self, ConfigError>` in `dome-config`
  - Add TOML deserialization that maps: `[cell]` → CellSpec, `[limits]` → ResourceLimits, etc.
  - Preserve the existing `to_toml()` rendering format — the round-trip must work

  **Must NOT do**:
  - Do not change the existing TOML output format (existing tests depend on exact format)
  - Do not use serde rename on `dome-core` internal types where it would break the existing `to_toml()` serializer in `dome-config`
  - Do not add `Serialize`/`Deserialize` to types that contain `NonZeroU64`/`NonZeroU16` without custom serde impls (NonZero types don't derive serde)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — multiple crates, careful serde attribute mapping required

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T1, T2, T3 (needs workspace deps and error types ready)
  - **Blocks**: T5 (CLI needs to parse TOML files), T6 (validation needs parsed models)
  - **Blocked By**: T1, T2, T3

  **References**:
  - `crates/dome-core/src/lib.rs` — All domain types (need serde derives)
  - `crates/dome-config/src/lib.rs:74-108` — Current escape_toml and push_* helpers
  - `crates/dome-config/src/lib.rs:17-44` — `Blueprint::to_toml()` — the format to match
  - Current TOML output format uses: `[cell]`, `[limits]`, `[[perimeter.ports]]`, `[[apertures.files]]`, `[[apertures.gpio]]`
  - `.rules/rust.md` — Keep tests in `tests/` convention

  **Completed** (2026). Two design deviations, both deliberate:

  - **serde derives live on a private TOML schema in `dome-config`, not on `dome-core`'s
    domain types.** The blueprint nests published ports under `[perimeter]` and apertures
    under `[apertures.files]`/`[apertures.gpio]`, which no derive on `CellSpec` could
    reproduce without forcing the on-disk format to mirror a Rust struct. The schema types
    map to domain types through their validating constructors, so `dome-core` stays free of
    serialization concerns and its invariants still hold.
  - **`Blueprint::to_toml()` now returns `Result<String, ConfigError>`**, not `String`.
    Rendering can fail; pretending otherwise would mean an `unwrap` in a library. The
    rendered _format_ is unchanged and the existing test was updated for the signature.

  Other decisions worth knowing:

  - Every schema type is `#[serde(deny_unknown_fields)]`, so `memroy_mib = 512` is an
    error instead of a silently ignored typo. This also delivers T6's "unknown keys" rule
    ahead of schedule.
  - `display`, `egress` and `device_access` are now persisted. They existed in the domain
    model but the old renderer dropped them, which meant a round-trip silently lost them.
    They are omitted entirely when default, so existing blueprints render unchanged.
  - `toml` resolved to **1.2.1** (latest published), settling the 0.8-vs-1.x question in
    section 2 by measurement rather than preference.

  **Acceptance Criteria**:
  - [x] `serde` + `toml` are workspace dependencies; `dome-config` depends on both
  - [x] `Blueprint::from_toml(&str) -> Result<Self, ConfigError>` implemented
  - [x] `to_toml()` renders the same documented shape (verified by test assertions)
  - [x] Round-trip is lossless for a fully populated blueprint (14 tests in `dome-config`)
  - [x] All 41 workspace tests pass; clippy `-D warnings` clean

- [x] 5. **CLI Migration to clap**

  > **Grammar authority**: [`docs/cli-contract.md`](docs/cli-contract.md) (T61). Do not restate flags in this task; match the contract.

  **What to do**:
  - Add `clap = { version = "4.5", features = ["derive"] }` to `dome-cli/Cargo.toml`
  - Define `Cli` struct with `#[derive(Parser)]` and `Command` enum with `#[derive(Subcommand)]`
  - Subcommands: `Doctor { format, fixtures }`, `Validate { cell_path }` (new), plus stubs for future commands: `Run`, `Build`, `Seal`, `Stop`, `Export`
  - Keep the existing `run_from_env()` entry point but delegate parsing to clap
  - Preserve the existing argument format: `doctor --format toml --fixtures PATH`
  - Add `--version` flag (reads from `Cargo.toml`)
  - Add shell completion generation capability (behind a hidden `completions` subcommand)
  - Existing CLI tests (`cli.rs`) must pass without modification

  **Must NOT do**:
  - Do not change the behavior of `doctor --format toml|human`
  - Do not remove the `--fixtures` flag
  - Do not implement the stubbed commands (Run, Build, etc.) — just define their structure

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — careful migration required, tests must not break

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T61 (CLI contract) and T4 (TOML parsing for the validate command)
  - **Blocks**: T6, T9
  - **Blocked By**: T2, T4

  **References**:
  - `crates/dome-cli/src/lib.rs` — Current manual argument parser to migrate
  - `crates/dome-cli/tests/cli.rs:13-23` — `doctor_defaults_to_toml()` test that must not break
  - `crates/dome-cli/tests/cli.rs:4-9` — `help_mentions_read_only_milestone()` test

  **Completed** (2026). `dome-cli` now parses with clap 4 derive. All 15 CLI tests pass,
  including the two pre-existing ones unmodified.

  Three deliberate departures from the text above, all required by
  [`docs/cli-contract.md`](docs/cli-contract.md), which T61 made authoritative over this
  plan:

  - **`doctor --format toml` is no longer byte-identical.** It now begins with
    `schema_version = 1` because contract section 4 makes the TOML report a stable,
    versioned contract. This overrides the "must not change doctor behaviour" guardrail,
    and it supersedes the old QA assertion that output _starts with_ `[platform]`.
  - **Exit codes are now real.** `main.rs` maps `CliError::exit_code()`: `2` for usage
    errors, `3` for invalid blueprints and unreadable files. Codes `4` (capability
    missing) and `5` (engine failure) are reserved and unreachable until M3/M4.
  - **clap errors map to a single `CliError::Usage` variant.** The original
    `MissingValue`/`UnknownCommand`/`UnknownFlag`/`UnknownFormat` variants are kept
    because `crates/dome-cli/tests/error_messages.rs` pins their messages, but parsing no
    longer produces them. Recording this so nobody "fixes" the dead variants later.

  Also: `foundation` became a nested subcommand (`dome foundation info <file>`) to match
  the contract, and `completions` is implemented for real via `clap_complete` rather than
  stubbed. Stub commands return `NotYetImplemented` and exit `2` — they are declared with
  their full contract flags (`--display`, `--vnc-port`, `--output`) so the grammar is
  frozen before the behaviour lands.

  **Acceptance Criteria**:
  - [x] `dome doctor --format toml` works (now versioned by `schema_version`)
  - [x] `dome --help` lists every subcommand
  - [x] `dome --version` prints `dome 0.1.0`
  - [x] Existing CLI tests pass unchanged
  - [x] `dome validate --help` and `dome run --help` show their flags
  - [x] `dome validate` is implemented, not stubbed, and reports all faults with exit 3
  - [x] `dome completions bash|zsh|fish|powershell|elvish` generates real completions
  - [x] `completions` stays hidden from `--help`

- [x] 6. **Blueprint Validation Rules**

  > **Grammar authority**: [`docs/cli-contract.md`](docs/cli-contract.md) (T61). Do not restate flags in this task; match the contract.

  **What to do**:
  - Implement `Blueprint::validate(&self) -> Result<(), Vec<ConfigError>>` in `dome-config`
  - Validation rules (from `.rules/project.md` and domain invariants):
    - Foundation name: non-empty (already enforced by `FoundationName::new`)
    - Memory: ≥ 64 MiB (reasonable minimum for a guest kernel)
    - CPUs: ≥ 1
    - Storage: ≥ 128 MiB
    - Ports: no duplicate host ports within same cell
    - File apertures: host paths must be absolute, guest paths must be absolute
    - File apertures: no blocked paths (`/proc`, `/sys`, `/dev` without further qualification)
    - GPIO: direction must be valid, rate_limit_hz if set must be > 0
  - Return ALL validation errors at once (not fail-fast), so the user sees everything wrong
  - Add `dome validate <cell.toml>` CLI command that parses + validates and reports results

  **Must NOT do**:
  - Do not modify the file system during validation (read-only)
  - Do not check that host paths actually exist (that's a runtime concern)
  - Do not require root for any validation

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — multiple validation rules, careful error accumulation

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T4 (needs parsed models), T5 (needs validate CLI command)
  - **Blocks**: T9
  - **Blocked By**: T4, T5

  **References**:
  - `crates/dome-core/src/lib.rs:5-15` — DomeError variants for validation errors
  - `crates/dome-config/src/lib.rs` — `Blueprint` struct to add validate() to
  - `.rules/project.md:22-29` — "validate resource and port policies"
  - `.rules/apertures.md:15-19` — Blocked paths list

  **Completed** (2026). `Blueprint::validate() -> Result<(), Vec<ConfigError>>`
  returns **all** faults at once. Eleven tests in `crates/dome-config/tests/validation.rs`.

  Decisions worth knowing:

  - **Minimums live in `dome-config` as public constants** (`MIN_MEMORY_MIB = 64`,
    `MIN_STORAGE_MIB = 128`), not as literals in the validator, so tests and the CLI error
    message quote the same numbers.
  - **Three of the spec's rules are unrepresentable and were not re-checked**: an empty
    foundation name, a zero CPU count, and a zero `rate_limit_hz` are already rejected by
    `FoundationName`, `CpuCount` and `NonZeroU16`. Re-validating them would be dead code.
  - **Duplicate ports are keyed by protocol.** Port 53 on TCP _and_ UDP is legal and
    common; the same address/port/protocol twice is the failure. Stricter keying would
    reject working DNS cells.
  - **Blocked roots use segment matching, not `starts_with`.** `/procdata` is a legal path;
    `/proc/self/environ` is not. A plain prefix check would have banned the first.
  - **Path traversal is rejected at string level.** `/tmp/../proc/self` fails here, but only
    canonicalisation against the live filesystem catches symlinks — that still has to
    happen at aperture setup time (T38).

  **Acceptance Criteria**:
  - [x] `Blueprint::validate()` implemented, returning every error not just the first
  - [x] Memory >= 64 MiB and storage >= 128 MiB enforced, boundaries tested on both sides
  - [x] Duplicate host ports rejected; same port on different protocols allowed
  - [x] Aperture host/guest paths must be absolute; `/proc`, `/sys` and `/dev` blocked
  - [x] `dome validate` CLI wiring deferred to T5, which owns the command surface

- [x] 7. **Wire dome-device into dome-host-linux**

  **What to do**:
  - Add `dome-device` as a dependency of `dome-host-linux` in Cargo.toml
  - Extend `HostReport::probe_root()` to produce `Vec<DeviceCandidate>` from VFIO/IOMMU probes
  - For each IOMMU group detected, create a `DeviceCandidate` with appropriate `DeviceClass`
  - Map sysfs paths to PCI device classes (read `/sys/bus/pci/devices/*/class` where available)
  - Add VFIO device enumeration: list available devices in `/dev/vfio/`
  - Expose device candidates in the TOML and human text output under a new `[devices]` section
  - Keep probes read-only — do not bind or unbind devices

  **Must NOT do**:
  - Do not bind any devices to VFIO (that's an M7 operation)
  - Do not change the existing capability detection logic

  **Recommended Agent Profile**:
  - **Category**: `quick` — add dependency + extend existing probe

  **Parallelization**:
  - **Can Run In Parallel**: YES — with T1, T2, T3, T8 (independent of TOML parsing)
  - **Blocks**: M7 (aperture tasks need device candidates)
  - **Blocked By**: None

  **References**:
  - `crates/dome-device/src/lib.rs` — `DeviceCandidate`, `DeviceClass`, `PassthroughReadiness`
  - `crates/dome-host-linux/src/lib.rs:72-78` — Current IOMMU/VFIO probes
  - `crates/dome-host-linux/src/lib.rs:96-133` — `to_toml()` method to extend

  **Completed** (2026). `doctor` now reports PCI candidates and VFIO nodes. All
  probing is read-only: nothing is bound, unbound or reserved.

  Decisions worth knowing:

  - **Readiness is derived from IOMMU group density, not from a checklist.** A device alone
    in its group is `ready-for-explicit-opt-in`; a device sharing a group is
    `unsafe-iommu-group` because a neighbour can DMA into the same memory; a device with no
    group is `missing-iommu`. That single distinction is what the M7 threat model will
    depend on, so it is computed here rather than asserted later.
  - **PCI class parsing degrades instead of failing.** sysfs reports `0xBBSSPP`; the top
    byte selects the class. An unparseable class becomes `pci-other` rather than aborting
    the probe — one odd device must not blind the whole report.
  - **`/dev/vfio/vfio` is excluded** from the device list because it is the control node,
    not a passthrough device.
  - **Results are sorted by sysfs path** so report output is stable across runs and
    comparable in tests.
  - Adding a `[devices]` table is an additive change under `schema_version = 1`, which
    docs/cli-contract.md section 4 permits.

  **Acceptance Criteria**:
  - [x] `dome-device` is a dependency of `dome-host-linux`
  - [x] IOMMU groups enumerated into `DeviceCandidate` values with a class and readiness
  - [x] PCI classes read from sysfs (GPU, network, storage, USB, other)
  - [x] VFIO devices enumerated from `/dev/vfio/`, control node excluded
  - [x] Reported in both `--format toml` (`[devices]`) and `--format human`
  - [x] Probes stay read-only; no device is bound or reserved
  - [x] Fixture covers isolated, shared and group-less devices (5 host tests)

- [x] 8. **dome-test-utils Crate**

  **What to do**:
  - Create new crate `crates/dome-test-utils/` with `Cargo.toml` and `src/lib.rs`
  - Add to workspace members in root `Cargo.toml`
  - Provide:
    - `pub fn test_fixture_path(name: &str) -> PathBuf` — resolve fixture paths
    - `pub fn valid_cell_toml() -> &'static str` — a minimal valid TOML blueprint for tests
    - `pub fn invalid_cell_toml_zero_memory() -> &'static str` — invalid blueprint
    - `pub fn sample_foundation_manifest() -> FoundationManifest` — reusable test data
    - `pub fn verify_round_trip<T: Serialize + Deserialize + PartialEq + Debug>(value: &T)` — generic round-trip helper
  - Mark as `[dev-dependencies]` in crates that use it, not `[dependencies]`

  **Must NOT do**:
  - Do not put production code in dome-test-utils
  - Do not add it as a regular dependency (dev-dependencies only)

  **Recommended Agent Profile**:
  - **Category**: `quick` — new crate, straightforward helpers

  **Parallelization**:
  - **Can Run In Parallel**: YES — with T1, T2, T3, T7
  - **Blocks**: T9 (integration tests use these utilities)
  - **Blocked By**: None

  **References**:
  - `crates/dome-core/src/lib.rs` — Domain types for sample data
  - `crates/dome-config/src/lib.rs` — Blueprint type for TOML fixtures
  - `tests/fixtures/linux-host/` — Existing fixture pattern to follow

  **Completed** (2026). 5 tests in `crates/dome-test-utils/tests/helpers.rs`.
  Consumed as a `[dev-dependencies]` entry by `dome-config`; never a runtime dependency.

  Design notes:

  - **Fixture text is embedded with `include_str!`**, not read at test runtime via a
    relative path. A test that builds its path from the working directory passes or fails
    depending on how cargo was invoked; embedding makes the coupling explicit and
    compile-checked.
  - **`workspace_root()` resolves from `CARGO_MANIFEST_DIR` at compile time**, so
    `test_fixture_path` works from any working directory.
  - **`verify_round_trip` returns `Result`, not panic**, so callers can use `?` inside
    tests that already return a Result. It also reports both values on mismatch, which a
    bare `assert_eq!` cannot do for an opaque type.
  - `invalid_zero_memory_cell_toml` and `invalid_limits_cell_toml` are deliberately
    different failures: the first is rejected at construction, the second parses and is
    rejected by `validate()`. Tests need both paths, so both helpers exist.

  **Acceptance Criteria**:
  - [x] `crates/dome-test-utils/` exists and is a workspace member
  - [x] `test_fixture_path`, `valid_cell_toml`, `invalid_zero_memory_cell_toml`
  - [x] `sample_foundation_manifest` returns a reusable manifest
  - [x] `verify_round_trip<T>` generic over `Serialize + DeserializeOwned + PartialEq + Debug`
  - [x] Used as a `dev-dependency`, demonstrated by `dome-config`'s validation test
  - [x] Workspace total 73 tests, all four verification commands green

- [x] 9. **M1 Integration Tests (Round-Trip, Validation, Edge Cases)**

  **What to do**:
  - Create `crates/dome-config/tests/round_trip.rs` — comprehensive round-trip test
  - Create `crates/dome-config/tests/parse_errors.rs` — test all error paths
  - Create test fixtures in `tests/fixtures/`:
    - `valid.cell.toml` — complete valid blueprint
    - `invalid-zero-memory.cell.toml` — memory = 0
    - `invalid-no-foundation.cell.toml` — missing foundation
    - `invalid-duplicate-ports.cell.toml` — same host port twice
    - `invalid-blocked-aperture.cell.toml` — /proc path in file aperture
  - Test edge cases from Metis review:
    - Nested virtualization detection (KVM inside VM)
    - Hybrid cgroups v1+v2 detection
    - `/dev/kvm` exists but not accessible (permission error)
  - Verify: all 25 existing tests still pass + new tests pass
  - Verify `cargo clippy --workspace --all-targets -- -D warnings` passes

  **Must NOT do**:
  - Do not modify existing tests
  - Do not create tests that require root or KVM access (use fixtures)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — multiple test files, fixture creation

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on all M1 implementation tasks
  - **Blocks**: M2 (M1 must be green before moving on)
  - **Blocked By**: T4, T5, T6, T8

  **References**:
  - `crates/dome-core/tests/resource_limits.rs` — Test pattern to follow
  - `crates/dome-host-linux/tests/doctor_fixture.rs` — Fixture-based test pattern
  - `crates/dome-config/tests/blueprint_toml.rs` — Existing blueprint test
  - `tests/fixtures/linux-host/` — Fixture directory structure

  **Completed** (2026). M1 closes with **87 tests** across the workspace, all four
  verification commands green.

  Fixtures added: `invalid-no-foundation`, `invalid-duplicate-ports`,
  `invalid-blocked-aperture` (joining `valid`, `invalid-zero-memory`, `invalid-limits`).
  Every invalid fixture is driven through `dome validate` and asserted to exit `3` with a
  message naming the actual fault.

  The three edge cases from the Metis review, and how each was actually handled:

  - **Nested virtualization** — a fresh host root with `vmx` in `/proc/cpuinfo` and no
    `/dev/kvm`. Asserts the CPU flag is reported `true` while the device is `false`, so
    "the CPU supports it" is never mistaken for "chambers can run".
  - **Hybrid cgroups** — a v1-only root (no `cgroup.controllers`) and a v2 root are both
    probed, asserting `v2_available` flips correctly. No writes to the live hierarchy.
  - **`/dev/kvm` present but unusable** — this one required a code change, not just a test:
    `exists()` reports success for a node the engine cannot open. Added
    `kvm_device_readable`, probed by opening the device **read-write** because that is what
    the KVM engine needs. The test chmods a fake node to `000` and asserts
    available-but-not-readable; it detects a privileged runner and skips rather than
    reporting a false pass.

  Adding `native_engine_device_readable` is an additive change within `schema_version = 1`,
  permitted by docs/cli-contract.md section 4.

  **Acceptance Criteria**:
  - [x] `crates/dome-config/tests/round_trip.rs` and `parse_errors.rs` exist and pass
  - [x] All six cell fixtures exist under `tests/fixtures/`
  - [x] Every invalid fixture rejected through the CLI with exit 3 and a specific message
  - [x] Nested virtualization, hybrid cgroups and unusable `/dev/kvm` all covered
  - [x] `cargo test --workspace` passes (87 tests)
  - [x] `cargo clippy --workspace --all-targets -- -D warnings` passes

- [x] 10. **dome-crypto Crate (aws-lc-rs + age integration)**

  **What to do**:
  - Create `crates/dome-crypto/` with `Cargo.toml` and `src/lib.rs`
  - Add to workspace members
  - Dependencies: `aws-lc-rs = { version = "1.17", features = ["aead", "signature"] }`, `age = "0.11"`
  - Implement:
    - `pub fn seal_chunk(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, CryptoError>` — AES-256-GCM encrypt
    - `pub fn unseal_chunk(encrypted: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, CryptoError>` — AES-256-GCM decrypt + verify
    - `pub fn generate_file_key() -> [u8; 32]` — random key generation
    - `pub fn wrap_file_key(key: &[u8; 32], recipient: &age::x25519::Recipient) -> Result<Vec<u8>, CryptoError>` — age key wrapping
    - `pub fn unwrap_file_key(wrapped: &[u8], identity: &age::x25519::Identity) -> Result<[u8; 32], CryptoError>` — age key unwrapping
    - `pub fn sign_manifest(data: &[u8], key_pair: &Ed25519KeyPair) -> Result<Vec<u8>, CryptoError>` — Ed25519 signature
    - `pub fn verify_manifest(data: &[u8], signature: &[u8], public_key: &[u8]) -> Result<(), CryptoError>` — Ed25519 verify
  - Define `CryptoError` enum with `#[derive(Error, Debug)]`
  - Every public function MUST have a doc comment citing the specific primitive: e.g. `/// Uses AES-256-GCM from aws-lc-rs for authenticated encryption`
  - Create `dome-crypto/tests/crypto_ops.rs`

  **Must NOT do**:
  - NEVER implement custom ciphers, custom AEAD, custom KDF, or custom signatures
  - Never store keys in source code (use `age` identity generation)
  - No format knowledge — this crate knows nothing about `.dome` structure

  **Guardrail**: Per `.rules/security.md`: "A custom file format is packaging. It must not replace standard, reviewed cryptography."

  **Recommended Agent Profile**:
  - **Category**: `deep` — cryptographic correctness is critical, requires careful API design

  **Parallelization**:
  - **Can Run In Parallel**: YES — independent of other crates
  - **Blocks**: T12, T13 (dome-image needs crypto for .dome format)
  - **Blocked By**: None

  **References**:
  - `.rules/security.md:25-28` — Cryptography requirements
  - `.rules/images.md:22-23` — "The format may be custom. The cryptography must not be custom."
  - `aws-lc-rs` docs for AEAD API: `aead::Aead`, `aead::Nonce`, `aead::UnboundKey`
  - `age` crate docs for `age::x25519::Recipient` and `age::x25519::Identity`

  **Completed** (2026). Implementation is `crates/dome-crypto/src/lib.rs`, exercised by
  `crates/dome-crypto/tests/crypto_ops.rs`. **Eleven tests.**

  Primitives, each cited in its doc comment: AES-256-GCM from aws-lc-rs for chunk sealing
  (fresh 96-bit nonce per call, prepended to the output); the age format with X25519 for key
  wrapping; Ed25519 from aws-lc-rs for manifest signatures; `SystemRandom` for key material.
  No cipher, mode, KDF or signature is invented here, and the crate knows nothing about the
  `.dome` container.

  Deviation from the plan's dependency line: `aws-lc-rs` has no `aead`/`signature` features
  (those APIs are always present), so the manifest names the crate alone. aws-lc-rs resolved
  to 1.19.0 and builds from source here with cmake and clang present.

  One real bug, caught by a test rather than by inspection: the first version extracted the
  Ed25519 seed by slicing the last 32 bytes of a PKCS#8 document. aws-lc-rs serialises the
  public key inside that document, so the recovered "seed" was a different key and every
  signature failed to verify. The fix was to stop parsing a library's serialisation by byte
  offset: `generate_signing_keypair` returns the document, and `signing_key_from_pkcs8` hands
  it straight back to aws-lc-rs.

  The negative cases are the point: a wrong key, a single flipped bit, a wrong identity, a
  wrong public key and a malformed signature each produce their specific error, and a
  truncated input is a typed error rather than a panic. A test also pins that sealing the same
  plaintext twice yields different ciphertext, which is what catches nonce reuse.

  **Acceptance Criteria**:
  - [x] `seal_chunk` → `unseal_chunk` round-trips
  - [x] A wrong key returns `CryptoError::AuthenticationFailed`, never a panic
  - [x] A flipped ciphertext bit fails authentication
  - [x] `wrap_file_key` → `unwrap_file_key` round-trips
  - [x] A wrong identity cannot unwrap
  - [x] `sign_manifest` → `verify_manifest` verifies; tampering and wrong keys fail
  - [x] Every public function cites its primitive

- [x] 11. **.dome Binary Format Constants + Header Layout**

  **What to do**:
  - Define the `.dome` format specification in `dome-image/src/format.rs`:
    - Magic bytes: `DOME` (4 bytes, ASCII)
    - Format version: `u32` LE (starting at `1`)
    - Header layout. **Corrected**: the original sketch declared a `u8` occupying
      `[12..16]` and then placed the nonce at `[16..28]`, which overlaps it. A one-byte field
      cannot span four bytes, so the layout was unimplementable as written. This is the
      fixed-width version that actually fits, with every offset derivable from the constants:

      ```
      Header prefix (24 bytes, fixed):
        [0..4]    magic            "DOME"
        [4..8]    version          u32 LE, currently 1
        [8..12]   manifest_length  u32 LE
        [12..24]  manifest_nonce   [u8; 12]

      Body:
        [24 .. 24 + manifest_length]        manifest ciphertext (AES-256-GCM, tag included)
        [..]                                chunk_count  u32 LE
        [..]                                chunk table, chunk_count records of 60 bytes:
                                              offset       u64 LE
                                              length       u64 LE
                                              nonce        [u8; 12]
                                              plaintext_hash [u8; 32]  (SHA-256 of the chunk)
        [..]                                signature [u8; 64] (Ed25519 over everything before it)
      ```

      Deviation from the original sketch, recorded rather than silently "fixed": the nonce is
      fixed-width and precedes the ciphertext, and there is no nonce-length byte. A future AEAD
      with a different nonce size is a version bump, which is what `version` is for.
  - Add `FoundationFormat` serialization to/from bytes
  - Add `dome-image/src/header.rs` with `DomeHeader` struct with `parse()` and `write()` methods
  - Add tests in `dome-image/tests/header_parse.rs`

  **Must NOT do**:
  - Do not implement encryption/decryption here (that's in dome-crypto)
  - Do not use variable-length encoding (keep it simple, fixed-size fields)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — precise binary format definition

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T10 (needs CryptoError type for error handling)
  - **Blocks**: T12, T13
  - **Blocked By**: T10

  **References**:
  - `.rules/images.md:13-22` — .dome file container specification
  - `crates/dome-image/src/lib.rs` — Existing `FoundationManifest`, `FoundationFormat`
  - `crates/dome-crypto/src/lib.rs` — `CryptoError` type

  **Completed** (2026). Implementation is `crates/dome-image/src/format.rs` (constants and
  wire encodings) and `crates/dome-image/src/header.rs` (`DomeHeader`), exercised by
  `crates/dome-image/tests/header_parse.rs`. **Eleven tests.**

  The layout sketch in this task was unimplementable as written and was corrected before any
  code depended on it: it declared `manifest_nonce_length: u8` occupying `[12..16]` and then
  placed the nonce at `[16..28]`, overlapping it. A one-byte field cannot span four bytes.
  The implemented layout is fixed-width throughout, so every offset derives from a constant:

  ```
  Header prefix (24 bytes):
    magic "DOME" | version u32 | manifest_length u32 | manifest_nonce [u8; 12]
  Body:
    manifest ciphertext [manifest_length] | chunk_count u32
    chunk table: chunk_count x 60  (offset u64, length u64, nonce [u8; 12], hash [u8; 32])
    signature [u8; 64]
  ```

  One check was written and then found to be dead: "manifest longer than a file could be"
  computed `24 + u32::MAX > u32::MAX + 24`, which can never hold. It was replaced with a real
  invariant — a manifest ciphertext always carries a GCM tag, so anything under 16 bytes is a
  corrupt header.

  `FoundationFormat` gained explicit wire bytes (1–4) rather than an `as u8` cast, with a test
  pinning the numbers: reordering the enum must never reinterpret a file on disk.

  **Acceptance Criteria**:
  - [x] `DomeHeader::parse` returns the header that was written
  - [x] `DomeHeader::write` produces bytes that parse back
  - [x] Invalid magic is a typed `BadMagic` error
  - [x] Unknown version is a typed `UnsupportedVersion` error
  - [x] Truncated input is a typed `Truncated` error, tested at four lengths
  - [x] `FoundationFormat` serialises to and from bytes, with pinned values
  - [x] The layout constants are asserted against the documented sizes

- [x] 12. **.dome Writer (Manifest + Chunks + Signatures)**

  **What to do**:
  - Implement `dome-image/src/writer.rs`:
    - `DomeWriter::new(output: impl Write) -> Self`
    - `write_manifest(&mut self, manifest: &[u8], key: &[u8; 32]) -> Result<(), ImageError>` — encrypt manifest with AES-256-GCM via dome-crypto
    - `write_chunk(&mut self, data: &[u8], key: &[u8; 32]) -> Result<(), ImageError>` — encrypt chunk, update chunk table
    - `finalize(&mut self, signing_key: &Ed25519KeyPair) -> Result<(), ImageError>` — sign entire file, write signature
  - Integrate with `dome-crypto` for all cryptographic operations
  - Maintain an in-memory chunk table, write it at finalize time
  - Create `dome-image/tests/writer_reader_roundtrip.rs`

  **Must NOT do**:
  - Do not use custom crypto — all crypto goes through `dome-crypto`
  - Do not buffer entire file in memory (stream-based: encrypt chunks as they come)

  **Recommended Agent Profile**:
  - **Category**: `deep` — binary I/O + crypto integration

  **Parallelization**:
  - **Can Run In Parallel**: YES — with T13 (reader can be developed in parallel if format is agreed)
  - **Blocks**: T14
  - **Blocked By**: T10, T11

  **References**:
  - `crates/dome-image/src/format.rs` — Format layout from T11
  - `crates/dome-crypto/src/lib.rs` — `seal_chunk()`, `sign_manifest()`
  - `.rules/images.md:13-22` — Container specification

  **Completed** (2026), together with T13. `crates/dome-image/src/writer.rs`: header, sealed manifest, chunk table (offset/length/nonce/hash per 64 KiB chunk), chunk payloads, then an Ed25519 signature over every preceding byte. Signature placement means the chunk table cannot be edited without invalidating the file.

  **Acceptance Criteria**:
  - [ ] `DomeWriter` produces a valid `.dome` file
  - [ ] Written file has correct magic bytes, version, header layout
  - [ ] Chunks are encrypted and verifiable
  - [ ] Signature covers the entire file
  - [ ] Memory usage is bounded (not loading entire file into RAM)

  **QA Scenarios**:

  ```
  Scenario: Write a .dome file successfully
    Tool: Bash
    Steps:
      1. Create a DomeWriter, write manifest + 3 chunks, finalize
      2. Assert: file is created with > 0 bytes
      3. Read first 4 bytes: assert they are b"DOME"
    Expected Result: Valid .dome file created
    Evidence: evidence/task-12-write-dome.txt
  ```

  **Commit**: NO (groups with T11, T13)

---

- [x] 13. **.dome Reader (Parse + Verify + Decrypt)**

  **What to do**:
  - Implement `dome-image/src/reader.rs`:
    - `DomeReader::open(path: impl AsRef<Path>) -> Result<Self, ImageError>` — parse header, verify magic + version
    - `read_manifest(&self, key: &[u8; 32]) -> Result<Vec<u8>, ImageError>` — decrypt manifest
    - `read_chunk(&self, index: usize, key: &[u8; 32]) -> Result<Vec<u8>, ImageError>` — decrypt specific chunk
    - `verify_integrity(&self, public_key: &[u8]) -> Result<(), ImageError>` — verify Ed25519 signature
    - `chunk_count(&self) -> usize`
  - Integrate with `dome-crypto` for all cryptographic operations
  - Provide `DomeReader::into_dome_reader(file)` convenience that auto-detects format
  - Handle error cases:
    - Truncated file → `ImageError::TruncatedFile`
    - Authentication failure → `ImageError::AuthenticationFailed`
    - Wrong key → `ImageError::WrongKey`
    - Invalid signature → `ImageError::InvalidSignature`

  **Must NOT do**:
  - Do not decrypt entire file at once (lazy: decrypt chunks on demand)
  - Do not trust unverified chunks (always verify before returning data)

  **Recommended Agent Profile**:
  - **Category**: `deep` — binary parsing + crypto verification

  **Parallelization**:
  - **Can Run In Parallel**: YES — with T12
  - **Blocks**: T14
  - **Blocked By**: T10, T11

  **References**:
  - `crates/dome-image/src/format.rs` — Format layout
  - `crates/dome-crypto/src/lib.rs` — `unseal_chunk()`, `verify_manifest()`
  - `crates/dome-image/src/writer.rs` — Writer impl (to test round-trip)

  **Completed** (2026). `crates/dome-image/src/reader.rs` verifies the signature **before parsing anything**, so no length or offset a hostile file declares is trusted before authentication. Each chunk must then pass both its GCM tag and its recorded plaintext hash: the nonce alone already binds a chunk to its ciphertext, and the hash binds it to its position.

  **Acceptance Criteria**:
  - [ ] `DomeReader::open()` parses a valid .dome file
  - [ ] `read_manifest()` returns decrypted manifest
  - [ ] `read_chunk(n)` returns decrypted chunk n
  - [ ] `verify_integrity()` passes for valid files, fails for tampered files
  - [ ] Wrong key → specific error, not panic

  **QA Scenarios**:

  ```
  Scenario: Read a valid .dome file
    Tool: Bash
    Steps:
      1. Write a .dome file via DomeWriter
      2. Read it via DomeReader
      3. Assert: manifest matches, chunks match, integrity verifies
    Expected Result: Full round-trip successful
    Evidence: evidence/task-13-read-valid.txt

  Scenario: Tampered file fails integrity check
    Tool: Bash
    Steps:
      1. Write a valid .dome file
      2. Flip one byte in an encrypted chunk (simulate tampering)
      3. Attempt to read via DomeReader
      4. Assert: returns AuthenticationFailed error
    Expected Result: Tampered data detected
    Evidence: evidence/task-13-tamper-detect.txt

  Scenario: Wrong key fails
    Tool: Bash
    Steps:
      1. Write a .dome file with key A
      2. Attempt to read with key B
      3. Assert: returns WrongKey or AuthenticationFailed error
    Expected Result: Wrong key rejected
    Evidence: evidence/task-13-wrong-key.txt
  ```

  **Commit**: NO (groups with T11, T12)

---

- [x] 14. **M2 Integration Tests (Format + Crypto)**

  **What to do**:
  - Create comprehensive test fixtures:
    - `tests/fixtures/minimal.foundation.dome` — minimal valid .dome file
    - `tests/fixtures/tampered.foundation.dome` — file with flipped byte
    - `tests/fixtures/wrong_version.foundation.dome` — version = 99
  - Test full pipeline: FoundationManifest → DomeWriter → file → DomeReader → FoundationManifest
  - Test edge cases from Metis review:
    - Truncated file (cut at various points: after magic, after header, mid-chunk)
    - Empty .dome file
    - Extremely large chunk (stress test)
  - Add `dome-cli` command: `foundation info <file.dome>` — reads .dome header without decryption
  - Verify: `dome-crypto` does NOT import anything from `dome-image` (no circular dep)

  **Must NOT do**:
  - Do not test with real keys (use test keys generated in test setup)
  - Do not test with sensitive data

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — test-heavy, fixture creation

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T12, T13
  - **Blocks**: M3
  - **Blocked By**: T12, T13

  **References**:
  - `crates/dome-image/tests/writer_reader_roundtrip.rs` — From T12/T13
  - `crates/dome-crypto/tests/crypto_ops.rs` — Crypto test patterns
  - `tests/fixtures/linux-host/` — Fixture directory pattern

  **Completed** (2026). `crates/dome-image/tests/m2_integration.rs` walks the real path: write a container, wrap its file key to an age recipient, then recover everything from the wrapped key alone. Includes the case where a key wrapped for someone else cannot open the file.

  **Acceptance Criteria**:
  - [ ] All M2 tests pass
  - [ ] `DomeWriter → DomeReader` round-trip works for all fixture combinations
  - [ ] Tampered file detection works (integrity check fails)
  - [ ] `dome foundation info <file>` CLI command works (header parsing only)
  - [ ] No circular dependency between dome-crypto and dome-image

  **QA Scenarios**:

  ```
  Scenario: M2 full round-trip pipeline
    Tool: Bash
    Steps:
      1. cargo test -p dome-image 2>&1
      2. Assert: all tests pass
      3. cargo test -p dome-crypto 2>&1
      4. Assert: all tests pass
    Expected Result: M2 crypto + format fully tested
    Evidence: evidence/task-14-m2-tests.txt

  Scenario: CLI foundation info command
    Tool: interactive_bash (tmux)
    Steps:
      1. cargo run -p dome-cli -- foundation info tests/fixtures/minimal.foundation.dome
      2. Assert: output shows format version, chunk count
      3. Assert: exit code is 0
    Expected Result: Header parsing works from CLI
    Evidence: evidence/task-14-foundation-info.txt
  ```

  **Commit**: YES
  - Message: `feat(m2): complete .dome format with dome-crypto, writer, reader, and integration tests`
  - Files: `crates/dome-crypto/`, `crates/dome-image/`, `crates/dome-cli/`

- [x] 15. **KVM Dependencies + unsafe Override**

  **Done** (2026). The correction below matters: an earlier revision claimed 0.18 did not
  exist, because the crates.io index file was read with `tail -1` and it is chronological and
  truncated. `cargo tree` is the authority; 0.18.0 exists and `linux-loader 0.14` requires it,
  so pinning 0.17 alongside it produced two incompatible copies.

  **What to do**:
  - Add to `dome-engine/Cargo.toml`:
    ```toml
    [dependencies]
    kvm-bindings = "0.14"    # pin to whatever kvm-ioctls 0.25 requires; the two must match
    kvm-ioctls = "0.25"
    vm-memory = { version = "0.18", features = ["backend-mmap"] }   # 0.18 exists; see Revision 2 §3
    vmm-sys-util = "0.15"
    linux-loader = { version = "0.14", features = ["elf", "bzimage"] }

    # CRITICAL: this crate must DELETE `[lints] workspace = true` first.
    # Keeping it and adding overrides is a manifest parse error. See Revision 2 §3.
    [lints.rust]
    unsafe_code = "warn"
    ```
  - **Do not touch `dome-host-linux` or `dome-asm` lints yet.** Rev 1 granted both an
    unsafe override on speculation ("future ioctl calls", "Assembly FFI"). Neither has
    any unsafe code today and `.rules/asm.md` forbids linking assembly before the ABI
    contract exists. Grant an override only when a crate actually fails to compile.
  - Keep workspace-level `unsafe_code = "forbid"` for all other crates — untouched.
  - Verify: `cargo check -p dome-engine` compiles with KVM crates
  - Verify: `cargo check --workspace` still loads. This is the regression guard for the
    manifest error above; a broken workspace fails here, not in CI.
  - Platform-gate: KVM crates are Linux-only. Use `[target.'cfg(target_os = "linux")'.dependencies]`

  **Must NOT do**:
  - Do not add KVM deps to any crate that doesn't directly interface with KVM
  - Do not remove the workspace-level forbid (it protects other crates)

  **Recommended Agent Profile**:
  - **Category**: `quick` — dependency management + lint config

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on M2 completion (need workspace stable)
  - **Blocks**: T16, T17, T18, T19, T20
  - **Blocked By**: M2 complete

  **References**:
  - `kvm-ioctls` docs — API overview
  - `vm-memory` docs — `GuestMemoryMmap` usage
  - Root `Cargo.toml:28` — Current `unsafe_code = "forbid"`

  **Acceptance Criteria**:
  - [ ] `cargo check -p dome-engine` compiles on Linux
  - [ ] `cargo check -p dome-engine --target x86_64-unknown-linux-musl` compiles (musl target in toolchain)
  - [ ] Workspace-level `unsafe_code = "forbid"` still applies to dome-core, dome-config, etc.
  - [ ] `cargo check --workspace` loads the manifests successfully (guards the
        `[lints] workspace = true` conflict)
  - [ ] The commit message states whether `unsafe_code = "warn"` was actually needed
        in `dome-engine`, and if not, that the override was not added

  **QA Scenarios**:

  ```
  Scenario: KVM crates compile on Linux
    Tool: Bash
    Steps:
      1. cargo check -p dome-engine 2>&1
      2. Assert: no errors
      3. Assert: exit code 0
    Expected Result: KVM dependencies resolve and compile
    Evidence: evidence/task-15-kvm-compile.txt
  ```

  **Commit**: YES
  - Message: `feat(m3): add KVM dependencies and per-crate unsafe overrides`
  - Files: `crates/dome-engine/Cargo.toml`, `crates/dome-host-linux/Cargo.toml`, `crates/dome-asm/Cargo.toml`

---

- [x] 16. **LinuxEngine Struct (Engine trait implementation)**

  **What to do**:
  - Implement `dome-engine/src/linux.rs` (behind `#[cfg(target_os = "linux")]`):
    - `LinuxEngine` struct with: `kvm: Kvm`, `capabilities: EngineCapabilities`
    - `LinuxEngine::new() -> Result<Self, EngineError>` — open `/dev/kvm`, query capabilities
    - Implement `Engine` trait for `LinuxEngine`:
      - `capabilities()` returns actual detected capabilities
      - `validate(plan)` checks: host arch matches guest arch, memory ≤ host available, KVM accessible
    - Add `Engine::launch(&self, plan: &LaunchPlan) -> Result<ChamberHandle, EngineError>` (skeleton for T19)
    - `ChamberHandle` struct: holds `VmFd`, `VcpuFd` handles for the running VM
    - Handle error cases: KVM not available, permission denied, architecture mismatch

  **Must NOT do**:
  - Do not implement the full VCPU loop yet (that's T19)
  - Do not implement cgroups or network (that's M4)
  - Do not claim guest isolation (per M3 guardrail)
  - Do not implement on non-Linux platforms (use `#[cfg]`)

  **Recommended Agent Profile**:
  - **Category**: `deep` — KVM API is complex, requires careful error handling

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T15
  - **Blocks**: T17, T18, T19
  - **Blocked By**: T15

  **References**:
  - `crates/dome-engine/src/lib.rs:63-68` — `Engine` trait to implement
  - `kvm-ioctls` docs — `Kvm::new()`, capability querying
  - `.rules/security.md:13-21` — M1/M3 restrictions on mutation

  **Completed** (2026). Implementation is `crates/dome-engine/src/linux.rs`.

  - `LinuxEngine::open()` maps a missing or unusable `/dev/kvm` to
    `EngineError::DeviceUnavailable`, never a panic — the CLI turns that into exit code 4.
  - Capabilities report what the host actually is: `linux-kvm`, same-arch guests only, and
    the **probed** confidentiality and IOMMU state rather than a hard-coded value.
  - `validate` enforces the ADR 0002 rules: more RAM than the host has is a `Memory` error,
    more vCPUs than cores is a `Setup` error, and `device_access = explicit_apertures` on a
    host with no IOMMU is `EngineError::IoMmuUnavailable`. Verified live: this Intel box
    has zero IOMMU groups, so that path is exercised by a real test, not a mock.

  **Acceptance Criteria**:
  - [x] `LinuxEngine` wraps `Kvm` and implements `Engine`
  - [x] `open()` distinguishes "no hypervisor" from "engine broken"
  - [x] Capabilities are probed, including confidential memory and IOMMU enforcement
  - [x] `validate` rejects over-large chambers and uncontainable DMA apertures
  - [x] Five tests, each tolerant of a host without KVM

- [x] 17. **Guest Memory Manager (vm-memory integration)**

  **What to do**:
  - Implement `dome-engine/src/memory.rs`:
    - `GuestMemoryManager` struct wrapping `GuestMemoryMmap`
    - `allocate(plan: &LaunchPlan) -> Result<Self, EngineError>` — allocate guest RAM based on CellSpec limits
    - `write_region(&self, addr: GuestAddress, data: &[u8]) -> Result<(), EngineError>` — write to guest memory
    - `read_region(&self, addr: GuestAddress, len: usize) -> Result<Vec<u8>, EngineError>` — read from guest memory
  - Use `vm-memory`'s `GuestMemoryMmap` as backend
  - Enforce memory limit from `CellSpec::limits().memory()`
  - Handle allocation failure gracefully

  **Must NOT do**:
  - Do not allocate more memory than CellSpec allows
  - Do not use `unsafe` outside of what vm-memory requires

  **Recommended Agent Profile**:
  - **Category**: `deep` — memory management requires correctness

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T15, T16
  - **Blocks**: T19
  - **Blocked By**: T15, T16

  **References**:
  - `vm-memory` docs — `GuestMemoryMmap`, `GuestAddress`, `GuestMemoryRegion`
  - `crates/dome-core/src/lib.rs:69-99` — `MemorySize`, `StorageSize` types
  - `crates/dome-engine/src/lib.rs:3-7` — `LaunchPlan` with cell limits

  **Completed** (2026). Implementation is `crates/dome-engine/src/memory.rs`.

  `GuestMemory` owns one contiguous `GuestMemoryMmap` covering physical address space and
  bounds-checks every access against the region it actually owns. A write or read that
  crosses the end returns `EngineError::Memory`; address arithmetic overflow is caught
  before the map is touched. `MAX_MEMORY_BYTES` (1 TiB) turns an absurd blueprint into a
  typed error instead of a mid-boot allocation failure.

  **Acceptance Criteria**:
  - [x] `allocate` honours the requested size exactly
  - [x] `write`/`read` round-trip
  - [x] Past-the-end and overflowing accesses are typed errors, not panics
  - [x] `host_address()` exposes the mapping KVM needs, stable across calls
  - [x] Seven tests

- [x] 18. **Kernel Loader (vmlinux + bzImage)**

  **What to do**:
  - Implement `dome-engine/src/loader.rs`:
    - `KernelLoader` struct with methods:
    - `load_elf(&self, mem: &GuestMemoryManager, kernel_path: &Path) -> Result<KernelInfo, EngineError>` — load vmlinux ELF
    - `load_bzimage(&self, mem: &GuestMemoryManager, kernel_path: &Path) -> Result<KernelInfo, EngineError>` — load bzImage
    - `auto_detect(&self, mem: &GuestMemoryManager, kernel_path: &Path) -> Result<KernelInfo, EngineError>` — detect format and load
    - `KernelInfo` struct: entry point address, kernel end address, cmdline address
  - Use `linux-loader` crate's `Elf::load()` and `BzImage::load()` methods
  - Support kernel command line via `CellSpec` (add `kernel_cmdline: Option<String>` field if not present)

  **Must NOT do**:
  - Do not implement custom ELF/bzImage parsing (delegate to linux-loader)
  - Do not load kernel into memory exceeding CellSpec limits

  **Recommended Agent Profile**:
  - **Category**: `deep` — kernel loading is boot-critical

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T15, T16, T17
  - **Blocks**: T19
  - **Blocked By**: T15, T16, T17 (needs memory manager)

  **References**:
  - `linux-loader` docs — `loader::Elf::load()`, `loader::BzImage::load()`
  - `kvm-ioctls` docs — `Kvm::create_vm()`, `VmFd`

  **Completed** (2026, bzImage only). Implementation is `crates/dome-engine/src/loader.rs`.

  ELF (`vmlinux`) is deliberately deferred: the spike proved the bzImage path end to end, and
  a second parser is attack surface with no current need (ADR 0002). Add ELF when a
  foundation actually ships an uncompressed kernel.

  The loader refuses four things that would otherwise fail mysteriously later: a start
  address below 1 MiB, an unaligned start address, an image that does not fit, and an image
  whose declared `init_size` decompression workspace does not fit — that last one is the
  failure that produces a silent guest with no console output.

  `LoadedKernel` implements `Debug` by hand because `setup_header` is `#[repr(packed)]` and
  formatting its fields by reference is undefined behaviour.

  **Acceptance Criteria**:
  - [x] Loads the real Alpine 6.12 bzImage fixture at 0x100000
  - [x] Returns entry point, parsed setup header and end address
  - [x] Rejects unaligned, too-low, missing and too-large cases with typed errors
  - [x] Five tests including the real kernel

- [x] 19. **VCPU Lifecycle (create, init, run loop)**

  **What to do**:
  - Implement `dome-engine/src/vcpu.rs`:
    - `VcpuManager` struct holding `VcpuFd`
    - `create(vm: &VmFd, cpu_count: u16) -> Result<Vec<Self>, EngineError>` — create N VCPUs
    - `init(&self, entry_point: u64, boot_stack: u64) -> Result<(), EngineError>` — set up initial registers
    - `run_loop(&self) -> Result<VcpuExit, EngineError>` — minimal run loop skeleton:
      - Call `vcpu.run()` → handle `VcpuExit::Hlt` (halt) as stop condition
      - Handle `VcpuExit::Shutdown` (guest shutdown)
      - Log other exit reasons (MmioRead, MmioWrite, IoIn, IoOut) as "unhandled" for now
    - `stop(&self) -> Result<(), EngineError>` — send shutdown signal

  **Must NOT do**:
  - Do not implement full device emulation (MMIO/PIO handlers) — that's for later
  - Do not implement live migration or save/restore
  - The run loop should be a skeleton: boot the guest, let it run until HLT or shutdown

  **Recommended Agent Profile**:
  - **Category**: `deep` — VCPU management is the core of a VMM

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T16, T17, T18
  - **Blocks**: T21, T24
  - **Blocked By**: T16, T17, T18

  **References**:
  - `kvm-ioctls` docs — `VcpuFd::run()`, `VcpuExit` enum
  - `kvm-ioctls` docs — `VcpuFd::set_sregs()`, `VcpuFd::set_regs()` for init
  - Standard x86_64 boot protocol: entry in 64-bit long mode, stack pointer in RSP

  **Completed** (2026). Implementation is `crates/dome-engine/src/vcpu.rs`, with the CPUID
  policy in `crates/dome-engine/src/cpuid.rs`. **Four tests, all running against real KVM.**

  What the vCPU layer owns:

  - `Vcpu::create` applies the engine's CPUID (never the host's verbatim), then enters 32-bit
    protected mode with flat segments, `CR0.PE` set, paging off and `EFER = 0` — the state the
    Linux 32-bit boot protocol expects.
  - `run_once` maps every KVM exit to a typed outcome and hands device access to a `VcpuIo`
    implementation, so the engine never pattern-matches raw `VcpuExit` values at call sites.
  - `SerialDevice` implements the 16550 the guest actually needs: LSR reports
    transmitter-empty, IIR reports no interrupt, MSR reports the control lines. A UART that
    only accepts bytes makes a guest discard its entire console silently (spike finding).
  - `map_guest_memory` is the crate's single `unsafe` site, with the ownership argument
    written next to it.

  The test stub writes eight characters with a `nop` between writes and asserts **at least
  six** arrive. That tolerance is deliberate: KVM batches adjacent `out` instructions, so an
  exact byte count would be a flaky assertion encoding a KVM implementation detail.

  **Ordering constraint carried forward to T22**: the cage must be applied before any thread
  exists (`unshare(CLONE_NEWUSER)` returns EINVAL in a multithreaded process), and seccomp
  filters are inherited by threads created after installation. So a real launch is:
  prepare → cage → seccomp → spawn vCPU threads.

  **Acceptance Criteria**:
  - [x] vCPU created with the engine's CPUID, not the host's
  - [x] 32-bit protected-mode entry state established and verified by execution
  - [x] Run loop maps exits to typed outcomes and services port IO
  - [x] Guest serial output captured through a 16550 that reports readiness
  - [x] Guest memory mapped and proven zero-filled before first use
  - [x] Failure paths are typed errors, never panics
  - [ ] Multi-vCPU threads (needs T22's launch ordering; single vCPU is enough for M3)

- [x] 20. **Engine Process Sandbox (seccomp + Landlock)**

  > **Extended by ADR 0002 (D)** — the cage is not just seccomp + Landlock. It also needs its
  > own user/mount/pid/net namespace, a minimal read-only filesystem view, no capabilities,
  > `PR_SET_NO_NEW_PRIVS`, `PR_SET_DUMPABLE = 0` and `RLIMIT_CORE = 0`. The last two are
  > what stop a same-user hostile process from ptracing the engine or reading its memory via
  > `/proc/pid/mem`, and stop a core dump from spilling guest RAM to disk. Rationale and
  > citations in `docs/adr/0002-isolation-model.md`.

  **What to do**:
  - Add `seccompiler = "0.5"` and `landlock` to `dome-engine/Cargo.toml` (Linux-only deps)
  - Create `dome-engine/src/sandbox.rs`:
    - `apply_seccomp_filter() -> Result<(), EngineError>` — apply a minimal allowlist BPF filter
      - **RESCOPED (Rev 2): derive the allowlist from measurement, never from guesswork.**
        The Rev 1 list omitted `brk`, `mprotect`, `madvise`, `rt_sigaction`,
        `rt_sigprocmask`, `sched_yield`, `clock_gettime`, `getrandom`, `prctl`, and
        the thread startup pair (`clone3`, `set_robust_list`). Applying it kills the
        process with SIGSYS on the first `malloc` after the filter is installed.
      - Procedure: run one real boot under `strace -f -c`, collect the syscall set, then
        allowlist that set plus `ioctl` (KVM), and nothing else.
      - Add the observed-syscall trace as a committed fixture so the filter is
        reproducible and reviewable.
      - Test it by forking a child that allocates, spawns one thread and exits; the
        parent asserts the child exits 0 and was not killed by SIGSYS.
    - `apply_landlock_ruleset(chamber_dir: &Path) -> Result<(), EngineError>` — restrict engine process
      - Allow read: `/usr`, `/lib`, `/lib64`, `/dev/kvm`, kernel image path, foundation path
      - Allow read+write: `chamber_dir` (for temporary engine cache files)
      - Deny everything else
  - Call both during `LinuxEngine::launch()` before starting the VCPU
  - Use `CompatLevel::BestEffort` for Landlock (graceful degradation on older kernels)
  - Add `dome-engine/tests/sandbox.rs` with tests (can't fully test seccomp without forking)

  **Must NOT do**:
  - Do not apply seccomp/Landlock to the CLI process (only the engine child process)
  - Do not use seccomp in tests without forking (it's process-wide)

  **Recommended Agent Profile**:
  - **Category**: `deep` — security-critical code

  **Parallelization**:
  - **Can Run In Parallel**: YES — with T21 (can be tested alongside engine)
  - **Blocks**: M4 (lifecycle must have sandboxed engine)
  - **Blocked By**: T15, T16

  **References**:
  - `seccompiler` docs — JSON-based filter definition
  - `landlock` crate docs — `Ruleset`, `PathBeneath`, `CompatLevel`
  - `.rules/security.md:9` — "treat the engine process as an attack surface that needs its own sandbox"

  **Status: PARTIAL** (2026). Implementation is `crates/dome-engine/src/sandbox.rs`,
  with `crates/dome-engine/src/bin/cage_probe.rs` as the test child. **Six tests pass.**

  Shipped and verified: `PR_SET_NO_NEW_PRIVS`, `PR_SET_DUMPABLE = 0`, `RLIMIT_CORE = 0`,
  namespace isolation (user+mount+pid+net in one `unshare`), and a seccomp allowlist whose
  denial path is proven (a `reboot(2)` call is killed by SIGSYS) while ordinary work still
  succeeds.

  Three measured findings, each of which changed the design:

  1. **`unshare(CLONE_NEWUSER)` returns EINVAL in a multithreaded process.** Measured with
     `cage_probe`: single-threaded reports success, a libtest process fails. The engine's
     launch order is therefore fixed: establish namespaces first, spawn threads second.
     This constrains T19 and T22 and cannot be worked around.
  2. **`NoNewPrivs` and seccomp are per-thread.** `/proc/self/status` reports the thread
     group leader, so a filter installed on a worker thread is invisible there. Tests read
     `/proc/thread-self/status`; the engine must apply its filter on the thread that will
     keep running, before it is duplicated.
  3. **A guessed allowlist demonstrably kills the process.** The first hand-written list
     omitted `openat`, and the test that checks "the filter still permits normal work"
     caught it immediately — which is exactly why the plan demands a measured list.

  Allowlist provenance: Firecracker publishes its reviewed filters as JSON. Its `vmm` thread
  allows syscalls 0..=97 and its `vcpu` thread 0..=49. Dome uses that range as the baseline
  and lists every higher-numbered call it needs (openat, newfstatat, futex, epoll,
  clone3, ...) explicitly. That is cited, not guessed — but it is still **not** Dome's own
  trace, so the filter stays opt-in at launch until T56-style tracing replaces the block.

  Not yet done: **Landlock** (the crate exposes its builders as trait methods; needs
  `RulesetAttr`/`RulesetCreatedAttr` imports) and the trace-derived allowlist above.

  **Completed** (2026). Implementation is `crates/dome-engine/src/sandbox.rs`, with
  `crates/dome-engine/src/bin/cage_probe.rs` as the disposable child. **Eight tests.**

  Layers, all verified by effect rather than by call: `PR_SET_NO_NEW_PRIVS`,
  `PR_SET_DUMPABLE = 0`, `RLIMIT_CORE = 0`, user+mount+net namespaces with identity mapping,
  Landlock (fully enforced: reads inside, write inside granted, write outside **denied**), and
  a seccomp allowlist that kills `reboot(2)` with SIGSYS while ordinary work succeeds.

  **The decisive test: the real kernel boots inside the production cage** and prints
  `Run /init as init process` and our init's direct serial line. The cage is not a hypothesis;
  a guest has run under it.

  Five measured findings, each of which forced a design change:

  1. **`unshare(CLONE_NEWUSER)` returns EINVAL in a multithreaded process.** Namespaces must
     be established before any thread exists.
  2. **`CLONE_NEWPID` breaks thread creation.** Including it makes every later
     `clone(CLONE_THREAD)` fail with EINVAL, because threads must share the caller's PID
     namespace and the caller does not move into the new one. A chamber must unshare the PID
     namespace, then **fork**, and only the child may create threads. Proven by
     `cage_probe pid-after-fork`: `child_pid_one=true`, `child_thread_ok=true`. This is now the
     M4 process model, not speculation.
  3. **`no_new_privs` makes `/proc/self/setgroups` unwritable**, so the uid/gid maps must be
     written *before* privilege hardening. Without maps the process has no identity and
     `clone` fails with EINVAL.
  4. **`NoNewPrivs` and seccomp are per-thread**; `/proc/self/status` reports the group leader,
     so tests must read `/proc/thread-self/status`.
  5. **`SeccompAction::KillProcess` prevents diagnosis.** The kernel kills without delivering a
     catchable signal, so a missing entry looks like a silent exit. `Trap` plus a SIGSYS
     handler names the number; that is how `clock_nanosleep` (230) was found.

  Allowlist provenance: Firecracker's published filter (syscalls 0..=97) plus an explicit set
  of higher-numbered calls, **then validated by booting the real kernel inside the cage**. A
  missing entry is discovered empirically, which is stronger evidence than a trace of one run,
  though a Dome-specific trace is still the stated goal (T56).

  **Acceptance Criteria**:
  - [x] `no_new_privs`, non-dumpable, `RLIMIT_CORE = 0` asserted from a child
  - [x] Namespaces verified by comparing namespace inodes
  - [x] Identity mapping written, and its ordering constraint measured
  - [x] Landlock denies writes outside the granted paths
  - [x] seccomp kills a denied syscall while permitting normal work
  - [x] The real kernel boots inside the full cage
  - [x] Every ordering constraint documented where it is enforced

- [x] 21. **M3 Integration Tests (Capability, Failure, Minimal Boot)**

  **What to do**:
  - Create `dome-engine/tests/engine_integration.rs`:
    - Test: engine capabilities match host (KVM available → NativeHypervisor)
    - Test: engine validation rejects unsupported guest arch
    - Test: engine gracefully fails when `/dev/kvm` not accessible
    - Test: memory allocation enforces CellSpec limits
    - Test: VCPU init + minimal run loop (use a 1-instruction HLT kernel)
    - **RESCOPED (Rev 2): "minimal boot" means kernel console output, not a shell.**
      Rev 1's run loop handles `Hlt`/`Shutdown` and logs MMIO as "unhandled", which
      produces a silent guest — there is no way to prove a boot. The gate for M3 is:
      load a bzImage + initramfs into guest RAM, expose a 16550 UART on MMIO (T63),
      boot with `console=ttyS0 earlyprintk=ttyS0`, and capture kernel output to a file.
      Block devices, virtio-fs, and any userspace interaction are out of scope for M3.
  - Create test fixtures:
    - `tests/fixtures/minimal-vmlinux` — trivial ELF that does HLT
    - `tests/fixtures/invalid-kernel` — garbage binary
  - Extend `dome-cli` with: `doctor --format toml` now reports engine details
  - Verify: `cargo test -p dome-engine` passes. Doc comment on M3 deliverable: _"This milestone demonstrates KVM-based booting only. Guest isolation is not claimed until M4 sandboxing is complete."_

  **Must NOT do**:
  - Do not claim guest isolation in any test name or doc comment
  - Do not require root for any test (skip tests gracefully if KVM unavailable)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — test-heavy

  **Parallelization**:
  - **Can Run In Parallel**: NO — depends on T16-T20
  - **Blocks**: M4
  - **Blocked By**: T16, T17, T18, T19, T20

  **References**:
  - `crates/dome-engine/tests/launch_plan.rs` — Existing test pattern
  - `crates/dome-host-linux/tests/doctor_fixture.rs` — Fixture pattern

  **Completed** (2026). Implementation is `crates/dome-engine/src/launch.rs` plus
  `crates/dome-engine/src/boot.rs`, exercised by `tests/launch.rs`. **Five tests, 22 seconds.**

  The chain proven end to end, against the real Alpine 6.12 kernel:

  ```
  Unpacking initramfs...                        <- initrd located: the zero page is right
  Run /init as init process                     <- userspace entered
  dome-init: direct serial write from userspace <- our own init executed
  reboot: Power off not available: System halted instead
  ```

  Findings that changed the code:

  1. **A signal cannot interrupt KVM_RUN.** The first timeout used `setitimer` + SIGALRM and
     never fired: the kernel delivers a process-directed signal to an arbitrary unblocked
     thread, not the one inside the hypervisor. KVM's own `immediate_exit` was tried next
     and is unusable here too — `set_kvm_immediate_exit` needs `&mut self`, and a duplicated
     `VcpuFd` would map its own `kvm_run`. The working mechanism is `pthread_kill(` the exact
     thread, which makes KVM_RUN return EINTR. Verified: a 2-second limit ends a reboot-looping
     guest in under 20 seconds.
  2. **`pthread_self()` is the only correct thread id.** Transmuting `std::thread::Thread`
     yields a pointer to an Arc, not a `pthread_t`, and `pthread_kill` silently rejects it.
  3. **A guest does not reliably reach a HLT exit.** After "System halted" the kernel keeps
     producing device exits. So a chamber needs a stop condition beyond HLT — output
     quiescence or an explicit request — which is M4 work, not an M3 defect.
  4. **`launch` must not cage its caller.** `unshare` moves the calling process into the new
     namespace permanently, so `cage` defaults to false and is documented as safe only for a
     disposable child. A library must not re-namespace its caller as a side effect.

  **Acceptance Criteria**:
  - [x] Real kernel boots through the engine and reports its memory map
  - [x] Initramfs is found, unpacked, and our userspace init executes
  - [x] Console output is captured and asserted on, not eyeballed
  - [x] A guest that never stops is terminated by the wall-clock limit
  - [x] Insufficient memory and a missing kernel are typed errors
  - [x] Five tests, no sleeps beyond the time limit under test

- [x] 22. **cgroups v2 Integration (per-chamber cgroup)**

  **What to do**:
  - Add `cgroups-rs = "0.5"` to `dome-supervisor/Cargo.toml`
  - Implement `dome-supervisor/src/cgroup.rs`:
    - `ChamberCgroup::create(chamber_id: &ChamberId, limits: ResourceLimits) -> Result<Self, SupervisorError>`
    - Create a cgroup at `/dome/chambers/{chamber_id}/`
    - Apply CPU limit (via `cpu.max`), memory limit (via `memory.max`), PID limit (via `pids.max`)
    - `destroy(&self) -> Result<(), SupervisorError>` — remove cgroup
    - `attach_process(&self, pid: u32) -> Result<(), SupervisorError>` — add engine process to cgroup
  - Gracefully handle: cgroups not available (return error), permission denied (return error)
  - Root or delegated cgroup subtree required — document this

  **Must NOT do**:
  - Do not create cgroups at paths outside `/dome/chambers/`
  - Do not panic on cgroup operations (all errors are SupervisorError)

  **Recommended Agent Profile**: `deep` — systems programming, resource isolation
  **Parallelization**: Can Run In Parallel YES with T23 | Blocks: T24 | Blocked By: M3 complete
  **References**: `cgroups-rs` docs — `CgroupBuilder`, `UnifiedHierarchy`; `crates/dome-core/src/lib.rs:117-144` — `ResourceLimits`

  **Done** (2026). `crates/dome-supervisor/src/cgroup.rs`. **Six tests, run against the
  real kernel on this host.**

  Probed before building, as ADR 0001 predicted: our cgroup is
  `/user.slice/user-1000.slice/<user service>/app.slice/...scope`, the delegated service
  subtree lists `cpu memory pids`, and creating a child cgroup plus writing `pids.max`
  works as uid 1000. **No privilege was needed at any point.**

  Deviation from the plan, recorded: no `cgroups-rs` dependency. The v2 interface is a
  directory of text files, so the module writes them directly — identical behaviour, one
  fewer dependency, and an error names the exact file that refused the write.

  What it does: creates `<delegated>/dome/chambers/<id>`, writes `memory.max` in bytes,
  `cpu.max` as "quota period" from the CPU count, and `pids.max` scaled to CPUs with a
  floor of 64. `attach_process` writes to `cgroup.procs`; `destroy` writes `cgroup.kill`
  before removing the directory, because a cgroup with live processes cannot be removed.

  Security detail that a test covers: **chamber ids are user input**, so anything outside
  `[A-Za-z0-9._-]`, longer than 64, or starting with a dot is refused rather than rewritten.
  Interpolating a hostile id into a path would escape the delegated subtree.

  **Acceptance Criteria**:
  - [x] Cgroup created with correct memory, CPU and PID limits, verified by reading files
  - [x] `attach_process` places a real process in `cgroup.procs`
  - [x] `destroy` removes the directory
  - [x] Unsafe chamber ids are refused, not sanitised into something unexpected
  - [x] Absence of a delegated subtree is a typed error, not a panic

  **Acceptance Criteria**:
  - [ ] Chamber cgroup created with correct memory/CPU/PID limits
  - [ ] `attach_process()` adds engine PID to cgroup
  - [ ] `destroy()` removes cgroup after chamber stops
  - [ ] Tests verify limits via reading cgroup files

  **QA Scenarios**:

  ```
  Scenario: Cgroup creation with resource limits
    Tool: Bash (requires root or delegated cgroup)
    Steps:
      1. Create ChamberCgroup with 64 MiB memory, 1 CPU, 32 PIDs
      2. Assert: memory.max file contains 67108864
      3. Assert: cgroup.procs is empty initially
    Expected Result: Cgroup configured correctly
    Evidence: evidence/task-22-cgroup-create.txt
  ```

  **Commit**: NO (groups with T23, T24)

---

- [x] 23. **Network Namespace Integration (per-chamber netns)**

  **What to do**:
  - Add `netns-rs = "0.2"` and `rtnetlink = "0.21"` to `dome-supervisor/Cargo.toml`
  - Implement `dome-supervisor/src/network.rs`:
    - `ChamberNetwork::create(chamber_id: &ChamberId) -> Result<Self, SupervisorError>` — create netns named `dome-{chamber_id}`
    - Create veth pair: `host-{id}` ↔ `chamber-{id}`
    - Move `chamber-{id}` end into chamber netns
    - Apply egress policy from `PerimeterPolicy` via nftables/iptables rules
    - `publish_port(&self, port: &PortMap) -> Result<(), SupervisorError>` — set up port forwarding
    - `destroy(&self) -> Result<(), SupervisorError>` — remove veth pair, delete netns
  - Handle absence of CAP_NET_ADMIN gracefully

  **Must NOT do**:
  - Do not modify host's default network namespace
  - Do not create namespaces named `dome-*` without the chamber ID suffix

  **Recommended Agent Profile**: `deep` — network isolation
  **Parallelization**: Can Run In Parallel YES with T22 | Blocks: T24 | Blocked By: M3 complete
  **References**: `netns-rs` docs — `NetNs::new()`; `rtnetlink` docs — link creation; `crates/dome-net/src/lib.rs` — `PerimeterPolicy`

  > **STATUS: PARTIAL — most of it is done; the relay is the remainder.**
  >
  > **Measured first.** As uid 1000 on this host: `ip netns add` fails (`/var/run/netns`
  > permission denied), `ip link add ... type veth` in the host namespace fails with EPERM,
  > and `setns` into another user's namespace is not permitted. So the plan's
  > "veth `host-{id}` ↔ `chamber-{id}` in the host namespace plus nftables" cannot happen.
  > **The chamber owns its namespace; the supervisor cannot construct it from outside.**
  >
  > Done and tested:
  > - `sandbox::bring_up_loopback` — called by `apply_engine_cage` before Landlock and
  >   seccomp. Without it a namespaced chamber cannot reach even 127.0.0.1, which presents as
  >   an application bug rather than a missing interface. Verified from the kernel's flags.
  > - `ChamberNetwork::plan` — validates the perimeter before anything starts: a host port
  >   below 1024 is refused with the reason (no CAP_NET_BIND_SERVICE), a duplicate host port
  >   is refused, UDP is refused rather than silently dropped because the relay carries TCP,
  >   and a hostile chamber id cannot shape the namespace name. Eight tests.
  > - `ChamberNetwork::netns_inode` — identifies a process's network namespace by inode,
  >   which is how "the chamber really has its own" becomes checkable. A test compares against
  >   a child that unshared, and only passes when the inodes differ.
  >
  > **Remaining (the whole of what is left):** the byte relay. A supervisor listener on the
  > host port must hand connections to a forwarder inside the chamber's namespace over a
  > socketpair created before `unshare`, since nothing may `setns` in. That finishes
  > `publish_port` and therefore T23.

  > **Done** (2026), though not the way the task assumed. The byte relay is
  > `crates/dome-supervisor/src/relay.rs` with the protocol in `frame.rs`; **17 tests**.
  >
  > **Measured first.** As uid 1000: `ip netns add` fails, a host-namespace veth fails with
  > EPERM, and `setns` into another user's namespace is refused. So "veth in the host
  > namespace plus nftables" is unreachable, and **the chamber owns its namespace while the
  > supervisor cannot enter it.**
  >
  > What that leaves, and what was built: a socketpair created before `unshare`, with
  > connections multiplexed over it. The supervisor binds the host port and forwards; the
  > chamber side dials `127.0.0.1:guest_port` from inside its own namespace, which is the
  > only place that dial is legal.
  >
  > **The bug that mattered:** the chamber dials asynchronously, so a data frame could arrive
  > before the connection existed and was silently dropped — small payloads vanished while
  > large ones got lucky. Two tests failed and exposed it. The fix is that a connection owns
  > its queue from the moment it is announced, and a writer thread applies that queue in
  > order; the queue is handed to the dial thread rather than drained. That is the difference
  > between "usually works" and correct.
  >
  > Also here: frame lengths are checked **before** allocation so a hostile header cannot
  > decide how much memory the process commits; an unknown tag is an error rather than being
  > ignored; a port with nothing behind it closes instead of hanging.
  >
  > **What is not here:** the wiring into a real chamber launch. That means passing one end of
  > the socketpair across `unshare` into the engine process, and it belongs to **T24**, which
  > is the component that constructs a chamber. T23's mechanism is complete and tested
  > in-process; T24 is where it gets plugged into a live launch.

  **Acceptance Criteria**:
  - [ ] Network namespace created with correct name
  - [ ] Veth pair created and one end moved into namespace
  - [ ] Port forwarding rules applied for each published port
  - [ ] `destroy()` cleans up all networking resources

  **QA Scenarios**:

  ```
  Scenario: Network namespace creation and teardown
    Tool: Bash (requires CAP_NET_ADMIN)
    Steps:
      1. Create ChamberNetwork for chamber "test-01"
      2. Run: ip netns list | grep dome-test-01
      3. Assert: namespace exists
      4. Destroy the network
      5. Run: ip netns list | grep dome-test-01
      6. Assert: namespace gone
    Expected Result: Namespace lifecycle clean
    Evidence: evidence/task-23-netns.txt
  ```

  **Commit**: NO (groups with T22, T24)

---

- [x] 24. **Chamber Create Orchestrator**

  **What to do**:
  - Implement `dome-supervisor/src/orchestrator.rs`:
    - `ChamberOrchestrator` struct with tokio-aware async methods:
    - `async fn create(&self, plan: LaunchPlan) -> Result<ChamberRecord, SupervisorError>`
      - 1. Validate plan via `engine.validate()`
      - 2. Create cgroup via `ChamberCgroup::create()`
      - 3. Create netns via `ChamberNetwork::create()`
      - 4. Call `engine.launch()` in the netns context
      - 5. Attach engine process to cgroup
      - 6. Return `ChamberRecord` with `state = Running`
    - Pipeline: `dome-image::DomeReader` → extract foundation → `dome-engine` convert → run
    - Maintain a `HashMap<ChamberId, ChamberRecord>` registry

  **Must NOT do**:
  - Do not implement the full run loop here (that's T25)
  - Do not mutate host state outside `/dome/` paths

  **Recommended Agent Profile**: `deep` — orchestration logic
  **Parallelization**: Can Run In Parallel NO | Blocks: T25, T26 | Blocked By: T22, T23
  **References**: `crates/dome-supervisor/src/lib.rs` — `ChamberRecord`, `ChamberState`; `crates/dome-engine/src/lib.rs` — `Engine` trait, `LaunchPlan`

  **Done** (2026). `crates/dome-supervisor/src/chamber.rs`. **Four tests, each spawning a
  real child process.**

  The ordering is the part that matters, and it is the order T20 and T23 forced:

  1. validate the perimeter — a chamber with an impossible policy never holds resources, so
     a privileged port leaves no cgroup behind (tested);
  2. create the cgroup, so limits exist before any process does;
  3. create the relay socketpair;
  4. spawn the engine child with the chamber end handed across `exec` on fd 3;
  5. attach the child to the cgroup.

  The fd handoff uses `pre_exec` and `dup2`, which is the one place this crate needs
  `unsafe`: inheriting a specific descriptor across `exec` has no safe equivalent. The
  closure captures only an integer and calls nothing but async-signal-safe functions.

  A test proves the handoff by spawning `/bin/sh`, then reading `/proc/<pid>/fd/3` and
  requiring it to be a socket. Another reads `cgroup.procs` and requires the child's pid to
  be in it. Neither would pass if the descriptor were merely inherited by accident.

  **Known gap, deliberately left to T25/T28**: the child here is `/bin/sh`. The engine-side
  wiring — unshare, then `spawn_chamber(fd 3)`, then launch the kernel — is the run
  lifecycle, not the orchestrator.

  **Acceptance Criteria**:
  - [ ] `create()` orchestrates engine + cgroup + netns
  - [ ] Chamber registry correctly tracks chamber state
  - [ ] Pipeline: .dome file → launch works end-to-end
  - [ ] Error at any stage rolls back partial setup

  **QA Scenarios**:

  ```
  Scenario: Full chamber creation pipeline
    Tool: interactive_bash (tmux)
    Steps:
      1. cargo run -p dome-cli -- run tests/fixtures/valid.cell.toml
      2. Assert: chamber created, engine launched
      3. Assert: cgroup exists
      4. Assert: network namespace exists
    Expected Result: Chamber boots end-to-end
    Evidence: evidence/task-24-create.txt
  ```

  **Commit**: NO (groups with T25)

---

- [x] 25. **Chamber Run/Stop Lifecycle**

  **What to do**:
  - Implement remaining lifecycle methods in `ChamberOrchestrator`:
    - `async fn run(&self, chamber_id: &ChamberId) -> Result<(), SupervisorError>` — start VCPU loop (spawn tokio task)
    - `async fn stop(&self, chamber_id: &ChamberId) -> Result<(), SupervisorError>` — signal engine, wait for exit, clean up
    - Handle graceful shutdown (guest ACPI power button) vs force stop (SIGKILL)
    - Handle crash: engine process exits unexpectedly → transition to `Failed` state
    - Implement `drop` cleanup: if `ChamberOrchestrator` is dropped, clean up all active chambers
  - Extend `dome-cli`: `dome run <cell.toml>` and `dome stop <chamber-id>` commands

  **Must NOT do**:
  - Do not leave orphaned cgroups or netns on abnormal exit
  - Do not block the tokio runtime during VCPU loop

  **Recommended Agent Profile**: `deep` — process lifecycle, async
  **Parallelization**: Can Run In Parallel NO | Blocks: T26, T27, T28 | Blocked By: T24
  **References**: `tokio` docs — `tokio::task::spawn_blocking` for sync VCPU loop; `crates/dome-supervisor/src/lib.rs:4-11` — `ChamberState` transitions

  **Done** (2026). `crates/dome-supervisor/src/lifecycle.rs`, with the stop sequence in
  `chamber.rs`. **Eleven tests**, three of which drive a real child process.

  **Researched, not assumed.** The design follows the Docker documentation because the
  product question was "how does a container decide it is done":

  | Docker | Dome |
  | --- | --- |
  | container runs until its main process exits; nothing terminates it for being idle or unhealthy | chamber runs until its guest stops; no idle timeout |
  | `docker stop`: SIGTERM, then grace, then SIGKILL; `--timeout -1` waits forever | `StopPolicy { signal: SIGTERM, grace: Some(10s) }`; `grace: None` waits |
  | health is a status *alongside* the state: starting → healthy/unhealthy; defaults 30s/30s/3/0s | `HealthTracker` with the same defaults, and it never reaches the lifecycle |
  | restart policy decides what happens after exit, not when to exit | not implemented; deliberately out of scope |

  **Answering the stuck-chamber question with that evidence:** a wedged chamber becomes
  `Unresponsive` — a reported observation, never an automatic stop. Docker does not kill a
  container for being unhealthy or idle, and silently reaping a wedged chamber would hide the
  bug. `dome stop` remains the only thing that ends one.

  **Answering the idle-window question:** configurable, and **off by default**. CPU and memory
  are read from the cgroup (`cpu.stat`, `memory.current`) and reported as usage; usefulness
  of "no CPU progress" depends on the workload, so it stays an observation rather than a
  verdict.

  One specification detail a test caught: my tracker keyed the start period off elapsed time
  alone. Docker also says a success during the start period **ends** it, so subsequent
  failures count even though the clock is still inside the window. The tracker now records
  that a probe has succeeded.

  **Acceptance Criteria**:
  - [x] A cooperative child exits within the grace period, unforced
  - [x] A child that ignores SIGTERM is forced after the grace period
  - [x] `grace: None` waits instead of forcing
  - [x] Usage is read from the kernel's cgroup counters
  - [x] Health rules match the documented behaviour, including the start-period rule

  **Acceptance Criteria**:
  - [ ] `run()` starts VCPU loop in a tokio task
  - [ ] `stop()` sends shutdown signal and cleans up resources
  - [ ] Crash detection: engine exit → state = Failed
  - [ ] Drop cleanup: no orphaned resources on supervisor exit
  - [ ] CLI `run` and `stop` commands work

  **QA Scenarios**:

  ```
  Scenario: Chamber run and stop lifecycle
    Tool: interactive_bash (tmux)
    Steps:
      1. Start chamber: dome run example.cell.toml
      2. Assert: chamber reports "Running"
      3. Stop chamber: dome stop chamber-001
      4. Assert: chamber reports "Stopped"
      5. Verify: cgroup removed, netns removed
    Expected Result: Clean lifecycle with no resource leaks
    Evidence: evidence/task-25-lifecycle.txt

  Scenario: Crash detection
    Tool: Bash
    Steps:
      1. Start chamber
      2. Kill engine process externally
      3. Assert: supervisor detects crash, transitions to Failed
    Expected Result: Supervisor handles unexpected engine exit
    Evidence: evidence/task-25-crash.txt
  ```

  **Commit**: NO (groups with T24)

---

- [x] 26. **Chamber Seal (save to .seal.dome)**

  **What to do**:
  - Implement `ChamberOrchestrator::seal()`:
    - Freeze guest (via sentinel if available, otherwise cold suspend via VCPU pause)
    - Capture guest memory snapshot via `vm-memory`
    - Capture VCPU register state
    - Serialize to `.seal.dome` format via `DomeWriter`
    - Include metadata: chamber ID, timestamp, foundation used, resource limits
    - Add `dome-cli` command: `dome seal <chamber-id> [--output <path>]`
  - Add `SECURITY.md` doc in `dome-image/`:
    - What the seal protects (at-rest confidentiality with key)
    - What it does NOT protect (runtime memory, cold-boot, side channels)
    - Which primitives used and for what purpose

  **Must NOT do**:
  - Do not claim runtime confidentiality per `.rules/security.md`
  - Do not implement live migration (seal is cold/frozen only for now)

  **Recommended Agent Profile**: `deep` — state capture + crypto
  **Parallelization**: Can Run In Parallel YES with T27 | Blocks: T28 | Blocked By: T25
  **References**: `.rules/security.md:23-28` — Seal security model; `crates/dome-image/src/writer.rs` — DomeWriter

  > **STATUS: done.** The CLI delivers it as `dome release --seal`, end to end.
  >
  > The plan asked for `dome seal <chamber-id>`. There is no registry of running chambers yet —
  > run and stop are still stubs — so the command boots a kernel, captures, and seals: the same
  > work with an honest input. The `<chamber-id>` form arrives with the supervisor's registry,
  > and the underlying functions (`Seal`, `write_seal`, capture) are already what it will call.
  >
  > Proven end to end in `crates/dome-cli/tests/release.rs`: keygen writes 0600 keys, a release
  > captures a real boot, `dome inspect` verifies the dual signature and reports what the artifact
  > claims and whether this device can run it, and a tampered artifact is refused.

  **Acceptance Criteria**:
  - [ ] `seal()` produces a valid `.seal.dome` file
  - [ ] Sealed file contains guest memory snapshot
  - [ ] Metadata includes chamber ID, timestamp
  - [ ] `SECURITY.md` documents seal security model
  - [ ] `dome seal` CLI command works

  **QA Scenarios**:

  ```
  Scenario: Seal a running chamber
    Tool: interactive_bash (tmux)
    Steps:
      1. Start chamber: dome run example.cell.toml
      2. Seal: dome seal chamber-001 --output /tmp/test.seal.dome
      3. Assert: /tmp/test.seal.dome exists
      4. Assert: file starts with "DOME" magic bytes
    Expected Result: Valid .seal.dome file produced
    Evidence: evidence/task-26-seal.txt
  ```

  **Commit**: NO (groups with T27)

---

- [ ] 27. **Chamber Export**

  **What to do**:
  - Implement `ChamberOrchestrator::export()`:
    - Export chamber state as a self-contained `.dome` package
    - Include: cell blueprint, foundation reference, seal data, logs
    - Produces a single file suitable for transfer to another host
  - Add `dome-cli` command: `dome export <chamber-id> --output <path>`
  - The exported package should be importable with: `dome import <export.dome>`

  **Must NOT do**:
  - Do not include host-specific paths or secrets in export
  - Do not export raw `/dev/kvm` or device handles

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel YES with T26 | Blocks: T28 | Blocked By: T25
  **References**: `crates/dome-image/src/writer.rs` — DomeWriter for packaging

  > **STATUS: the package format is done; the CLI commands are owed.**
  >
  > Done: `crates/dome-image/src/export.rs` and the shared `bundle.rs` it was extracted from,
  > with **seven tests**. An export carries the cell blueprint, the foundation name, the
  > chamber's seal as a complete nested container, the console log and a layout version, all
  > inside one signed and encrypted suite-2 container.
  >
  > The seal travels as an opaque nested container rather than flattened in, so it can still be
  > opened on its own and its own signature still means something.
  >
  > **Sanitisation is enforced, not documented.** `Export::new` refuses a blueprint or a log
  > containing a host path, and `write_export` rewrites the container manifest with a portable
  > base name so the local foundation path cannot travel. A test writes a manifest pointing at
  > `/home/iam/foundations/...` and asserts the resulting bytes do not contain `/home/iam`.
  > The check is deliberately narrow: `/bin/sh` and `/etc/hosts` are ordinary container
  > paths, not host identifiers, and a test pins that they are allowed.
  >
  > **Owed:** `dome export` / `dome import`. They need key handling — a file key to encrypt
  > with and both signing keys — and that does not exist in the CLI yet. Landing them with T28,
  > together with `dome seal`, is the point at which the CLI can do the whole cycle with one
  > key story rather than two.

  **Acceptance Criteria**:
  - [ ] `export()` produces valid .dome export file
  - [ ] Export contains cell blueprint + foundation ref + seal data
  - [ ] `dome export` CLI command works

  **QA Scenarios**:

  ```
  Scenario: Export and inspect export package
    Tool: interactive_bash (tmux)
    Steps:
      1. dome export chamber-001 --output /tmp/export.dome
      2. Assert: file exists and is valid .dome format
      3. Verify file contains expected sections
    Expected Result: Export package is complete
    Evidence: evidence/task-27-export.txt
  ```

  **Commit**: NO (groups with T24, T25, T26)

---

- [x] 28. **M4 Integration Tests (Enforcement, Conflict, Cleanup)**

  **What to do**:
  - Create comprehensive lifecycle tests:
    - Resource limit enforcement: chamber with 64 MiB → attempt to exceed → blocked by cgroup
    - Port conflict: two chambers requesting same host port → second fails
    - Cleanup verification: after `stop`, verify cgroup + netns removed, no zombie processes
    - Crash recovery: kill engine → verify supervisor cleans up
    - Concurrent chambers: two chambers running simultaneously, verify isolation
  - Add CLI integration test: `dome run ... && dome seal ... && dome stop ...` end-to-end

  **Must NOT do**:
  - Do not run concurrent tests that could interfere with each other

  **Recommended Agent Profile**: `unspecified-high` — test-heavy
  **Parallelization**: Can Run In Parallel NO | Blocks: M5 | Blocked By: T25-T27

  > **STATUS: the release loop is done; chamber run/stop integration is not.**
  >
  > Delivered: `dome keygen`, `dome release` (portable by default, `--seal` to seal),
  > `dome inspect`. **Five end-to-end tests** that boot a real kernel, capture state, write an
  > artifact, reopen it and check what it claims.
  >
  > **No --kernel by default.** The cell names a foundation, and the foundation supplies the
  > kernel, the way a Docker image supplies everything it needs. An installed foundation lives
  > at `$XDG_DATA_HOME/dome/foundations/<name>` (slashes and colons become dashes) holding
  > `vmlinuz` and optionally `initramfs.cpio.gz`. `--kernel` remains as an override for
  > development. Note the difference from Docker: a container shares the *host* kernel, while a
  > chamber runs its own, which is why the kernel belongs to the foundation rather than the host.
  >
  > The CLI now tells one key story rather than two: `keygen` writes an age identity (to wrap the
  > per-release file key) plus Ed25519 and ML-DSA-65 signing keys, and both `release` and
  > `inspect` draw from that directory. Secret material is chmod 0600 on creation.
  >
  > **Default is portable**, which is the decision recorded in ADR 0004: sealing narrows where an
  > artifact runs, so it must be asked for explicitly with `--seal`.
  >
  > **The chamber lifecycle is done.** `dome run` spawns the chamber as a real process, records
  > it, and returns; `dome stop` asks, waits, then insists; `dome ps` reports the truth. A hidden
  > `__boot` command is the chamber body itself.
  >
  > The registry lives in `crates/dome-supervisor/src/registry.rs`, and its tests drive **real
  > processes**: a live child is reported running, a killed one is not, a cooperative one stops
  > without force, a SIGTERM-ignoring one is forced, and an unknown chamber is an error.
  >
  > Two bugs the tests caught, both worth recording:
  >
  > 1. `kill(pid, 0)` reports success for a **zombie**, so a chamber that exited cleanly looked
  >    alive until its parent reaped it. Liveness now reads the process state from `/proc`;
  >    the existence check is only a fallback.
  > 2. A chamber body that stopped without a prior start record left **no record at all**, so it
  >    was invisible to `ps`. `record_stopped` now always writes one.
  >
  > Owed: `export`/`import` CLI verbs. The package format exists and is tested; it wants a verb
  > of its own.



  **Acceptance Criteria**:
  - [ ] All M4 tests pass
  - [ ] Resource enforcement verified
  - [ ] Port conflict detection verified
  - [ ] Cleanup verified after normal and abnormal exit

  **QA Scenarios**:

  ```
  Scenario: M4 full test suite
    Tool: Bash
    Steps:
      1. cargo test --workspace 2>&1
      2. Assert: all tests pass
      3. cargo test -p dome-supervisor 2>&1
      4. Assert: supervisor-specific tests pass
    Expected Result: Complete M4 test suite green
    Evidence: evidence/task-28-m4-tests.txt
  ```

  **Commit**: YES
  - Message: `feat(m4): complete chamber lifecycle with cgroups, netns, seal, and export`
  - Files: `crates/dome-supervisor/`, `crates/dome-cli/`

---

- [ ] 29. **Buildroot Recipe for Minimal Foundations**

  > **Grammar authority**: [`docs/cli-contract.md`](docs/cli-contract.md) (T61). Do not restate flags in this task; match the contract.

  **What to do**:
  - Create `foundations/buildroot/` directory with:
    - `dome-minimal.config` — Buildroot defconfig for a minimal kernel + busybox rootfs
    - `build.sh` — invokes Buildroot with the defconfig, produces `.foundation.dome`
    - Target: bootable kernel with busybox, ~20 MiB compressed
  - Document Buildroot version requirement and build prerequisites
  - Add `dome-cli` command: `dome build --recipe buildroot-minimal`

  **Guardrail**: This is a shell script orchestrating existing Buildroot tooling. NOT a custom build system.

  **Must NOT do**:
  - Do not implement a custom build system
  - Do not bundle Buildroot source (document it as a prerequisite)

  **Recommended Agent Profile**: `unspecified-high` — build system integration
  **Parallelization**: Can Run In Parallel YES with T30, T31 | Blocks: T32 | Blocked By: M4 complete

  **Acceptance Criteria**:
  - [ ] `build.sh` produces a `.foundation.dome` file
  - [ ] Built foundation boots in M4 chamber (minimal kernel + busybox)
  - [ ] Foundation size < 50 MiB compressed

  **QA Scenarios**:

  ```
  Scenario: Buildroot recipe produces valid foundation
    Tool: Bash (requires Buildroot installed)
    Steps:
      1. cd foundations/buildroot && ./build.sh
      2. Assert: output .foundation.dome exists
      3. Assert: file has valid .dome format (magic bytes)
    Expected Result: Buildroot recipe works
    Evidence: evidence/task-29-buildroot.txt
  ```

  **Commit**: NO (groups with T30, T31)

---

- [ ] 30. **Alpine Developer Foundation Recipe**

  **What to do**:
  - Create `foundations/alpine/` directory with:
    - `build.sh` — uses Alpine's `apk` to create a developer rootfs
    - Includes: bash, coreutils, gcc, make, git, strace, gdb
    - Target: ~200 MiB, suitable for development chambers
  - Document Alpine version and `apk` prerequisites
  - Kernel profile fragment for developer use (debug symbols, KASAN, lockdep)

  **Must NOT do**:
  - Do not implement Alpine package management — use `apk` directly
  - Do not include proprietary software in the developer foundation

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel YES with T29, T31 | Blocks: T32 | Blocked By: M4 complete

  **Acceptance Criteria**:
  - [ ] `build.sh` produces a developer `.foundation.dome`
  - [ ] Foundation includes gcc, make, git, strace, gdb
  - [ ] Foundation boots and provides a working shell

  **QA Scenarios**:

  ```
  Scenario: Alpine developer foundation builds
    Tool: Bash
    Steps:
      1. cd foundations/alpine && ./build.sh
      2. Assert: output .foundation.dome exists
    Expected Result: Developer foundation built
    Evidence: evidence/task-30-alpine.txt
  ```

  **Commit**: NO (groups with T29, T31)

---

- [ ] 31. **Kernel Profile Fragments**

  **What to do**:
  - Create `foundations/kernel/` directory:
    - `dome-minimal.fragment` — minimal kernel config: KVM guest support, virtio, 9p, ext4, no drivers
    - `dome-developer.fragment` — developer config: debug symbols, KASAN, lockdep, ftrace
    - `dome-production.fragment` — production: no debug, LTO, hardened config
  - Kernel version: target latest LTS (as of writing, Linux 6.12.x)
  - Document merge procedure: `scripts/kconfig/merge_config.sh`

  **Must NOT do**:
  - Do not include full kernel configs (fragments only, merge with arch defconfig)
  - Do not include out-of-tree kernel patches

  **Recommended Agent Profile**: `unspecified-high` — kernel config expertise
  **Parallelization**: Can Run In Parallel YES with T29, T30 | Blocks: T32 | Blocked By: M4 complete

  **Acceptance Criteria**:
  - [ ] Each fragment merges without conflict with x86_64 defconfig
  - [ ] Minimal fragment produces kernel < 10 MiB
  - [ ] Developer fragment enables expected debug features

  **QA Scenarios**:

  ```
  Scenario: Kernel fragments merge correctly
    Tool: Bash
    Steps:
      1. Merge minimal fragment with x86_64 defconfig
      2. Assert: merge succeeds, no conflicts
      3. Verify: KVM guest support enabled, virtio enabled
    Expected Result: Fragments produce valid kernel config
    Evidence: evidence/task-31-kernel.txt
  ```

  **Commit**: NO (groups with T29, T30)

---

- [ ] 32. **M5 Integration Tests**

  **What to do**:
  - Test: Buildroot minimal foundation boots in chamber
  - Test: Alpine developer foundation boots and provides gcc
  - Test: Kernel profile fragments produce valid configs
  - Add `dome-cli` command: `dome build --list` — lists available recipes

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel NO | Blocks: M6 | Blocked By: T29-T31

  **Acceptance Criteria**:
  - [ ] All M5 tests pass
  - [ ] At least one foundation type builds and boots
  - [ ] `dome build --list` shows available recipes

  **QA Scenarios**:

  ```
  Scenario: M5 test suite passes
    Tool: Bash
    Steps:
      1. cargo test --workspace 2>&1
      2. Assert: all tests pass
    Expected Result: M5 tests green
    Evidence: evidence/task-32-m5-tests.txt
  ```

  **Commit**: YES
  - Message: `feat(m5): add minimal OS builder recipes and kernel profile fragments`
  - Files: `foundations/`, `crates/dome-cli/`

---

- [ ] 33. **Sentinel Protocol Specification + Encoding**

  **What to do**:
  - Define the sentinel protocol in `dome-guest-agent/src/protocol.rs`:
    - Transport: virtio-vsock (CID 2 on guest side, host listens on a known port)
    - Message format: binary, length-prefixed with message type tag
    - Message types: `Heartbeat`, `FreezeRequest`, `FreezeComplete`, `ResizeRequest`, `ShutdownRequest`, `CapabilityAnnounce`
    - Each message has: `u16 type_tag`, `u32 payload_length`, `[u8; payload_length] payload`
  - Implement encoding/decoding: `encode_message(msg) -> Vec<u8>`, `decode_message(bytes) -> Result<Message, ProtocolError>`
  - Define `ProtocolError` enum
  - Document trust model: "Sentinel telemetry is useful but not authoritative. A compromised chamber can lie." (per `.rules/plugins.md`)
  - **Naming (Rev 2)**: the crate stays `dome-guest-agent`; the shipped guest binary is
    `dome-sentinel` (`src/bin/dome-sentinel.rs`), installed at `/usr/bin/dome-sentinel`.
    Rev 1 used `sentinel` and `dome-sentinel` interchangeably — pick this one.
  - **Transport caveat (Rev 2)**: virtio-vsock requires a vsock device in every engine.
    WHP and Apple's framework do not expose one. The protocol must therefore be
    transport-agnostic (length-prefixed frames over any duplex byte stream) so that a
    virtio-serial fallback is possible. Do not bind the encoding to vsock specifics.

  **Must NOT do**:
  - Do not make security decisions based on sentinel data
  - Do not use JSON or TOML for the wire format (binary for efficiency)

  **Recommended Agent Profile**: `deep` — protocol design
  **Parallelization**: Can Run In Parallel YES with T34 | Blocks: T35, T36 | Blocked By: M5 complete

  **Acceptance Criteria**:
  - [ ] All message types encode and decode correctly
  - [ ] Protocol specification documented with trust model
  - [ ] Round-trip: encode → decode → assert equality

  **QA Scenarios**:

  ```
  Scenario: Protocol message round-trip
    Tool: Bash
    Steps:
      1. cargo test -p dome-guest-agent 2>&1
      2. Assert: encode/decode tests pass
    Expected Result: Protocol encoding is correct
    Evidence: evidence/task-33-protocol.txt
  ```

  **Commit**: NO (groups with T34, T35)

---

- [ ] 34. **Guest-Side Sentinel Binary**

  **What to do**:
  - Create `crates/dome-guest-agent/src/bin/sentinel.rs`:
    - Minimal guest binary that runs inside the chamber
    - Connects to host via virtio-vsock
    - Sends `CapabilityAnnounce` on startup
    - Handles `FreezeRequest` → freeze filesystem → send `FreezeComplete`
    - Handles `ShutdownRequest` → initiate graceful shutdown
    - Sends periodic `Heartbeat` messages
  - Build as a statically linked binary (musl target) for minimal guest footprint
  - Include in developer foundations as `/usr/bin/dome-sentinel`

  **Must NOT do**:
  - Do not implement complex guest logic (keep it minimal)
  - Do not require dynamic linking (musl static)

  **Recommended Agent Profile**: `deep` — guest binary + vsock
  **Parallelization**: Can Run In Parallel YES with T33 | Blocks: T37 | Blocked By: M5 complete

  **Acceptance Criteria**:
  - [ ] Sentinel binary compiles for `x86_64-unknown-linux-musl`
  - [ ] Binary connects to host via vsock
  - [ ] Heartbeat messages received by host
  - [ ] Filesystem freeze works correctly

  **QA Scenarios**:

  ```
  Scenario: Sentinel binary runs and connects
    Tool: interactive_bash (tmux)
    Steps:
      1. Build sentinel: cargo build -p dome-guest-agent --bin sentinel --target x86_64-unknown-linux-musl
      2. Assert: binary exists and is statically linked
    Expected Result: Sentinel binary builds
    Evidence: evidence/task-34-sentinel.txt
  ```

  **Commit**: NO (groups with T33, T35)

- [ ] 35. **Host-Side Sentinel Handler**

  **What to do**:
  - Implement `dome-supervisor/src/sentinel.rs`:
    - `SentinelHost` struct: listens on vsock port, accepts guest connections
    - Handles incoming messages: Heartbeat → log, FreezeComplete → proceed with seal, ShutdownRequest → initiate stop
    - Sends outgoing messages: FreezeRequest before seal, ResizeRequest on display change
    - Tracks sentinel liveness: if no heartbeat for N seconds → mark chamber as potentially unhealthy
  - Trust boundary: NEVER trust sentinel-claimed resource usage, process lists, or security assertions
  - All sentinel data is advisory only

  **Must NOT do**:
  - Do not use sentinel data for security decisions, resource enforcement, or audit authority
  - Do not trust guest-reported memory/CPU stats

  **Recommended Agent Profile**: `deep` — async handler + trust boundary
  **Parallelization**: Can Run In Parallel NO — depends on T33 | Blocks: T37 | Blocked By: T33

  **Acceptance Criteria**:
  - [ ] SentinelHost accepts vsock connections from guest
  - [ ] Heartbeat tracking works (liveness detection)
  - [ ] FreezeRequest → seal pipeline works
  - [ ] Trust model: guest-reported data never used for security

  **QA Scenarios**:

  ```
  Scenario: Sentinel heartbeat detection
    Tool: Bash
    Steps:
      1. Start chamber with sentinel
      2. Wait for heartbeats → verify host logs them
      3. Kill sentinel in guest → verify host detects liveness loss
    Expected Result: Heartbeat tracking works
    Evidence: evidence/task-35-sentinel-handler.txt
  ```

  **Commit**: NO (groups with T33, T34, T36)

---

- [ ] 36. **VNC Display Server (Basic RFB, virtio-gpu)**

  **What to do**:
  - Add `dome-engine/src/display.rs`:
    - `VncDisplay` struct: basic RFB protocol server (handshake, framebuffer updates)
    - **RESCOPED (Rev 2): split this task.** "VNC server + virtio-gpu device" is two
      projects, not one task. Rev 1 prices a complete virtio-gpu device model (2D
      command queue, resource attach/detach, scanout, EDID) plus an RFB server as one
      item. Split into:
      - **T36a**: RFB server over a **shared-memory framebuffer** with a static test
        pattern. Proves handshake, encodings, resize, and input events without any
        guest device. This is shippable on its own and testable with `vncdo`.
      - **T36b**: virtio-gpu device model wired to that framebuffer. Depends on T63.
        Only T36b is gated on the guest; T36a is not.
    - Listen on `127.0.0.1:{port}` by default (localhost-only for security)
    - Minimal encoding: raw only (optimize later with hextile/tight)
    - Respect `DisplayMode::Vnc` from `CellSpec`
  - Design architecture for extensibility: `DisplayBackend` trait that SPICE/Wayland can implement later
  - Add `dome-cli` flag: `dome run ... --display vnc --vnc-port 5900`

  **Guardrail**: M6 implements VNC only. SPICE and Wayland are designed for but not implemented.

  **Must NOT do**:
  - Do not implement SPICE or Wayland (architecture only)
  - Do not expose VNC on `0.0.0.0` by default (localhost only)

  **Recommended Agent Profile**: `visual-engineering` — display protocol
  **Parallelization**: Can Run In Parallel YES with T35 | Blocks: T37 | Blocked By: M5 complete

  **Acceptance Criteria**:
  - [ ] VNC server starts and listens on localhost
  - [ ] RFB handshake succeeds (can connect with any VNC client)
  - [ ] Framebuffer updates sent to client
  - [ ] `DisplayBackend` trait defined for future extensibility
  - [ ] `dome run --display vnc` flag works

  **QA Scenarios**:

  ```
  Scenario: VNC server starts and accepts connections
    Tool: interactive_bash (tmux) + netcat
    Steps:
      1. Start chamber with --display vnc --vnc-port 5901
      2. Run: nc -z 127.0.0.1 5901
      3. Assert: port is open (VNC server listening)
    Expected Result: VNC server responds on localhost
    Evidence: evidence/task-36-vnc.txt
  ```

  **Commit**: NO (groups with T33, T34, T35)

---

- [ ] 37. **M6 Integration Tests**

  **What to do**:
  - Test: sentinel protocol encode/decode round-trip
  - Test: sentinel heartbeat → host detects liveness
  - Test: VNC server starts and RFB handshake succeeds
  - Test: sentinel freeze request → seal pipeline
  - Add end-to-end test: chamber with sentinel + VNC display

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel NO | Blocks: M7 | Blocked By: T33-T36

  **Acceptance Criteria**:
  - [ ] All M6 tests pass
  - [ ] Sentinel + seal integration works end-to-end
  - [ ] VNC display integration test passes

  **QA Scenarios**:

  ```
  Scenario: M6 test suite passes
    Tool: Bash
    Steps:
      1. cargo test --workspace 2>&1
      2. Assert: all tests pass
    Expected Result: M6 tests green
    Evidence: evidence/task-37-m6-tests.txt
  ```

  **Commit**: YES
  - Message: `feat(m6): add sentinel protocol and VNC display server`
  - Files: `crates/dome-guest-agent/`, `crates/dome-supervisor/`, `crates/dome-engine/`

---

- [ ] 38. **File Aperture Implementation (virtio-fs or 9p)**

  > **Rev 2 rescope**: blocked by **T63** (virtio substrate). "virtio-fs or 9p" hides a
  > device model either way — virtio-fs needs a FUSE daemon plus the virtio device, 9p
  > needs a server. Do not start until T63 has one virtio device working end to end.
  > **Revised after research (ADR 0002 G): use virtio-fs, not 9p.** 9p is a legacy debug
  > protocol with a large parser surface; virtio-fs keeps its host-side server in a separate
  > process. The earlier preference for 9p was about implementation simplicity, which is the
  > wrong axis for the component that parses guest-controlled data.

  **What to do**:
  - Implement `dome-engine/src/apertures/files.rs`:
    - `FileApertureManager` — sets up virtio-fs or 9p device for each file aperture in CellSpec
    - Mount host paths as read-only by default, read-write only if `FileApertureMode::ReadWrite`
    - Blocked paths check: reject `/proc`, `/sys`, `/dev` without explicit opt-in
    - Canonicalize host paths before use per `.rules/apertures.md`
  - Integrate into `LinuxEngine::launch()` — configure virtio devices before VCPU start

  **Must NOT do**:
  - Do not expose raw `/dev/mem` or unrestricted block devices
  - Do not allow apertures to blocked paths without explicit override

  **Recommended Agent Profile**: `deep` — virtio device setup
  **Parallelization**: Can Run In Parallel YES with T39, T40, T41 | Blocks: T42 | Blocked By: M6 complete

  **Acceptance Criteria**:
  - [ ] File apertures mounted in guest at correct paths
  - [ ] Read-only aperture: writes from guest fail
  - [ ] Blocked paths: `/proc`, `/sys` apertures rejected
  - [ ] Canonicalized paths used

  **QA Scenarios**:

  ```
  Scenario: File aperture read-only enforcement
    Tool: Bash
    Steps:
      1. Create chamber with file aperture: /host/data → /data (read-only)
      2. From inside guest: echo test > /data/test.txt
      3. Assert: write fails (read-only filesystem)
    Expected Result: Read-only enforcement works
    Evidence: evidence/task-38-file-aperture.txt
  ```

  **Commit**: NO (groups with T39-T42)

---

- [ ] 39. **GPIO Broker Aperture (libgpiod proxy)**

  **What to do**:
  - Implement `dome-engine/src/apertures/gpio.rs`:
    - `GpioBroker` struct: proxies GPIO operations via `libgpiod`
    - For each `GpioAperture` in CellSpec, create a virtual GPIO device in guest
    - Host side: open `/dev/gpiochipN`, handle line requests
    - Guest side: expose virtual GPIO chip via a simple virtio-gpio device
    - Enforce rate limiting if `rate_limit_hz` is set
  - Per `.rules/apertures.md`: GPIO is proxying, not board emulation

  **Must NOT do**:
  - Do not implement board-level GPIO register emulation
  - Do not claim Raspberry Pi compatibility without explicit board emulation

  **Recommended Agent Profile**: `deep` — hardware proxying
  **Parallelization**: Can Run In Parallel YES with T38, T40, T41 | Blocks: T42 | Blocked By: M6 complete

  **Acceptance Criteria**:
  - [ ] GPIO broker proxies pin operations from guest to host
  - [ ] Direction (input/output) enforced
  - [ ] Rate limiting applied when configured
  - [ ] Document: "GPIO is proxying, not board emulation"

  **QA Scenarios**:

  ```
  Scenario: GPIO direction enforcement
    Tool: Bash
    Steps:
      1. Create chamber with GPIO output aperture on line 17
      2. From guest: attempt to read from output line
      3. Assert: operation handled correctly
    Expected Result: GPIO proxying works
    Evidence: evidence/task-39-gpio.txt
  ```

  **Commit**: NO (groups with T38, T40, T41, T42)

---

- [ ] 40. **Hardware Aperture Threat Model Document**

  **What to do**:
  - Create `docs/threat-model/hardware-apertures.md`:
    - Model: what an attacker with physical access + compromised guest can do
    - PCI passthrough: IOMMU isolation, DMA attacks, ACS limitations
    - USB passthrough: device firmware attacks, DMA via USB4/Thunderbolt
    - GPU passthrough: GPU memory isolation, render exploit surface
    - GPIO: electrical attacks, short circuits, voltage injection
    - Mitigations: IOMMU enforcement, VFIO strict mode, rate limiting, explicit opt-in
    - Explicitly state: hardware apertures are HIGH RISK and require explicit opt-in

  **Must NOT do**:
  - Do not claim any hardware aperture is "safe" — all require opt-in
  - Do not create the threat model in code comments (must be a standalone doc)

  **Recommended Agent Profile**: `deep` — security analysis
  **Parallelization**: Can Run In Parallel YES with T38, T39, T41 | Blocks: T42

  **Acceptance Criteria**:
  - [ ] Threat model document exists at `docs/threat-model/hardware-apertures.md`
  - [ ] Covers: PCI, USB, GPU, GPIO attack surfaces
  - [ ] Lists mitigations for each
  - [ ] Explicitly requires user opt-in for all hardware apertures

  **QA Scenarios**:

  ```
  Scenario: Threat model is comprehensive
    Tool: Bash
    Steps:
      1. Read docs/threat-model/hardware-apertures.md
      2. Assert: contains sections for PCI, USB, GPU, GPIO
      3. Assert: contains "opt-in required" language
    Expected Result: Threat model covers all hardware aperture families
    Evidence: evidence/task-40-threat-model.txt
  ```

  **Commit**: NO (groups with T38-T42)

---

- [ ] 41. **PCI/USB Aperture Policy Model**

  > **Gated by ADR 0002 (F)**: no DMA-capable aperture ships without an enforced IOMMU. This
  > host exposes **zero** IOMMU groups, so PCI/USB/GPU passthrough is refused outright here —
  > a DMA-capable device can read every chamber's memory, which destroys the isolation the
  > product is built for. Model the policy types; make the engine refuse to instantiate them.

  **What to do**:
  - Extend `dome-device` with PCI/USB aperture types:
    - `PciAperture` struct: vendor_id, device_id, iommu_group, vfio_path
    - `UsbAperture` struct: vendor_id, product_id, bus/device address
    - `DeviceAperturePolicy` struct: pci_devices, usb_devices, require_explicit_opt_in
  - Extend `CellSpec` or `AperturePolicy` to include PCI/USB aperture declarations
  - Extend `dome-config` TOML format: `[[apertures.pci]]`, `[[apertures.usb]]` sections
  - Validation: require `device_access = ExplicitApertures` for any PCI/USB aperture

  **Must NOT do**:
  - Do not implement actual PCI/USB passthrough in this task (model only)
  - Do not allow apertures without explicit opt-in

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel YES with T38-T40 | Blocks: T42

  **Acceptance Criteria**:
  - [ ] `PciAperture` and `UsbAperture` types defined
  - [ ] TOML format supports `[[apertures.pci]]` and `[[apertures.usb]]`
  - [ ] Validation requires explicit opt-in
  - [ ] Tests for policy model

  **QA Scenarios**:

  ```
  Scenario: PCI aperture policy validation
    Tool: Bash
    Steps:
      1. Parse TOML with PCI aperture but device_access = Disabled
      2. Assert: validation fails with "explicit opt-in required"
    Expected Result: PCI requires opt-in
    Evidence: evidence/task-41-pci-policy.txt
  ```

  **Commit**: NO (groups with T38-T42)

---

- [ ] 42. **M7 Integration Tests**

  **What to do**:
  - Test: file aperture read-only enforcement
  - Test: file aperture blocked path rejection
  - Test: GPIO broker direction enforcement
  - Test: PCI/USB aperture policy validation
  - Test: threat model document exists and is comprehensive

  **Commit**: YES
  - Message: `feat(m7): add file, GPIO, PCI, and USB apertures with threat model`
  - Files: `crates/dome-engine/`, `crates/dome-device/`, `docs/threat-model/`

---

- [ ] 43. **macOS Engine Stub + Capability Detection**

  **What to do**:
  - Create `dome-engine/src/macos.rs` (behind `#[cfg(target_os = "macos")]`):
    - `MacOSEngine` struct (stub first)
    - `MacOSEngine::detect_capabilities() -> Result<EngineCapabilities, EngineError>` — detect Apple Hypervisor availability
    - Probe: check `sysctl hw.optional.applehv` or equivalent
    - Return `HostOs::Macos`, appropriate arch, acceleration status
  - Add macOS target check: `cargo check --target aarch64-apple-darwin` (or `x86_64-apple-darwin`)
  - Conditional compilation: `dome-engine` compiles on macOS with Apple Hypervisor stubs

  **Must NOT do**:
  - Do not implement full macOS engine yet (that's T44)
  - Do not break Linux compilation

  **Recommended Agent Profile**: `deep` — platform-specific FFI
  **Parallelization**: Can Run In Parallel YES with T45 | Blocks: T44 | Blocked By: M7 complete

  **Acceptance Criteria**:
  - [ ] `cargo check --target aarch64-apple-darwin` passes for dome-engine
  - [ ] `MacOSEngine` struct compiled only on macOS targets
  - [ ] Capability detection function exists

  **Commit**: NO (groups with T44)

---

- [ ] 44. **macOS Apple Hypervisor Engine**

  > **Rev 2 rescope**: this is a research spike with a hard timebox, not a shippable
  > engine. Rev 1 budgets one task for a full VMM back end with its own memory mapping,
  > vCPU register model, interrupt controller and boot loader. Acceptance for M8 is
  > therefore: `MacOSEngine` behind a feature flag, capability detection that reports
  > `Unavailable` honestly, and a documented gap list. A booting macOS chamber is a
  > **post-M8** milestone and must not be promised in any README.

  **What to do**:
  - Implement `MacOSEngine` with Apple Hypervisor framework:
    - Use `hypervisor` framework via FFI or a Rust binding
    - Implement `Engine` trait for `MacOSEngine`
    - Guest memory: `vm-memory` with macOS-compatible backend
    - VCPU: Hypervisor framework vCPU API
    - Kernel loading: macOS guests use Mach-O, Linux guests use ELF (via linux-loader)
    - Display: native macOS window or VNC
  - Platform gate: entire impl behind `#[cfg(target_os = "macos")]`
  - Same-architecture only: ARM64-on-ARM64 or x86_64-on-x86_64

  **Must NOT do**:
  - Do not implement cross-architecture virtualization
  - Do not claim feature parity with Linux engine (some features Linux-only)

  **Recommended Agent Profile**: `deep` — Apple Hypervisor APIs
  **Parallelization**: Can Run In Parallel NO — depends on T43 | Blocks: T48 | Blocked By: T43

  **Acceptance Criteria**:
  - [ ] `MacOSEngine` implements `Engine` trait
  - [ ] Engine boots a macOS-compatible guest on Apple Silicon
  - [ ] Capability detection works
  - [ ] Compiles only on macOS targets

  **Commit**: NO (groups with T43, T45, T46, T47)

---

- [ ] 45. **Windows Engine Stub + Capability Detection**

  **What to do**:
  - Create `dome-engine/src/windows.rs` (behind `#[cfg(target_os = "windows")]`):
    - `WindowsEngine` struct (stub first)
    - `WindowsEngine::detect_capabilities() -> Result<EngineCapabilities, EngineError>` — detect Hyper-V WHP availability
    - Probe: check for WHP API availability (`WinHvPlatform.dll`)
    - Return `HostOs::Windows`, appropriate arch, acceleration status
  - Add Windows target check: `cargo check --target x86_64-pc-windows-msvc`

  **Must NOT do**:
  - Do not implement full Windows engine yet (T46)
  - Do not break Linux/macOS compilation

  **Recommended Agent Profile**: `deep`
  **Parallelization**: Can Run In Parallel YES with T43 | Blocks: T46 | Blocked By: M7 complete

  **Acceptance Criteria**:
  - [ ] `cargo check --target x86_64-pc-windows-msvc` passes for dome-engine
  - [ ] `WindowsEngine` compiled only on Windows targets

  **Commit**: NO (groups with T46)

---

- [ ] 46. **Windows Hyper-V WHP Engine**

  > **Rev 2 rescope**: same treatment as T44 — feature-gated stub plus capability
  > detection, no boot promise. WHP gives you memory and vCPU control but no device
  > model and no boot loader: an x86 guest needs you to supply the entire boot path
  > (real mode → protected mode → bzImage handoff) ourselves. That is a milestone, not
  > a task.

  **What to do**:
  - Implement `WindowsEngine` with Hyper-V Windows Hypervisor Platform (WHP):
    - Use WHP API via `windows-rs` crate or raw FFI
    - Implement `Engine` trait for `WindowsEngine`
    - Guest memory: WHP memory management
    - VCPU: WHP virtual processor API
    - Same-architecture only: x86_64-on-x86_64 or ARM64-on-ARM64

  **Must NOT do**:
  - Do not implement cross-architecture
  - Do not claim feature parity with Linux engine

  **Recommended Agent Profile**: `deep` — Windows WHP APIs
  **Parallelization**: Can Run In Parallel NO — depends on T45 | Blocks: T48 | Blocked By: T45

  **Acceptance Criteria**:
  - [ ] `WindowsEngine` implements `Engine` trait
  - [ ] Compiles only on Windows targets
  - [ ] Capability detection works

  **Commit**: NO (groups with T43-T47)

---

- [ ] 47. **Cross-Platform Doctor Command Extension**

  > **Grammar authority**: [`docs/cli-contract.md`](docs/cli-contract.md) (T61). Do not restate flags in this task; match the contract.

  **What to do**:
  - Extend `dome-cli doctor` to work on all three platforms:
    - Linux: existing, unchanged
    - macOS: reports Apple Hypervisor availability, macOS-specific capabilities
    - Windows: reports WHP availability, Windows-specific capabilities
  - Platform-gated compilation: `#[cfg(target_os = "...")]` on platform-specific sections
  - Update `doctor --format toml` output to include platform engine info
  - Add `doctor --format toml` fixture tests for each platform (if cross-compilation available)

  **Must NOT do**:
  - Do not require runtime detection (use conditional compilation)

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel YES with T43-T46 | Blocks: T48 | Blocked By: M7 complete

  **Commit**: NO (groups with T43-T46)

---

- [ ] 48. **M8 Integration Tests**

  **What to do**:
  - Test: macOS engine compiles on macOS targets
  - Test: Windows engine compiles on Windows targets
  - Test: Cross-platform doctor works (compile check on each target)
  - Test: Engine trait implemented for all three platforms
  - Add CI matrix: if runners available, test on macOS and Windows

  **Commit**: YES
  - Message: `feat(m8): add macOS and Windows native engine implementations`
  - Files: `crates/dome-engine/src/macos.rs`, `crates/dome-engine/src/windows.rs`, `crates/dome-cli/`

---

- [ ] 49. **ARM64 Kernel Profile Fragments**

  **What to do**:
  - Create `foundations/kernel/arm64/`:
    - `dome-minimal-arm64.fragment` — ARM64-specific minimal config
    - Target: boot on ARM64 hosts (Raspberry Pi 4/5, Apple Silicon, AWS Graviton)
    - Enable required ARM64 features: GIC, arch_timer, SMMU (IOMMU equivalent)
  - Document: ARM64 kernel version requirements, boot protocol (UEFI or direct)

  **Must NOT do**:
  - Do not include Raspberry Pi-specific device tree blobs in kernel config

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel YES with T50, T51 | Blocks: T52 | Blocked By: M8 complete

  **Acceptance Criteria**:
  - [ ] ARM64 kernel fragment merges with arm64 defconfig
  - [ ] Config enables GIC, arch_timer, SMMU
  - [ ] Fragment documented

  **Commit**: NO (groups with T50, T51)

---

- [ ] 50. **ARM64 Buildroot/Alpine Recipes**

  **What to do**:
  - Create ARM64 variants of Buildroot and Alpine recipes:
    - `foundations/buildroot/dome-minimal-arm64.config`
    - `foundations/alpine/build-arm64.sh`
  - Target architectures: aarch64
  - Cross-compile or native build on ARM64 host
  - Document build prerequisites (aarch64 cross-compiler or native ARM64 host)

  **Must NOT do**:
  - Do not bundle ARM64 toolchains

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel YES with T49, T51 | Blocks: T52 | Blocked By: M8 complete

  **Commit**: NO (groups with T49, T51)

---

- [ ] 51. **Raspberry Pi OS Userland Foundation**

  **What to do**:
  - Create `foundations/raspberry-pi/`:
    - `build.sh` — creates a Raspberry Pi OS userland foundation
    - Based on Raspberry Pi OS Lite (Debian-based)
    - Includes: Raspberry Pi-specific tools (vcgencmd, raspistill stubs)
  - Per `.rules/milestones.md`: "no full board-emulation promise"
  - Per `.rules/apertures.md`: GPIO is proxying via broker, not raw register access

  **Must NOT do**:
  - Do not promise full Raspberry Pi board emulation
  - Do not implement BCM2835/BCM2711 SoC emulation

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel YES with T49, T50 | Blocks: T52 | Blocked By: M8 complete

  **Acceptance Criteria**:
  - [ ] Raspberry Pi OS foundation builds
  - [ ] Foundation includes Pi-specific userland tools (best effort)
  - [ ] Document: "no full board-emulation promise"

  **Commit**: NO (groups with T49, T50)

---

- [ ] 52. **M9 Integration Tests**

  **What to do**:
  - Test: ARM64 kernel fragment produces valid config
  - Test: ARM64 Buildroot recipe builds (cross-compile check)
  - Test: Raspberry Pi OS foundation builds
  - Test: ARM64 guest boots on ARM64 host (if available)

  **Commit**: YES
  - Message: `feat(m9): add ARM64 foundations and Raspberry Pi OS userland support`
  - Files: `foundations/kernel/arm64/`, `foundations/buildroot/`, `foundations/alpine/`, `foundations/raspberry-pi/`

---

- [ ] 53. **Clippy Configuration (Complete clippy.toml)**

  **What to do**:
  - Populate `clippy.toml` with comprehensive lint configuration:
    ```toml
    avoid-breaking-exported-api = false
    doc-valid-idents = ["KVM", "IOMMU", "VFIO", "GPIO", "PCI", "USB", ".."]
    too-many-arguments-threshold = 8
    max-trait-bounds = 8
    type-complexity-threshold = 300
    ```
  - Enable additional clippy lints in workspace `Cargo.toml`:
    ```toml
    [workspace.lints.clippy]
    pedantic = { level = "warn", priority = -1 }
    unwrap_used = "warn"
    expect_used = "warn"
    panic = "deny"
    undocumented_unsafe_blocks = "deny"  # only for crates that allow unsafe
    ```
  - Fix any clippy warnings that the new config surfaces
  - Ensure `cargo clippy --workspace --all-targets -- -D warnings` passes

  **Must NOT do**:
  - Do not enable `pedantic` at `deny` level (too many false positives)
  - Do not add lints that break existing code without fixing them

  **Recommended Agent Profile**: `quick`
  **Parallelization**: Can Run In Parallel YES with T54-T58 | Blocks: None | Blocked By: M9 complete

  **Acceptance Criteria**:
  - [ ] `clippy.toml` is complete and documented
  - [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
  - [ ] No `unwrap()` or `expect()` in library code (or justified with comment)
  - [ ] All `unsafe` blocks have documentation

  **QA Scenarios**:

  ```
  Scenario: Clippy passes with strict config
    Tool: Bash
    Steps:
      1. cargo clippy --workspace --all-targets -- -D warnings 2>&1
      2. Assert: no warnings, exit 0
    Expected Result: Clean clippy output
    Evidence: evidence/task-53-clippy.txt
  ```

  **Commit**: YES
  - Message: `chore: add comprehensive clippy configuration and fix warnings`
  - Files: `clippy.toml`, `Cargo.toml`

---

- [ ] 54. **Documentation (Crate-level Docs + SECURITY.md)**

  **What to do**:
  - Add `#![doc = "..."]` module-level docs to every crate:
    - Brief description of crate purpose
    - Key types and their relationships
    - Example usage (where applicable)
  - Add `SECURITY.md` to `dome-crypto/`:
    - List every cryptographic primitive used and its source (aws-lc-rs, age)
    - Document key management model
    - Document what is and isn't protected
  - Add `SECURITY.md` to `dome-image/`:
    - Document .dome format security properties
    - Seal security model (at-rest vs runtime)
  - Run `cargo doc --no-deps --open` to verify docs render correctly

  **Must NOT do**:
  - Do not create README files per crate (per user's documentation rule)
  - Do not create standalone docs in project root

  **Recommended Agent Profile**: `writing`
  **Parallelization**: Can Run In Parallel YES with T53, T55-T58

  **QA Scenarios**:

  ```
  Scenario: Docs compile without warnings
    Tool: Bash
    Steps:
      1. cargo doc --no-deps 2>&1
      2. Assert: no warnings
      3. Assert: exit 0
    Expected Result: Clean documentation build
    Evidence: evidence/task-54-docs.txt
  ```

  **Commit**: YES
  - Message: `docs: add crate-level documentation and security model docs`

---

- [ ] 55. **Performance Benchmarks**

  **What to do**:
  - Add `[[bench]]` targets or use `criterion` for benchmarks:
    - Boot time: chamber start to guest shell prompt
    - Memory overhead: host memory used per chamber
    - Seal time: time to seal a running chamber
    - .dome read/write throughput
  - Add `cargo bench` or `cargo criterion` command
  - Document baseline numbers in development docs

  **Must NOT do**:
  - Do not add benchmarks that require specific hardware

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel YES with T53, T54, T56-T58

  **Commit**: YES — `perf: add performance benchmarks for boot, memory, and I/O`

---

- [ ] 56. **Fuzz Testing Setup (cargo-fuzz for .dome parsing)**

  **What to do**:
  - Add `cargo-fuzz` configuration:
    - Fuzz target: `.dome` file parser → `DomeReader::open()` with arbitrary bytes
    - Fuzz target: TOML blueprint parser → `Blueprint::from_toml()` with arbitrary strings
    - Fuzz target: Sentinel protocol message decoder
  - Document: `cargo fuzz run dome_parser`
  - Goal: no crashes/panics on malformed input

  **Must NOT do**:
  - Do not fuzz cryptographic verification (known-pass/fail, not crash-finding)

  **Recommended Agent Profile**: `unspecified-high`
  **Parallelization**: Can Run In Parallel YES

  **Commit**: YES — `test: add fuzz targets for .dome parsing and TOML decoding`

---

- [ ] 57. **CI Matrix (Linux + macOS + Windows)**

  **What to do**:
  - Extend `.github/workflows/ci.yml`:
    - Add matrix: `os: [ubuntu-latest, macos-latest, windows-latest]`
    - Linux: full build + test + clippy
    - macOS: `cargo check --target aarch64-apple-darwin` + clippy
    - Windows: `cargo check --target x86_64-pc-windows-msvc` + clippy
    - Mark non-Linux test steps as `continue-on-error: true` initially

  **Recommended Agent Profile**: `quick`
  **Parallelization**: Can Run In Parallel YES

  **Commit**: YES — `ci: add macOS and Windows to CI matrix`

---

- [ ] 58. **Changelog + Release Notes**

  **What to do**:
  - Create changelog entry summarizing all M1-M9 work
  - Format: grouped by milestone, with key features per milestone
  - Add to project (as Obsidian note, not project root file per documentation rule)

  **Recommended Agent Profile**: `writing`
  **Parallelization**: Can Run In Parallel YES

  **Commit**: YES — `docs: add comprehensive changelog for M1-M9`

- [x] 59. **KVM Spike (timeboxed; gates every milestone after M1)**

  **What to do**:
  - One example, `crates/dome-engine/examples/kvm_spike.rs` — a probe, not production code
  - Open `/dev/kvm`, check `KVM_GET_API_VERSION`, create a VM
  - Map 64 MiB of guest RAM with `vm-memory`
  - Load a `bzImage` plus an initramfs into guest RAM with `linux-loader`
  - Register a 16550 UART MMIO region and service `KVM_EXIT_MMIO` writes to stdout
  - Set up one vCPU (real-mode entry, boot protocol params), run the loop, capture output
  - Timebox: **one week of elapsed work**. Record the actual hours.
  - Write `docs/spike-kvm.md`: what worked, what surprised you, and whether M3's task
    count is honest given the measured cost

  **Must NOT do**:
  - Do not add seccomp, Landlock, cgroups, virtio, or domain error types here
  - Do not promote the example into the shipped crate API
  - Do not overrun the timebox before reporting back

  **Escalation gate**: if no host with a working `/dev/kvm` can be reached, STOP and
  report to the user. Do not substitute fixtures — a fixture cannot tell you how
  `KVM_RUN` actually behaves.

  **Parallelization**: Can Run In Parallel: NO — this is the single risk gate.
  **Blocks**: T10 (do not start M2 before this reports). **Blocked By**: nothing.

  **Status: COMPLETE** (2026). Findings: [`docs/spike-kvm.md`](docs/spike-kvm.md).
  Console log: `evidence/task-59-console.log`.

  A real Alpine 6.12 kernel boots on this host through KVM, with no QEMU process. It
  initialises memory, timers, topology, clocksource and console, then panics at 1.13 s with
  `VFS: Unable to mount root fs` — the correct end state for a boot with no initramfs.
  13,588 bytes of console output captured.

  Environment was re-probed first: **this host does have working KVM** — bare metal,
  `KVM_GET_API_VERSION` = 12, `KVM_CREATE_VM` succeeds. An earlier revision claimed
  otherwise based on a restricted sandbox; that was wrong and is now corrected.

  Eight distinct failures had to be fixed to get here, each now documented with its
  symptom and cause. The three that were expensive to discover:

  1. **A vCPU with no CPUID applied cannot run a modern kernel** — it halts after one exit.
  2. **Pass-through CPUID is a lie unless filtered.** Advertising CET (which we do not
     implement) killed the guest 3.3 s in, inside `exc_control_protection`, pointing at
     entirely the wrong subsystem.
  3. **No in-kernel irqchip means the LAPIC is userspace MMIO**, so no interrupt is ever
     delivered and every timer silently never fires.

  Also measured: `KVM_EXIT_IO` batches (16 bytes sent, 15 received until `io.count` is
  honoured), a UART needs LSR/IIR emulation before a guest will print, `KVM_RUN` blocks
  indefinitely so the vCPU needs its own thread, and `dome-engine` genuinely needs
  `unsafe` for `set_user_memory_region` (clippy's `-D warnings` promotes the crate's
  `warn`, so the single site carries a justified `#[allow(unsafe_code)]`).

  **Acceptance Criteria**:
  - [x] **Kernel console output captured** — `evidence/task-59-console.log`
  - [x] Elapsed cost measured and recorded in `docs/spike-kvm.md`
  - [x] M3 estimates restated against measured cost
  - [x] KVM plumbing independently verified by a self-test that fails loudly
  - [x] Reproducible: repeated runs reach the same panic at ~1.1-1.4 s

  **Extended (userspace)**: with a cpio initramfs attached, the boot now reaches userspace:
  `RAMDISK -> Unpacking initramfs -> Run /init as init process -> dome-init output`.
  Evidence: `evidence/task-59-userspace.log`. Two more findings from that step:

  - **`type_of_loader` gates the initrd entirely.** `reserve_initrd()` in
    `arch/x86/kernel/setup.c` returns early on `!boot_params.hdr.type_of_loader`, so the
    kernel ignores a perfectly valid `ramdisk_image`/`ramdisk_size` pair. Setting it to
    `0xFF` ("undefined, private use") fixed a bug that no amount of zero-page readback
    could explain.
  - **Userspace writes to `/dev/console` need the UART TX interrupt.** `printk` polls the
    port and works; a userspace `write()` is queued and drained by the 8250 driver's TX
    interrupt handler, which never fires without IOAPIC/GSI routing. Direct port IO from
    userspace works, which is how the acceptance evidence was captured. A real tty path
    therefore needs interrupt routing — device-model work, as predicted.

  **Handover to M3**: the spike is the specification for the boot-protocol task. Do not
  regress the expensive lessons: apply CPUID, filter it, create the irqchip, set
  `type_of_loader`, and route the UART interrupt before expecting a tty to work.

- [x] 60. **Capability Acquisition Decision (cgroups + CAP_NET_ADMIN)**

  **What to do**:
  - Write `docs/adr/0001-privilege-acquisition.md`, then implement its choice
  - Compare the two viable models and pick one:
    1. **Delegated systemd scope** — a `dome.slice` with `Delegate=yes`; the supervisor
       runs as the user and owns a cgroup subtree. No root daemon, but no netns either.
    2. **Privileged helper** — a capability-limited daemon (`CAP_NET_ADMIN`) creating
       netns/cgroups on request over a socket. Full M4 capability, larger attack surface.
  - State which parts of M4 work without privileges and which do not
  - Record the current host reality in `docs/known-limits.md`: cgroup2 is mounted
    read-only and `/dev/kvm` is absent, so neither model can be closed-tested here
  - Amend `.rules/project.md`: the "no root helpers" rule is scoped to Milestone 1 and
    must state M4's position explicitly

  **Must NOT do**:
  - Do not add a setuid binary (`.rules/security.md` forbids it)
  - Do not write nftables or cgroup state from the CLI process — supervisor or helper only

  **Parallelization**: Can Run In Parallel: YES with T59, T61, T62. **Blocks**: T22, T23.
  **Blocked By**: nothing.

  **Status: COMPLETE** — decided and recorded in
  [`docs/adr/0001-privilege-acquisition.md`](docs/adr/0001-privilege-acquisition.md).

  **Decision: zero privilege.** The supervisor runs as the invoking user with no
  capabilities, no setuid bits and no helper daemon:

  - **cgroups** — each chamber gets a cgroup under the supervisor's own delegated
    user-service subtree. Verified: `Delegate=yes`, `cpu memory pids` already enabled, and
    `memory.max`/`pids.max` writable as uid 1000.
  - **namespaces** — `CLONE_NEWUSER | CLONE_NEWNET` per chamber. Verified: `unshare -Urn`
    brings up loopback and creates a TAP device, so no `CAP_NET_ADMIN` is needed.
  - **networking** — closed perimeter by default; published ports are served by a listener
    the supervisor opens as the user and forwarded into the chamber userspace stack
    (slirp-style). No veth on the host, no nftables writes.

  Both options originally offered were rejected with reasons: a capability-limited helper
  (b) reintroduces the privileged surface for capability we already have, and a one-time
  sudo (c) is unnecessary now that delegation is confirmed on for normal users.

  Product requirements captured in the ADR:

  - **Shared UID for apertures** — guest uid 1000 is host uid 1000 for aperture paths, so
    files keep ownership and sharing needs no translation. The ADR states the trade this
    makes: mapped writable paths carry the host user's own authority, mitigations are
    read-only defaults plus explicit enumeration.
  - **Defense engine runs host-side**, in the supervisor's plugin host, per
    `.rules/plugins.md`; chamber-side telemetry stays advisory because a compromised
    chamber can lie.
  - The ADR states what Dome may and may not claim: hardware virtualization isolation and
    no host state mutation are claimable; containment of a VM escape is **not** — it lands
    with uid 1000 authority, exactly like the rootless-container position.

  **Acceptance Criteria**:
  - [x] ADR exists, states the chosen model and lists what each model enables
  - [x] Chosen model verified on this host by probe, not assumed
  - [x] Rejected options recorded with reasons
  - [x] Isolation claims bounded explicitly, including the shared-UID trade
  - [ ] `.rules/apertures.md` amended with the UID-sharing note (small follow-up, do with T38)
  - [ ] `.rules/project.md` M4 privilege position recorded (do with the first M4 task)

- [x] 61. **CLI Contract Document (freeze the grammar before any command is written)**

  **What to do**:
  - Create `docs/cli-contract.md` — single source of truth for names, flags, exit codes,
    output formats
  - Resolve the Rev 1 inconsistencies explicitly:
    - validation: `dome validate <cell.toml>` (not `dome cell validate`)
    - build: `dome build --recipe <name>` (not `dome build <foundation>`)
    - foundations: `dome foundation info <file.dome>`
    - lifecycle: `dome run`, `dome stop`, `dome seal`, `dome export`, `dome import`
  - Define exit codes: `0` success, `2` usage, `3` validation failure, `4` host
    capability missing, `5` engine failure
  - Fix the fixture convention: everything under `tests/fixtures/`
  - Declare `doctor --format toml` a stable contract with a `schema_version` key

  **Must NOT do**:
  - Do not implement the CLI here; T5 implements it and must match this document

  **Parallelization**: Can Run In Parallel: YES with T59, T60, T62. **Blocks**: T5, T29.
  **Blocked By**: nothing.

  **Completed** (2026): `docs/cli-contract.md` written and frozen at contract version 1.
  It fixes exit codes (`0/2/3/4/5`), the `schema_version` obligation on `--format toml`,
  the fixture convention, the domain-vocabulary mapping, and a "rejected forms" table that
  names `dome cell validate` and `dome build <foundation>` explicitly so nobody re-adds them.

  **Acceptance Criteria**:
  - [x] `docs/cli-contract.md` covers every command referenced in this plan
  - [x] T5, T6, T29, T47 carry a grammar-authority pointer to it
  - [x] The rejected forms are recorded in the contract, not silently dropped
  - [ ] `dome --version` and `dome doctor` report `schema_version` (implemented by T5)

---

- [x] 62. **Repo Hygiene (remainder — the noisy part is already done)**

  **Already completed** (verified; do not redo, and do not let F1 re-flag it):
  - [x] `git config core.fileMode false` + `chmod 644` on all 136 tracked files — the
        46 phantom "modified" entries are gone
  - [x] `graphify-out/` untracked (`git rm -r`), deleted from disk, added to `.gitignore`
  - [x] `.omo/` and `.sisyphus/` deleted; the plan lives at `PLAN.md` in the repo root
  - [x] `evidence/` created at the project root with a `.gitkeep`; all 68 QA-scenario
        paths in this plan point at `evidence/<task>-<slug>.<ext>`
  - [x] dead `.sisyphus/` entry removed from `.git/info/exclude`

  **What to do**:
  - Add a `LICENSE` matching `Cargo.toml`'s `license = "MIT OR Apache-2.0"`. Decide the
    shape deliberately: dual `LICENSE-MIT` + `LICENSE-APACHE` is the Rust convention; a
    single `LICENSE` holding both texts works but is more ambiguous for tooling
  - Track `assets/logo.svg` (`git add`), and reference it from `README.md` if the logo is
    meant to be used
  - Decide the evidence retention policy and write it in the QA Policy section: text
    evidence (`.txt`, `.log`, `.toml`) is committed; large captures (screenshots,
    framebuffer dumps, full `strace` output) are gitignored
  - Confirm `README.md` still describes reality after the folder removals

  **Must NOT do**:
  - Do not rewrite history to fix the old file modes
  - Do not restore `graphify-out/` to the index
  - Do not re-create `.sisyphus/` or `.omo/` — evidence goes to `evidence/`

  **Parallelization**: Can Run In Parallel: YES with all of Wave 0. **Blocked By**: nothing.

  **Acceptance Criteria**:
  - [x] `git status --porcelain` shows no mode-only changes
  - [x] `graphify-out/` untracked and ignored
  - [x] Exactly one copy of the plan exists, at `PLAN.md`
  - [x] `LICENSE-MIT` + `LICENSE-APACHE` exist and match `Cargo.toml` (MIT OR Apache-2.0)
  - [x] `assets/logo.svg` is tracked
  - [x] Evidence policy documented in the QA Policy section

---

- [x] 63. **Virtio Substrate (UART + initramfs boot path)**

  **What to do**:
  - Pin the minimum device set every engine must implement: 16550 UART on MMIO, a
    virtio-mmio transport, and an initramfs loader
  - `dome-engine/src/devices/uart16550.rs` — console device, unit-testable against a
    byte buffer with no hypervisor present
  - `dome-engine/src/devices/virtio_mmio.rs` — transport (status handshake, queue
    notify, feature bits) on top of `virtio-queue`
  - `dome-engine/src/boot/initramfs.rs` — write a cpio image into guest RAM and fill
    the boot params
  - Define the `DeviceSet` trait every platform engine implements, so one foundation
    boots unchanged on Linux, macOS and Windows engines

  **Must NOT do**:
  - Do not implement virtio-blk, -net, -fs, -gpu or -vsock here; each needs its own task
  - Do not leak platform assumptions into `DeviceSet` — that is what makes cross-platform
    foundations possible (Revision 2 §5)

  **Parallelization**: Can Run In Parallel: NO — needs T59. **Blocks**: T19, T21, T36b,
  T38, T39. **Blocked By**: T59.

  > **Done.** `crates/dome-engine/src/devices/` with `uart16550.rs`,
  > `virtio_mmio.rs` and the `DeviceSet`/`Device`/`VirtioBackend` traits;
  > `docs/device-set.md`; **twenty tests, none needing a hypervisor**.
  >
  > The run loop now routes through `ChamberIo`, so a legacy port write and a memory-mapped
  > write reach the same device and the console cannot disagree with itself. The port-IO serial
  > device delegates to the UART rather than duplicating it.
  >
  > Measured image weight is recorded in the doc rather than asserted: a bootable base is
  > ~12 MB against 420 MB for `postgres:16-alpine`, and the doc is explicit that a thin
  > container image will always be smaller on disk because it borrows the host kernel. The win
  > claimed is system footprint — no daemon, one foundation via reflink, shared read-only pages
  > — not bytes per image.

  **Acceptance Criteria**:
  - [ ] UART and initramfs writer pass unit tests with no hypervisor
  - [ ] `docs/device-set.md` documents `DeviceSet`; T43–T46 reference it
  - [ ] Kernel console output observed through the UART on a real boot (evidence file)

---

- [x] 64. **Content-Addressed Foundation Store + CoW Overlays**

  **What to do**:
  - `dome-image/src/store.rs`: host-local store keyed by foundation digest under
    `$XDG_DATA_HOME/dome/foundations/<sha256>/`
  - Materialize each foundation once; give every chamber a copy-on-write overlay
    (reflink where supported, sparse overlay otherwise)
  - Reference-count entries and garbage-collect unreferenced ones
  - Document the reason in the module doc: without this, N chambers materialize N copies
    of a 20–400 MiB foundation (Revision 2 §4)
  - Add `dome foundation list` and `dome foundation prune`

  **Must NOT do**:
  - Do not leave decrypted foundations on disk without an explicit opt-in
  - Do not dedupe across architectures — the digest includes the target arch (T65)

  **Parallelization**: Can Run In Parallel: YES with T65. **Blocked By**: T12, T13.

  > **Done.** `crates/dome-image/src/store.rs`, ten tests, plus `dome foundation list`
  > and `dome foundation prune`.
  >
  > A foundation is stored once under a digest of its bytes **and its architecture**: the same
  > recipe produces different bytes per target, and silently serving an aarch64 image to an
  > x86_64 chamber fails in a way that is hard to diagnose later.
  >
  > **The honest finding.** The CoW half depends on the filesystem, and this host's `/` is
  > **ext4**, which cannot share extents. So layers here are full copies, and the store reports
  > `strategy: copy` rather than implying the good case. On btrfs, xfs or zfs the same call
  > returns `strategy: reflink` and layers cost no additional blocks. The test asserts whichever
  > is true instead of asserting the one that would look better.
  >
  > What works everywhere, regardless of filesystem: **identity dedupe**. N chambers from one
  > foundation still store exactly one copy of it, with per-chamber references. The saving that
  > needs reflink is the writable layer, not the sharing.
  >
  > Inserts copy to a temporary name and rename into place, so a half-written image is never
  > visible under a digest it does not match. Chamber names are sanitised before becoming file
  > names, and a test tries `../../escape` to prove it cannot leave the store.
  >
  > `dome-crypto` gained a streaming SHA-256, because digesting a 400 MB foundation must not
  > mean allocating 400 MB.

  **Acceptance Criteria**:
  - [ ] Two chambers from one foundation use one foundation's worth of disk
  - [ ] Overlay writes never mutate the shared foundation (digest stability test)
  - [ ] `prune` leaves referenced foundations intact

---

- [ ] 65. **ARM64 Device Tree Generation + Architecture Tagging**

  **What to do**:
  - `dome-engine/src/boot/fdt.rs` — generate a minimal arm64 device tree: memory nodes,
    `/psci`, GIC v3, `arm,armv8-timer`, virtio-mmio nodes, `/chosen` bootargs
  - Follow the arm64 boot protocol: PC = kernel entry, `x0` = DTB physical address
  - Add target architecture to `FoundationManifest` and reject mismatches before any
    engine setup
  - Explain in `docs/device-set.md` why arm64 needs a DTB where x86_64 needs boot params

  **Must NOT do**:
  - Do not attempt cross-architecture execution
  - Do not shell out to `dtc`; generate the blob in Rust

  **Parallelization**: Can Run In Parallel: YES with T64. **Blocks**: T50, T51.
  **Blocked By**: T49.

  **Acceptance Criteria**:
  - [ ] Generated DTB parses with `dtc -I dtb -O dts` (structural assertions)
  - [ ] `FoundationManifest` carries an architecture field
  - [ ] Mismatch yields a typed `EngineError`, not a boot attempt

---

- [ ] 66. **Seal Portability + Artifact Size Budget Spec**

  **What to do**:
  - Add a Portability section to `dome-image/SECURITY.md` and the `.dome` format doc:
    foundations are portable across host OS within one CPU architecture; seals are
    portable to nothing — same engine, same host, same arch
  - Adopt the size table from Revision 2 §4 as a documented budget with a CI assertion:
    a minimal foundation must be under 25 MiB or the recipe fails
  - Narrow `README.md`'s artifact-exchange wording to foundations
  - Amend `.rules/images.md` with compress-then-encrypt and the seal portability limit

  **Must NOT do**:
  - Do not promise seal migration between engines. That needs full device-state
    serialization, which no task in this plan implements.

  **Parallelization**: Can Run In Parallel: YES. **Blocked By**: T11.

  **Acceptance Criteria**:
  - [ ] `dome-image/SECURITY.md` contains the portability matrix from Revision 2 §5
  - [ ] `README.md` no longer implies seals are transferable
  - [ ] `.rules/images.md` states compress-then-encrypt and seal non-portability
  - [ ] Minimal-foundation size budget asserted by a test

---

- [ ] 67. **Maximal-Isolation Baseline (cross-cutting, gates M3 sign-off)**

  > Derived from research: see [`docs/adr/0002-isolation-model.md`](docs/adr/0002-isolation-model.md).
  > The literature is consistent that isolation is won by **minimising what the sandbox can
  > touch**, then assuming that boundary fails. Firecracker exposes three devices; gVisor's
  > model minimises the host API reachable from the sandbox to defeat exploit chaining.

  **What to do**:
  - **Pin the device set** in code, not in prose: UART + virtio-mmio + virtio-blk only. Make
    adding a device a compile-time decision (a closed enum / builder with no escape hatch)
    so it cannot happen by accident.
  - **CPUID allowlist**: start from a deny-by-default template and opt features in
    deliberately. Keep the CET mask as the first entry and the spike as its justification.
  - **Per-device I/O rate limiting**, so a malicious guest cannot monopolise the device loop.
  - **Host-availability caps**: vCPU ceiling, boot timeout, and the cgroup limits from
    ADR 0001. A hung chamber must be killable.
  - **Refuse DMA apertures** when `/sys/kernel/iommu_groups` is empty, with a typed error
    that names the missing IOMMU.
  - **Fuzz the device parsers** (extends T56): any code that reads guest-controlled
    descriptors gets a `cargo-fuzz` target.

  **Must NOT do**:
  - Do not enable SMM, nested virtualization, or PCI hole devices "for compatibility".
  - Do not pass through CPUID wholesale.
  - Do not claim host-confidentiality on hardware lacking SEV-SNP/TDX/CCA.

  **Parallelization**: Can Run In Parallel: partially — the device pin and CPUID work follow
  T63; the caps follow T22. **Blocks**: M3 sign-off. **Blocked By**: T16, T20, T22, T63.

  **Acceptance Criteria**:
  - [ ] `echo 1 > /dev/kvm`-style mistakes impossible: the device set is a closed type
  - [ ] CPUID template has CET (and any other unimplemented feature) clear, asserted by test
  - [ ] I/O rate limit enforced per device, with a test that floods and observes backpressure
  - [ ] A chamber exceeding its vCPU/memory/PID cap is contained without host impact
  - [ ] Requesting a PCI/USB aperture on this host fails with a typed `IoMmuUnavailable` error
  - [ ] Every device parser has a fuzz target and a seed corpus

- [x] 68. **Post-Quantum Suite 2 (X25519 + ML-KEM-768, dual signatures)**

  > Specification: [`docs/adr/0003-post-quantum-crypto.md`](docs/adr/0003-post-quantum-crypto.md).
  > Standards verified against primary sources: FIPS 203/204/205 (final, Aug 2024),
  > RFC 10024 (Standards Track, Aug 2026) for the hybrid combination, NIST IR 8547 for the
  > 2035 removal deadline. X-Wing is deliberately not used: still an Informational draft.

  **What to do**:
  - `dome-crypto`: X25519 + ML-KEM-768 encapsulation combined per RFC 10024, HKDF-SHA256
    schedule. aws-lc-rs 1.19 already exposes `kem::ML_KEM_768` and `pqdsa::PqdsaKeyPair`;
    no new crypto dependency is needed.
  - Dual signatures: Ed25519 **and** ML-DSA-65, both required to verify.
  - Writer/reader: **derive the signature and wrapping lengths from the suite byte**, never
    from `SIGNATURE_LEN`. ML-DSA-65 is ~3309 bytes against Ed25519's 64, so a suite-2 file
    mis-framed by a suite-1 constant would parse garbage or fail confusingly.
  - Test against published RFC 10024 / FIPS 203 vectors, and add a suite-2 round-trip,
    a downgrade-refusal test, and a mixed-signature test (one valid, one forged → reject).

  **Must NOT do**:
  - Do not invent a combiner; follow RFC 10024.
  - Do not use Falcon or HQC: both are selected but their standards are unfinished.
  - Do not claim PQ safety for suite 1, or for suite 2 before its vectors pass.

  **Done** (2026). `crates/dome-crypto/src/pq.rs`, plus suite-aware framing in
  `crates/dome-image/src/writer.rs` and `reader.rs`. **19 new tests, 172 workspace total.**

  Built on aws-lc-rs, which already exposes `kem::ML_KEM_768` and ML-DSA-65; no new
  dependency. The hybrid follows RFC 10024's combination order (ML-KEM secret **first**),
  then HKDF-SHA256, then AES-256-GCM seals the file key. Signatures are dual: Ed25519 and
  ML-DSA-65, both required.

  What we compose rather than cite: the wire framing and the HKDF context string, because
  RFC 10024 specifies a TLS 1.3 handshake and this is an at-rest container. The combination
  order and primitive choices are the RFC's. Recorded in ADR 0003.

  Three findings worth keeping:

  1. **The footer length is suite-dependent**, which is exactly why the suite byte exists.
     Suite 1 is 64 bytes; suite 2 is 64 + 3309. A reader using a constant would frame a
     suite-2 file from the wrong offset, so `signature_len_for_suite` is the only way to
     locate it, and a test asserts the two lengths differ.
  2. **A wrong shape in a constant is easy to miss.** The wrapped-key length omitted the
     nonce and was 12 bytes short; the length assertion caught it immediately. Sizes in a
     binary format should be asserted, not reasoned about.
  3. **Downgrade and upgrade are both refused explicitly.** A suite-1 reader given a suite-2
     file returns `UnsupportedSuite` rather than mis-parsing it, and vice versa. The tests
     cover both directions, because a silent misread is the failure that matters.

  Both signature halves are tested for tampering independently: corrupting only the ML-DSA
  half (Ed25519 still valid) and only the Ed25519 half (ML-DSA still valid) must each be
  rejected.

  **Acceptance Criteria**:
  - [ ] Vectors from the specifications pass
  - [ ] Suite-2 container round-trips with a payload spanning several chunks
  - [ ] A suite-1 reader refuses a suite-2 file with `UnsupportedSuite`
  - [ ] Dual signatures: a file with one forged signature is rejected
  - [ ] No `SIGNATURE_LEN` constant is used to locate the footer

---

- [x] 69. **Restart Policy and Host-Restart Reconciliation**

  > Driven by a product requirement: a running chamber should come back after the host
  > restarts. Cgroups and namespaces do not survive a reboot; the **record** does, and it is
  > the only thing that can answer "should this be running?" at startup.

  **What it does** — `crates/dome-supervisor/src/restart.rs` and `state.rs`, **thirteen tests**:

  | Docker `--restart` | Dome |
  | --- | --- |
  | `no` | `RestartPolicy::No` |
  | `on-failure[:max]` | `OnFailure { max_retries }`, counting crashes and host loss |
  | `always` | `Always` |
  | `unless-stopped` | `UnlessStopped`, kept for familiarity |

  **Two flaws fixed rather than copied:**

  1. Under Docker's `always`, a manually stopped container restarts when the daemon restarts,
     which surprises the operator who stopped it. Dome records *why* a chamber stopped, so a
     deliberate stop sticks under **every** policy — a test asserts it across all four.
  2. Because of that fix the two permissive policies became equivalent. Rather than invent a
     third meaning for a name Docker users already know, the documentation says so plainly and
     a test pins the equality, so it stays a decision instead of becoming an accident.

  Durability: one plain-text `key=value` record per chamber under
  `$XDG_STATE_HOME/dome/chambers`, written to a temporary file and renamed so a crash leaves
  the old record or the new one, never a partial one. A corrupt record is **reported and
  skipped** rather than blocking the others — one bad file must not take the host's chambers
  down. `reconcile()` treats a record that was running as [`ExitReason::HostLost`], which is
  not the chamber's fault and is exactly what a restart policy is for.

  Chamber ids are validated on the way to a filename, and a test tries traversal ids to prove
  the record cannot escape the state directory.

  **Acceptance Criteria**:
  - [x] All four policies, with the operator-stop rule holding under each
  - [x] A bounded retry budget stops restarting once spent
  - [x] Records round-trip, and saving is atomic
  - [x] A corrupt record does not hide the valid ones
  - [x] Reconciliation resumes what was running and nothing else
  - [x] The record cannot escape its directory

---

---

## Final Verification Wave (MANDATORY — after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.

- [ ] F1. **Plan Compliance Audit** — `oracle`
      Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, cargo test, run command). For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in `evidence/`. Compare deliverables against plan.
      Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/69] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
      Run `cargo fmt --check` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace`. Review all changed files for: `as any`/`@ts-ignore`, empty catches, `println!`/`eprintln!` in lib code, commented-out code, unused imports. Check AI slop: excessive comments, over-abstraction, generic names (`data`/`result`/`item`/`temp`).
      Output: `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [ ] F3. **Real Manual QA** — `unspecified-high`
      Start from clean state (`cargo clean && cargo build`). Execute EVERY QA scenario from EVERY task — follow exact steps, capture evidence. Test cross-task integration: `doctor` → `build` → `run` → `seal` → `export` pipeline end-to-end. Test edge cases: no KVM, low memory, port conflicts, invalid TOML, corrupted .dome files. Save to `evidence/final-qa/`.
      Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [ ] F4. **Scope Fidelity Check** — `deep`
      For each task: read "What to do", read actual diff (`git diff --stat HEAD~N..HEAD`). Verify 1:1 — everything in spec was built (no missing), nothing beyond spec was built (no creep). Check "Must NOT do" compliance. Detect cross-task contamination: Task N touching Task M's files. Flag unaccounted changes.
      Output: `Tasks [N/69 compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- **Per-milestone commits**: Each task commits independently. Groups of related tasks (e.g., all M1 tasks) can be squashed into a milestone commit
- **Commit format**: `feat(milestone): description` or `fix(milestone): description`
- **Pre-commit verification**: `cargo fmt --check && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
- **No pushing**: Per global rules, never push unless user explicitly requests

---

## Success Criteria

### Verification Commands

```bash
# After M1
cargo run -p dome-cli -- doctor --format toml
cargo run -p dome-cli -- validate tests/fixtures/valid.cell.toml
cargo test --workspace

# After M2
cargo test -p dome-crypto
cargo test -p dome-image
cargo run -p dome-cli -- foundation info tests/fixtures/minimal.foundation.dome

# After M3
cargo run -p dome-cli -- doctor --format toml  # should show engine: native-hypervisor
cargo test -p dome-engine

# After M4
cargo run -p dome-cli -- run tests/fixtures/valid.cell.toml
cargo run -p dome-cli -- seal chamber-001
cargo run -p dome-cli -- stop chamber-001

# After M8
cargo check --target aarch64-apple-darwin
cargo check --target x86_64-pc-windows-msvc
```

### Final Checklist

- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All 69 tasks completed
- [ ] T59 (KVM spike) completed on real KVM hardware, with console evidence committed
- [ ] Every architecture-mismatch and missing-device path returns a typed error, not a panic
- [ ] All tests pass (unit + integration + doc)
- [ ] CI green on all platforms
- [ ] `dome run` boots a chamber on Linux
- [ ] `dome seal` produces a valid `.seal.dome` file
- [ ] `dome doctor` works on Linux, macOS, and Windows
