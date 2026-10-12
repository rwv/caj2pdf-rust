#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Restore original complete TTF programs without changing PDF glyph IDs.

The input is a complete-font diagnostic from wine_font_programs.py. --fonts
maps selected PDF resources (e.g. /F1) to unchanged original TTF files whose
tables differ only in head/cmap from the embedded prepared fonts. Resource
paths resolve against the map. Unlisted/unused resources stay untouched.
Requires fontTools, pikepdf, qpdf and pdftotext. Whole-file processing is for
external research only; it is not the converter's production memory path.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess

import pikepdf
from fontTools.ttLib import TTFont


def sha(data):
    return hashlib.sha256(data).hexdigest()


def extracted_text(path):
    return subprocess.run(['pdftotext', str(path), '-'], capture_output=True,
                          check=True).stdout


def invariants(pdf):
    """Bind all used PDF text/geometry to its pre-replacement state."""
    pages = []
    for page in pdf.pages:
        contents = page.get('/Contents')
        streams = contents if isinstance(contents, pikepdf.Array) else [contents]
        fonts = {}
        for key, font in page.Resources.get('/Font', {}).items():
            descendant = font.DescendantFonts[0]
            fonts[str(key)] = dict(
                name=str(font.BaseFont), widths=str(descendant.W),
                default_width=str(descendant.get('/DW')),
                mapping=sha(descendant.CIDToGIDMap.read_bytes()),
                unicode=sha(font.ToUnicode.read_bytes()))
        pages.append(dict(boxes={key: str(page.get(key)) for key in
                                 ['/MediaBox', '/CropBox', '/Rotate', '/UserUnit']},
                          contents=[sha(stream.read_bytes()) for stream in streams],
                          fonts=fonts))
    return pages


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--fonts', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output must be new')
    paths = {key: args.fonts.parent / path
             for key, path in json.loads(args.fonts.read_text()).items()}
    changes = []
    with pikepdf.open(args.input) as pdf:
        before = invariants(pdf)
        seen = set()
        for page in pdf.pages:
            for key, resource in page.Resources.get('/Font', {}).items():
                if resource.objgen in seen or str(key) not in paths:
                    continue
                seen.add(resource.objgen)
                descendant = resource.DescendantFonts[0]
                if not any(descendant.CIDToGIDMap.read_bytes()):
                    continue
                old = descendant.FontDescriptor.FontFile2.read_bytes()
                raw = paths[str(key)].read_bytes()
                if raw[:4] not in [b'\x00\x01\x00\x00', b'true']:
                    raise ValueError('replacement must be a standalone TrueType font')
                prepared = TTFont(io.BytesIO(old), lazy=True)
                original = TTFont(io.BytesIO(raw), lazy=True)
                assert set(prepared.reader.keys()) == set(original.reader.keys())
                assert all(prepared.reader[tag] == original.reader[tag]
                           for tag in prepared.reader.keys() if tag not in ['head', 'cmap'])
                # This admits only cmap changes and head checksum adjustment.
                a, b = bytearray(prepared.reader['head']), bytearray(original.reader['head'])
                a[8:12] = b[8:12] = b'\x00' * 4
                assert a == b
                stream = pdf.make_stream(raw)
                stream.Length1 = len(raw)
                descendant.FontDescriptor.FontFile2 = stream
                changes.append(dict(resource=str(key), prepared_sha256=sha(old),
                                    original_sha256=sha(raw), bytes=len(raw)))
        assert changes, 'no used mapped resource was replaced'
        pdf.save(args.output, deterministic_id=True)
    with pikepdf.open(args.output) as pdf:
        assert invariants(pdf) == before
    subprocess.run(['qpdf', '--check', str(args.output)], capture_output=True, check=True)
    assert extracted_text(args.input) == extracted_text(args.output)
    receipt = dict(input_sha256=sha(args.input.read_bytes()),
                   output_sha256=sha(args.output.read_bytes()), fonts=changes,
                   qpdf=True, extracted_text_identical=True,
                   pdf_text_and_geometry_identical=True,
                   original_glyph_ids_outlines_metrics_and_other_tables_identical=True)
    args.output.with_suffix('.json').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
