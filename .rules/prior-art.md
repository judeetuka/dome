# Prior art: learn from the projects that already did this

Dome is not the first sandbox engine. Docker, Firecracker, QEMU/KVM, gVisor, Kata and systemd
have been solving these problems in production for years, and their design documents are
free to read. **Consult them before designing anything non-trivial**, and record in the plan
what was taken and what was rejected.

## The method

1. State the problem in one sentence.
2. Find the primary source: upstream design docs, man pages, specification documents. Not
   blog summaries, not recollection.
3. Note what each project does and **why**.
4. Take the best, and name the flaw being fixed. Copying a design without naming its flaw is
   how you inherit it.

## What has already been learned this way

- **Firecracker** (`docs/design.md`): a minimal device model is the security boundary. Three
  devices, no BIOS, no PCI hole. Adopted in ADR 0002.
- **Firecracker**: defence in depth at process level (seccomp, cgroups, namespaces, jailer)
  assumes the virtualisation boundary fails. Adopted in T20.
- **gVisor** (security model): minimise the host interface the sandbox can reach, so a second
  exploit has nowhere to go. Adopted as the device-set rule.
- **Kata** (threat model): a guest can starve the host without escaping, so availability is an
  isolation property. Adopted as the cgroup and relay limits.
- **Docker** (`docker stop`, `HEALTHCHECK`): signal → grace → force for stopping; health is a
  status alongside state and never a terminator. Adopted in T25.
- **Docker** (`--restart`): restart policy decides what happens after exit. Adopted, with one
  flaw fixed — see T70.
- **QEMU / libvirt**: the VMM does not supervise itself; lifecycle belongs to an external
  manager. Adopted: the supervisor owns lifecycle, the engine stays a dumb executor.

## Flaws deliberately not copied

- Docker's `always` restarts a manually stopped container on daemon restart, which surprises
  people. Dome records *why* a chamber stopped, so reconciliation is deterministic.
- Docker has no notion of which process may restart what; Dome keeps the record explicit.
