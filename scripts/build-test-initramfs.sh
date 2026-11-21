#!/bin/sh
# Build the minimal initramfs used by the KVM boot test.
#
# Produces tests/fixtures/kernel/initramfs.cpio.gz containing a single static /init.
# A static binary is required: the initramfs has no dynamic loader in it.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "compiling static init"
cc -static -O2 -o "$work/init" "$repo_root/scripts/test-initramfs/init.c"

mkdir -p "$work/root/proc" "$work/root/sys" "$work/root/dev"
cp "$work/init" "$work/root/init"
chmod 0755 "$work/root/init"

echo "packing cpio (newc) and compressing"
( cd "$work/root" && find . -print0 | cpio --null --create --format=newc --quiet ) \
    | gzip -9 > "$repo_root/tests/fixtures/kernel/initramfs.cpio.gz"

ls -l "$repo_root/tests/fixtures/kernel/initramfs.cpio.gz"
