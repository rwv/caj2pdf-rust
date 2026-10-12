#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original bounded grayscale JPEG ramps/stripes in one C8 control page.

Requires Pillow. Writes only to a new external directory. Original border
pixels expose vertical resampling while the central rows vary horizontally.
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


def generate(output):
    output.mkdir(parents=True, exist_ok=False)
    cells, payloads, words = [], [], []
    for row, size in enumerate([33, 65, 129, 257, 513, 1025]):
        for column, kind in enumerate(['ramp', 'stripes']):
            if kind == 'ramp':
                values = bytes(round(i*255/(size-1)) for i in range(size))
            else:
                bands = [0, 255, 0, 255, 255, 0, 255]
                values = bytes(bands[min(6, i*7//size)] for i in range(size))
            pixels = bytearray(values*size)
            pixels[:size] = pixels[-size:] = bytes(size)
            for y in range(size):
                pixels[y*size] = pixels[(y+1)*size-1] = 0
            buffer = io.BytesIO()
            Image.frombytes('L', (size, size), bytes(pixels)).save(
                buffer, format='JPEG', quality=100)
            payload = buffer.getvalue()
            (output / f'{kind}-{size}.jpg').write_bytes(payload)
            x, y, width, height = [200, 1600][column], 200+row*400, 1113, 200
            words += [0x800a, 0xd300, 0xc000 | (4652+x), 4274+y,
                      0xc000 | width, height, 0xc050, 0xc033, 0xc037,
                      0xc000, 0xc06c, 0xc032, 0xc0f2, 0xc07a]
            cells.append(dict(index=len(cells), row=row, column=column,
                              kind=kind, source_pixels=[size, size],
                              x=x, y=y, width=width, height=height,
                              jpeg_sha256=digest(payload),
                              cell_raw=[x-50, y-50, x+width+50, y+height+50]))
            payloads.append(payload)
    words += [0x8004, 1]
    header = bytearray(80)
    struct.pack_into('<IIII', header, 0, 200, 0, 1, 2)
    header[16:28] = '北大二扫1.00'.encode('gbk')
    struct.pack_into('<HHHH', header, 28, 4652, 4274, 3000, 2600)
    text = struct.pack('<'+'H'*len(words), *words)
    base, body = 100+len(text), bytearray()
    for payload in payloads:
        body += struct.pack('<III', 1, base+len(body)+12, len(payload)) + payload
    data = header + struct.pack('<IIIII', 100, len(text), len(payloads),
                                 0, base+len(body)) + text + body
    (output / 'gray-grid.caj').write_bytes(data)
    result = dict(page_size=[3000, 2600], cells=cells, source_sha256=digest(data))
    (output / 'manifest.json').write_text(json.dumps(result, indent=2)+'\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(json.dumps(generate(args.output)))
