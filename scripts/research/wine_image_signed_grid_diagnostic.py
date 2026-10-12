#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Declared-scale diagnostics for signed C8 image coordinates, not production."""
from pathlib import Path
from fractions import Fraction
from decimal import Decimal, localcontext
import hashlib, math, re, subprocess
import pikepdf
IMAGE = re.compile(b'q\\s+([-0-9.]+) 0 0 ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) cm (/Im\\d+) Do Q')
UNIT = Fraction(72, 742)

def sha(p):
    with Path(p).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()

def fmt(v):
    with localcontext() as c:
        c.prec = 60
        return format(Decimal(v.numerator) / Decimal(v.denominator), '.30f').rstrip('0').rstrip('.') or '0'

def convert(source, output, zoom, rounding, carry=False):
    source, output = (Path(source), Path(output))
    if output.exists() or rounding not in ['floor', 'truncate'] or (not 1 <= zoom <= 1600):
        raise ValueError('Expected new output, known rounding, bounded zoom')
    q = Fraction(zoom, 75)
    quantize = math.floor if rounding == 'floor' else int
    records = []
    payloads = []
    after_contents = []
    with pikepdf.open(source) as pdf:
        boxes = [list(p.MediaBox) for p in pdf.pages]
        fonts = sorted((hashlib.sha256(o.read_bytes()).hexdigest() for o in pdf.objects if isinstance(o, pikepdf.Stream) and o.get('/Length1')))
        for page_index, page in enumerate(pdf.pages, 1):
            H = Fraction(str(page.MediaBox[3]))
            images = page.Resources.XObject
            expected = {k for k in images.keys() if re.fullmatch('/Im\\d+', k)}
            seen = set()

            def replace(match):
                a, d, e, f = [Fraction(v.decode()) for v in match.group(1, 2, 3, 4)]
                name = match[5].decode()
                if a <= 0 or d == 0 or name in seen:
                    raise ValueError('Unsupported image matrix or repeated draw')
                seen.add(name)
                image = images[name]
                payloads.append((page_index, name, hashlib.sha256(image.read_raw_bytes()).hexdigest()))
                left, top = (e, H - max(f, f + d))
                grid = [v / UNIT for v in [left, top, a, abs(d)]]
                raw = [round(v) for v in grid]
                errors = [abs(v - n) for v, n in zip(grid, raw)]
                if max(errors) > Fraction(1, 100000):
                    raise ValueError('Outside current integer C8 grid tolerance')
                x, y, w, h = [v * UNIT for v in raw]
                x0, x1 = (quantize(x * q), quantize((x + w) * q) + 1)
                y0, y1 = (quantize(y * q), quantize((y + h) * q) + 1)
                na = Fraction(x1 - x0) / q
                nd = Fraction(y1 - y0) / q * (1 if d > 0 else -1)
                dx = Fraction(quantize(x * q) - math.floor(x * q)) / q if carry else Fraction(0)
                dy = Fraction(quantize(y * q) - math.floor(y * q)) / q if carry else Fraction(0)
                ne = e + dx
                nf = f + max(Fraction(0), d) - max(Fraction(0), nd) - dy
                records.append(dict(page=page_index, image=name, before=[str(v) for v in [a, d, e, f]], raw=raw, raw_grid_errors=[str(v) for v in errors], device_rectangle=[x0, y0, x1, y1], origin_change_points=[str(dx), str(dy)], after=[fmt(v) for v in [na, nd, ne, nf]]))
                return f'q\n{fmt(na)} 0 0 {fmt(nd)} {fmt(ne)} {fmt(nf)} cm {name} Do Q'.encode()
            before = page.Contents.read_bytes()
            after, count = IMAGE.subn(replace, before)
            assert seen == expected and count == len(expected)
            assert IMAGE.sub(b'IMAGE', before) == IMAGE.sub(b'IMAGE', after)
            page.Contents = pikepdf.Stream(pdf, after)
            after_contents.append(after)
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf:
        assert [list(p.MediaBox) for p in pdf.pages] == boxes
        assert [p.Contents.read_bytes() for p in pdf.pages] == after_contents
        assert sorted((hashlib.sha256(o.read_bytes()).hexdigest() for o in pdf.objects if isinstance(o, pikepdf.Stream) and o.get('/Length1'))) == fonts
        for page, name, checksum in payloads:
            assert hashlib.sha256(pdf.pages[page - 1].Resources.XObject[name].read_raw_bytes()).hexdigest() == checksum
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(source_sha256=sha(source), output_sha256=sha(output), zoom=zoom, rounding=rounding, carry=carry, records=records, page_boxes=[[str(v) for v in b] for b in boxes], font_hashes=fonts, image_payloads_unchanged=True, nonimage_operators_unchanged=True, scope='Target-device diagnostic: changes physical extent and optionally origin. Raw-grid admission is numerical, not pixel fitting. Not a production correction.')

if __name__ == '__main__':
    import argparse, json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--zoom', type=int, required=True)
    parser.add_argument('--rounding', choices=['floor', 'truncate'], required=True)
    parser.add_argument('--carry', action='store_true')
    args = parser.parse_args()
    print(json.dumps(convert(args.source, args.output, args.zoom, args.rounding, args.carry), indent=2))
