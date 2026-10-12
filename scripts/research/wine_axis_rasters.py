#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce original axis and Latin-origin controls (no vendor inputs needed).

Optional font variants accept only the study's five hash-pinned, original MIT
geometric fonts. FontTools is needed for that option; source generation uses
only the standard library. Outputs must use a new directory.
"""
import argparse
import json
from pathlib import Path
import shutil
import struct

from wine_mode0_font_rounding import digest
from wine_mode0_page_extents import document as hnb_document


def document(words, width, height):
    data = bytearray(hnb_document(words, width, height))
    struct.pack_into('<I', data, 148, 2)
    return bytes(data)


def axis_grid(family, reverse):
    profiles = [('35x35', 0x1084, None), ('42x35', 0x10a4, None),
                ('42x42', 0x10a5, None)]
    profiles += [(f'{x}x{y}', 0x1084, [x, y])
                 for x, y in [(28, 28), (43, 28), (28, 43), (43, 43)]]
    codes = [0xd6d0, 0xcec4, 0xa0c1, 0xa0cd, 0xa0b1, 0xa0e5]
    if family == 'high':
        profiles = profiles[3:]
        codes = [0xd6d0, 0xa0c1, 0xa0cd, 0xa0b1]
    words, glyphs = [], []
    for i, (name, style, axes) in enumerate(reversed(profiles) if reverse else profiles):
        for j, code in enumerate(reversed(codes) if reverse else codes):
            if family == 'low':
                x, y = 200 + j * 300 + 13 * reverse, 200 + i * 280 + 7 * reverse
                cell = [x - 40, y - 40, x + 200, y + 180]
            else:
                x, y = 50 + j * 140 + 7 * reverse, 60 + i * 140 + 9 * reverse
                cell = [x - 5, y - 20, x + 135, y + 115]
            words += [0x8001, 4274 + y, 0x8002, style, 0x801d, 0,
                      0x8067, 6, 0x80ce, 1]
            if axes:
                words += [0x8070, axes[0], 0x8071, axes[1]]
            words += [4652 + x, code]
            glyphs.append(dict(row=i, column=j, profile=name, style=style,
                               axes=axes, code=code, x=x, y=y, cell_raw=cell))
    size = (2100, 2200) if family == 'low' else (660, 640)
    return document(words, *size), glyphs


def anchor_grid(reverse, expanded):
    profiles = [(0x1084, None), (0x10a4, None), (0x1084, 43)]
    codes = [(0xd6d0, 0), (0xa0c1, 0), (0xa0cd, 0), (0xa0b1, 0),
             (0xa0e5, 0), (0xa0c1, 4)]
    words, glyphs = [], []
    for i, (style, axis) in enumerate(reversed(profiles) if reverse else profiles):
        for j, (code, state) in enumerate(reversed(codes) if reverse else codes):
            if expanded:
                x, y = 60 + j * 210 + 7 * reverse, 100 + i * 220 + 9 * reverse
                cell = [x - 10, y - 70, x + 180, y + 150]
            else:
                x, y = 50 + j * 160 + 7 * reverse, 80 + i * 170 + 9 * reverse
                cell = [x - 5, y - 45, x + 150, y + 115]
            words += [0x8001, 4274 + y, 0x8002, style, 0x801d, state,
                      0x8067, 6, 0x80ce, 1]
            if axis:
                words += [0x8070, axis, 0x8071, axis]
            words += [4652 + x, code]
            glyphs.append(dict(row=i, column=j, style=style, axis=axis,
                               code=code, state=state, x=x, y=y, cell_raw=cell))
    size = (1300, 800) if expanded else (1080, 650)
    return document(words, *size), glyphs


def font_variants(source, output):
    from fontTools.ttLib import TTFont

    receipt = Path(__file__).resolve().parents[2] / 'docs/research/wine-mode0-font-rounding-20261011.json'
    expected = json.loads(receipt.read_text())['geometric_font_sha256']
    for name, checksum in expected.items():
        if digest(source / name) != checksum:
            raise ValueError(f'{name}: expected original geometric control')
    for variant in ['base', 'advance2000', 'width500', 'shiftx200', 'shifty200']:
        folder = output / f'fonts-{variant}'
        folder.mkdir()
        for name in expected:
            destination = folder / name
            if name != 'HGBZ_CNKI.ttf':
                shutil.copyfile(source / name, destination)
                continue
            with TTFont(source / name, recalcTimestamp=False, recalcBBoxes=False) as font:
                font['head'].xMin = font['head'].yMin = 0
                font['head'].xMax = font['head'].yMax = 1200
                glyph = font['glyf']['square']
                if variant == 'advance2000':
                    font['hmtx']['square'] = (2000, 0)
                    font['hhea'].advanceWidthMax = font['OS/2'].xAvgCharWidth = 2000
                elif variant != 'base':
                    for i, (x, y) in enumerate(glyph.coordinates):
                        if variant == 'width500':
                            x //= 2
                        elif variant == 'shiftx200':
                            x += 200
                        else:
                            y += 200
                        glyph.coordinates[i] = (x, y)
                    glyph.recalcBounds(font['glyf'])
                    if variant == 'shiftx200':
                        font['hmtx']['square'] = (1000, 200)
                font.save(destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--geometric-fonts', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    fixtures = []
    for reverse in [False, True]:
        for family in ['low', 'high']:
            data, glyphs = axis_grid(family, reverse)
            name = f'axis-raster-{family}-{int(reverse)}.caj'
            (args.output / name).write_bytes(data)
            fixtures.append(dict(name=name, sha256=digest(args.output / name), glyphs=glyphs))
        for expanded in [False, True]:
            data, glyphs = anchor_grid(reverse, expanded)
            name = f'latin-anchor{"2" if expanded else ""}-{int(reverse)}.caj'
            (args.output / name).write_bytes(data)
            fixtures.append(dict(name=name, sha256=digest(args.output / name), glyphs=glyphs))
    if args.geometric_fonts:
        font_variants(args.geometric_fonts, args.output)
    files = {str(p.relative_to(args.output)): digest(p)
             for p in sorted(args.output.rglob('*')) if p.is_file()}
    (args.output / 'manifest.json').write_text(json.dumps(dict(fixtures=fixtures, files=files), indent=2) + '\n')


if __name__ == '__main__':
    main()
