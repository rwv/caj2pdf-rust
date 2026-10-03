#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original two-glyph controls for additional native C8 framing."""

import argparse
import hashlib
import json
from pathlib import Path
import struct

from c8_encoded_prefix_fixture import document


CONTROLS = (
    ("baseline", ()),
    ("80ce-0", (0x80CE, 0)), ("80ce-1", (0x80CE, 1)),
    ("801c-4", (0x801C, 4)), ("801d-3", (0x801D, 3)),
    ("8024-2800", (0x8024, 0x2800)), ("8024-281d", (0x8024, 0x281D)),
    ("8021-2000", (0x8021, 0x2000)),
    ("80d0-0", (0x80D0, 0)), ("80d1-1", (0x80D1, 1)),
    ("80d2-0", (0x80D2, 0)),
    ("8070-4", (0x8070, 4)), ("8071-4", (0x8071, 4)),
    ("81ff-1", (0x81FF, 1, 0, 200)),
    ("81ff-2", (0x81FF, 2, 0, 200)),
    ("81ff-3", (0x81FF, 3, 0, 200)),
    ("80cc-0204", (0x80CC, 0x0204, 33, 5)),
)


def control_document(words):
    base = document(None)
    # Preserve the run controls and first glyph; place one control before a
    # second, asymmetric glyph. In the original geometric font these are a
    # full square and upper half-square, so losing either is visible.
    records = base[100:120] + struct.pack("<" + "H" * len(words), *words)
    records += struct.pack("<HHHH", 4802, 0xCEC4, 0x8004, 1)
    header = bytearray(base[:100])
    struct.pack_into("<I", header, 84, len(records))
    struct.pack_into("<I", header, 96, 100 + len(records))
    return bytes(header + records)


def mixed_documents():
    """Original controls with independently distinguishable font resources."""
    from c8_image_fixture import jpeg, mixed_control

    labels = {"baseline", "80ce-0", "80ce-1", "8021-2000", "80d0-0",
              "80d1-1", "80d2-0", "81ff-1", "81ff-2", "81ff-3", "80cc-0204"}
    for state in (0, 4):
        for label, control in CONTROLS:
            if label not in labels:
                continue
            data = mixed_control(jpeg(), control)
            if state == 0:
                data = data.replace(struct.pack("<HH", 0x801D, 4),
                                    struct.pack("<HH", 0x801D, 0))
            yield f"mixed-{state}-{label}.caj", data


def color_documents():
    """Discriminate the required 81ff payload from unverified colors."""
    from c8_image_fixture import jpeg, mixed_control

    # Historical probe names are hypotheses, not established RGB meanings.
    controls = (
        ("baseline", ()),
        ("all", (0x81FF, 1, 0, 200, 0x81FF, 2, 0, 200, 0x81FF, 3, 0, 200)),
        ("third", (0x81FF, 3, 0, 200)),
        ("first-red", (0x81FF, 1, 255, 200)),
        ("second-red", (0x81FF, 2, 255, 200)),
        ("all-gray", (0x81FF, 1, 0x4444, 0x44, 0x81FF, 2, 0x4444, 0x44,
                      0x81FF, 3, 0x4444, 0x44)),
    )
    for label, control in controls:
        yield f"color-{label}.caj", mixed_control(jpeg(), control)
    for value in (1, 2, 3):
        data = bytearray(mixed_control(jpeg()))
        length = struct.unpack_from("<I", data, 84)[0]
        data[100:100] = struct.pack("<4H", 0x81FF, value, 0, 200)
        struct.pack_into("<I", data, 84, length + 8)
        struct.pack_into("<I", data, 96, len(data))
        descriptor = 100 + length + 8
        struct.pack_into("<I", data, descriptor + 4, descriptor + 12)
        yield f"color-once-{value}.caj", data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = []
    for label, words in CONTROLS:
        data = control_document(words)
        name = f"control-{label}.caj"
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "words": words,
                         "sha256": hashlib.sha256(data).hexdigest()})
    for name, data in (*mixed_documents(), *color_documents()):
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
