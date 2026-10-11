#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original PDF image-pattern/clip/hairline diagnostics; no scale fitting."""
from pathlib import Path
from fractions import Fraction
import hashlib
import io
import re
import subprocess
import pikepdf
from PIL import Image
IMAGE = re.compile(b'q\\s+([-0-9.]+) 0 0 ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) cm (/Im\\d+) Do Q')
KINDS = ['clip', 'tile1', 'tile2', 'tile3', 'tile2-gap', 'tile2-hairline', 'rect-hairline', 'rect-hairline-sa', 'tile2-positive', 'tile2-page', 'tile2-page-positive']

def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def font_hashes(pdf):
    return sorted((hashlib.sha256(o.read_bytes()).hexdigest() for o in pdf.objects if isinstance(o, pikepdf.Stream) and o.get('/Length1')))

def convert(source, output, kind):
    source, output = (Path(source), Path(output))
    if output.exists() or kind not in KINDS:
        raise ValueError('Expected a new output and known diagnostic kind')
    records, payloads, afters = ([], [], [])
    with pikepdf.open(source) as pdf:
        boxes = [list(p.MediaBox) for p in pdf.pages]
        fonts = font_hashes(pdf)
        for page_number, page in enumerate(pdf.pages, 1):
            if list(page.MediaBox)[:2] != [0, 0] or page.get('/Rotate', 0) != 0:
                raise ValueError('Expected unrotated zero-origin pages')
            images = page.Resources.XObject
            if page.Resources.get('/Pattern') or page.Resources.get('/ExtGState'):
                raise ValueError('Expected no existing patterns/extended state')
            patterns = pikepdf.Dictionary()
            if kind.startswith('tile'):
                page.Resources.Pattern = patterns
            if kind == 'rect-hairline-sa':
                page.Resources.ExtGState = pikepdf.Dictionary(DiagSA=pikepdf.Dictionary(Type=pikepdf.Name.ExtGState, SA=True))

            def replacement(match):
                a, d, e, f = match.group(1, 2, 3, 4)
                if Fraction(a.decode()) <= 0 or Fraction(d.decode()) == 0:
                    raise ValueError('Expected axis-aligned nondegenerate image')
                name = match[5].decode()
                image = images[name]
                if image.get('/Subtype') != pikepdf.Name.Image:
                    raise ValueError('Expected image XObject')
                payloads.append((page_number, name, hashlib.sha256(image.read_raw_bytes()).hexdigest()))
                matrix = b'[' + b' '.join([a, b'0', b'0', d, e, f]) + b']'
                prefix = b'q\n' + b' '.join([a, b'0', b'0', d, e, f]) + b' cm '
                record = dict(page=page_number, image=name, matrix=matrix.decode())
                if kind == 'clip':
                    draw = b'0 0 1 1 re W n ' + match[5] + b' Do'
                elif kind.startswith('rect'):
                    width, height = (int(image.Width), int(image.Height))
                    if not (1 <= width <= 1024 and 1 <= height <= 1024):
                        raise ValueError('Black rectangle diagnostic image bound')
                    with Image.open(io.BytesIO(image.read_raw_bytes())) as pixels:
                        if pixels.mode != 'L' or pixels.size != (width, height) or any(pixels.tobytes()):
                            raise ValueError('Rectangle requires uniformly black image')
                    draw = (b'/DiagSA gs ' if kind.endswith('-sa') else b'') + b'0 w 0 J 0 j 0 G 0 g 0 0 1 1 re B'
                else:
                    tiling = int(kind[4])
                    step = 2 if kind == 'tile2-gap' else 1
                    pattern_name = '/Diag' + name.removeprefix('/Im')
                    pattern_content = b'q ' + match[5] + b' Do Q'
                    if kind == 'tile2-positive':
                        pattern_content = b'0.5 g 0 0 1 1 re f'
                    pattern = pikepdf.Stream(pdf, pattern_content)
                    pattern.Type, pattern.PatternType = (pikepdf.Name.Pattern, 1)
                    pattern.PaintType, pattern.TilingType = (1, tiling)
                    pattern.BBox = pikepdf.Array([0, 0, 1, 1])
                    pattern.XStep, pattern.YStep = (step, step)
                    pattern.Matrix = pikepdf.Object.parse(matrix)
                    if kind.startswith('tile2-page'):
                        coords = [Fraction(v.decode()) for v in [a, d, e, f]]
                        av, dv, ev, fv = coords

                        def decimal(value):
                            from decimal import Decimal, localcontext
                            with localcontext() as context:
                                context.prec = 60
                                return format(Decimal(value.numerator) / Decimal(value.denominator), 'f')
                        box = [ev, min(fv, fv + dv), ev + av, max(fv, fv + dv)]
                        pattern.BBox = pikepdf.Object.parse(('[' + ' '.join(map(decimal, box)) + ']').encode())
                        pattern.XStep = page.MediaBox[2]
                        pattern.YStep = page.MediaBox[3]
                        del pattern.Matrix
                        pattern_content = prefix + match[5] + b' Do Q'
                        if kind.endswith('-positive'):
                            pattern_content = prefix + b'0.5 g 0 0 1 1 re f Q'
                        pattern.write(pattern_content)
                    pattern.Resources = pikepdf.Dictionary(XObject=pikepdf.Dictionary({name: image}))
                    patterns[pattern_name] = pattern
                    draw = f'/Pattern cs {pattern_name} scn '.encode()
                    if kind == 'tile2-hairline':
                        draw += f'/Pattern CS {pattern_name} SCN 0 w 0 J 0 j 0 0 1 1 re B'.encode()
                    else:
                        draw += b'0 0 1 1 re f'
                    record.update(pattern=pattern_name, tiling=tiling, step=step, content_sha256=hashlib.sha256(pattern_content).hexdigest())
                records.append(record)
                return prefix + draw + b' Q'
            before = page.Contents.read_bytes()
            after, count = IMAGE.subn(replacement, before)
            if count != len(images):
                raise ValueError('Unsupported or unaccounted image placement')
            page.Contents = pikepdf.Stream(pdf, after)
            afters.append(after)
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf:
        assert [list(p.MediaBox) for p in pdf.pages] == boxes
        assert font_hashes(pdf) == fonts
        assert [p.Contents.read_bytes() for p in pdf.pages] == afters
        for page, name, checksum in payloads:
            assert hashlib.sha256(pdf.pages[page - 1].Resources.XObject[name].read_raw_bytes()).hexdigest() == checksum
        for record in records:
            if 'pattern' in record:
                pattern = pdf.pages[record['page'] - 1].Resources.Pattern[record['pattern']]
                if not kind.startswith('tile2-page'):
                    assert list(pattern.Matrix) == list(pikepdf.Object.parse(record['matrix'].encode()))
                else:
                    assert '/Matrix' not in pattern
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(kind=kind, source_sha256=sha(source), output_sha256=sha(output), page_boxes=[[str(x) for x in b] for b in boxes], font_hashes=fonts, records=records, image_payloads_unchanged=True, image_matrix_tokens_unchanged=True, scope='No zoom parameter or physical matrix adjustment. Hairline variants add device-dependent strokes; positive control deliberately changes content.')
if __name__ == '__main__':
    import argparse
    import json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--kind', choices=KINDS, required=True)
    parser.add_argument('--receipt', type=Path)
    args = parser.parse_args()
    result = convert(args.source, args.output, args.kind)
    if args.receipt:
        args.receipt.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
