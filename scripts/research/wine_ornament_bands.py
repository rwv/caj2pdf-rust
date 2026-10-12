#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original full-width/inset/hinted ornament controls; no vendor outlines.

Requires fontTools 4.62.1. Output must be a new external directory. The existing
geometry control varies style sizes and explicit axes independently of shape.
"""
import argparse
import hashlib
import json
from pathlib import Path

from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont
from fontTools.ttLib.tables.ttProgram import Program
from wine_ornament_geometry import controls, marker_font


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    output = parser.parse_args().output
    output.mkdir(parents=True, exist_ok=False)
    data, rows = controls()
    (output / 'bands.caj').write_bytes(data)
    for kind in ('full', 'inset', 'hinted'):
        path = output / f'{kind}.ttf'
        marker_font(path, 1)
        with TTFont(path, recalcTimestamp=False) as font:
            pen = TTGlyphPen(None)
            left, right = (50, 950) if kind == 'inset' else (-10, 1010)
            for bottom, top in ((300, 360), (600, 900)):
                pen.moveTo((left, bottom))
                for point in ((right, bottom), (right, top), (left, top)):
                    pen.lineTo(point)
                pen.closePath()
            glyph = pen.glyph()
            if kind == 'hinted':
                glyph.program = Program()
                # A harmless stack operation makes the glyph ineligible;
                # production never interprets glyph instructions.
                glyph.program.fromAssembly(['PUSHB[ ]', '0', 'POP[ ]'])
            font['glyf']['upper'] = glyph
            font['hmtx']['upper'] = (1000, left)
            font.save(path)
    files = sorted(p for p in output.iterdir() if p.is_file())
    manifest = dict(rows=rows, origin=[4652, 4274], extent=[4500, 4500],
                    files={p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                           for p in files})
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
