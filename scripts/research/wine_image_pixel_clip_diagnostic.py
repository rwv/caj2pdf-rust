#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Probe clipping decoded source pixels before resizing, with no zoom parameter."""
from pathlib import Path
from fractions import Fraction
from decimal import Decimal, localcontext
import hashlib, io, math, re, subprocess, zlib
import pikepdf
from PIL import Image
IMAGE = re.compile(b'q\\s+([-0-9.]+) 0 0 ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) cm (/Im\\d+) Do Q')

def sha(path):
    with Path(path).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()

def fmt(value):
    with localcontext() as c:
        c.prec = 60
        return format(Decimal(value.numerator) / Decimal(value.denominator), '.30f').rstrip('0').rstrip('.') or '0'

def font_hashes(pdf):
    return sorted((hashlib.sha256(o.read_bytes()).hexdigest() for o in pdf.objects if isinstance(o, pikepdf.Stream) and o.get('/Length1')))

def convert(source, output, rounding):
    source, output = (Path(source), Path(output))
    if output.exists() or rounding not in ['floor', 'ceil', 'nearest']:
        raise ValueError('Expected new output and known pixel rounding')
    quantize = math.floor if rounding == 'floor' else math.ceil if rounding == 'ceil' else lambda v: math.floor(v + Fraction(1, 2))
    records = []
    afters = []
    with pikepdf.open(source) as pdf:
        boxes = [list(p.MediaBox) for p in pdf.pages]
        fonts = font_hashes(pdf)
        for page_number, page in enumerate(pdf.pages, 1):
            if list(page.MediaBox)[:2] != [0, 0] or page.get('/Rotate', 0) != 0:
                raise ValueError('Expected unrotated zero-origin page')
            pw, ph = map(lambda v: Fraction(str(v)), list(page.MediaBox)[2:])
            images = page.Resources.XObject
            expected = len(images)

            def replace(match):
                a, d, e, f = [Fraction(v.decode()) for v in match.group(1, 2, 3, 4)]
                if a <= 0 or d == 0:
                    raise ValueError('Expected supported axis-aligned image')
                left, right = (e, e + a)
                top, bottom = (ph - max(f, f + d), ph - min(f, f + d))
                l, r, t, b = (max(Fraction(0), left), min(pw, right), max(Fraction(0), top), min(ph, bottom))
                if l >= r or t >= b:
                    raise ValueError('Entirely clipped image outside this experiment')
                name = match[5].decode()
                image = images[name]
                if (l, r, t, b) == (left, right, top, bottom):
                    records.append(dict(page=page_number, image=name, changed=False))
                    return match[0]
                if image.get('/Filter') != pikepdf.Name.DCTDecode or image.get('/BitsPerComponent') != 8 or any((key in image for key in ['/Decode', 'DecodeParms', 'Mask', 'SMask'])):
                    raise ValueError('Expected ordinary JPEG')
                mode = {'/DeviceGray': 'L', '/DeviceRGB': 'RGB'}.get(str(image.ColorSpace))
                width, height = (int(image.Width), int(image.Height))
                if mode is None or not 0 < width * height <= 10000000:
                    raise ValueError('JPEG layout/bound')
                original = image.read_raw_bytes()
                with Image.open(io.BytesIO(original)) as loaded:
                    if loaded.size != (width, height) or loaded.mode != mode:
                        raise ValueError('Decoded layout differs')
                    pixels = loaded.copy()
                if d > 0:
                    pixels = pixels.transpose(Image.Transpose.FLIP_TOP_BOTTOM)
                exact = [(l - left) * width / a, (bottom - b) * height / abs(d), (r - left) * width / a, (bottom - t) * height / abs(d)]
                crop = [quantize(v) for v in exact]
                if not (0 <= crop[0] < crop[2] <= width and 0 <= crop[1] < crop[3] <= height):
                    raise ValueError('Empty or out-of-range pixel crop')
                clipped = pixels.crop(tuple(crop))
                data = clipped.tobytes()
                resource = pikepdf.Stream(pdf, zlib.compress(data))
                resource.Type = pikepdf.Name.XObject
                resource.Subtype = pikepdf.Name.Image
                resource.Width = clipped.width
                resource.Height = clipped.height
                resource.ColorSpace = image.ColorSpace
                resource.BitsPerComponent = 8
                resource.Filter = pikepdf.Name.FlateDecode
                if '/Interpolate' in image:
                    resource.Interpolate = image.Interpolate
                images['/Original' + name[1:]] = image
                images[name] = resource
                na, nd, ne, nf = (r - l, -(b - t), l, ph - t)
                assert ne == l and ne + na == r and (ph - nf == t) and (ph - (nf + nd) == b)
                records.append(dict(page=page_number, image=name, changed=True, jpeg_sha256=hashlib.sha256(original).hexdigest(), source_pixels=[width, height], exact_pixel_box=[str(v) for v in exact], pixel_box=crop, decoded_output_sha256=hashlib.sha256(data).hexdigest(), output_pixels=list(clipped.size), before=[str(v) for v in [a, d, e, f]], after=[fmt(v) for v in [na, nd, ne, nf]]))
                return f'q\n{fmt(na)} 0 0 {fmt(nd)} {fmt(ne)} {fmt(nf)} cm {name} Do Q'.encode()
            before = page.Contents.read_bytes()
            after, count = IMAGE.subn(replace, before)
            assert count == expected
            assert IMAGE.sub(b'IMAGE', before) == IMAGE.sub(b'IMAGE', after)
            page.Contents = pikepdf.Stream(pdf, after)
            afters.append(after)
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf:
        assert [list(p.MediaBox) for p in pdf.pages] == boxes and font_hashes(pdf) == fonts
        assert [p.Contents.read_bytes() for p in pdf.pages] == afters
        for record in records:
            if record['changed']:
                resources = pdf.pages[record['page'] - 1].Resources.XObject
                assert hashlib.sha256(resources[record['image']].read_bytes()).hexdigest() == record['decoded_output_sha256']
                assert hashlib.sha256(resources['/Original' + record['image'][1:]].read_raw_bytes()).hexdigest() == record['jpeg_sha256']
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(rounding=rounding, source_sha256=sha(source), output_sha256=sha(output), records=records, font_hashes=fonts, scope='No zoom parameter. Quantized decoded-pixel crop stretched to the mathematical page intersection; boundary quantization can change the pixel field and is a tested hypothesis, not presumed equivalent geometry/content.')

if __name__ == '__main__':
    import argparse, json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--rounding', choices=['floor', 'ceil', 'nearest'], required=True)
    args = parser.parse_args()
    print(json.dumps(convert(args.source, args.output, args.rounding), indent=2))
