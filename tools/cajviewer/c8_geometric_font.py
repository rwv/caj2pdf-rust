# SPDX-License-Identifier: MIT
# Original rectangular glyphs; no external font or outline input.
import argparse
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

# Family names select the viewer resource slots; all outlines below are original.
# The extra cmap entries are observed character queries, not copied font data.
def font(path, family, units=1000):
    names = ['.notdef', 'square', 'upper', 'lower']
    glyphs = {}
    for name, rectangle in zip(names,[None,(0,0,units,units),(0,units//2,units,units),(0,0,units,units//2)]):
        pen=TTGlyphPen(None)
        if rectangle:
            x0,y0,x1,y1=rectangle
            pen.moveTo((x0, y0))
            pen.lineTo((x1, y0))
            pen.lineTo((x1, y1))
            pen.lineTo((x0, y1))
            pen.closePath()
        glyphs[name]=pen.glyph()
    fb=FontBuilder(units,isTTF=True)
    fb.setupGlyphOrder(names)
    fb.setupCharacterMap({0x4e2d:'square',0x6587:'upper',65:'square',77:'lower',49:'square',31908:'square',37213:'lower',25969:'square'})
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({name:(units,0) for name in names})
    fb.setupHorizontalHeader(ascent=units,descent=0)
    fb.setupNameTable({'familyName':family,'styleName':'Regular','uniqueFontIdentifier':'caj2pdf-original-square-control-'+family,'fullName':family,'psName':family,'version':'Version 1.0','copyright':'Original caj2pdf-rust test outlines; MIT'})
    fb.setupOS2(sTypoAscender=units,sTypoDescender=0,usWinAscent=units,usWinDescent=0,fsType=0)
    fb.setupPost()
    fb.setupMaxp()
    fb.font['head'].created=fb.font['head'].modified=0
    fb.save(path)

def main():
    parser = argparse.ArgumentParser(description="Generate original geometric fonts for C8 viewer controls.")
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    for family in ("HGHT_CNKI", "HGBZ_CNKI", "HGHZ_CNKI"):
        font(args.output / (family + ".ttf"), family)


if __name__ == "__main__":
    main()
