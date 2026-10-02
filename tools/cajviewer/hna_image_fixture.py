#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original HN-A image-marker controls outside the repository."""

import argparse
import hashlib
import json
from pathlib import Path
import struct

from c8_image_fixture import jpeg


def document(payload, *, x=0, y=0, width=320, height=240, markers=True, prefix=True):
    header = bytearray(348)
    header[:8] = b"HN\0\0" + struct.pack("<I", 400)
    struct.pack_into("<IIII", header, 136, 400, 0, 1, 6)
    struct.pack_into("<HH", header, 168, 320, 240)
    flags = 0xc000 if markers else 0
    words = [0x8003, 320, 0x8003, 240] if prefix else []
    words += [0x800a, 0xd300, flags | x, y, flags | width, height,
              0xc050, 0xc033, 0xc037, 0xc000, 0xc06c, 0xc032, 0xc0f2, 0xc07a,
              0x8004, 1]
    text = struct.pack(f"<{len(words)}H", *words)
    descriptor = 368 + len(text)
    # One image and the observed one-based page ordinal share this row word.
    index = struct.pack("<5I", 368, len(text), 0x10001, 0, 0)
    return header + index + text + struct.pack("<3I", 2, descriptor + 12, len(payload)) + payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    payload = jpeg()
    controls = {"framed": {}, "plain": {"markers": False},
                "offset": {"x": 20, "y": 30, "width": 280, "height": 180},
                "unprefixed": {"prefix": False}}
    manifest = []
    for name, parameters in controls.items():
        data = document(payload, **parameters)
        (args.output / (name + ".caj")).write_bytes(data)
        manifest.append({"name": name, "parameters": parameters,
                         "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
