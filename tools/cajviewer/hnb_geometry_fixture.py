#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original HN-B axis controls; no production geometry is inferred."""

import argparse
import hashlib
import json
from pathlib import Path
import struct

from c8_style_fixture import document


CONTROLS = (
    ("zero-none", 0, ()),
    ("zero-width36", 0, (0x8070, 36)),
    ("zero-height36", 0, (0x8071, 36)),
    ("zero-both72", 0, (0x8070, 72, 0x8071, 72)),
    ("normal-both72", 0x1084, (0x8070, 72, 0x8071, 72)),
    ("model-style4", 0x1084, ()),
    ("model-explicit35", 0, (0x8070, 35, 0x8071, 35)),
    ("model-explicit35-reset4", 0, (0x8070, 35, 0x8071, 35, 0x8002, 0x1084)),
    ("model-explicit72-reset4", 0, (0x8070, 72, 0x8071, 72, 0x8002, 0x1084)),
    ("model-style12", 0xE58C, ()),
    ("model-explicit112", 0, (0x8070, 112, 0x8071, 112)),
)


def control(style, words):
    """Reuse original glyph records in a separately authored compact container."""
    c8 = document([(style, 0, 6)] * 3, run_words=words)
    header = bytearray(216)
    struct.pack_into("<III", header, 0, 0x4E48, 200, 136)
    struct.pack_into("<IIII", header, 136, 0, 0, 1, 2)
    header[152:164] = c8[16:28]
    header[164:172] = c8[28:36]
    return bytes(header) + struct.pack("<III", 228, len(c8) - 100, 0) + c8[100:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = []
    for name, style, words in CONTROLS:
        data = control(style, words)
        filename = f"axis-{name}.caj"
        (args.output / filename).write_bytes(data)
        manifest.append({"file": filename, "style": style, "words": words,
                         "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
