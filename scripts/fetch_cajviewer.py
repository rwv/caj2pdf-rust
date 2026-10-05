#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Explicitly fetch the pinned Linux viewer installer; never execute it."""

import argparse
import http.client
import json
import os
from pathlib import Path
import tempfile
import urllib.request

from cajviewer_canary import (
    CanaryError, CHUNK_BYTES, INSTALLER_SIZE, verify_installer,
)

MIRROR_URL = (
    "https://github.com/rwv/cajviewer-binaries/releases/download/"
    "linux-9.0.0-24093/cajviewer-9.0.0-24093-linux-x86_64.deb"
)
OFFICIAL_URL = "https://download.cnki.net/cajviewer_9.0_amd64.deb"


def fetch(destination: Path) -> dict:
    """Reuse verified bytes or publish a verified temporary file without overwrite."""
    destination = destination.absolute()
    repository = Path(__file__).resolve().parents[1]
    if destination.resolve().is_relative_to(repository):
        raise CanaryError("keep vendor installers outside this repository")
    cached = os.path.lexists(destination)
    if cached:
        identity = verify_installer(destination)
    else:
        destination.parent.mkdir(parents=True, exist_ok=True)
        descriptor, name = tempfile.mkstemp(prefix=".cajviewer-", dir=destination.parent)
        temporary = Path(name)
        try:
            with os.fdopen(descriptor, "wb") as output:
                with urllib.request.urlopen(MIRROR_URL, timeout=30) as response:
                    size = 0
                    while chunk := response.read(min(CHUNK_BYTES, INSTALLER_SIZE + 1 - size)):
                        size += len(chunk)
                        if size > INSTALLER_SIZE:
                            raise CanaryError("download exceeds pinned installer size")
                        output.write(chunk)
            identity = verify_installer(temporary)
            # Same-directory hard link publishes complete bytes and refuses a raced path.
            os.link(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)
    return {"status": "PASS", "scope": "installer-integrity-only",
            "cached": cached, "installer": identity, "download_url": MIRROR_URL,
            "official_source_url": OFFICIAL_URL, "app_launches": 0, "vendor_passes": 0}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path, help="installer path outside the repository")
    args = parser.parse_args()
    try:
        report = fetch(args.destination)
    except (CanaryError, OSError, ValueError, http.client.HTTPException) as error:
        print(json.dumps({"status": "FAIL", "scope": "installer-integrity-only",
                          "reason": str(error), "app_launches": 0, "vendor_passes": 0}))
        return 1
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
