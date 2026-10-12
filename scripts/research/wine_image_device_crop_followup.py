#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Separate the prior zero-clipped-y origin carry from a sampling guard row."""
from pathlib import Path
from fractions import Fraction
import hashlib, io, math, re, subprocess, tempfile, zlib
import pikepdf
from PIL import Image
from wine_image_device_crop_diagnostic import IMAGE, UNIT, convert as device_crop, fmt, fonts, sha
EDGE = re.compile(b'\\nq\\s+[-0-9.]+ 0 0 [-0-9.]+ [-0-9.]+ [-0-9.]+ cm /GuardIm\\d+ Do Q')

def convert(source, output, zoom, guard, edge_strip=False):
    source, output = (Path(source), Path(output))
    if output.exists() or (guard and edge_strip):
        raise ValueError('Expected a new output')
    q = Fraction(zoom, 75)
    records, afters, edges = ([], [], [])
    with tempfile.TemporaryDirectory(prefix='device-crop-followup-', dir=output.parent) as temporary:
        baseline = Path(temporary) / 'baseline.pdf'
        before = device_crop(source, baseline, zoom, 'truncate', 'nearest')
        by_image = {(r['page'], r['image']): r for r in before['records']}
        with pikepdf.open(baseline) as pdf:
            boxes, original_fonts = ([list(p.MediaBox) for p in pdf.pages], fonts(pdf))
            for page_number, page in enumerate(pdf.pages, 1):
                images = page.Resources.XObject

                def replace(match):
                    name = match[5].decode()
                    record = by_image[page_number, name]
                    a, d, e, f = [Fraction(v.decode()) for v in match.group(1, 2, 3, 4)]
                    change = dict(page=page_number, image=name, carry='0', guard_added=False, before=[str(v) for v in [a, d, e, f]])
                    raw_y = record['raw'][1]
                    if raw_y < 0 and record['device_rectangle'][1] == 0:
                        carry = Fraction(int(raw_y * UNIT * q) - math.floor(raw_y * UNIT * q)) / q
                        f -= carry
                        change['carry'] = str(carry)
                    if guard and record['device_rectangle'][1] < 0:
                        original = images['/Original' + name[1:]]
                        window = record['source_window']
                        if window[3] < int(original.Height):
                            padded = [*window[:3], window[3] + 1]
                            with Image.open(io.BytesIO(original.read_raw_bytes())) as loaded:
                                sample = loaded.crop(tuple(padded))
                            data = sample.tobytes()
                            image = pikepdf.Stream(pdf, zlib.compress(data))
                            image.Type = pikepdf.Name.XObject
                            image.Subtype = pikepdf.Name.Image
                            image.Width, image.Height = sample.size
                            image.ColorSpace, image.BitsPerComponent = (original.ColorSpace, 8)
                            image.Filter = pikepdf.Name.FlateDecode
                            if '/Interpolate' in original:
                                image.Interpolate = original.Interpolate
                            height = window[3] - window[1]
                            pitch = -d / height
                            old_bottom = f + d
                            d -= pitch
                            f += pitch
                            assert f + d == old_bottom
                            images[name] = image
                            change.update(guard_added=True, source_window=window, padded_window=padded, original_visible_pixel_pitch=str(pitch), decoded_output_sha256=hashlib.sha256(data).hexdigest())
                    extra = b''
                    if edge_strip and record['device_rectangle'][1] < 0:
                        original = images['/Original' + name[1:]]
                        window = record['source_window']
                        visible_height = record['visible_device_rectangle'][3] - record['visible_device_rectangle'][1]
                        band = max(0, visible_height // (window[3] - window[1]) - 1)
                        if band and window[3] < int(original.Height):
                            with Image.open(io.BytesIO(original.read_raw_bytes())) as loaded:
                                sample = loaded.crop((window[0], window[3], window[2], window[3] + 1))
                            data = sample.tobytes()
                            image = pikepdf.Stream(pdf, zlib.compress(data))
                            image.Type, image.Subtype = (pikepdf.Name.XObject, pikepdf.Name.Image)
                            image.Width, image.Height = sample.size
                            image.ColorSpace, image.BitsPerComponent = (original.ColorSpace, 8)
                            image.Filter = pikepdf.Name.FlateDecode
                            guard_name = '/Guard' + name[1:]
                            images[guard_name] = image
                            top = Fraction(str(page.MediaBox[3]))
                            band_d = -Fraction(band) / q
                            extra = f'\nq\n{fmt(a)} 0 0 {fmt(band_d)} {fmt(e)} {fmt(top)} cm {guard_name} Do Q'.encode()
                            edges.append(dict(page=page_number, image=guard_name, source_image=name, source_row=window[3], height_device=band, matrix=[fmt(v) for v in [a, band_d, e, top]], decoded_output_sha256=hashlib.sha256(data).hexdigest()))
                    change['after'] = [fmt(v) for v in [a, d, e, f]]
                    records.append(change)
                    return f'q\n{fmt(a)} 0 0 {fmt(d)} {fmt(e)} {fmt(f)} cm {name} Do Q'.encode() + extra
                old = page.Contents.read_bytes()
                changed, count = IMAGE.subn(replace, old)
                assert count == len([r for r in before['records'] if r['page'] == page_number])
                assert IMAGE.sub(b'IMAGE', old) == IMAGE.sub(b'IMAGE', EDGE.sub(b'', changed))
                page.Contents = pikepdf.Stream(pdf, changed)
                afters.append(changed)
            pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf:
        assert fonts(pdf) == original_fonts and [list(p.MediaBox) for p in pdf.pages] == boxes
        assert [p.Contents.read_bytes() for p in pdf.pages] == afters
        for record in before['records']:
            images = pdf.pages[record['page'] - 1].Resources.XObject
            name = '/Original' + record['image'][1:] if record['cropped'] else record['image']
            assert hashlib.sha256(images[name].read_raw_bytes()).hexdigest() == record['jpeg_sha256']
        for record in records:
            if record['guard_added']:
                image = pdf.pages[record['page'] - 1].Resources.XObject[record['image']]
                assert hashlib.sha256(image.read_bytes()).hexdigest() == record['decoded_output_sha256']
        for record in edges:
            image = pdf.pages[record['page'] - 1].Resources.XObject[record['image']]
            assert hashlib.sha256(image.read_bytes()).hexdigest() == record['decoded_output_sha256']
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    result = dict(source_sha256=sha(source), output_sha256=sha(output), zoom=zoom, guard=guard, device_crop=before, records=records, font_hashes=original_fonts, scope='Target-scale follow-up. Carry only applies when raw y is negative but truncated device y is zero. The optional single encoded guard row extends above the page; the previous visible rows retain their pixel pitch and bottom anchor. No production correction.')
    if edge_strip:
        result.update(edge_strip=True, edges=edges, scope='Target-scale edge diagnostic: paint the retained adjacent original row over max(0, floor(visible_device_height/cropped_pixel_height)-1) top device rows. This changes the visible pixel field and is a tested hypothesis, not a geometry-preserving production fix.')
    return result

if __name__ == '__main__':
    import argparse, json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--zoom', type=int, required=True)
    group = parser.add_mutually_exclusive_group()
    group.add_argument('--guard', action='store_true')
    group.add_argument('--edge-strip', action='store_true')
    args = parser.parse_args()
    print(json.dumps(convert(args.source, args.output, args.zoom, args.guard, args.edge_strip), indent=2))
