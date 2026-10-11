#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original C8 descriptor-orientation and negative-origin image controls.

Requires Pillow and the adjacent original control generators. Writes only to
a new external directory; this is bounded fixture code, not a CAJ parser API.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from wine_image_endpoint_controls import document
from wine_image_sampling_controls import generate as gray_grid


def digest(data):
    return hashlib.sha256(data).hexdigest()


def descriptor_type2(original):
    """Change only descriptor kind bytes in a small original single-page C8."""
    assert 100 <= len(original) <= 4 * 1024 * 1024
    data = bytearray(original)
    start, length, count, reserved, end = struct.unpack_from('<IIIII', data, 80)
    assert start == 100 and reserved == 0 and 1 <= count <= 32
    at, changed = start + length, []
    for _ in range(count):
        kind, offset, size = struct.unpack_from('<III', data, at)
        assert kind == 1 and offset == at + 12 and offset + size <= len(data)
        struct.pack_into('<I', data, at, 2)
        changed.append(at)
        at = offset + size
    assert at == end == len(data)
    assert [i for i, (a, b) in enumerate(zip(original, data)) if a != b] == changed
    return bytes(data), changed


def generate(output):
    output.mkdir(parents=True, exist_ok=False)
    gray = gray_grid(output / 'gray-type1')
    gray_bytes = (output / 'gray-type1/gray-grid.caj').read_bytes()
    asymmetric_cells = [dict(x=400, y=500, width=3500, height=2500,
                             pixel_dimensions=[256, 192])]
    asymmetric_bytes = document(asymmetric_cells, (5105, 4800), asymmetric=True)
    files = []
    for name, original, cells, size in [
        ('gray', gray_bytes, gray['cells'], [3000, 2600]),
        ('asymmetric', asymmetric_bytes, asymmetric_cells, [5105, 4800]),
    ]:
        data, offsets = descriptor_type2(original)
        filename = f'{name}-type2.caj'
        (output / filename).write_bytes(data)
        files.append(dict(name=name, filename=filename, source_sha256=digest(data),
                          type1_source_sha256=digest(original), cells=cells,
                          page_size=size, changed_descriptor_offsets=offsets))
    for axis in ['x', 'y']:
        cells = []
        for index, position in enumerate([-123, -7, -1, 0, 1, 7, 123, 1484, 1484]):
            along = 1112 if index == 8 else 1113
            x, y, width, height = (
                (position, 200 + index * 400, along, 280) if axis == 'x'
                else (200 + index * 400, position, 280, along))
            cells.append(dict(index=index, axis=axis, position=position,
                              reference=index >= 7, x=x, y=y, width=width,
                              height=height, pixel_dimensions=[256, 192]))
        original = document(cells, (3900, 3900), asymmetric=True)
        type2, offsets = descriptor_type2(original)
        for kind, data, changes in [(1, original, []), (2, type2, offsets)]:
            name = f'{axis}-type{kind}'
            filename = name + '.caj'
            (output / filename).write_bytes(data)
            files.append(dict(name=name, filename=filename, axis=axis,
                              descriptor_kind=kind, cells=cells,
                              page_size=[3900, 3900], source_sha256=digest(data),
                              changed_descriptor_offsets=changes))
    manifest = dict(files=files)
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(json.dumps(generate(args.output)))
