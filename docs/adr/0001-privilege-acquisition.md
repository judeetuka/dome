# ADR 0001 — How Dome acquires cgroup and network privileges

Status: **Accepted** (2026). Supersedes the open question in PLAN.md T60.
Owner: T22 / T23 implement it.

## Context

M4 needs two host facilities per chamber: a cgroup (to enforce memory/CPU/PID limits) and a
network namespace (to give the chamber its own stack and a closed perimeter). Both were
initially reported as unavailable because `mkdir /sys/fs/cgroup/...` and `ip netns add`
returned `EACCES`.

That reading was wrong. Probing the right places on this host gives:

| Capability | Probe | Result |
|---|---|---|
| cgroup delegation | `systemctl show user@1000.service -p Delegate` | `Delegate=yes` |
| cgroup ownership | `ls -ld` the user service cgroup dir | owned by the user (`iam:iam`) |
| controllers | `cat .../cgroup.subtree_control` | `cpu memory pids` already enabled |
| creation as uid 1000 | `mkdir <user-service>/dome-probe` | **succeeds** |
| memory limit | write `memory.max` | **succeeds** (`67108864`) |
| PID limit | write `pids.max` | **succeeds** |
| netns + device | `unshare -Urn` then `ip tuntap add` | **loopback up, TAP created** |

So the root-owned `/sys/fs/cgroup` root and the absence of `CAP_NET_ADMIN` in the initial
namespace do not matter: the user session already owns a delegated subtree, and user
namespaces provide a netns with a working TAP device.

## Decision

**Zero privilege.** The supervisor runs as the invoking user with no capabilities, no
setuid bits, and no helper daemon. It acquires both facilities like this:

- **cgroups** — create each chamber's cgroup under the supervisor's own delegated
  user-service subtree, then write `memory.max`, `cpu.max` and `pids.max`. If the parent
  `cgroup.subtree_control` lacks a controller, degrade to whatever is present and report
  the missing controller rather than silently claiming enforcement.
- **namespaces** — create each chamber's netns with `clone3`/`unshare` using
  `CLONE_NEWUSER | CLONE_NEWNET`. Inside it the process is root and may build a loopback
  plus a TAP device.
- **networking** — the default perimeter is a chamber with loopback only and no egress.
  Published ports are served by a listener **the supervisor itself opens as the user** and
  forwarded into the chamber's userspace network stack (slirp-style), the same approach
  rootless containers use. No veth pair touches the host namespace, no nftables rule is
  written, and no `CAP_NET_ADMIN` is ever needed.

## Consequences

Good:

- Nothing privileged exists to attack. There is no daemon holding capabilities.
- Nothing about host networking or the host cgroup hierarchy is reconfigured, so the
  chamber cannot break the machine's configuration even if the VMM is compromised.
- Teardown is implicit: when the session scope ends, its cgroups and namespaces go with it.
- No host firewall changes, so Dome does not interleave with the user's existing rules.

Costs, stated plainly:

- Port forwarding goes through a userspace proxy: a throughput and latency cost per flow.
- Egress policy is enforced **in the proxy**, not in the kernel. The proxy must therefore
  be a fail-closed allowlist, and a proxy bug is a perimeter bug. This is why the perimeter
  type is a policy object rather than a set of rules.
- If a host lacks user-namespace support, per-chamber netns is unavailable and Dome must
  report that rather than falling back to the host namespace.

## Rejected options

- **(b) a capability-limited helper daemon.** Rejected: it reintroduces exactly the
  privileged surface the design is trying to avoid, for capability we already have.
- **(c) one-time sudo to enable delegation.** Rejected as unnecessary — delegation is
  already on for this user, and the same one-time setup would have to be re-run for every
  new user account. If a host genuinely lacks delegation, the answer is to degrade loudly
  (report "limits not enforceable"), not to escalate.

## Isolation claims this supports, and the ones it does not

Dome may claim:

- Every chamber is isolated by **hardware virtualization** (KVM), not by namespaces.
- A chamber cannot touch host kernel state, host networking, or the host cgroup hierarchy.
- Host resources reach a chamber only through **explicit apertures**.

Dome must **not** claim:

- That escaping the VMM is contained. A VM escape lands with the supervisor's authority:
  **uid 1000** — the same authority as any other process the user runs. This is identical to
  the rootless-container position and must be stated wherever isolation is described.
- Runtime confidentiality against a malicious host root.

## Shared-UID apertures (product requirement)

The product needs file sharing to "just work", so a chamber's user identity maps to the
host user: **guest uid 1000 is host uid 1000** for aperture paths. No chown dance, no
translation layer, files keep their ownership.

This is a deliberate trade and it narrows one boundary:

- Anything mapped through a writable aperture is writable by the chamber **to the same
  degree the host user can write it**. A chamber compromise therefore has the same reach as
  a compromised process running as the user.
- Mitigations, all mandatory: apertures are **read-only unless explicitly declared
  writable**; paths are explicit and enumerated, never a blanket home or filesystem root;
  host paths are canonicalised and `/proc`, `/sys` and broad `/dev` are refused (already
  enforced in `dome-config::validate`).
- `.rules/apertures.md` must carry this note: UID sharing is a usability feature, not a
  security boundary.

## Plugin placement (defense engine)

The defense engine runs on the **host side**, inside the supervisor's plugin host, per
`.rules/plugins.md`. A chamber-side agent may report telemetry but is advisory only: a
compromised chamber can lie, and the host-side perimeter and aperture brokers must remain
independent of anything the guest says. Resource-heavy inference belongs host-side and is
itself subject to the supervisor's own limits.
