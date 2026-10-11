#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce eight original HN-B mode-0 extent and origin controls.

Uses only the standard library and independently established project framing.
No external document, font or vendor implementation is read. Output must be new.
The four extent controls distinguish additive page padding at 75/100/150%.
The four grids retain the separate, unresolved glyph-position experiments.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def document(words, width, height, origin=(4652, 4274)):
    words = [*words, 0x8004, 1]
    records = struct.pack('<' + 'H' * len(words), *words)
    header = bytearray(216)
    struct.pack_into('<III', header, 0, 0x4e48, 200, 136)
    struct.pack_into('<IIII', header, 136, 0, 0, 1, 0)
    header[152:164] = '北大二扫1.00'.encode('gbk')
    struct.pack_into('<HHHH', header, 164, *origin, width, height)
    return bytes(header) + struct.pack('<III', 228, len(records), 0) + records


def grid(reverse=False, origin=(4652, 4274)):
    ox, oy = origin
    # Keep the initial, superseded state present in the measured controls.
    words = [0x8001, oy + 426, 0x8002, 0x1084, 0x801d, 0, 0x8067, 6]
    styles = [0x1000, 0x1084, 0x10a4, 0x10a5, 0x0ca4, 0x154a, 0]
    for row, style in enumerate(reversed(styles) if reverse else styles):
        y = oy + 200 + row * 450 + (7 if reverse else 0)
        words += [0x8001, y, 0x8002, style, 0x801d, 0, 0x8067, 6, 0x80ce, 1]
        if style == 0:
            words += [0x8070, 36, 0x8071, 36]
        codes = [0xd6d0] * 5 if style == 0x154a else [
            0xd6d0, 0xa3c1, 0xa3b1, 0xa1af, 0xa980]
        for x, code in zip((300, 750, 1250, 1850, 2400), codes):
            words += [ox + x + (13 if reverse else 0), code]
    return document(words, 3100, 3400, origin)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    files = {}

    def emit(name, data):
        (args.output / name).write_bytes(data)
        files[name] = hashlib.sha256(data).hexdigest()

    for width, height in ((2500, 2300), (3100, 3400), (5105, 4200), (4736, 2800)):
        words = [0x8001, 4500, 0x8002, 0x1084, 0x801d, 0, 0x8067, 6,
                 4900, 0xd6d0, 5250, 0xa3c1]
        emit(f'mode0-extent-{width}-{height}.caj', document(words, width, height))
    emit('mode0-origin-0.caj', grid())
    emit('mode0-origin-1.caj', grid(reverse=True))
    for origin in ((0, 0), (0, 1)):
        emit(f'mode0-origin-invariant-{origin[0]}-{origin[1]}.caj', grid(origin=origin))
    (args.output / 'manifest.json').write_text(json.dumps(files, indent=2) + '\n')


if __name__ == '__main__':
    main()
