# Dome CLI contract

**Status**: frozen for M1-M9. Every command, flag, and exit code in this document is
authoritative. If an implementation task disagrees with this file, this file wins and the
task gets corrected.

**Contract version**: 1 (see T61 in [PLAN.md](../PLAN.md))

## 1. Global rules

- The binary is `dome`, one executable, subcommand-first: `dome <command> [args] [flags]`.
- Positional arguments always name a **file or an ID**, never a recipe or a mode.
- Flags always carry the mode. `--recipe`, `--format`, `--display`, `--output`.
- A command with no arguments prints its own help and exits `0`.
- `dome` with no arguments prints top-level help and exits `0`.
- Unknown command, unknown flag, or a flag missing its value is a usage error: exit `2`.
- Nothing writes to the host before the arguments have been fully validated.

## 2. Exit codes

These are stable. Scripts depend on them.

| Code | Meaning | Typical cause |
| --- | --- | --- |
| `0` | Success | command completed, including `--help` |
| `2` | Usage error | unknown command, unknown flag, missing value, unparseable value |
| `3` | Validation failure | cell blueprint is invalid, `.dome` file is malformed |
| `4` | Host capability missing | no `/dev/kvm`, no cgroups v2, no `CAP_NET_ADMIN` |
| `5` | Engine failure | VM create/load/run failed after capabilities were confirmed |

Exit `4` is deliberately distinct from `5`: *"this machine cannot run chambers"* is not
the same failure as *"the engine broke"*, and operators need to tell them apart.

## 3. Commands

| Command | Maturity | Owner task |
| --- | --- | --- |
| `dome doctor` | exists | T5, extended by T47 |
| `dome validate <cell.toml>` | M1 | T5, rules from T6 |
| `dome foundation info <file.dome>` | M2 | T14 |
| `dome foundation list` / `prune` | M2.5 | T64 |
| `dome run <cell.toml>` | M4 | T25 |
| `dome stop <chamber-id>` | M4 | T25 |
| `dome seal <chamber-id>` | M4 | T26 |
| `dome export <chamber-id>` | M4 | T27 |
| `dome import <file.dome>` | M4 | T27 |
| `dome build --recipe <name>` | M5 | T29, T32 |
| `dome completions <shell>` | M1 (hidden) | T5 |

### `dome doctor [--format toml|human] [--fixtures PATH]`

Read-only host capability report. The only command that is allowed to run on a host with
no hypervisor at all — it must succeed everywhere and report what is missing.

- `--format` defaults to `toml`. Accepted: `toml`, `human`. Anything else: exit `2`.
- `--fixtures PATH` reads probes from a fake root instead of `/`. Used by tests only.
- Exit `0` on every host, including one with no `/dev/kvm` and no cgroups. Reporting a
  missing capability is success, not failure.

### `dome validate <cell.toml>`

Parse and validate a blueprint. Does not touch the host, does not need KVM.

- Reports **every** validation error at once, never just the first.
- Exit `0` when valid, exit `3` when invalid.
- `--format toml|human` accepted, same default as `doctor`.

### `dome run <cell.toml> [--name ID] [--display none|vnc] [--vnc-port PORT]`

Create and start a chamber. The cell's `[cell] name` or the file stem becomes the chamber
ID when `--name` is absent; duplicate IDs are rejected, not suffixed.

- Exit `4` when the host cannot run chambers (capability probe fails before any setup).
- Exit `5` when engine setup fails after capabilities were confirmed.
- `--display` defaults to the cell's `apertures`/display setting, then to `none`.
- `--vnc-port` is only valid with `--display vnc`; otherwise exit `2`.

### `dome stop <chamber-id>`

Graceful shutdown, then forced cleanup. Idempotent: stopping a stopped chamber exits `0`.

### `dome seal <chamber-id> [--output PATH]`

Save running state to a `.seal.dome`. Default output is `<chamber-id>.seal.dome` in the
current directory. A seal is **host-, engine-, and architecture-locked** — it is local
cache, not a distributable artifact (see Revision 2 section 5 in PLAN.md).

### `dome export <chamber-id> --output PATH`

Self-contained package: blueprint, foundation reference, seal data, logs. `--output` is
required — there is no default, because exports are large and land where you say.

### `dome import <file.dome>`

Load an exported package onto this host. Rejects a package whose foundation architecture
does not match the host with exit `3`, before any engine setup.

### `dome build --recipe <name>` / `dome build --list`

Invoke a foundation build recipe. Recipes are shell scripts under `foundations/`; Dome
does not implement a build system (guardrail in `.rules/milestones.md`).

- `--recipe` and `--list` are mutually exclusive; neither present is exit `2`.
- Unknown recipe name is exit `3`, and the error lists the known recipes.

### `dome foundation info <file.dome>`

Read the header of a `.dome` file without decrypting it. Exit `3` on bad magic or an
unsupported format version.

## 4. Machine-readable output

`--format toml` is a **stable contract**, not a debugging convenience.

- The top-level table always carries `schema_version = <integer>`, starting at `1`.
- Keys are never renamed. New keys may be added in a minor revision; existing keys and
  their types do not change within a schema version.
- An unknown key added by a newer Dome must be ignored by older consumers, not rejected.
- `null` is written for an absent optional value, never an empty string.

The `schema_version` key is added by **T5** (the renderer lives in `dome-host-linux`, the
CLI only selects the format). T47 extends the same contract for macOS and Windows.

## 5. Rejected forms

These appeared in earlier drafts of the plan and are **not** valid. Do not implement them.

| Rejected | Correct | Why |
| --- | --- | --- |
| `dome cell validate <file>` | `dome validate <file>` | nested subcommands add depth without meaning |
| `dome build <foundation>` | `dome build --recipe <name>` | positional arguments never name recipes |
| `dome run --vnc-port 5900` (without `--display vnc`) | add `--display vnc` | a port flag that silently does nothing is a bug |
| `dome seal --output` (with no chamber) | `dome seal <chamber-id>` | the chamber is not optional |

## 6. Fixtures

All test fixtures live under `tests/fixtures/`. Shared blueprints used by CLI tests:

| Path | Purpose |
| --- | --- |
| `tests/fixtures/valid.cell.toml` | complete, valid blueprint (T9) |
| `tests/fixtures/invalid-zero-memory.cell.toml` | rejected blueprint (T9) |
| `tests/fixtures/linux-host/` | fake `/proc`, `/sys`, `/dev` root for `doctor --fixtures` |

QA evidence is written to `evidence/` at the repository root, not into `tests/`.

## 7. Terminology in the CLI surface

The domain vocabulary is not decoration — it is the user-facing contract.

| Word | Means | Never called |
| --- | --- | --- |
| `foundation` | bootable base OS artifact | image, base box, template |
| `cell` | TOML blueprint on disk | config, manifest, spec |
| `chamber` | running instance | container, VM, sandbox |
| `perimeter` | network boundary | network, net |
| `aperture` | explicit host resource mapping | mount, passthrough, volume |
| `seal` | saved chamber state | snapshot, checkpoint |
| `sentinel` | guest-side helper binary (`/usr/bin/dome-sentinel`) | agent, guest-agent |

## 8. Changing this document

Adding a command or a flag requires a task in PLAN.md. Renaming one requires a new
contract version and a note in this section explaining what broke.

| Version | Change |
| --- | --- |
| 1 | Initial freeze. Resolves `dome cell validate` vs `dome validate`, `dome build <foundation>` vs `--recipe`, and the fixture path convention. |
