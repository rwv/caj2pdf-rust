#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Move the same image clip before cm, copying original rectangle tokens.

Research-only extension of wine_image_clip_diagnostic. It retains a separate
.unit.pdf intermediate, so both image-space and page-space constructions can
be reproduced. Original matrices and rectangle coordinates are copied without
arithmetic; the rendering effect must still be measured against the reference.
Generated PDFs, fonts and corpus bytes must remain external.
"""
from pathlib import Path
import hashlib
import re
import subprocess
import pikepdf
from wine_image_clip_diagnostic import CLIP, DRAW, convert as unit_clip, fonts, sha, text_digest

CLIPPED_DRAW = re.compile(DRAW.pattern.replace(b'cm )', b'cm )' + re.escape(CLIP)))

def convert(source, output):
    source, output = Path(source), Path(output)
    if output.exists():
        raise ValueError('Expected a new output')
    intermediate = output.with_suffix('.unit.pdf')
    original = unit_clip(source, intermediate)
    pages, rectangles, contents = [], [], []
    with pikepdf.open(intermediate) as pdf:
        boxes = [list(p.MediaBox) for p in pdf.pages]
        for number, page in enumerate(pdf.pages, 1):
            before = page.Contents.read_bytes()
            def replace(match):
                a, d, e, f = match.group(2, 3, 4, 5)
                matrix = b' '.join([a, b'0', b'0', d, e, f]) + b' cm '
                assert match[1].endswith(matrix)
                prefix = match[1][:-len(matrix)]
                rectangle = b' '.join([e, f, a, d]) + b' re W n '
                rectangles.append(dict(page=number, image=match[6].decode(),
                                       tokens=rectangle.decode(), matrix_tokens=matrix.decode()))
                return prefix + rectangle + matrix + match[6] + match[7]
            after, count = CLIPPED_DRAW.subn(replace, before)
            assert count == before.count(CLIP)
            page.Contents = pikepdf.Stream(pdf, after)
            contents.append(after)
            pages.append(dict(page=number, image_count=count,
                              content_sha256=hashlib.sha256(after).hexdigest()))
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(source) as old, pikepdf.open(output) as new:
        assert [list(p.MediaBox) for p in new.pages] == boxes
        assert fonts(new) == original['font_hashes']
        assert [p.Contents.read_bytes() for p in new.pages] == contents
        for number, (a, b) in enumerate(zip(old.pages, new.pages), 1):
            restored = b.Contents.read_bytes()
            for rectangle in rectangles:
                if rectangle['page'] == number:
                    matrix = rectangle['matrix_tokens'].encode()
                    restored = restored.replace(rectangle['tokens'].encode() + matrix, matrix)
            assert restored == a.Contents.read_bytes()
        for record in original['records']:
            image = new.pages[record['page'] - 1].Resources.XObject[record['image']]
            assert hashlib.sha256(image.read_raw_bytes()).hexdigest() == record['payload_sha256']
    assert text_digest(output) == original['semantic_text']
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    return dict(source_sha256=sha(source), output_sha256=sha(output), unit_clip_sha256=sha(intermediate),
                pages=pages, rectangles=rectangles, image_records=original['records'],
                font_hashes=original['font_hashes'], semantic_text=original['semantic_text'],
                scope='Same mathematical clip in page coordinates before image cm; rectangle and matrix tokens copied, with no arithmetic, zoom parameter, resampling or font change.')


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
    result = convert(args.source, args.output)
    if args.receipt:
        args.receipt.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] for key in ["output_sha256", "unit_clip_sha256"]}))
