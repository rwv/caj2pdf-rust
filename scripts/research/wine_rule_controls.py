#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original C8/HN-B rule controls without external documents or fonts."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from wine_mode0_page_extents import document as mode_zero_document

ORIGIN = (4652, 4274)
STYLES = [(0x8006, value) for value in
          (0xa381, 0xa383, 0xa385, 0xa387, 0xa38b, 0xa38d)]
STYLES += [(0x8007, 0xa380), (0x8007, 0xa382), (0x8008, 0xa380)]


def words(phased=False, reverse=False, legacy=False):
    records = []
    rows = []
    for index, (tag, style) in enumerate(STYLES):
        y = 143 + index * 157 if phased else 140 + index * 160
        dx = index * 7 + 3 if phased else 0
        for points in ([[120+dx, y], [740+dx, y]],
                       [[900+dx, y], [900+dx, y+100]],
                       [[1030+dx, y], [1220+dx, y+100]]):
            if reverse:
                points = list(reversed(points))
            # The admitted mode-0 subset uses this sole tag/style pair.
            pair = (0x8006, 0xa385) if legacy else (tag, style)
            rows.append(dict(tag=pair[0], style=pair[1], points=points))
            records += [*pair, *[value+ORIGIN[axis]
                                for point in points
                                for axis, value in enumerate(point)]]
    return records, rows


def c8_document(pages):
    header = bytearray(80)
    struct.pack_into('<IIII', header, 0, 200, 0, len(pages), 2)
    header[16:28] = '北大二扫1.00'.encode('gbk')
    struct.pack_into('<HHHH', header, 28, *ORIGIN, 1400, 1800)
    index, body = bytearray(), bytearray()
    for page in pages:
        values = [*page, 0x8004, 1]
        payload = struct.pack('<'+'H'*len(values), *values)
        offset = 80 + 20*len(pages) + len(body)
        index += struct.pack('<IIIII', offset, len(payload), 0, 0,
                             offset+len(payload))
        body += payload
    return bytes(header+index+body)


def generate(output):
    output.mkdir(exist_ok=False)
    baseline, baseline_rows = words()
    phased, phased_rows = words(phased=True)
    reversed_words, reversed_rows = words(phased=True, reverse=True)
    legacy, legacy_rows = words(phased=True, reverse=True, legacy=True)
    documents = [
        ('rules', c8_document([baseline, [0x81ff, 1, 0, 200, *baseline]]),
         [baseline_rows, baseline_rows]),
        ('rules-phased', c8_document([phased, reversed_words]),
         [phased_rows, reversed_rows]),
        ('rules-mode0', mode_zero_document(legacy, 1400, 1800), [legacy_rows]),
    ]
    files = []
    for name, data, rows in documents:
        (output/(name+'.caj')).write_bytes(data)
        files.append(dict(name=name, sha256=hashlib.sha256(data).hexdigest(),
                          rows=rows, pages=len(rows)))
    manifest = dict(origin=ORIGIN, page_size=[1400, 1800], files=files,
                    scope='Nine admitted tag/style pairs; horizontal, vertical '
                          'and diagonal segments; default and black-text '
                          'states; phase variation and reversed endpoints; '
                          'separate admitted mode-0 controls.')
    (output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(f"Created {len(generate(args.output)['files'])} original documents")
