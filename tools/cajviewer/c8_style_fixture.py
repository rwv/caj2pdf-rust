#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original, fixed-position C8 controls for manual viewer observations."""

import argparse
import hashlib
import json
from pathlib import Path
import struct


# These are observed raw values, not an interpreted font/style API.
VARIANTS = (
    ("baseline", 0x1084, 0, 6),
    ("vertical", 0x1085, 0, 6),
    ("horizontal", 0x10A4, 0, 6),
    ("weight", 0x1084, 4, 6),
    ("font5", 0x1084, 0, 5),
    ("font8", 0x1084, 0, 8),
    ("font9", 0x1084, 0, 9),
    ("highbits", 0x0884, 0, 6),
)


def document(styles):
    """One page of eight rows containing the original test string 中文AM1."""
    records = bytearray()
    for row, (style, control, font) in enumerate(styles):
        pairs = [
            (0x8001, 4700 + row * 500),
            (0x8002, style),
            (0x801D, control),
            (0x8067, font),
        ]
        pairs.extend(
            (5200 + column * 350, code)
            for column, code in enumerate((0xD6D0, 0xCEC4, 0xA0C1, 0xA0CD, 0xA0B1))
        )
        for pair in pairs:
            records.extend(struct.pack("<HH", *pair))
    records.extend(struct.pack("<HH", 0x8004, 1))
    header = bytearray(80)
    struct.pack_into("<IIII", header, 0, 0xC8, 0, 1, 2)
    # Observed format identifier; no source document text or font data is copied.
    header[16:28] = "北大二扫1.00".encode("gbk")
    struct.pack_into("<HHHH", header, 28, 4652, 4274, 5105, 7469)
    index = struct.pack("<IIIII", 100, len(records), 0, 0, 100 + len(records))
    return bytes(header + index + records)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    fixtures = [("grid", [v[1:] for v in VARIANTS])]
    fixtures.extend((name, [(style, control, font)] * 8) for name, style, control, font in VARIANTS)
    manifest = []
    for name, styles in fixtures:
        data = document(styles)
        (args.output / f"{name}.caj").write_bytes(data)
        manifest.append({"name": name, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "rows": styles})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
