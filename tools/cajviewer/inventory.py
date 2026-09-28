#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opaque public runtime identities, without executing the vendor application."""

from __future__ import annotations

import json
import os
from pathlib import Path
import stat

from cajviewer_canary import CanaryError, hash_regular, run_bounded


def inventory():
    records = []
    total = 0
    for root in ("/opt/cajviewer", "/usr/lib/x86_64-linux-gnu", "/usr/share/fonts", "/etc/fonts", "/etc/X11", "/opt/canary"):
        for directory, subdirectories, files in os.walk(root, followlinks=False):
            for name in sorted(subdirectories + files):
                path = Path(directory) / name
                metadata = path.lstat()
                record = {"path": str(path), "mode": stat.S_IMODE(metadata.st_mode)}
                if stat.S_ISLNK(metadata.st_mode):
                    record.update(type="symlink", target=os.readlink(path))
                elif stat.S_ISDIR(metadata.st_mode):
                    record.update(type="directory")
                elif stat.S_ISREG(metadata.st_mode):
                    total += metadata.st_size
                    if total > 4 * 1024 ** 3:
                        raise CanaryError("runtime inventory byte budget exceeded")
                    record.update(type="opaque-file", **hash_regular(path))
                else:
                    raise CanaryError("unexpected runtime inventory file type")
                records.append(record)
                if len(records) > 8192:
                    raise CanaryError("runtime inventory count budget exceeded")
    tools = {}
    for name in ("python3", "Xvfb", "xdotool", "xclip", "xdpyinfo", "openbox", "fc-list", "dpkg-query"):
        path = (Path("/usr/bin") / name).resolve(strict=True)
        tools[name] = {"path": str(path), **hash_regular(path)}
    observations = {}
    for name, argv in (("packages", ["dpkg-query", "-W", "-f=${Package}\t${Version}\t${Architecture}\n"]),
                       ("fontconfig", ["fc-list", "--format", "%{file}\t%{family}\t%{style}\n"])):
        result = run_bounded(argv, deadline_seconds=10, output_limit=256 * 1024)
        if result["status"] != "PASS":
            raise CanaryError("runtime metadata helper failed")
        observations[name] = result["stdout"].decode("utf-8", "strict")
    return {"status": "PASS", "scope": "opaque-runtime-inventory-only", "files": sorted(records, key=lambda r: r["path"]),
            "tools": tools, "observations": observations, "files_hashed_bytes": total,
            "app_launches": 0, "vendor_passes": 0}


if __name__ == "__main__":
    print(json.dumps(inventory(), sort_keys=True))
