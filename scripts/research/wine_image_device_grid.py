#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Probe measured image extents at a declared Wine zoom; not a production fix.

This deliberately changes physical image matrices for one device scale. It is
not equivalent geometry, a general PDF correction, or proof at other zooms.
"""
import argparse
from decimal import Decimal, localcontext
from fractions import Fraction
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess

import pikepdf

IMAGE = re.compile(
    rb'q\s+([-0-9.]+) 0 0 ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) cm (/Im\d+) Do Q'
)


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def fmt(value):
    with localcontext() as context:
        context.prec = 60
        number = Decimal(value.numerator) / Decimal(value.denominator)
        return format(number, '.30f').rstrip('0').rstrip('.') or '0'


def font_hashes(pdf):
    return sorted(hashlib.sha256(obj.read_bytes()).hexdigest()
                  for obj in pdf.objects
                  if isinstance(obj, pikepdf.Stream) and obj.get('/Length1'))


def convert(source, output, zoom, keep_origin=False):
    source, output = Path(source), Path(output)
    if output.exists() or not 1 <= zoom <= 1600:
        raise ValueError('Use a new output and a zoom from 1 to 1600')
    scale = Fraction(zoom, 75)
    records, payloads = [], []
    with pikepdf.open(source) as pdf:
        fonts = font_hashes(pdf)
        boxes = [list(page.MediaBox) for page in pdf.pages]
        for page_number, page in enumerate(pdf.pages, 1):
            height = Fraction(str(page.MediaBox[3]))
            images = page.Resources.XObject

            def replacement(match):
                a, d, e, f = [Fraction(v.decode()) for v in match.group(1, 2, 3, 4)]
                if a <= 0 or d == 0:
                    raise ValueError('Expected positive x scale and nonzero y scale')
                name = match[5].decode()
                image = images[name]
                if image.Subtype != pikepdf.Name.Image:
                    raise ValueError('Expected image XObjects')
                payloads.append((page_number, name,
                                 hashlib.sha256(image.read_raw_bytes()).hexdigest()))
                left = e
                top = height - max(f, f+d)
                x0 = math.floor(left*scale)
                x1 = math.floor((left+a)*scale) + 1
                y0 = math.floor(top*scale)
                y1 = math.floor((top+abs(d))*scale) + 1
                new_a = Fraction(x1-x0) / scale
                new_d = Fraction(y1-y0) / scale * (1 if d > 0 else -1)
                new_e = Fraction(x0) / scale
                new_f = height - Fraction(y1 if d > 0 else y0) / scale
                if keep_origin:
                    new_e = e
                    new_f = f+d-new_d if d > 0 else f
                    assert new_f+max(Fraction(0), new_d) == f+max(Fraction(0), d)
                values = [new_a, new_d, new_e, new_f]
                records.append(dict(
                    page=page_number, image=name,
                    before=[str(v) for v in [a, d, e, f]],
                    after=[fmt(v) for v in values], device_rectangle=[x0, y0, x1, y1]
                ))
                return (f'q\n{fmt(new_a)} 0 0 {fmt(new_d)} {fmt(new_e)} '
                        f'{fmt(new_f)} cm {name} Do Q').encode()

            before = page.Contents.read_bytes()
            after, count = IMAGE.subn(replacement, before)
            if count != len(images):
                raise ValueError('Unsupported or unaccounted image placement')
            assert IMAGE.sub(b'IMAGE', before) == IMAGE.sub(b'IMAGE', after)
            page.Contents = pikepdf.Stream(pdf, after)
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf:
        assert [list(page.MediaBox) for page in pdf.pages] == boxes
        assert font_hashes(pdf) == fonts
        for page, name, checksum in payloads:
            assert hashlib.sha256(
                pdf.pages[page-1].Resources.XObject[name].read_raw_bytes()
            ).hexdigest() == checksum
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(source_sha256=sha(source), output_sha256=sha(output), target_zoom=zoom,
                keep_origin=keep_origin, records=records, font_hashes=fonts,
                page_boxes=[[str(value) for value in box] for box in boxes],
                image_payloads_unchanged=True, nonimage_operators_unchanged=True,
                scope='Declared device-grid diagnostic at this zoom only; changes '
                      'physical matrices and is not a production correction.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--zoom', type=int, required=True)
    parser.add_argument('--origin', choices=['floor', 'keep'], required=True)
    parser.add_argument('--receipt', type=Path)
    args = parser.parse_args()
    result = convert(args.source, args.output, args.zoom, args.origin == 'keep')
    if args.receipt:
        args.receipt.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
