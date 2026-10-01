#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original HN-B layout-marker controls in a new external directory."""

import argparse
import hashlib
import json
from pathlib import Path
import struct


def page(rows, codes, number):
    data = bytearray()
    for row in range(rows):
        pairs = [(0x8001, 4700 + row * 500), (0x8002, 0x1084),
                 (0x801D, 0), (0x8067, 6)]
        pairs.extend((5200 + column * 350, code) for column, code in enumerate(codes))
        for pair in pairs:
            data.extend(struct.pack("<HH", *pair))
    data.extend(struct.pack("<HH", 0x8004, number))
    return data


def document(width, marker, prefix=(), run=()):
    # Unequal spans and distinct invented strings expose incorrect page lookup.
    pages = [page(8, (0xD6D0, 0xCEC4, 0xA0C1, 0xA0CD, 0xA0B1), 1),
             page(4, (0xA0D0, 0xA0C1, 0xA0C7, 0xA0C5, 0xA0B2), 2)]
    pages[0][16:16] = struct.pack("<" + "H" * len(run), *run)
    pages[0][:0] = struct.pack("<" + "H" * len(prefix), *prefix)
    header = bytearray(216)
    struct.pack_into("<III", header, 0, 0x4E48, 200, 136)
    struct.pack_into("<IIII", header, 136, marker, 0, 2, 2)
    header[152:164] = "北大二扫1.00".encode("gbk")
    struct.pack_into("<HHHH", header, 164, 4652, 4274, 5105, 7469)
    offset = 216 + 2 * width
    index = bytearray()
    for data in pages:
        index.extend(struct.pack("<III", offset, len(data), 0))
        index.extend(bytes(width - 12))
        offset += len(data)
    return bytes(header + index + b"".join(pages))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = []
    for marker in (0, 0xC8):
        for width in (12, 20):
            name = f"hnb-{'zero' if marker == 0 else 'original'}-{width}.caj"
            data = document(width, marker)
            (args.output / name).write_bytes(data)
            manifest.append({"file": name, "marker": marker, "row_bytes": width,
                             "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    for suffix, prefix in (
        ("none", ()), ("bare", (0xC052, 0xA385)),
        ("footer", (0xC052, 0xA385, 0xFFFF, 5)),
        ("end", (0xC052, 0xA385, 0x8004, 1)),
        ("glyph", (0xC052, 0xA385, 5200, 0xD6D0)),
    ):
        name = f"hnb-c052-{suffix}.caj"
        data = document(12, 0, prefix)
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "marker": 0, "row_bytes": 12,
                         "prefix_words": prefix, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    for suffix, words in (
        ("none", ()), ("weight4", (0x801D, 4)),
        ("control1c", (0x801C, 4)), ("font7", (0x8067, 7)),
        ("control72", (0x8072, 0)), ("control24", (0x8024, 0x281D)),
        ("control53", (0xC053, 0x00E9)),
        ("drawing385", (0x8006, 0xA385, 5200, 4800, 6300, 4850, 0xFFFF, 5)),
    ):
        name = f"hnb-run-{suffix}.caj"
        data = document(12, 0, run=words)
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "marker": 0, "row_bytes": 12,
                         "run_words": words, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    for suffix, words in (
        ("bare", (0x8006, 0xA385, 5200, 4800, 6300, 4850)),
        ("footer", (0x8006, 0xA385, 5200, 4800, 6300, 4850, 0xFFFF, 5)),
        ("footer-only", (0xFFFF, 5)),
        ("next-y", (0x8006, 0xA385, 5200, 4800, 6300, 4850, 0x8001, 5000)),
    ):
        name = f"hnb-drawing-boundary-{suffix}.caj"
        data = document(12, 0, run=words)
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "run_words": words, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    for suffix, words in (
        ("8069-none", ()), ("8069-bare", (0x8069, 0x1084)),
        ("8069-next-y", (0x8069, 0x1084, 0x8001, 5000)),
        ("8069-y-only", (0x8001, 5000)),
        ("8024-bare", (0x8024, 0x2800)),
        ("8024-next-y", (0x8024, 0x2800, 0x8001, 5000)),
    ):
        name = f"hnb-{suffix}.caj"
        data = document(12, 0, run=words)
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "marker": 0, "row_bytes": 12,
                         "run_words": words, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    for suffix, following in (
        ("bare", ()), ("footer", (0xFFFF, 5)), ("next-y", (0x8001, 5000)),
    ):
        name = f"hnb-a381-{suffix}.caj"
        words = (0x8006, 0xA381, 5200, 4800, 6300, 4850) + following
        data = document(12, 0, run=words)
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "marker": 0, "row_bytes": 12,
                         "run_words": words, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
