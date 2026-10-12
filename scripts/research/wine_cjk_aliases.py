#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce original C8 CJK-mode alphabet and coordinate controls.

No external font or document input and no third-party Python dependencies.
The output directory must be new. These are research fixtures, not a converter.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def document(words, width=4000, height=2800):
    # Framing from this project's independently authored MIT C8 controls.
    words = [0x8001, 4700, 0x8002, 0x1084, 0x801d, 0, 0x8067, 6,
             *words, 0x8004, 1]
    records = struct.pack('<' + 'H' * len(words), *words)
    header = bytearray(80)
    struct.pack_into('<IIII', header, 0, 0xc8, 0, 1, 2)
    header[16:28] = '北大二扫1.00'.encode('gbk')
    struct.pack_into('<HHHH', header, 28, 4652, 4274, width, height)
    index = struct.pack('<IIIII', 100, len(records), 0, 0, 100 + len(records))
    return bytes(header + index + records)


def grid(entries):
    words = []
    for i, (style, family, mode, code) in enumerate(entries):
        words += [0x8001, 4600 + i // 8 * 270, 0x8002, style,
                  0x801d, 0, 0x80ce, mode]
        if style == 0:
            words += [0x8070, 34, 0x8071, 34]
        if family is not None:
            words += [0x801c, family]
        words += [5050 + i % 8 * 350, code]
    return document(words)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    files = {}

    def emit(name, data):
        (args.output / name).write_bytes(data)
        files[name] = hashlib.sha256(data).hexdigest()

    codes = [*range(0xa0b0, 0xa0ba), *range(0xa0c1, 0xa0db),
             *range(0xa0e1, 0xa0fb), 0xa3c1, 0xd6d0]
    configurations = [(f'{style:04x}', [(style, None, 0, code) for code in codes])
                      for style in (0x1084, 0x0884, 0x0c84, 0x04c6, 0)]
    mixed = [(style, family, mode, code)
             for style in (0x1084, 0x0884, 0x04c6)
             for family in (None, 2, 3, 4)
             for code, mode in ((0xa0c1, 0), (0xa0cd, 0), (0xa0b1, 0),
                                (0x0070, 0), (0xa0c1, 1))]
    configurations.append(('families', mixed))
    for name, entries in configurations:
        for reverse in (False, True):
            ordered = list(reversed(entries)) if reverse else entries
            emit(f'cjk-ascii-{name}-{int(reverse)}.caj', grid(ordered))

    words = []
    for i, y in enumerate((4800, 5400, 6000, 6600, 7200, 7800)):
        words += [0x8001, y, 0x8002, 0x1084 if i < 5 else 0x10a5,
                  0x801d, 0, 0x8067, 6, 0x80ce, 1]
        for x in range(5000, 8201, 400):
            words += [x, 0xa0c1]
    emit('coordinate-grid.caj', document(words, 4500, 4500))
    (args.output / 'manifest.json').write_text(json.dumps(files, indent=2) + '\n')


if __name__ == '__main__':
    main()
