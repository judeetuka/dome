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
| **Assembled** (Buildroot) | exactly the kernel plus the packages you listed | ~12–20 MiB | rebuilding |
| **mise toolchains** | self-contained per-tool directories, no root and no distribution | depends on the tool | reinstalling the toolchain |
| **Your own thing** | whatever you produce, as long as it boots | anything | your call |

**No distribution base is on that list, and that is deliberate.** Alpine was in the plan as a
convenience; it is out. A base distro brings a package manager, an init, and a set of opinions
about what belongs on a system, and the point of a foundation is that it carries *only* what the
recipe names.

The assembled route is the lean one, and it is not a compromise: Buildroot compiles the kernel
and only the userland you name, statically or against musl, with no package manager on the
device. There is nothing to patch at runtime because there is nothing you did not ask for. That
is the same reasoning behind the three-device model — fewer parts is the security property.

## Shells

`busybox` provides `ash`, which is enough to run an init script and to debug a boot. `bash`,
`zsh` and `fish` are ordinary packages: Buildroot has recipes for bash and zsh, and fish is
buildable. Adding one costs its binary and libraries, nothing more. A foundation that carries
all four shells deliberately is a few megabytes, not a hundred.

## Where packages come from without a distribution

[mise](https://mise.jdx.dev) installs self-contained tool directories: a version of node, python
or rust lives under its own path and needs no root, no system package manager, and no base image.
That fits a foundation exactly, because "install a package" becomes "copy a directory".

```toml
[runtime]
packages = ["mise:node@20.11.0", "mise:python@3.12"]

[build]
packages = ["mise:rust@1.95.0"]
```

Two honest caveats:

- Most mise toolchains link against the host's C library, so shipping one means shipping that
  library too — which is what `static_linking = false` with `libc` declares. A statically linked
  binary avoids the question entirely.
- Copying a toolchain into an image is not implemented yet; today a recipe with packages is
  **refused** rather than silently producing an image without them.

## Immutability and the writable layer

The foundation on disk is never written to. A chamber mounts its own layer over it (see
`crates/dome-image/src/store.rs`), so:

- many chambers from one foundation store exactly **one** copy of it;
- a chamber that writes does not affect the foundation or any sibling;
- deleting a chamber deletes only its layer.

On a filesystem that can share extents, the layer costs almost nothing. On ext4 it is a copy,
and `dome foundation list` reports which case you are in rather than leaving you to guess.

## No base at all: kernel plus what the recipe names

A foundation does not need a distribution, a base image, or an init system. The kernel execs one
binary ([kernel documentation on init](https://docs.kernel.org/admin-guide/init.html)), and that
binary is the only thing that has to work.

The kernel's own list of ways that fails is the design brief:

1. the init binary does not exist on the root filesystem;
2. the console is unavailable, so there is nowhere to say so;
3. **the binary's library dependencies are missing** (`readelf -d` shows `NEEDED`);
4. the binary's architecture does not match;
5. it is a script whose interpreter is not itself working.

Every one of those is removed by making the workload a **static** binary: no libraries to miss,
one file to place, one architecture to match.

### What the choices cost, measured

Measured on an Arch host, because these numbers decide the design:

| Piece | Installed |
| --- | --- |
| glibc | **50.7 MiB** |
| systemd (+ libs, dbus) | **45.5 MiB** |
| bash | 9.6 MiB |
| Arch `linux` package (kernel + modules) | 151.5 MiB — **not used** |
| Dome kernel image (bzImage) | 11.7 MB |

The first row is the one that matters most. **Any dynamically linked binary drags in glibc**, so
"kernel plus your program" is only literally true when the program is static. That is why
`static_linking = true` is the default in a recipe rather than an option.

### Init is a field, not a decision made for you

| `init` | What it means |
| --- | --- |
| `none` | the kernel execs the workload directly: no init at all |
| `sentinel` | Dome's own minimal init; reaps orphans and execs the workload |
| `busybox` | BusyBox's init applet |
| `systemd`, `openrc`, `runit`, `s6`, `dinit` | a service manager, your choice |
| `path:/usr/local/sbin/foo` | any binary at that path in the image |

Worth knowing before you pick: **runit, s6, dinit, openrc, tini and dumb-init are not in the
official Arch repositories** (checked with `pacman -Si` on this host). Choosing one means the AUR
or building it, which the builder must be able to do. systemd is in every repository and costs
about 45 MiB.

### The kernel comes from a profile, not from the command line

A user names a profile; the profile resolves to an image on this host. Nobody passes a kernel
path, for the same reason nobody passes a C library:

```toml
[foundation]
kernel = "dome-minimal"     # a profile, not a file
```

```text
dome kernel import --profile dome-minimal --from <bzImage>   # once, by whoever provides it
dome build --recipe dome.toml --output foundations/dome-minimal
```

Kernel profiles live in `$XDG_DATA_HOME/dome/kernels/<profile>/vmlinuz`. Importing validates the
boot signature, so a file that is not a kernel image is refused where it is installed rather than
where it fails to boot. `--kernel` still exists on `build` as a hidden development override.

### Our own kernel, never a distribution kernel package

151.5 MiB of that table is the Arch `linux` package, and Dome does not use it. Dome builds its
own kernel image (11.7 MB) from a profile (T31) with the devices this engine actually exposes —
virtio-mmio, a 16550 UART — compiled in. There are no modules, because there is no hardware to
have modules for. That single decision is worth more than every package choice combined.

### Sections, because one list answers three questions

Compiling needs a toolchain; running does not. A recipe therefore has two stages, and the
release is assembled from the first one only:

```toml
[foundation]
name = "app"
arch = "x86_64"
kernel = "dome-minimal"   # Dome builds it; no distribution kernel involved
init = "sentinel"         # none | sentinel | busybox | systemd | openrc | runit | s6 | dinit | path:/...
static_linking = true     # the default; see below
workload = "/usr/bin/app"
package_manager = "none"  # none (default) | apk | pacman

[runtime]
packages = ["fish@3.7.1", "neovim"]      # what the release keeps; empty is valid

[build]
packages = ["rust@1.80.0", "gcc", "git"] # used to build, never shipped
artifacts = ["/build/target/release/app"]

[shell]
default = "fish"
packages = ["fish@3.7.1"]
```

Each section answers a different question: what the release keeps, what only the build needs, and
what an interactive user gets. Packages may be pinned with `name@version`.

Because the release is built from the runtime closure and the artifacts alone, the toolchain
cannot survive by accident — there is no "remove it afterwards and hope the dependencies went
with it" step. Nothing that was installed only for building is ever present to remove.

Validation is strict about the things that would be quiet mistakes: a package listed in both
stages is refused (nobody could tell which copy the image has), an artifact path must be
absolute, and an unknown init, architecture or package manager is rejected by name.

### One config file, and it can include others

A project does not repeat itself. `dome.toml` may include fragments, and the rule is the boring
one because a clever merge is a merge nobody can predict:

```toml
include = ["base.toml", "rust.toml"]

[foundation]
name = "app"          # wins over anything an include said
```

- includes are applied first, **the including file wins** for any single value;
- **lists concatenate**, include entries first, with duplicates removed;
- a cycle is refused rather than followed, and a missing include names the file it wanted.

### Any package may name its own manager

Without a prefix, the recipe's builder decides. With one, the package says which tool fetches it:

```toml
[runtime]
packages = ["fish", "pacman:docker", "yay:visual-studio-code-bin", "pacman:git@2.45.2"]
```

The prefix survives into the plan, so a report shows exactly what would be fetched and with what.

### Static linking is the default, and turning it off is bookkept

Static linking is not a style preference here. Any dynamically linked binary needs a **C library
and the dynamic loader**, which is why glibc shows up as 50.7 MiB in the table above. The kernel's
own documentation lists missing library dependencies as a way boot fails, and a static binary
cannot fail that way.

Setting `static_linking = false` is supported, and the engine does the arithmetic rather than
leaving it to memory: **the C library is added to the release closure automatically**, glibc by
default or `libc = "musl"`. The plan says so out loud.

What you give up by turning it off: a smaller total image (many binaries share one libc), and
things that need a dynamic loader at runtime — `dlopen` for plugins, NSS for user and DNS
lookups through `libnss_*`, and language runtimes that load native extensions. Those are real
needs; they are just not the default, because most workloads do not have them.

### Who the chamber runs as, and where the project lives

```toml
[user]
mode = "host"         # none (default) | host | custom
# name = "dev"
# uid  = 1000
# gid  = 1000
# home = "/home/dev"

[workspace]
path = "/home/dev/project"     # inside the image
host = "/srv/project"          # optional mapping; an aperture, so its rules apply
```

`mode = "host"` copies the host's identity — name, uid, gid — into the chamber. That is what
keeps a mapped project directory owned by you instead of by root, and it is also how Windows and
macOS behave the same way: neither has a meaningful uid, so the engine maps the host identity to
a numeric uid inside the image. `custom` is there for a fixed uid that must match other systems.

### The package manager is opt-in, and usually absent

By default the release has **no package manager**. The builder fetches the named packages on the
host and writes them into the root filesystem, which means the image contains nothing that can
install anything — a much smaller surface than an image carrying apk or pacman.

`package_manager = "apk"` or `"pacman"` puts one in, for images that genuinely should install at
runtime. That is a choice with a cost and it is recorded as one: the plan says the image can
install packages, rather than leaving it to be discovered.

### What an empty recipe means

```toml
[foundation]
name = "app"
init = "sentinel"
```

That is a complete, valid foundation: Dome's kernel, the sentinel init, static linking, no
packages, no build stage, no package manager. Roughly 12 MB, and it boots. Anything more is
something you asked for.

The recipe is validated (`crates/dome-config/src/foundation_recipe.rs`, twenty-one tests) and
`plan()` states what the choices cost: it adds the init system's own dependencies to the closure,
and reports the measured price of a service manager or of dynamic linking rather than leaving it
to be discovered.

## The base foundation: clean sheet, then install

The smallest useful foundation is **a kernel plus the least userland that can install more**.
Nothing else. No editors, no compilers, no package bloat shipped "just in case" — because every
preinstalled package is a package you did not ask for and will one day have to patch.

Two bases, because they trade differently:

| Base | Package manager | Universe | Character |
| --- | --- | --- | --- |
| `base-musl` | `apk` | Alpine | leanest; nothing you did not name |
| `base-arch` | `pacman` (and `yay` for the AUR) | Arch + AUR | widest universe, heaviest base |

### What the pieces actually cost

Measured on this Arch host, not estimated:

| Piece | Installed size |
| --- | --- |
| `bash` | 9.6 MiB |
| `pacman` + keyring + mirrorlist | ~7 MiB |
| `gcc` | 220.9 MiB |
| Arch `linux` package (kernel **and modules**) | 151.5 MiB |
| Dome's own kernel image (bzImage) | 11.7 MB |

Two conclusions, one of them against intuition:

1. **A bare Arch base is not lean.** Its kernel package alone is 151.5 MiB installed, thirteen
   times our 11.7 MB image, because Arch ships modules for every piece of hardware. Arch earns
   its place through the package universe, not through size.
2. **"Compilers out of the box" and "no bloat" cannot both be true.** `gcc` alone is 220.9 MiB.
   A base that ships a toolchain is an image most chambers will never need.

### The workflow this buys

Install once, commit, reuse — and the committed set becomes a shared, digest-addressed foundation:

```text
dome foundation commit --from base.img --name base        # import a base
dome run app.cell.toml                                    # chamber starts from "base"
  # inside: pacman -S --needed fish rust neovim
dome foundation commit --from <chamber layer> --name dev  # commit what was installed
dome run other.cell.toml                                  # starts from "dev", pays nothing again
```

The last step is the important one. Because foundations are content-addressed (T64), two people
who installed the same set converge on the same digest and the bytes are stored once. The user
starts from a clean sheet, adds only what they need, and the result is shareable rather than
private to one machine.

### Caveats worth stating

- A chamber that installs packages needs **network access** (a declared perimeter) and room in
  its writable layer.
- `commit` stores the layer **unfiltered**. A package cache, a downloaded source tarball or a
  private key in that layer travels into every chamber started from the result. Cleaning the
  layer first is the caller's job, and the store does not pretend to know what is sensitive.
- A committed foundation is immutable. Changing it means committing again, which produces a new
  digest — that is the reproducibility benefit, and also the reason a base should not carry
  anything you would have to patch weekly.

## Stacks: several chambers that belong together

A `.stack.toml` names services, each backed by a cell, and the order they start in:

```toml
[stack]
name = "app"

[services.web]
cell = "web.cell.toml"

[services.api]
cell = "api.cell.toml"
depends_on = ["web"]
```

A dependency says what it is waiting **for**, because Docker's `depends_on` only waits for the
process to start — the classic race where the database is running long before it accepts
connections:

```toml
[services.api]
cell = "api.cell.toml"
depends_on = ["db:healthy", "cache:port:6379"]   # or just "web" for started
```

| Form | Waits until |
| --- | --- |
| `"web"` | the chamber's process is running (Docker's semantics) |
| `"db:healthy"` | the chamber's health check reports healthy (T25's tracker) |
| `"cache:port:6379"` | something accepts a connection on that port inside the chamber |

Dependencies decide the start order, and a cycle is refused at parse time rather than showing up
later as chambers waiting on each other. Service names are hostnames on the stack's own network.

**The trust boundary is membership.** Chambers in a stack can reach each other, and nothing
outside the stack can reach them unless a cell publishes a port through its perimeter. That means
a stack is a group of chambers that already trust each other — the same reasoning as the shared
filesystem in ADR 0005, and the same warning applies: two services in one stack are one failure
domain in the network sense.

## What a foundation is not

- It is not a container image. A container image borrows the host kernel; a foundation carries
  its own, which is why it is larger and why it can be confidential.
- It is not a distribution. Alpine is one recipe, chosen for developer ergonomics, not a
  dependency of the design.
- It is not mutable at runtime. Changes belong in a rebuild or in the chamber's layer.
