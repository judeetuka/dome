# What a foundation is

A foundation is a bootable, immutable base: a kernel, an optional initramfs, and a root
filesystem. That is the whole contract. Nothing about it requires a distribution, and nothing
requires you to accept someone else's idea of what belongs in an operating system.

## Nothing about Dome needs an OS vendor

The engine loads a bzImage and boots it. Whatever userland is inside the root filesystem is the
foundation author's business. That gives three honest options, and Dome's own recipes use the
first two:

| Approach | What you get | Rough size | Change a package by |
| --- | --- | --- | --- |
| **Assembled** (Buildroot) | exactly the kernel plus the packages you listed: musl, busybox, your binary | ~12–20 MiB | rebuilding |
| **Distribution base** (Alpine) | apk, coreutils, shells, compilers | ~200 MiB | rebuilding, or `apk add` inside a dev chamber |
| **Your own thing** | whatever you produce, as long as it boots | anything | your call |

The assembled route is the lean one, and it is not a compromise: Buildroot compiles the kernel
and only the userland you name, statically or against musl, with no package manager on the
device. There is nothing to patch at runtime because there is nothing you did not ask for. That
is the same reasoning behind the three-device model — fewer parts is the security property.

## Shells

`busybox` provides `ash`, which is enough to run an init script and to debug a boot. `bash`,
`zsh` and `fish` are ordinary packages: Buildroot has recipes for bash and zsh, and fish is
buildable. Adding one costs its binary and libraries, nothing more. A foundation that carries
all four shells deliberately is a few megabytes, not a hundred.

## Why the developer foundation exists anyway

Compiling a project needs a compiler, headers, and a package manager to fetch dependencies.
That is most of what a distribution is *for*. Rather than pretend a 15 MiB image can build Rust,
there are two recipes:

- `foundations/buildroot/` — the minimal, production-shaped foundation (T29)
- `foundations/alpine/` — the developer foundation with gcc, make, git, strace and gdb (T30)

They share a kernel profile (T31), so a program built in one runs in the other.

## Immutability and the writable layer

The foundation on disk is never written to. A chamber mounts its own layer over it (see
`crates/dome-image/src/store.rs`), so:

- many chambers from one foundation store exactly **one** copy of it;
- a chamber that writes does not affect the foundation or any sibling;
- deleting a chamber deletes only its layer.

On a filesystem that can share extents, the layer costs almost nothing. On ext4 it is a copy,
and `dome foundation list` reports which case you are in rather than leaving you to guess.

## What a foundation is not

- It is not a container image. A container image borrows the host kernel; a foundation carries
  its own, which is why it is larger and why it can be confidential.
- It is not a distribution. Alpine is one recipe, chosen for developer ergonomics, not a
  dependency of the design.
- It is not mutable at runtime. Changes belong in a rebuild or in the chamber's layer.
