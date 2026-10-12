#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Wrap images in original converter PDFs with Form XObjects.

Research-only input: unrotated zero-origin pages, one content stream per page,
unique axis-aligned image draws, and no unrelated XObject resources. Original
image programs, net transform tokens, text operators and fonts are retained.
An opacity below one is a deliberately non-equivalent positive control.

Opaque forms preserve mathematical image support, but their clipping and
compositing can change raster pixels. This is not a production fidelity fix.
Generated PDFs and any external font or corpus bytes must remain external.
"""
from pathlib import Path
from fractions import Fraction
from decimal import Decimal
import hashlib
import re
import subprocess

import fitz
import pikepdf

IMAGE = re.compile(b'q\\s+([-0-9.]+) 0 0 ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) cm (/Im\\d+) Do Q')
FORM = re.compile(IMAGE.pattern.replace(b'/Im', b'/FormIm'))
KINDS = ['unit', 'unit-isolated', 'unit-isolated-wide', 'page', 'page-isolated']

def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def fonts(pdf):
    return sorted(
        hashlib.sha256(o.read_bytes()).hexdigest()
        for o in pdf.objects
        if isinstance(o, pikepdf.Stream) and o.get('/Length1')
    )

def text_digest(path):
    with fitz.open(path) as pdf:
        value = ''.join((page.get_text() for page in pdf)).encode()
    return dict(bytes=len(value), sha256=hashlib.sha256(value).hexdigest())

def convert(source, output, kind, alpha=Decimal(1)):
    source, output = (Path(source), Path(output))
    if output.exists() or kind not in KINDS or (not Decimal(0) < alpha <= Decimal(1)):
        raise ValueError('Expected new output, declared form kind and bounded opacity')
    records, afters, payloads = ([], [], [])
    source_text = text_digest(source)
    with pikepdf.open(source) as pdf:
        boxes, original_fonts = ([list(p.MediaBox) for p in pdf.pages], fonts(pdf))
        for page_number, page in enumerate(pdf.pages, 1):
            box = list(page.MediaBox)
            if box[:2] != [0, 0] or page.get('/Rotate', 0) != 0:
                raise ValueError('Expected unrotated zero-origin pages')
            before = page.Contents.read_bytes()
            images = page.Resources.get('/XObject')
            if images is None:
                assert IMAGE.search(before) is None
                afters.append(before)
                continue
            expected = set(images.keys())
            seen = set()

            def replace(match):
                a, d, e, f = [Fraction(v.decode()) for v in match.group(1, 2, 3, 4)]
                if a <= 0 or d == 0:
                    raise ValueError('Expected nondegenerate axis-aligned image')
                name = match[5].decode()
                if name in seen:
                    raise ValueError('Repeated image draw outside this experiment')
                seen.add(name)
                image = images[name]
                if image.get('/Subtype') != pikepdf.Name.Image:
                    raise ValueError('Expected image XObject')
                checksum = hashlib.sha256(image.read_raw_bytes()).hexdigest()
                payloads.append((page_number, name, checksum))
                original_matrix = b' '.join([match[1], b'0', b'0', match[2], match[3], match[4]])
                if kind.startswith('page'):
                    bounds = box
                    outer = b'1 0 0 1 0 0'
                    inner = original_matrix
                    body = match[0]
                else:
                    bounds = [-1, -1, 2, 2] if kind.endswith('wide') else [0, 0, 1, 1]
                    outer = original_matrix
                    inner = b'1 0 0 1 0 0'
                    body = b'q ' + match[5] + b' Do Q'
                # One transform is identity; the other retains original tokens.
                # An omitted Form /Matrix avoids dictionary-number rounding.
                assert (
                    outer == original_matrix and inner == b'1 0 0 1 0 0'
                    or inner == original_matrix and outer == b'1 0 0 1 0 0'
                )
                form = pikepdf.Stream(pdf, body)
                form.Type = pikepdf.Name.XObject
                form.Subtype = pikepdf.Name.Form
                form.FormType = 1
                form.BBox = pikepdf.Array(bounds)
                # Only the original image enters this independent dictionary,
                # so adding the Form to page resources cannot create recursion.
                form.Resources = pikepdf.Dictionary(
                    XObject=pikepdf.Dictionary({name: image})
                )
                isolated = 'isolated' in kind
                if isolated:
                    form.Group = pikepdf.Dictionary(
                        S=pikepdf.Name.Transparency, I=True, K=False,
                        CS=pikepdf.Name.DeviceRGB
                    )
                if alpha != 1:
                    form.Resources.ExtGState = pikepdf.Dictionary(
                        DiagAlpha=pikepdf.Dictionary(
                            Type=pikepdf.Name.ExtGState, ca=alpha, CA=alpha
                        )
                    )
                    body = b'q /DiagAlpha gs\n' + body + b'\nQ'
                    form.write(body)
                form_name = '/Form' + name[1:]
                if form_name in images:
                    raise ValueError('Form name collision')
                images[form_name] = form
                records.append(dict(
                    page=page_number, image=name, form=form_name,
                    jpeg_sha256=checksum, bbox=[str(v) for v in bounds],
                    matrix_before=original_matrix.decode(),
                    outer=outer.decode(), inner=inner.decode(), isolated=isolated,
                    form_stream_sha256=hashlib.sha256(body).hexdigest()
                ))
                return b'q\n' + outer + b' cm ' + form_name.encode() + b' Do Q'
            after, count = IMAGE.subn(replace, before)
            assert seen == expected and count == len(expected)
            assert IMAGE.sub(b'IMAGE', before) == FORM.sub(b'IMAGE', after)
            page.Contents = pikepdf.Stream(pdf, after)
            afters.append(after)
        pdf.save(output, deterministic_id=True, min_version='1.4')
    with pikepdf.open(output) as pdf:
        assert [list(p.MediaBox) for p in pdf.pages] == boxes and fonts(pdf) == original_fonts
        assert [p.Contents.read_bytes() for p in pdf.pages] == afters
        for record in records:
            resources = pdf.pages[record['page'] - 1].Resources.XObject
            form = resources[record['form']]
            assert '/Matrix' not in form
            assert [str(v) for v in form.BBox] == record['bbox']
            assert hashlib.sha256(form.read_bytes()).hexdigest() == record['form_stream_sha256']
            assert hashlib.sha256(form.Resources.XObject[record['image']].read_raw_bytes()).hexdigest() == record['jpeg_sha256']
        for page, name, checksum in payloads:
            assert hashlib.sha256(pdf.pages[page - 1].Resources.XObject[name].read_raw_bytes()).hexdigest() == checksum
    assert text_digest(output) == source_text
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(
        source_sha256=sha(source), output_sha256=sha(output), kind=kind,
        alpha=str(alpha), records=records, font_hashes=original_fonts,
        semantic_text=source_text, image_payloads_unchanged=True,
        image_matrix_tokens_unchanged=True, nonimage_operators_unchanged=True,
        scope=(
            'No zoom parameter or image decode. Ordinary/opaque forms preserve '
            'net image matrices and mathematical visible support. Half-opacity '
            'is an explicit non-equivalent positive control. Pixel identity '
            'is measured, not assumed.'
        )
    )


if __name__ == "__main__":
    import argparse
    import json

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--kind", choices=KINDS, required=True)
    parser.add_argument("--alpha", type=Decimal, default=Decimal(1))
    parser.add_argument("--receipt", type=Path)
    args = parser.parse_args()
    if not Decimal(0) < args.alpha <= Decimal(1):
        parser.error("alpha must be greater than zero and at most one")
    if args.receipt and args.receipt.exists():
        parser.error("receipt must be a new file")
    result = convert(args.source, args.output, args.kind, args.alpha)
    if args.receipt:
        args.receipt.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] for key in ["output_sha256", "kind", "alpha"]}))
