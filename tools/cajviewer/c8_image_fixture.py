#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original asymmetric C8 image-position controls outside Git."""

import argparse
import hashlib
import io
import json
from pathlib import Path
import struct

from PIL import Image


def jpeg():
    image = Image.new("RGB", (32, 24), "white")
    for y in range(24):
        for x in range(32):
            color = (255, 255, 255)
            if x < 3 or y < 3:
                color = (0, 0, 0)
            elif x > 24 and y > 15:
                color = (255, 0, 0)
            elif x < 12 and y > 10:
                color = (0, 0, 255)
            image.putpixel((x, y), color)
    output = io.BytesIO()
    image.save(output, format="JPEG", quality=100, subsampling=0)
    return output.getvalue()


def document(payload, *, dx=0, dy=0, dw=0, dh=0, origin_delta=0, trailing_words=None):
    header = bytearray(80)
    struct.pack_into("<IIII", header, 0, 200, 0, 1, 2)
    header[16:28] = "北大二扫1.00".encode("gbk")
    struct.pack_into("<HHHH", header, 28, 4652 + origin_delta,
                     4274 + origin_delta, 300, 230)
    # These trailing framing words remain opaque; they are not source text,
    # font data, or an assertion that the renderer understands their semantics.
    words = [0x800A, 0xD300, 0xC000 | (4682 + dx), 4314 + dy,
             0xC000 | (80 + dw), 50 + dh, 0xC050, 0xC033, 0xC037,
             0xC000, 0xC06C, 0xC032, 0xC0F2, 0xC07A, 0x8004, 1]
    if trailing_words is not None:
        assert len(trailing_words) == 8
        words[6:14] = trailing_words
    text = struct.pack("<16H", *words)
    descriptor = 100 + len(text)
    end = descriptor + 12 + len(payload)
    index = struct.pack("<IIIII", 100, len(text), 1, 0, end)
    return header + index + text + struct.pack("<III", 2, descriptor + 12, len(payload)) + payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    payload = jpeg()
    controls = {
        "source": {}, "shift-x": {"dx": 20}, "shift-y": {"dy": 20},
        "wider": {"dw": 20}, "taller": {"dh": 20},
        "origins-only": {"origin_delta": 20},
        "origins-and-image": {"origin_delta": 20, "dx": 20, "dy": 20},
        "trailing-ascii": {"trailing_words": [0xC041, 0xC042, 0xC043, 0xC000, 0xC061, 0xC062, 0xC063, 0xC064]},
        "trailing-extremes": {"trailing_words": [0xC000, 0xC0FF, 0xC080, 0xC001, 0xC0FE, 0xC07F, 0xC000, 0xC0FF]},
    }
    manifest = []
    for name, parameters in controls.items():
        data = document(payload, **parameters)
        (args.output / (name + ".caj")).write_bytes(data)
        manifest.append({"name": name, "parameters": parameters,
                         "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
