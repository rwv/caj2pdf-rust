#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Cross-build BSD candidate tests; execution in a matching VM is mandatory.

The sysroot comes from the official release sets, each checked against the
digest pinned below from the project's published checksum file.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import urllib.request

FREEBSD = "https://download.freebsd.org/releases"
# 14.3 left the main mirror; the archive serves only plain HTTP, so the pinned
# MANIFEST digest is the integrity check.
FREEBSD_ARCHIVE = "http://ftp-archive.freebsd.org/pub/FreeBSD-Archive/old-releases"
NETBSD = "https://cdn.netbsd.org/pub/NetBSD/NetBSD-11.0"
OPENBSD = "https://cdn.openbsd.org/pub/OpenBSD/7.9"
FREEBSD_MEMBERS = ("./lib/*", "./usr/lib/*", "./usr/include/*", "./libexec/*")
LIBS = ("./usr/lib/*",)
DEVELOPMENT = ("./usr/lib/*", "./usr/include/*")

# (os, arch): (Rust target, Clang triple, [(set URL, algorithm, digest, members)])
SYSROOTS = {
    ("freebsd", "aarch64"): ("aarch64-unknown-freebsd", "aarch64-unknown-freebsd14.3", [
        (f"{FREEBSD_ARCHIVE}/arm64/aarch64/14.3-RELEASE/base.txz", "sha256",
         "f83e824cb7a20dbadb2888a8bd253e6a1ac35024cc6dc8af9f3e229f75ec7129", FREEBSD_MEMBERS),
    ]),
    ("freebsd", "riscv64"): ("riscv64gc-unknown-freebsd", "riscv64-unknown-freebsd15.1", [
        (f"{FREEBSD}/riscv/riscv64/15.1-RELEASE/base.txz", "sha256",
         "2a655295d3536848fb1c07c4ab55142ea15043b928490f4be098a984d12e6828", FREEBSD_MEMBERS),
    ]),
    ("freebsd", "powerpc64"): ("powerpc64-unknown-freebsd", "powerpc64-unknown-freebsd15.1", [
        (f"{FREEBSD}/powerpc/powerpc64/15.1-RELEASE/base.txz", "sha256",
         "ab46528c6a6b8a59233e0c37610a51de6155d76e4d561b9f5f8a84da07c218a1", FREEBSD_MEMBERS),
    ]),
    ("netbsd", "aarch64"): ("aarch64-unknown-netbsd", "aarch64-unknown-netbsd11.0", [
        (f"{NETBSD}/evbarm-aarch64/binary/sets/base.tar.xz", "sha512",
         "d17b3253959e110edba1755f481e707fd79a1a4a35bd7096a305c2959d6a5964"
         "713d4c35f439d76ca03d9252f8652d73f521ea6fc07a27ffa5ca22a80df6e7c5", ("./lib/*", "./usr/lib/*")),
        (f"{NETBSD}/evbarm-aarch64/binary/sets/comp.tar.xz", "sha512",
         "0719f3d94fd5cdedf83ad7418f1e254c898b388c320f28d3d39b8abf96fe6ea5"
         "8b605be8dc9e02d4d76021f87e3f0cd2bc327560e887abc572fbae04742305fd", DEVELOPMENT),
    ]),
    ("openbsd", "aarch64"): ("aarch64-unknown-openbsd", "aarch64-unknown-openbsd7.9", [
        (f"{OPENBSD}/arm64/base79.tgz", "sha256",
         "9a4b16701ec11103847ca314527de5caf5f811c1dbc0e348d56ed8e9d9f16cb1", LIBS),
        (f"{OPENBSD}/arm64/comp79.tgz", "sha256",
         "87d257a0e191a32fdebe9c2755e7245c81472f41456ea2fd1ca9832ed55fc417", DEVELOPMENT),
    ]),
    ("openbsd", "riscv64"): ("riscv64gc-unknown-openbsd", "riscv64-unknown-openbsd7.9", [
        (f"{OPENBSD}/riscv64/base79.tgz", "sha256",
         "ff874be82064dae142bc56578865de08d07c1b6835794953748a27e0cc89dfb4", LIBS),
        (f"{OPENBSD}/riscv64/comp79.tgz", "sha256",
         "97d38097b9ee8bd89c103949682964bd77c7adf57a344ead9d3ac4dd6b304a8c", DEVELOPMENT),
    ]),
}
TOOLCHAIN = "+nightly-2026-09-29"

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("os", choices=sorted({system for system, _ in SYSROOTS}))
parser.add_argument("arch", choices=sorted({arch for _, arch in SYSROOTS}))
args = parser.parse_args()
if (args.os, args.arch) not in SYSROOTS:
    parser.error(f"no pinned sysroot for {args.os} {args.arch}")
