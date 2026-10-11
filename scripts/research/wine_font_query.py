#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate an original cmap-query font for an opt-in Wine viewer experiment.

Every BMP entry paints the resource number and one byte of the queried code
as twelve rectangular holes. No font, document or vendor code is an input.
Requires fontTools; this is a research control, not a usable text font.
"""
import argparse
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.tables._c_m_a_p import CmapSubtable


def generate(output, family, resource, shift):
    if output.exists():
        raise ValueError('refusing to overwrite an existing font')
    if not 1 <= resource <= 15 or shift not in (0, 8):
        raise ValueError('resource must fit four bits and shift must be 0 or 8')
    names = ['.notdef'] + [f'b{i}' for i in range(256)]
    glyphs = {}
    for i, name in enumerate(names):
        pen = TTGlyphPen(None)
        if i:
            marker = resource * 256 + i - 1
            pen.moveTo((0, 0))
            for point in [(1000, 0), (1000, 1000), (0, 1000)]:
                pen.lineTo(point)
            pen.closePath()
            for bit in range(12):
                if marker & (1 << bit):
                    x, y = 100 + bit % 4 * 220, 100 + bit // 4 * 300
                    pen.moveTo((x, y))
                    for point in [(x, y + 180), (x + 120, y + 180), (x + 120, y)]:
                        pen.lineTo(point)
                    pen.closePath()
        glyphs[name] = pen.glyph()
    builder = FontBuilder(1000, isTTF=True)
    builder.setupGlyphOrder(names)
    builder.setupCharacterMap({65: 'b65'})
    cmap = CmapSubtable.newSubtable(13)
    cmap.platformID, cmap.platEncID, cmap.language = 3, 10, 0
    cmap.cmap = {cp: f'b{cp >> shift & 255}' for cp in range(65536)}
    builder.font['cmap'].tables = [cmap]
    builder.setupGlyf(glyphs)
    builder.setupHorizontalMetrics({name: (1000, 0) for name in names})
    builder.setupHorizontalHeader(ascent=1000, descent=0)
    builder.setupNameTable(dict(
        familyName=family, styleName='Regular',
        uniqueFontIdentifier=f'MIT-query-{shift}-{family}', fullName=family,
        psName=family, version='Version 1.0',
        copyright='Original caj2pdf-rust geometric control; MIT',
    ))
    builder.setupOS2(sTypoAscender=1000, sTypoDescender=0,
                    usWinAscent=1000, usWinDescent=0, fsType=0)
    builder.setupPost()
    builder.setupMaxp()
    builder.font['head'].created = builder.font['head'].modified = 2082844800
    builder.save(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--family', required=True)
    parser.add_argument('--resource', type=int, required=True)
    parser.add_argument('--shift', type=int, choices=(0, 8), required=True)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    generate(args.output, args.family, args.resource, args.shift)


if __name__ == '__main__':
    main()
