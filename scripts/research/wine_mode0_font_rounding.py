#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce original mode-0 font-metric and pixel-rounding controls.

The four documents need only the standard library and no external inputs.
Optional font variants require fontTools and the five hash-pinned, original
MIT geometric controls from the study, not CAJViewer's resource programs.
The output directory must be new; input programs are never modified.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from wine_mode0_page_extents import document


def metric_grid(reverse=False):
    words = []
    styles = [0x1000, 0x1084, 0x10a4, 0x10a5, 0x0ca4, 0x0884]
    codes = [0xd6d0, 0xa3c1, 0xa980]
    for row, style in enumerate(reversed(styles) if reverse else styles):
        for column in range(12):
            x = 250 + column * 300 + (17 if reverse else 0)
            y = 200 + row * 450 + (0, 3, 7, 11)[column // 3] + (13 if reverse else 0)
            code = codes[2 - column % 3 if reverse else column % 3]
            words += [0x8001, 4274 + y, 0x8002, style, 0x801d, 0,
                      0x8067, 6, 0x80ce, 1, 4652 + x, code]
    return document(words, 4000, 3000)


def high_zoom_grid(held_out=False):
    words = []
    styles = [0x1084, 0x10a4, 0x10a5]
    codes = [0xa3c1, 0xd6d0] if held_out else [0xd6d0, 0xa3c1]
    for row, style in enumerate(reversed(styles) if held_out else styles):
        for column, code in enumerate(codes):
            x = 35 + column * 245 + (7 if held_out else 0)
            y = 30 + row * 140 + (13 if held_out else 0)
            words += [0x8001, 4274 + y, 0x8002, style, 0x801d, 0,
                      0x8067, 6, 0x80ce, 1, 4652 + x, code]
    return document(words, 500, 450)


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def font_variants(source, output, expected):
    from fontTools.ttLib import TTFont

    for name, checksum in expected.items():
        if digest(source / name) != checksum:
            raise ValueError(f'{name}: expected the original geometric control')
    for variant in ('base', 'all', 'win', 'hhea', 'typo', 'outline'):
        destination = output / f'fonts-{variant}'
        destination.mkdir()
        for name in expected:
            path = destination / name
            if variant == 'base':
                shutil.copyfile(source / name, path)
                continue
            font = TTFont(source / name, recalcTimestamp=False)
            if variant in ('all', 'win'):
                font['OS/2'].usWinAscent = 1250
                font['OS/2'].usWinDescent = 250
            if variant in ('all', 'hhea'):
                font['hhea'].ascent = 1250
                font['hhea'].descent = -250
            if variant in ('all', 'typo'):
                font['OS/2'].sTypoAscender = 1250
                font['OS/2'].sTypoDescender = -250
            if variant == 'outline':
                glyph = font['glyf']['square']
                glyph.coordinates.translate((0, 200))
                glyph.recalcBounds(font['glyf'])
            font.save(path)
            font.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--geometric-fonts', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    for reverse in (False, True):
        (args.output / f'mode0-metric-grid-{int(reverse)}.caj').write_bytes(metric_grid(reverse))
        (args.output / f'mode0-highzoom-{int(reverse)}.caj').write_bytes(high_zoom_grid(reverse))
    if args.geometric_fonts:
        receipt = Path(__file__).resolve().parents[2] / 'docs/research/wine-mode0-font-rounding-20261011.json'
        expected = json.loads(receipt.read_text())['geometric_font_sha256']
        font_variants(args.geometric_fonts, args.output, expected)
    files = {str(path.relative_to(args.output)): digest(path)
             for path in sorted(args.output.rglob('*')) if path.is_file()}
    (args.output / 'manifest.json').write_text(json.dumps(files, indent=2) + '\n')


if __name__ == '__main__':
    main()
