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


def jpeg(scale=1):
    image = Image.new("RGB", (32 * scale, 24 * scale), "white")
    for y in range(24 * scale):
        for x in range(32 * scale):
            u, v = x // scale, y // scale
            color = (255, 255, 255)
            if u < 3 or v < 3:
                color = (0, 0, 0)
            elif u > 24 and v > 15:
                color = (255, 0, 0)
            elif u < 12 and v > 10:
                color = (0, 0, 255)
            image.putpixel((x, y), color)
    output = io.BytesIO()
    image.save(output, format="JPEG", quality=100, subsampling=0)
    return output.getvalue()


def document(payload, *, dx=0, dy=0, dw=0, dh=0, origin_delta=0, trailing_words=None, prefix_words=(), page_size=(300, 230), end_value=1):
    header = bytearray(80)
    struct.pack_into("<IIII", header, 0, 200, 0, 1, 2)
    header[16:28] = "北大二扫1.00".encode("gbk")
    struct.pack_into("<HHHH", header, 28, 4652 + origin_delta,
                     4274 + origin_delta, *page_size)
    # These trailing framing words remain opaque; they are not source text,
    # font data, or an assertion that the renderer understands their semantics.
    words = [0x800A, 0xD300, 0xC000 | (4682 + dx), 4314 + dy,
             0xC000 | (80 + dw), 50 + dh, 0xC050, 0xC033, 0xC037,
             0xC000, 0xC06C, 0xC032, 0xC0F2, 0xC07A, 0x8004, end_value]
    if trailing_words is not None:
        assert len(trailing_words) == 8
        words[6:14] = trailing_words
    words = [*prefix_words, *words]
    text = struct.pack(f"<{len(words)}H", *words)
    descriptor = 100 + len(text)
    end = descriptor + 12 + len(payload)
    index = struct.pack("<IIIII", 100, len(text), 1, 0, end)
    return header + index + text + struct.pack("<III", 2, descriptor + 12, len(payload)) + payload



def mixed_control(payload, control=None, *, end_value=1):
    """Original glyph/segment/decoration/image context for control effects."""
    words = []
    for row, style in enumerate((0xA381, 0xA383, 0xA38B)):
        y = 4344 + row * 160
        words.extend((0x8001, y, 0x8002, 0x1084, 0x801D, 4, 0x8067, 6))
        if control:
            words.extend(control)
        words.extend((4672, 0xD6D0))
        if control:
            words.extend(control)
        words.extend((0x8006, style, 4832, y + 40, 5132, y + 80))
        if style != 0xA383:
            words.extend((0xFFFF, 5))
        words.extend((4972, 0xA0C1))
    if control:
        words.extend(control)
    words.extend((0x8010, 1, 4702, 4804, 4932, 4804, 0xFFFF, 5))
    if control:
        words.extend(control)
    return document(payload, dx=360, dy=680, dw=80, dh=50,
                    prefix_words=words, page_size=(600, 900), end_value=end_value)

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
    variants = [("mixed-control-baseline", None)]
    for tag, values in (
        (0x8072, (0, 0x1042, 0xA3A8, 0xA0F2)),
        (0x8073, (38, 39, 40, 41, 42)),
        (0x8074, (0, 0xB4A2, 0xD4B4, 0x24A7, 0xA1A1, 0xA3A9)),
        (0xC053, (0, 0x1377, 0x137B, 0xFFFF)),
        (0xC054, (0, 0x139E, 0x1676, 0xFFFF)),
    ):
        variants.extend((f"mixed-control-{tag:04x}-{value:04x}", (tag, value))
                        for value in values)
    for name, control in variants:
        data = mixed_control(payload, control)
        (args.output / (name + ".caj")).write_bytes(data)
        manifest.append({"name": name, "control": control, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    for value in (0, 39, 40, 41, 42, 43, 44, 0xFFFF):
        name = f"mixed-end-{value:04x}"
        data = mixed_control(payload, end_value=value)
        (args.output / (name + ".caj")).write_bytes(data)
        manifest.append({"name": name, "end_value": value, "bytes": len(data),
                         "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
