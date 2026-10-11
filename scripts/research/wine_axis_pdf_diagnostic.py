#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate the study's PDF axis diagnostics from an existing control PDF.

Requires pikepdf. Quantization is tied to one declared 96-dpi Wine zoom,
not a generic PDF correction. This tool does not modify the source PDF.
"""
import argparse
from decimal import Decimal, ROUND_FLOOR, localcontext
import hashlib
from pathlib import Path
import re

import pikepdf

MATRIX = re.compile(rb'(?m)^([0-9.]+) 0 0 ([0-9.]+) ([\-0-9.]+) ([\-0-9.]+) Tm ')
TEXT = re.compile(rb'BT (/F\d+) 1 Tf\n([0-9.]+) 0 0 ([0-9.]+) ([\-0-9.]+) ([\-0-9.]+) Tm ')


def font_hashes(pdf):
    return {obj.objgen: hashlib.sha256(obj.read_bytes()).hexdigest()
            for obj in pdf.objects
            if isinstance(obj, pikepdf.Stream) and obj.get('/Length1')}


def transform(data, kind, zoom):
    def quantize(match):
        with localcontext() as context:
            context.prec = 40
            a, d = Decimal(match[1].decode()), Decimal(match[2].decode())
            if a == d:
                return match[0]
            scale = Decimal(96) * zoom / 7200
            x = int((a * scale).to_integral_value(rounding=ROUND_FLOOR))
            y = int((d * scale).to_integral_value(rounding=ROUND_FLOOR))
            if min(x, y) <= 0:
                raise ValueError('both quantized axes must remain positive')
            guard = Decimal(1) / 1024 if kind == 'guard' else Decimal(0)
            value = format(d * (x + guard) / y, '.20f').rstrip('0').rstrip('.')
        return value.encode() + b' 0 0 ' + match[2] + b' ' + match[3] + b' ' + match[4] + b' Tm '

    def horizontal_scale(match):
        with localcontext() as context:
            context.prec = 50
            a, d = Decimal(match[2].decode()), Decimal(match[3].decode())
            ratio = format(100 * a / d, '.30f')
            if abs(d * Decimal(ratio) / 100 - a) >= Decimal('1e-25'):
                raise ValueError('equivalent matrix exceeds precision bound')
        return (b'BT ' + match[1] + b' ' + match[3] + b' Tf\n' + ratio.encode()
                + b' Tz\n1 0 0 1 ' + match[4] + b' ' + match[5] + b' Tm ')

    result, count = (TEXT.subn(horizontal_scale, data) if kind == 'tz'
                     else MATRIX.subn(quantize, data))
    if not count:
        raise ValueError('no supported control text matrices')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--kind', choices=['quantize', 'guard', 'tz'], required=True)
    parser.add_argument('--zoom', type=int)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output must be new')
    if args.kind != 'tz' and (args.zoom is None or args.zoom <= 0):
        parser.error('quantization requires a positive --zoom')
    with pikepdf.open(args.source) as pdf:
        before = font_hashes(pdf)
        for page in pdf.pages:
            page.Contents = pikepdf.Stream(pdf, transform(page.Contents.read_bytes(), args.kind, args.zoom))
        if before != font_hashes(pdf):
            raise ValueError('unexpected embedded font change')
        pdf.save(args.output)


if __name__ == '__main__':
    main()
