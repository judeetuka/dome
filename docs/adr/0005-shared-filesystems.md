# ADR 0005 — Sharing a filesystem between chambers is a trust decision

Status: **Proposed** (2026). The primitive does not exist yet; this records what it must do when
it arrives.

## The question

Can several chambers mount the same host filesystem?

**Yes, but the answer splits in two, because the two halves are not equally safe.**

## Read-only sharing: safe, and the default

A host directory exposed read-only to N chambers:

- no chamber can modify what another reads;
- the isolation claim ADR 0002 makes is untouched, because there is no write path between
  chambers at all;
- it covers the common cases: a dataset, a build cache keyed by content, a shared toolchain.

This is unremarkable and should simply work.

## Writable sharing: a deliberate merge of trust domains

A writable mount into two chambers makes them able to affect each other's files. That is exactly
the property the product exists to prevent, so it cannot be a silent convenience:

- **It must be declared.** Not implied by a path; a cell asks for it.
- **It must be marked.** A chamber with a writable shared mount is not isolated from its peers,
  and the tool should say so in the same way a degraded seal is marked, rather than leaving the
  operator to remember.
- **Two writers are one domain.** Anything either chamber can read, the other can corrupt. If one
  chamber is fed untrusted input, treat both as exposed.

## What to learn from the projects that did this first

- **Docker volumes** are shared by default and nothing in the command warns that two containers
  writing one volume have become one failure domain. That silent default is the flaw not copied.
- **Kata and Firecracker** have no shared-filesystem primitive at all; you attach virtio-fs or
  9p yourself, which is safe by omission but unhelpful for the common read-only case.
- **libvirt** makes sharing an explicit device with a mode, which is the right shape: the
  operation names itself.

## Decision

1. **Read-only shared mounts**: supported, safe, no ceremony beyond declaring them.
2. **Writable shared mounts**: supported, requiring an explicit opt-in that names them as a
   shared trust domain, and reported by `dome ps` so the state is visible after the fact.
3. **Never implicit.** No path becomes shared because two cells happen to mention it.
4. **Coherence via one host-side server per mount**, not per chamber. Two independent servers
   over one directory would produce silent lost updates; one server with one cache is coherent
   by construction.

## Not claimed

A shared mount is not a security boundary and does not become one. It moves data between
chambers by design. Anyone who needs chambers that cannot influence each other must not share a
writable mount between them, and the documentation says so plainly instead of leaving it to be
discovered.
