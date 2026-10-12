#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original RGB edge controls varying phase, resolution, descriptor and order.

The first and last decoded image rows are measured, not assumed lossless.
Outputs are external research inputs; no vendor code or document is copied.
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


def panel(width, height, reverse_edges):
    pixels = bytearray()
    for y in range(height):
        for x in range(width):
            if y == 0 or y == height - 1:
                value = 255 if (y == height - 1) != reverse_edges else 0
                color = (value,) * 3
            else:
                color = (48 + 177*x//(width-1), 40 + 160*y//(height-1),
                         96 + 64*((x//3 + y//3) % 2))
            pixels.extend(color)
    encoded = io.BytesIO()
    Image.frombytes('RGB', (width, height), bytes(pixels)).save(
        encoded, format='JPEG', quality=100, subsampling=0)
    payload = encoded.getvalue()
    with Image.open(io.BytesIO(payload)) as decoded:
        decoded.load()
        rows = [decoded.crop((0, y, width, y+1)).tobytes() for y in [0, height-1]]
    return payload, [dict(sha256=digest(row), minimum=min(row), maximum=max(row)) for row in rows]


def generate(output):
    output = Path(output)
    output.mkdir(exist_ok=False)
    files = []
    for descriptor in [1, 2]:
        for reverse_order in [False, True]:
            cells = []
            for row, phase in enumerate([0, 1, 4, 7, 10, 11]):
                for resolution, dimensions in enumerate([(33,25), (129,97), (513,385)]):
                    for reverse_edges in [False, True]:
                        column = 2*resolution + int(reverse_edges)
                        cells.append(dict(index=len(cells), phase=phase,
                            x=130+500*column, y=150+400*row+phase,
                            width=370, height=260, pixel_dimensions=list(dimensions),
                            reverse_edges=reverse_edges))
            if reverse_order:
                cells.reverse()
            words, payloads = [], []
            for cell in cells:
                payload, edge_rows = panel(*cell['pixel_dimensions'], cell['reverse_edges'])
                payloads.append(payload)
                cell.update(jpeg_sha256=digest(payload), decoded_edge_rows=edge_rows)
                x, y, width, height = [cell[k] for k in ['x','y','width','height']]
                words.extend([0x800a, 0xd300, 0xc000 | (4652+x), 4274+y,
                              0xc000 | width, height, 0xc050, 0xc033, 0xc037,
                              0xc000, 0xc06c, 0xc032, 0xc0f2, 0xc07a])
            words.extend([0x8004, 1])
            header = bytearray(80)
            struct.pack_into('<IIII', header, 0, 200, 0, 1, 2)
            header[16:28] = '北大二扫1.00'.encode('gbk')
            struct.pack_into('<HHHH', header, 28, 4652, 4274, 3100, 2600)
            text = struct.pack('<'+'H'*len(words), *words)
            base, body = 100+len(text), bytearray()
            for payload in payloads:
                body.extend(struct.pack('<III', descriptor, base+len(body)+12, len(payload)))
                body.extend(payload)
            data = header + struct.pack('<IIIII',100,len(text),len(cells),0,base+len(body)) + text + body
            name = f'type{descriptor}-order{int(reverse_order)}'
            (output/(name+'.caj')).write_bytes(data)
            files.append(dict(name=name, filename=name+'.caj', source_sha256=digest(data),
                              descriptor=descriptor, reverse_order=reverse_order,
                              page_size=[3100,2600], cells=cells))
    result = dict(files=files, scope='36 disjoint RGB panels per page. Three source resolutions, six raw top-origin phases, black/white first/last-row reversal, both image descriptor signs, and reversed paint order at identical physical positions. JPEG quality100 with no chroma subsampling; decoded boundary rows recorded.')
    (output/'manifest.json').write_text(json.dumps(result,indent=2)+'\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(f"Created {len(generate(args.output)['files'])} original controls")
