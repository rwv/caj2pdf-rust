#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Flip decoded sample order with an exactly compensating PDF image matrix."""
from pathlib import Path
from fractions import Fraction
from decimal import Decimal, localcontext
import hashlib, io, re, subprocess, zlib
import pikepdf
from PIL import Image
IMAGE = re.compile(b'q\\s+([-0-9.]+) 0 0 ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) cm (/Im\\d+) Do Q')

def sha(path):
    with Path(path).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()

def fmt(value):
    with localcontext() as context:
        context.prec = 60
        return format(Decimal(value.numerator) / Decimal(value.denominator), '.30f').rstrip('0').rstrip('.') or '0'

def fonts(pdf):
    return sorted((hashlib.sha256(o.read_bytes()).hexdigest() for o in pdf.objects if isinstance(o, pikepdf.Stream) and o.get('/Length1')))

def convert(source, output, axis):
    source, output = (Path(source), Path(output))
    if output.exists() or axis not in ['none', 'x', 'y']:
        raise ValueError('Expected new output and none/x/y')
    records = []
    after_content = []
    with pikepdf.open(source) as pdf:
        boxes = [list(p.MediaBox) for p in pdf.pages]
        font_hashes = fonts(pdf)
        for page_index, page in enumerate(pdf.pages, 1):
            images = page.Resources.XObject
            original_count = len(images)

            def replace(match):
                name = match[5].decode()
                image = images[name]
                if image.get('/Subtype') != pikepdf.Name.Image or image.get('/Filter') != pikepdf.Name.DCTDecode:
                    raise ValueError('Expected ordinary JPEG')
                if any((key in image for key in ['/Decode', '/Mask', '/SMask'])):
                    raise ValueError('Unexpected image decode/mask options')
                parameters = image.get('/DecodeParms')
                if parameters is not None and not (str(image.ColorSpace) == '/DeviceRGB' and isinstance(parameters, pikepdf.Dictionary) and dict(parameters) == {'/ColorTransform': 1}):
                    raise ValueError('Unsupported JPEG decode parameters')
                width, height = (int(image.Width), int(image.Height))
                if not 0 < width * height <= 10000000 or image.BitsPerComponent != 8:
                    raise ValueError('Image bound or bit depth')
                mode = {'/DeviceGray': 'L', '/DeviceRGB': 'RGB'}.get(str(image.ColorSpace))
                if mode is None:
                    raise ValueError('Expected gray or RGB')
                raw = image.read_raw_bytes()
                with Image.open(io.BytesIO(raw)) as loaded:
                    if loaded.mode != mode or loaded.size != (width, height):
                        raise ValueError('Decoded layout differs')
                    pixels = loaded.copy()
                decoded_sha = hashlib.sha256(pixels.tobytes()).hexdigest()
                if axis != 'none':
                    pixels = pixels.transpose(Image.Transpose.FLIP_LEFT_RIGHT if axis == 'x' else Image.Transpose.FLIP_TOP_BOTTOM)
                payload = pixels.tobytes()
                new = pikepdf.Stream(pdf, zlib.compress(payload))
                for key, value in image.items():
                    if key not in ['/Length', 'Filter', 'DecodeParms']:
                        new[key] = value
                new.Filter = pikepdf.Name.FlateDecode
                images['/Original' + name[1:]] = image
                images[name] = new
                a, d, e, f = [Fraction(v.decode()) for v in match.group(1, 2, 3, 4)]
                na, nd, ne, nf = (a, d, e, f)
                if axis == 'x':
                    na, ne = (-a, e + a)
                if axis == 'y':
                    nd, nf = (-d, f + d)
                assert sorted([e, e + a]) == sorted([ne, ne + na]) and sorted([f, f + d]) == sorted([nf, nf + nd])
                inverse = pixels if axis == 'none' else pixels.transpose(Image.Transpose.FLIP_LEFT_RIGHT if axis == 'x' else Image.Transpose.FLIP_TOP_BOTTOM)
                assert hashlib.sha256(inverse.tobytes()).hexdigest() == decoded_sha
                records.append(dict(page=page_index, image=name, width=width, height=height, mode=mode, jpeg_sha256=hashlib.sha256(raw).hexdigest(), decoded_sha256=decoded_sha, output_decoded_sha256=hashlib.sha256(payload).hexdigest(), before=[str(x) for x in [a, d, e, f]], after=[fmt(x) for x in [na, nd, ne, nf]]))
                return f'q\n{fmt(na)} 0 0 {fmt(nd)} {fmt(ne)} {fmt(nf)} cm {name} Do Q'.encode()
            before = page.Contents.read_bytes()
            after, count = IMAGE.subn(replace, before)
            assert count == original_count
            assert IMAGE.sub(b'IMAGE', before) == IMAGE.sub(b'IMAGE', after)
            page.Contents = pikepdf.Stream(pdf, after)
            after_content.append(after)
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf:
        assert [list(p.MediaBox) for p in pdf.pages] == boxes and fonts(pdf) == font_hashes
        assert [p.Contents.read_bytes() for p in pdf.pages] == after_content
        for r in records:
            images = pdf.pages[r['page'] - 1].Resources.XObject
            assert hashlib.sha256(images[r['image']].read_bytes()).hexdigest() == r['output_decoded_sha256']
            assert hashlib.sha256(images['/Original' + r['image'][1:]].read_raw_bytes()).hexdigest() == r['jpeg_sha256']
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(axis=axis, source_sha256=sha(source), output_sha256=sha(output), records=records, page_boxes=[[str(v) for v in b] for b in boxes], font_hashes=font_hashes, original_jpegs_retained=True, oriented_decoded_pixels_identical=True, mathematical_support_unchanged=True, nonimage_operators_unchanged=True)
if __name__ == '__main__':
    import argparse, json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--axis', choices=['none', 'x', 'y'], required=True)
    parser.add_argument('--receipt', type=Path)
    args = parser.parse_args()
    result = convert(args.source, args.output, args.axis)
    if args.receipt:
        args.receipt.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
