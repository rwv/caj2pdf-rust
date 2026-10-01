#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original C8 encoded-prefix controls outside the repository."""

import argparse
import hashlib
import json
from pathlib import Path
import struct


def document(characters, *, in_run=False):
    """None omits the prefix; 254 is an intentionally unsupported boundary probe."""
    records = bytearray()
    prefix = b""
    if characters is not None:
        text = ("fixture" * (characters // 7 + 1))[:characters]
        words = [0x80CC, 0x102 + characters] + [0xE000 | ord(c) for c in text]
        prefix = struct.pack("<" + "H" * len(words), *words)
    for pair in ((0x8001, 4294), (0x8002, 0x10A5), (0x801D, 0),
                 (0x8067, 6), (4672, 0xD6D0), (0x8004, 1)):
        records.extend(struct.pack("<HH", *pair))
    at = 16 if in_run else 0
    records[at:at] = prefix
    header = bytearray(80)
    struct.pack_into("<IIII", header, 0, 0xC8, 0, 1, 2)
    header[16:28] = "北大二扫1.00".encode("gbk")
    struct.pack_into("<HHHH", header, 28, 4652, 4274, 300, 230)
    index = struct.pack("<IIIII", 100, len(records), 0, 0, 100 + len(records))
    return bytes(header + index + records)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = []
    for label, count in (("none", None), ("empty", 0), ("5", 5),
                         ("28", 28), ("253", 253), ("254", 254), ("in-run", 5)):
        data = document(count, in_run=label == "in-run")
        name = f"prefix-{label}.caj"
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "characters": count,
                         "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
