#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original HN-B image/order controls in a new external directory."""

import argparse
import hashlib
import io
import json
from pathlib import Path
import struct

from PIL import Image

from c8_image_fixture import jpeg as asymmetric_jpeg


def jpeg(green=False):
    if not green:
        return asymmetric_jpeg()
    image = Image.new("RGB", (32, 24), (0, 180, 0))
    output = io.BytesIO()
    image.save(output, format="JPEG", quality=100, subsampling=-1)
    return output.getvalue()


def document(order, *, dx=0, dy=0, dw=0, origin_delta=0):
    header = bytearray(216)
    struct.pack_into("<III", header, 0, 0x4E48, 200, 136)
    struct.pack_into("<IIII", header, 136, 0xC8, 0, 1, 2)
    header[152:164] = "北大二扫1.00".encode("gbk")
    struct.pack_into("<HHHH", header, 164, 4652 + origin_delta,
                     4274 + origin_delta, 300, 230)
    image = struct.pack("<14H", 0x800A, 0xD300, 0xC000 | (4682 + dx),
                        4314 + dy, 0xC000 | (80 + dw), 50,
                        0xC050, 0xC033, 0xC037, 0xC000, 0xC06C,
                        0xC032, 0xC0F2, 0xC07A)
    glyph = struct.pack("<10H", 0x8001, 4330, 0x8002, 0x1084,
                        0x801D, 0, 0x8067, 6, 4700, 0xD6D0)
    text = b"".join(glyph if item == "T" else image for item in order)
    text += struct.pack("<HH", 0x8004, 1)
    payloads = [jpeg(item == "B") for item in order if item != "T"]
    offset = 236 + len(text)
    body = bytearray()
    for payload in payloads:
        body += struct.pack("<III", 2, offset + 12, len(payload)) + payload
        offset += 12 + len(payload)
    return header + struct.pack("<IIIII", 236, len(text), len(payloads), 0, offset) + text + body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    cases = [("base", "A", {}), ("shift-x", "A", {"dx": 20}),
             ("shift-y", "A", {"dy": 20}), ("wider", "A", {"dw": 20}),
             ("origin", "A", {"origin_delta": 20})]
    cases += [(name, order, {}) for name, order in (
        ("two-ab", "AB"), ("two-ba", "BA"), ("image-text", "AT"),
        ("text-image", "TA"), ("text-green", "TB"),
        ("image-text-green", "ATB"), ("text-two-ab", "TAB"),
        ("two-ab-text", "ABT"))]
    manifest = []
    for name, order, parameters in cases:
        data = document(order, **parameters)
        filename = f"hnb-image-{name}.caj"
        (args.output / filename).write_bytes(data)
        manifest.append({"file": filename, "order": order, "parameters": parameters,
                         "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
