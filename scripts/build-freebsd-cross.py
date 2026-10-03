#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Cross-build FreeBSD candidate tests; execution in a matching VM is mandatory."""
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

SYSROOTS = {
    "riscv64": (
        "riscv/riscv64", "riscv64gc-unknown-freebsd",
        "2a655295d3536848fb1c07c4ab55142ea15043b928490f4be098a984d12e6828",
    ),
    "powerpc64": (
        "powerpc/powerpc64", "powerpc64-unknown-freebsd",
        "ab46528c6a6b8a59233e0c37610a51de6155d76e4d561b9f5f8a84da07c218a1",
    ),
}
TOOLCHAIN = "+nightly-2026-09-29"

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("arch", choices=SYSROOTS)
args = parser.parse_args()
platform, target, expected = SYSROOTS[args.arch]
clang = shutil.which(os.environ.get("FREEBSD_CLANG", "clang"))
lld = shutil.which(os.environ.get("FREEBSD_LLD", "ld.lld"))
if not clang or not lld:
    parser.error("clang and ld.lld are required")
subprocess.run([clang, "--version"], check=True)
subprocess.run([lld, "--version"], check=True)
subprocess.run(["rustc", TOOLCHAIN, "-vV"], check=True)
stage = Path("target/freebsd-cross")
stage.mkdir(parents=True, exist_ok=False)

with tempfile.TemporaryDirectory(prefix="caj2pdf-freebsd-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
    root = Path(temporary)
    archive = root / "base.txz"
    url = f"https://download.freebsd.org/releases/{platform}/15.1-RELEASE/base.txz"
    digest = hashlib.sha256()
    with urllib.request.urlopen(url, timeout=60) as response, archive.open("wb") as output:
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
            output.write(chunk)
    if digest.hexdigest() != expected:
        raise RuntimeError("FreeBSD sysroot SHA256 does not match the pinned MANIFEST")
    print(f"Verified {url}: {expected}", flush=True)
    sysroot = root / "sysroot"
    sysroot.mkdir()
    subprocess.run([
        "tar", "-xJf", str(archive), "-C", str(sysroot), "--no-same-owner",
        "--warning=no-unknown-keyword", "--wildcards",
        "./lib/*", "./usr/lib/*", "./usr/include/*", "./libexec/*",
    ], check=True)
    linker = root / "linker.sh"
    linker.write_text("#!/bin/sh\nexec " + shlex.join([
        clang, f"--target={args.arch}-unknown-freebsd15.1",
        f"--sysroot={sysroot}", f"-fuse-ld={lld}",
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
