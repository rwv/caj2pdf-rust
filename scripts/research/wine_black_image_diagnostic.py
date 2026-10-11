#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Replace only verified solid-black grayscale images with equivalent diagnostics.

The source PDF is external. Original resources remain; rectangle and stencil
variants preserve the mathematical black area. The gray positive control does
not preserve color. This is not a production conversion path.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess

from PIL import Image
import pikepdf


def sha(data):
    return hashlib.sha256(data).hexdigest()


def file_sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def convert(source, output, kind):
    if output.exists():
        raise ValueError('Output must be new')
    records = []
    with pikepdf.open(source) as pdf:
        if len(pdf.pages) != 1:
            raise ValueError('Expected one original control page')
        page = pdf.pages[0]
        box = list(page.MediaBox)
        content = page.Contents.read_bytes()
        resources = page.Resources.XObject
        original_hashes = {}
        for name, resource in list(resources.items()):
            if resource.get('/Subtype') != pikepdf.Name.Image:
                raise ValueError('Expected only image XObjects')
            if (resource.get('/ColorSpace') != pikepdf.Name.DeviceGray or
                    resource.get('/Filter') != pikepdf.Name.DCTDecode or
                    resource.get('/BitsPerComponent') != 8 or
                    any(key in resource for key in ['/Mask', '/SMask', '/Decode'])):
                raise ValueError('Expected ordinary grayscale JPEG')
            width, height = int(resource.Width), int(resource.Height)
            if not (1 <= width <= 1024 and 1 <= height <= 1024):
                raise ValueError('Diagnostic image bound exceeded')
            data = resource.read_raw_bytes()
            with Image.open(io.BytesIO(data)) as image:
                if image.mode != 'L' or image.size != (width, height):
                    raise ValueError('JPEG dimensions/mode differ')
                if any(image.tobytes()):
                    raise ValueError('Image is not uniformly black')
            original_hashes[name] = sha(data)
            if kind == 'rectangle':
                replacement = b'0 g 0 0 1 1 re f'
            else:
                new_name = '/Stencil' + name.removeprefix('/Im')
                mask = pikepdf.Stream(pdf, bytes(((width+7)//8)*height))
                mask.Type = pikepdf.Name.XObject
                mask.Subtype = pikepdf.Name.Image
                mask.Width, mask.Height = width, height
                mask.ImageMask, mask.BitsPerComponent = True, 1
                mask.Decode = pikepdf.Array([0, 1])
                resources[new_name] = mask
                color = '0.5' if kind == 'stencil-positive' else '0'
                replacement = f'{color} g {new_name} Do'.encode()
            pattern = re.escape(name.encode()) + rb' Do\b'
            content, count = re.subn(pattern, lambda _: replacement, content)
            if count != 1:
                raise ValueError('Expected exactly one draw per original image')
            records.append(dict(name=name, width=width, height=height,
                                original_jpeg_sha256=sha(data)))
        page.Contents = pikepdf.Stream(pdf, content)
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf:
        assert list(pdf.pages[0].MediaBox) == box
        for name, digest in original_hashes.items():
            assert sha(pdf.pages[0].Resources.XObject[name].read_raw_bytes()) == digest
        assert pdf.pages[0].Contents.read_bytes() == content
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(kind=kind, source_sha256=file_sha(source),
                output_sha256=file_sha(output), page_box=[str(x) for x in box],
                images=records, content_sha256=sha(content), original_payloads_retained=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--kind', choices=['rectangle', 'stencil', 'stencil-positive'], required=True)
    parser.add_argument('--receipt', type=Path)
    args = parser.parse_args()
    result = convert(args.source, args.output, args.kind)
    if args.receipt:
        args.receipt.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
