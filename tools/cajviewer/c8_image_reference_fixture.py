#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original C8 image-reference framing and descriptor-order controls."""

import argparse
import hashlib
import io
import json
from pathlib import Path
import struct

from PIL import Image
from c8_image_fixture import document as old_document, jpeg


def second_jpeg():
    image = Image.new("RGB", (32, 24), "white")
    for y in range(24):
        for x in range(32):
            if x < 5:
                image.putpixel((x, y), (0, 150, 0))
            elif y < 6:
                image.putpixel((x, y), (255, 200, 0))
            elif x > 20 and y > 12:
                image.putpixel((x, y), (150, 0, 200))
    output = io.BytesIO()
    image.save(output, format="JPEG", quality=100, subsampling=0)
    return output.getvalue()


def document(names, coordinates, payloads):
    base = old_document(payloads[0])
    text = bytearray()
    for name, (x, y, width, height) in zip(names, coordinates, strict=True):
        if name is None:
            text.extend(base[100:128])
        else:
            record = struct.pack("<8H", 0x810A, 0xD300, x, y, width, height, 0, len(name))
            record += name + b"\0"
            record += bytes(-len(record) % 4)
            text.extend(record)
    # A following half-square checks that variable-length records preserve the
    # next glyph's boundary, even when the image itself still looks correct.
    text.extend(struct.pack("<12H", 0x8001, 4400, 0x8002, 0x10A5, 0x801D, 0,
                            0x8067, 6, 4802, 0xCEC4, 0x8004, 1))
    descriptor = 100 + len(text)
    body = bytearray()
    for payload in payloads:
        body.extend(struct.pack("<III", 2, descriptor + len(body) + 12, len(payload)))
        body.extend(payload)
    index = struct.pack("<IIIII", 100, len(text), len(payloads), 0, descriptor + len(body))
    return base[:80] + index + text + body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    first, second = jpeg(), second_jpeg()
    at = (4682, 4314, 80, 50)
    controls = [("old", [None], [at], [first])]
    for count in (0, 3, 4, 5, 8, 260):
        name = b"a" * count if count in (0, 260) else b"abcdefgh"[:count]
        controls.append((f"name{count}", [name], [at], [first]))
    for label, delta in (("shift-x", (20, 0, 0, 0)), ("shift-y", (0, 20, 0, 0)),
                         ("wider", (0, 0, 20, 0)), ("taller", (0, 0, 0, 20))):
        controls.append((label, [b"abc"], [tuple(a+b for a, b in zip(at, delta))], [first]))
    positions = [at, (4822, 4314, 80, 50)]
    controls.extend((
        ("two-chain-ordered", [b"abc", b"defg"], positions, [first, second]),
        ("two-chain-names-swapped", [b"defg", b"abc"], positions, [first, second]),
        ("two-chain-payloads-swapped", [b"abc", b"defg"], positions, [second, first]),
    ))
    manifest = []
    for label, names, coordinates, payloads in controls:
        data = document(names, coordinates, payloads)
        name = f"image-ref-{label}.caj"
        (args.output / name).write_bytes(data)
        manifest.append({"file": name, "reference_lengths": [len(n) if n is not None else None for n in names],
                         "coordinates": coordinates, "sha256": hashlib.sha256(data).hexdigest()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
