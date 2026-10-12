#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original C8 image controls for pixel-resolution and endpoint rounding studies.

Writes only to a new external directory. Requires Pillow; source image data are
original black panels or the project's original asymmetric color pattern.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import struct

from PIL import Image


def digest(data):
    return hashlib.sha256(data).hexdigest()


def jpeg(dimensions, asymmetric=False):
    if asymmetric:
        width, height = dimensions
        assert width % 32 == 0 and height == width * 3 // 4
        scale = width // 32
        image = Image.new('RGB', dimensions, 'white')
        for y in range(height):
            for x in range(width):
                u, v = x // scale, y // scale
                if u < 3 or v < 3:
                    color = (0, 0, 0)
                elif u > 24 and v > 15:
                    color = (255, 0, 0)
                elif u < 12 and v > 10:
                    color = (0, 0, 255)
                else:
                    color = (255, 255, 255)
                image.putpixel((x, y), color)
    else:
        image = Image.new('L', dimensions, 0)
    output = io.BytesIO()
    options = dict(quality=100)
    if asymmetric:
        options['subsampling'] = 0
    image.save(output, format='JPEG', **options)
    payload = output.getvalue()
    if not asymmetric:
        assert set(Image.open(io.BytesIO(payload)).tobytes()) == {0}
    return payload


def document(cells, page_size, asymmetric=False):
    header = bytearray(80)
    struct.pack_into('<IIII', header, 0, 200, 0, 1, 2)
    header[16:28] = '北大二扫1.00'.encode('gbk')
    struct.pack_into('<HHHH', header, 28, 4652, 4274, *page_size)
    words, payloads = [], []
    for cell in cells:
        payload = jpeg(cell['pixel_dimensions'], asymmetric)
        payloads.append(payload)
        cell['jpeg_sha256'] = digest(payload)
        x, y, width, height = (cell[k] for k in ['x', 'y', 'width', 'height'])
        words += [0x800a, 0xd300, 0xc000 | (4652+x), 4274+y,
                  0xc000 | width, height, 0xc050, 0xc033, 0xc037,
                  0xc000, 0xc06c, 0xc032, 0xc0f2, 0xc07a]
    words += [0x8004, 1]
    text = struct.pack('<' + 'H' * len(words), *words)
    base = 100 + len(text)
    body = bytearray()
    for payload in payloads:
        body += struct.pack('<III', 1, base+len(body)+12, len(payload)) + payload
    index = struct.pack('<IIIII', 100, len(text), len(payloads), 0, base+len(body))
    return bytes(header + index + text + body)


def grid(reverse=False, integer=False):
    cells = []
    dimensions = [(8, 6), (512, 384)] if integer else [
        (8, 6), (32, 24), (128, 96), (512, 384)]
    if reverse:
        dimensions.reverse()
    for row, pixels in enumerate(dimensions):
        for column in range(3 if integer else 4):
            if integer:
                x = [123, 1357, 2711][column]
                y = [149, 1465][row]
                width = height = [371, 742, 1113][column]
                cell_raw = [x-70, y-70, x+width+60, y+height+60]
            else:
                x = [120, 743, 1369, 1999][column] + 11*reverse
                y = [140, 691, 1243, 1797][row] + 17*reverse
                width = [420, 421, 427, 443][column]
                height = [320, 327, 341, 359][column]
                cell_raw = [x-70, y-70, x+520, y+450]
            cells.append(dict(row=row, column=column, x=x, y=y, width=width,
                              height=height, pixel_dimensions=list(pixels),
                              cell_raw=cell_raw))
    return cells, (4400, 3100) if integer else (2800, 2600)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    rows = []
    for name, page_width, x, y in [('baseline', 5105, 400, 500),
                                 ('width-plus1', 5106, 400, 500),
                                 ('placement-plus20', 5105, 420, 520)]:
        cells = [dict(x=x, y=y, width=3500, height=2500,
                      pixel_dimensions=[256, 192])]
        data = document(cells, (page_width, 4800), asymmetric=True)
        (args.output / (name+'.caj')).write_bytes(data)
        rows.append(dict(name=name, sha256=digest(data), cells=cells,
                         page_size=[page_width, 4800]))
    for name, reverse, integer in [('black-grid-0', False, False),
                                    ('black-grid-1', True, False),
                                    ('integer-grid', False, True)]:
        cells, size = grid(reverse, integer)
        data = document(cells, size)
        (args.output / (name+'.caj')).write_bytes(data)
        rows.append(dict(name=name, sha256=digest(data), cells=cells,
                         page_size=list(size)))
    (args.output / 'manifest.json').write_text(json.dumps(rows, indent=2)+'\n')


if __name__ == '__main__':
    main()
