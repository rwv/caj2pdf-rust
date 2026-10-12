#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original same-payload pairs separated by an exact device-pixel translation."""
from pathlib import Path
import hashlib, io, json, struct
from PIL import Image

def digest(data):
    return hashlib.sha256(data).hexdigest()

def generate(output):
    output = Path(output)
    output.mkdir(exist_ok=False)
    files = []
    for axis in ['x', 'y']:
        cells, payloads, words = ([], [], [])
        for resolution in [33, 129, 513]:
            values = bytes((round(i * 254 / (resolution - 1)) for i in range(resolution)))
            dimensions = (resolution, 17) if axis == 'x' else (17, resolution)
            pixels = values * 17 if axis == 'x' else b''.join((bytes([v]) * 17 for v in values))
            encoded = io.BytesIO()
            Image.frombytes('L', dimensions, pixels).save(encoded, format='JPEG', quality=100)
            payload = encoded.getvalue()
            for negative in [-123, -7, -1]:
                pair = len(cells) // 2
                for role, position in [('clipped', negative), ('in-page', negative + 1484)]:
                    cross = 200 + len(cells) * 200
                    box = (position, cross, 1113, 140) if axis == 'x' else (cross, position, 140, 1113)
                    x, y, width, height = box
                    words += [32778, 54016, 49152 | 4652 + x, 4274 + y, 49152 | width, height, 49232, 49203, 49207, 49152, 49260, 49202, 49394, 49274]
                    cells.append(dict(index=len(cells), pair=pair, role=role, axis=axis, negative_position=negative, position=position, x=x, y=y, width=width, height=height, pixel_dimensions=list(dimensions), jpeg_sha256=digest(payload)))
                    payloads.append(payload)
        words += [32772, 1]
        header = bytearray(80)
        struct.pack_into('<IIII', header, 0, 200, 0, 1, 2)
        header[16:28] = '北大二扫1.00'.encode('gbk')
        struct.pack_into('<HHHH', header, 28, 4652, 4274, 3900, 3900)
        text = struct.pack('<' + 'H' * len(words), *words)
        base, body = (100 + len(text), bytearray())
        for payload in payloads:
            body += struct.pack('<III', 2, base + len(body) + 12, len(payload)) + payload
        data = header + struct.pack('<IIIII', 100, len(text), len(cells), 0, base + len(body)) + text + body
        source = output / f'{axis}-pairs.caj'
        source.write_bytes(data)
        files.append(dict(name=axis, filename=source.name, source_sha256=digest(data), cells=cells, page_size=[3900, 3900], translation_raw=1484))
    result = dict(files=files, scope='Source pairs share identical JPEG bytes and physical extents. A 1484-raw-unit translation equals 144/192/288 device pixels at 75/100/150%. This probes translation invariance across clipping; differing signed endpoint sizes remain a competing explanation, not presumed equal native destinations.')
    (output / 'manifest.json').write_text(json.dumps(result, indent=2) + '\n')
    return result

if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(json.dumps(generate(args.output), indent=2))
