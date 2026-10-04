#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Prepare an isolated Pentium-compatible GNU sysroot and pinned QEMU for CI."""
import hashlib
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import urllib.request

# Debian Jessie i386 packages retain the Pentium baseline. These are external
# build/test inputs, never installed on the host or included in CLI archives.
PACKAGES = [
    ("pool/main/g/gcc-4.9/libgcc-4.9-dev_4.9.2-10+deb8u1_i386.deb",
     "26dea9f1b54c86dba2059a9b1416cc8f45de75b2f7a26f3fdc52cb82d37e45a0"),
    ("pool/main/g/gcc-4.9/libgcc1_4.9.2-10+deb8u1_i386.deb",
     "8d6f382ac18f77f2be38ab210428d6fba598fc3d5c197572f665330d79b4f27f"),
    ("pool/main/g/glibc/libc6_2.19-18+deb8u10_i386.deb",
     "1d1f82f905386ff6a1dab08ec69b3b1d8bddbfd7c41b68e295e0a204232ad1ae"),
    ("pool/main/g/glibc/libc6-dev_2.19-18+deb8u10_i386.deb",
     "287ea40a96c4eafb94843169f6bf802335182b68f78a1b4bab95851c3385866d"),
    ("pool/main/l/linux/linux-libc-dev_3.16.56-1+deb8u1_i386.deb",
     "4334b2ce28c3a6cf89ec5cfdd390d6e518209dd17f8439f74efe27f1591df806"),
]
root = Path(os.environ["RUNNER_TEMP"]) / "caj2pdf-i586"
env_file = Path(os.environ["GITHUB_ENV"])
clang = shutil.which(os.environ.get("I586_CLANG", "clang"))
lld = shutil.which(os.environ.get("I586_LLD", "ld.lld"))
if not clang or not lld:
    raise RuntimeError("clang and ld.lld are required")
root.mkdir(parents=True, exist_ok=False)
sysroot = root / "sysroot"


def extract(url, expected, destination):
    archive = root / url.rsplit("/", 1)[1]
    digest = hashlib.sha256()
    with urllib.request.urlopen(url, timeout=60) as response, archive.open("wb") as output:
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
            output.write(chunk)
    if digest.hexdigest() != expected:
        raise RuntimeError(f"SHA256 mismatch for {archive.name}")
    subprocess.run(["dpkg-deb", "-x", str(archive), str(destination)], check=True)
    print(f"Verified {archive.name}: {expected}", flush=True)


for name, expected in PACKAGES:
    extract("https://archive.debian.org/debian/" + name, expected, sysroot)
# Relocate package symlinks so absolute library references cannot accidentally
# resolve to host libraries. ELF and archive contents remain unchanged.
for path in sysroot.rglob("*"):
    if path.is_symlink() and (old := os.readlink(path)).startswith("/"):
        destination = sysroot / old.lstrip("/")
        if not destination.exists():
            raise RuntimeError(f"Missing sysroot symlink destination: {old}")
        path.unlink()
        path.symlink_to(os.path.relpath(destination, path.parent))

extract(
    "https://deb.debian.org/debian/pool/main/q/qemu/qemu-user_10.0.13+ds-0+deb13u1_amd64.deb",
    "ca6ede739327a20ae5a3498230c8a3a9a072c586e131bc6bcc1201eb1725e336",
    root / "qemu",
)
linker = root / "linker.sh"
linker.write_text("#!/bin/sh\nexec " + shlex.join([
    clang, "--target=i586-unknown-linux-gnu", "-march=pentium",
    f"--sysroot={sysroot}", f"--gcc-install-dir={sysroot}/usr/lib/gcc/i586-linux-gnu/4.9",
]) + ' "$@" ' + shlex.join([f"-fuse-ld={lld}", "-Wl,--fatal-warnings"]) + "\n")
linker.chmod(0o755)
qemu = root / "qemu/usr/bin/qemu-i386"
# QEMU -L redirects existing paths, but does not isolate the host ld.so.cache.
# Search the pinned guest libraries before consulting that cache.
runner = (f"{qemu} -cpu pentium -L {sysroot} "
          "-E LD_LIBRARY_PATH=/lib/i386-linux-gnu:/usr/lib/i386-linux-gnu")
subprocess.run([clang, "--version"], check=True)
subprocess.run([lld, "--version"], check=True)
subprocess.run([str(qemu), "--version"], check=True)
with env_file.open("a") as output:
    for name, value in {
        "CARGO_TARGET_I586_UNKNOWN_LINUX_GNU_LINKER": str(linker),
        "CARGO_TARGET_I586_UNKNOWN_LINUX_GNU_RUSTFLAGS": "-C linker-flavor=gcc -C link-self-contained=no",
        "CARGO_TARGET_I586_UNKNOWN_LINUX_GNU_RUNNER": runner,
        "CAJ2PDF_TEST_RUNNER": runner,
    }.items():
        output.write(f"{name}={value}\n")
