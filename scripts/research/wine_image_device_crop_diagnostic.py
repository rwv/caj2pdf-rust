#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Declared-scale source-pixel cropping derived from signed device endpoints."""
from pathlib import Path
from fractions import Fraction
from decimal import Decimal, localcontext
import hashlib, io, math, re, subprocess, zlib
import pikepdf
from PIL import Image
IMAGE = re.compile(b'q\\s+([-0-9.]+) 0 0 ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) cm (/Im\\d+) Do Q')
UNIT = Fraction(72, 742)

def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def fmt(value):
    with localcontext() as context:
        context.prec = 60
        return format(Decimal(value.numerator) / Decimal(value.denominator), '.30f').rstrip('0').rstrip('.') or '0'

def fonts(pdf):
    return sorted((hashlib.sha256(o.read_bytes()).hexdigest() for o in pdf.objects if isinstance(o, pikepdf.Stream) and o.get('/Length1')))

def convert(source, output, zoom, endpoints, pixels):
    source, output = (Path(source), Path(output))
    if output.exists() or endpoints not in ['floor', 'truncate'] or pixels not in ['floor', 'ceil', 'nearest'] or (not 1 <= zoom <= 1600):
        raise ValueError('Expected new output and declared endpoint/pixel rules')
    device = math.floor if endpoints == 'floor' else int
    quantize = math.floor if pixels == 'floor' else math.ceil if pixels == 'ceil' else lambda v: math.floor(v + Fraction(1, 2))
    q, records, afters = (Fraction(zoom, 75), [], [])
    with pikepdf.open(source) as pdf:
        boxes, original_fonts = ([list(p.MediaBox) for p in pdf.pages], fonts(pdf))
        for page_number, page in enumerate(pdf.pages, 1):
            if list(page.MediaBox)[:2] != [0, 0] or page.get('/Rotate', 0) != 0:
                raise ValueError('Expected unrotated zero-origin page')
            pw, ph = [Fraction(str(v)) for v in list(page.MediaBox)[2:]]
            images, seen = (page.Resources.XObject, set())
            expected = set(images.keys())

            def replace(match):
                a, d, e, f = [Fraction(v.decode()) for v in match.group(1, 2, 3, 4)]
                if a <= 0 or d >= 0:
                    raise ValueError('Expected negative-d JPEG control')
                name = match[5].decode()
                if name in seen:
                    raise ValueError('Repeated image draw outside experiment')
                seen.add(name)
                top = ph - f
                grid = [v / UNIT for v in [e, top, a, -d]]
                raw = [round(v) for v in grid]
                if max((abs(v - n) for v, n in zip(grid, raw))) > Fraction(1, 100000):
                    raise ValueError('Outside declared integer C8 grid')
                x, y, w, h = [v * UNIT * q for v in raw]
                x0, x1 = (device(x), device(x + w) + 1)
                y0, y1 = (device(y), device(y + h) + 1)
                if x1 <= 0 or y1 <= 0 or e + a > pw or (top - d > ph):
                    raise ValueError('Only partial left/top clipping in this experiment')
                cx0, cy0 = (max(0, x0), max(0, y0))
                image = images[name]
                payload = image.read_raw_bytes()
                record = dict(page=page_number, image=name, raw=raw, device_rectangle=[x0, y0, x1, y1], visible_device_rectangle=[cx0, cy0, x1, y1], jpeg_sha256=hashlib.sha256(payload).hexdigest(), cropped=x0 < 0 or y0 < 0)
                if record['cropped']:
                    if image.get('/Filter') != pikepdf.Name.DCTDecode or image.get('/BitsPerComponent') != 8 or any((k in image for k in ['/Decode', '/Mask', '/SMask'])):
                        raise ValueError('Expected ordinary JPEG')
                    parameters = image.get('/DecodeParms')
                    if parameters is not None and (not (str(image.ColorSpace) == '/DeviceRGB' and isinstance(parameters, pikepdf.Dictionary) and (dict(parameters) == {'/ColorTransform': 1}))):
                        raise ValueError('Unsupported JPEG decode parameters')
                    width, height = (int(image.Width), int(image.Height))
                    mode = {'/DeviceGray': 'L', '/DeviceRGB': 'RGB'}.get(str(image.ColorSpace))
                    if mode is None or not 0 < width * height <= 10000000:
                        raise ValueError('Decoded image bound/layout')
                    exact = [Fraction((cx0 - x0) * width, x1 - x0), Fraction(0), Fraction(width), Fraction(height) - Fraction((cy0 - y0) * height, y1 - y0)]
                    window = [quantize(v) for v in exact]
                    with Image.open(io.BytesIO(payload)) as loaded:
                        assert loaded.size == (width, height) and loaded.mode == mode
                        assert 0 <= window[0] < window[2] <= width and 0 <= window[1] < window[3] <= height
                        cropped = loaded.crop(tuple(window))
                    data = cropped.tobytes()
                    replacement = pikepdf.Stream(pdf, zlib.compress(data))
                    replacement.Type = pikepdf.Name.XObject
                    replacement.Subtype = pikepdf.Name.Image
                    replacement.Width, replacement.Height = cropped.size
                    replacement.ColorSpace, replacement.BitsPerComponent = (image.ColorSpace, 8)
                    replacement.Filter = pikepdf.Name.FlateDecode
                    if '/Interpolate' in image:
                        replacement.Interpolate = image.Interpolate
                    images['/Original' + name[1:]] = image
                    images[name] = replacement
                    record.update(exact_source_window=[str(v) for v in exact], source_window=window, output_pixels=list(cropped.size), decoded_output_sha256=hashlib.sha256(data).hexdigest())
                na, nd = (Fraction(x1 - cx0) / q, -Fraction(y1 - cy0) / q)
                ne, nf = (Fraction(0) if x0 < 0 else e, ph if y0 < 0 else f)
                record.update(before=[str(v) for v in [a, d, e, f]], after=[fmt(v) for v in [na, nd, ne, nf]])
                records.append(record)
                return f'q\n{fmt(na)} 0 0 {fmt(nd)} {fmt(ne)} {fmt(nf)} cm {name} Do Q'.encode()
            before = page.Contents.read_bytes()
            after, count = IMAGE.subn(replace, before)
            assert seen == expected and count == len(expected)
            assert IMAGE.sub(b'IMAGE', before) == IMAGE.sub(b'IMAGE', after)
            page.Contents = pikepdf.Stream(pdf, after)
            afters.append(after)
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf:
        assert fonts(pdf) == original_fonts and [list(p.MediaBox) for p in pdf.pages] == boxes
        assert [p.Contents.read_bytes() for p in pdf.pages] == afters
        for record in records:
            images = pdf.pages[record['page'] - 1].Resources.XObject
            name = record['image']
            original = images['/Original' + name[1:]] if record['cropped'] else images[name]
            assert hashlib.sha256(original.read_raw_bytes()).hexdigest() == record['jpeg_sha256']
            if record['cropped']:
                assert hashlib.sha256(images[name].read_bytes()).hexdigest() == record['decoded_output_sha256']
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(source_sha256=sha(source), output_sha256=sha(output), zoom=zoom, endpoints=endpoints, pixels=pixels, records=records, font_hashes=original_fonts, scope='Target-scale diagnostic; endpoints and pixel crops can change physical geometry and the sampled field. Only partial left/top clipping is supported. No production-equivalence claim.')

if __name__ == '__main__':
    import argparse, json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--zoom', type=int, required=True)
    parser.add_argument('--endpoints', choices=['floor', 'truncate'], required=True)
    parser.add_argument('--pixels', choices=['floor', 'ceil', 'nearest'], required=True)
    args = parser.parse_args()
    print(json.dumps(convert(args.source, args.output, args.zoom, args.endpoints, args.pixels), indent=2))