target, triple, sets = SYSROOTS[args.os, args.arch]
clang = shutil.which(os.environ.get("BSD_CLANG", "clang"))
lld = shutil.which(os.environ.get("BSD_LLD", "ld.lld"))
if not clang or not lld:
    parser.error("clang and ld.lld are required")
subprocess.run([clang, "--version"], check=True)
subprocess.run([lld, "--version"], check=True)
subprocess.run(["rustc", TOOLCHAIN, "-vV"], check=True)
stage = Path("target/bsd-cross")
stage.mkdir(parents=True, exist_ok=False)

with tempfile.TemporaryDirectory(prefix="caj2pdf-bsd-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
    root = Path(temporary)
    sysroot = root / "sysroot"
    sysroot.mkdir()
    for url, algorithm, expected, members in sets:
        archive = root / url.rsplit("/", 1)[1]
        digest = hashlib.new(algorithm)
        with urllib.request.urlopen(url, timeout=60) as response, archive.open("wb") as output:
            while chunk := response.read(1024 * 1024):
                digest.update(chunk)
                output.write(chunk)
        if digest.hexdigest() != expected:
            raise RuntimeError(f"{url} does not match its pinned {algorithm}")
        print(f"Verified {url}: {algorithm} {expected}", flush=True)
        subprocess.run([
            "tar", "-xf", str(archive), "-C", str(sysroot), "--no-same-owner",
            "--warning=no-unknown-keyword", "--wildcards", *members,
        ], check=True)
        archive.unlink()
    if args.os == "openbsd":
        # OpenBSD ships only versioned libraries (libc.so.103.0) and its own
        # linker picks the newest; give upstream LLD the unversioned names so
        # it links them dynamically instead of falling back to static archives.
        for library in sorted((sysroot / "usr/lib").glob("lib*.so.*")):
            plain = library.with_name(library.name.split(".so.", 1)[0] + ".so")
            if not plain.exists():
                plain.symlink_to(library.name)
    linker = root / "linker.sh"
    linker.write_text("#!/bin/sh\nexec " + shlex.join([
        clang, f"--target={triple}", f"--sysroot={sysroot}", f"-fuse-ld={lld}",
    ]) + ' "$@"\n')
    linker.chmod(0o755)
    env = dict(os.environ)
    env[f"CARGO_TARGET_{target.upper().replace('-', '_')}_LINKER"] = str(linker)
    env["CARGO_TARGET_DIR"] = str(root / "build")
    names = {"caj2pdf_core": "core-tests", "portable": "portable-tests", "caj2pdf": "caj2pdf"}
    found = set()
    for selection in (["-p", "caj2pdf-core", "--lib"], ["-p", "caj2pdf-cli", "--test", "portable"]):
        with tempfile.TemporaryFile(mode="w+") as messages:
            subprocess.run([
                "cargo", TOOLCHAIN, "test", "-Z", "build-std", "--locked",
                "--release", "--no-run", "--target", target,
                "--message-format=json-render-diagnostics", *selection,
            ], env=env, stdout=messages, check=True)
            messages.seek(0)
            for line in messages:
                artifact = json.loads(line)
                if artifact.get("reason") == "compiler-artifact" and artifact.get("executable"):
                    name = names[artifact["target"]["name"]]
                    shutil.copy2(artifact["executable"], stage / name)
                    found.add(name)
    if found != set(names.values()):
        raise RuntimeError(f"Missing candidate executables: {set(names.values()) - found}")
shutil.copy2("tests/fixtures/valid_nested_outline.pdf", stage / "input.pdf")
print(f"Built {target}; these files are unverified until the VM tests pass.")
