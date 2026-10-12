#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Insert a unit-square clip in project-generated axis-aligned image draws.

Research-only input: unrotated, zero-origin pages with one content stream,
unique image draws and optional project BilevelOverlay graphics state.
Image-free documents are copied byte-for-byte. Other content, image matrices,
font programs and encoded image bytes remain unchanged. No zoom is accepted.
Generated PDFs, fonts and corpus bytes must remain external.
"""
from pathlib import Path
from fractions import Fraction
import hashlib
import re
import shutil
import subprocess
import fitz
import pikepdf

DRAW = re.compile(
    rb'(q\s+(?:/BilevelOverlay gs\s+)?'
    rb'([-0-9.]+) 0 0 ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) cm )'
    rb'(/Im\d+)( Do Q)'
)
CLIP = b'0 0 1 1 re W n '

def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def fonts(pdf):
    return sorted(hashlib.sha256(o.read_bytes()).hexdigest() for o in pdf.objects
                  if isinstance(o, pikepdf.Stream) and o.get('/Length1'))

def text_digest(path):
    with fitz.open(path) as pdf:
        value = ''.join(page.get_text() for page in pdf).encode()
    return dict(bytes=len(value), sha256=hashlib.sha256(value).hexdigest())

def convert(source, output):
    source, output = Path(source), Path(output)
    if output.exists():
        raise ValueError('Expected new output')
    records, pages, contents = [], [], []
    semantic = text_digest(source)
    with pikepdf.open(source) as pdf:
        original_fonts = fonts(pdf)
        boxes = [list(p.MediaBox) for p in pdf.pages]
        for number, page in enumerate(pdf.pages, 1):
            if list(page.MediaBox)[:2] != [0, 0] or page.get('/Rotate', 0) != 0:
                raise ValueError('Expected unrotated zero-origin pages')
            before = page.Contents.read_bytes()
            if CLIP in before:
                raise ValueError('Input already has the diagnostic clip')
            images = page.Resources.get('/XObject', {})
            matches = list(DRAW.finditer(before))
            if len(matches) != len(images) or {m[6].decode() for m in matches} != set(images.keys()):
                raise ValueError('Unsupported, repeated or unaccounted image draw')
            for match in matches:
                a, d, e, f = [Fraction(v.decode()) for v in match.group(2, 3, 4, 5)]
                if a <= 0 or d == 0:
                    raise ValueError('Expected nondegenerate axis-aligned image')
                name = match[6].decode()
                image = images[name]
                if image.get('/Subtype') != pikepdf.Name.Image:
                    raise ValueError('Expected image XObject')
                records.append(dict(page=number, image=name,
                                    matrix=[v.decode() for v in match.group(2, 3, 4, 5)],
                                    bilevel_overlay=b'/BilevelOverlay gs' in match[1],
                                    bits_per_component=int(image.BitsPerComponent),
                                    filter=str(image.get('/Filter')),
                                    payload_sha256=hashlib.sha256(image.read_raw_bytes()).hexdigest()))
            after = DRAW.sub(lambda m: m[1] + CLIP + m[6] + m[7], before)
            assert after.replace(CLIP, b'') == before
            if matches:
                page.Contents = pikepdf.Stream(pdf, after)
            contents.append(after)
            pages.append(dict(page=number, image_count=len(matches),
                              original_content_sha256=hashlib.sha256(before).hexdigest(),
                              candidate_content_sha256=hashlib.sha256(after).hexdigest()))
        if records:
            pdf.save(output, deterministic_id=True)
        else:
            shutil.copyfile(source, output)
    with pikepdf.open(output) as pdf:
        assert [list(p.MediaBox) for p in pdf.pages] == boxes
        assert fonts(pdf) == original_fonts
        assert [p.Contents.read_bytes() for p in pdf.pages] == contents
        for r in records:
            image = pdf.pages[r['page'] - 1].Resources.XObject[r['image']]
            assert hashlib.sha256(image.read_raw_bytes()).hexdigest() == r['payload_sha256']
    assert text_digest(output) == semantic
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(source_sha256=sha(source), output_sha256=sha(output), records=records,
                pages=pages, font_hashes=original_fonts, semantic_text=semantic,
                image_payloads_unchanged=True, image_matrix_tokens_unchanged=True,
                operators_unchanged_after_removing_inserted_clips=True,
                byte_unchanged=sha(source) == sha(output),
                scope='No zoom, image decode, font change or matrix adjustment; retain optional BilevelOverlay state and image-free pages.')


if __name__ == "__main__":
    import argparse
    import json

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--receipt", type=Path)
    args = parser.parse_args()
    if args.receipt and args.receipt.exists():
        parser.error("receipt must be a new file")
    receipt = convert(args.source, args.output)
    if args.receipt:
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: receipt[key] for key in ["output_sha256", "byte_unchanged"]}))
