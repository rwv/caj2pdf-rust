#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate the original Wine ornament geometry control and marker fonts.

No vendor fonts or document inputs. Requires fontTools 4.62.1 to reproduce the
recorded font bytes. Output must be a new directory outside the source tree.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

ALIASES = {1: 23812, 2: 23813, 46: 23873, 117: 23957}


def controls():
    rows = []
    configurations = [(field, None, 1) for field in (2, 3, 4, 5, 6, 8)]
    configurations += [(5, None, 117), (4, 34, 1), (4, 40, 1)]
    words = [0x8001, 4700, 0x8002, 0x1084, 0x801d, 0, 0x8067, 6]
    for i, (field, axes, value) in enumerate(configurations):
        style = 0x1000 | field << 5 | field
        x, y = 4950, 4750 + i * 400
        words += [0x8001, y, 0x8002, style, 0x801d, 0, 0x80ce, 1]
        if axes is not None:
            words += [0x8070, axes, 0x8071, axes]
        words += [0x8010, value, x, y, x + 3300, y]
        rows.append(dict(field=field, style=style, axes=axes, value=value,
                         x=x, y=y, span=3300))
    words += [0x8004, 1]
    records = struct.pack('<' + 'H' * len(words), *words)
    header = bytearray(80)
    struct.pack_into('<IIII', header, 0, 0xc8, 0, 1, 2)
    header[16:28] = '北大二扫1.00'.encode('gbk')
    struct.pack_into('<HHHH', header, 28, 4652, 4274, 4500, 4500)
    index = struct.pack('<IIIII', 100, len(records), 0, 0, 100 + len(records))
    return bytes(header + index + records), rows


def marker_font(path, advance):
    # These are the project's original square/upper/lower rectangles, retaining
    # its existing geometric font names and metadata for exact reproduction.
    names = ['.notdef', 'square', 'upper', 'lower']
    rectangles = [None, (0, 0, 1000, 1000), (0, 500, 250, 1000), (0, 0, 1000, 500)]
    glyphs = {}
    for name, rectangle in zip(names, rectangles):
        pen = TTGlyphPen(None)
        if rectangle is not None:
            x0, y0, x1, y1 = rectangle
            pen.moveTo((x0, y0))
            for point in [(x1, y0), (x1, y1), (x0, y1)]:
                pen.lineTo(point)
            pen.closePath()
        glyphs[name] = pen.glyph()
    builder = FontBuilder(1000, isTTF=True)
    builder.setupGlyphOrder(names)
    builder.setupCharacterMap({0x4e2d: 'square', 0x6587: 'upper',
                               65: 'square', 77: 'lower', 49: 'square',
                               31908: 'square', 37213: 'lower', 25969: 'square',
                               23812: 'upper'})
    builder.setupGlyf(glyphs)
    metrics = {name: (1000, 0) for name in names}
    metrics['upper'] = (1000 * advance, 0)
    builder.setupHorizontalMetrics(metrics)
    builder.setupHorizontalHeader(ascent=1000, descent=0)
    family = 'HGBZ_CNKI'
    builder.setupNameTable(dict(familyName=family, styleName='Regular',
                               uniqueFontIdentifier='caj2pdf-original-square-control-' + family,
                               fullName=family, psName=family, version='Version 1.0',
                               copyright='Original caj2pdf-rust test outlines; MIT'))
    builder.setupOS2(sTypoAscender=1000, sTypoDescender=0,
                    usWinAscent=1000, usWinDescent=0, fsType=0)
    builder.setupPost()
    builder.setupMaxp()
    builder.font['head'].created = builder.font['head'].modified = 0
    builder.save(path)
    # Reopen only the just-generated original font, never an external font.
    with TTFont(path, recalcTimestamp=False) as font:
        for table in font['cmap'].tables:
            if table.isUnicode():
                for alias in ALIASES.values():
                    table.cmap[alias] = 'upper'
        font['hhea'].advanceWidthMax = 1000 * advance
        font.save(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    output = parser.parse_args().output
    output.mkdir(parents=True, exist_ok=False)
    data, rows = controls()
    (output / 'ornament-metrics.caj').write_bytes(data)
    for advance in (1, 2):
        directory = output / f'advance-{advance}'
        directory.mkdir()
        marker_font(directory / 'HGBZ_CNKI.ttf', advance)
    files = sorted(path for path in output.rglob('*') if path.is_file())
    manifest = dict(rows=rows, origin=[4652, 4274], extent=[4500, 4500],
                    files={str(path.relative_to(output)): hashlib.sha256(path.read_bytes()).hexdigest()
                           for path in files})
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
