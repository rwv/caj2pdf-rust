#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original geometric fonts for C8 viewer controls; no font input."""

import argparse
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen


# Names select viewer resource slots. Aliases are observed character queries;
# neither the names nor the aliases supply any external glyph outline data.
FAMILIES = ("HGHT_CNKI", "HGBZ_CNKI", "HGHZ_CNKI")
CHARACTERS = {
    0x4E2D: "square", 0x6587: "upper",
    65: "square", 77: "lower", 49: "square",
    31908: "square", 37213: "lower", 25969: "square",
}


def font(path, family, units=1000, *, extended_metrics=False, outline_shift=0, decoration_alias=False):
    names = [".notdef", "square", "upper", "lower"]
    rectangles = [
        None,
        (0, 0, units, units),
        (0, units // 2, units, units),
        (0, 0, units, units // 2),
    ]
    glyphs = {}
    for name, rectangle in zip(names, rectangles):
        pen = TTGlyphPen(None)
        if rectangle is not None:
            x0, y0, x1, y1 = rectangle
            pen.moveTo((x0, y0 + outline_shift))
            pen.lineTo((x1, y0 + outline_shift))
            pen.lineTo((x1, y1 + outline_shift))
            pen.lineTo((x0, y1 + outline_shift))
            pen.closePath()
        glyphs[name] = pen.glyph()
    ascent = units + (units // 2 if extended_metrics else 0)
    descent = -(units // 2) if extended_metrics else 0
    builder = FontBuilder(units, isTTF=True)
    builder.setupGlyphOrder(names)
    characters = dict(CHARACTERS)
    if decoration_alias:
        characters[23812] = "upper"
    builder.setupCharacterMap(characters)
    builder.setupGlyf(glyphs)
    builder.setupHorizontalMetrics({name: (units, 0) for name in names})
    builder.setupHorizontalHeader(ascent=ascent, descent=descent)
    builder.setupNameTable({
        "familyName": family,
        "styleName": "Regular",
        "uniqueFontIdentifier": "caj2pdf-original-square-control-" + family,
        "fullName": family,
        "psName": family,
        "version": "Version 1.0",
        "copyright": "Original caj2pdf-rust test outlines; MIT",
    })
    builder.setupOS2(
        sTypoAscender=ascent, sTypoDescender=descent,
        usWinAscent=ascent, usWinDescent=-descent, fsType=0,
    )
    builder.setupPost()
    builder.setupMaxp()
    builder.font["head"].created = builder.font["head"].modified = 0
    builder.save(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory outside the repository")
    parser.add_argument(
        "--variant", choices=("baseline", "extended-metrics", "shifted-outline", "decoration-alias"),
        default="baseline", help="original font metric/outline control",
    )
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    for family in FAMILIES:
        font(
            args.output / (family + ".ttf"), family,
            extended_metrics=args.variant in ("extended-metrics", "shifted-outline"),
            outline_shift=250 if args.variant == "shifted-outline" else 0,
            decoration_alias=args.variant == "decoration-alias",
        )


if __name__ == "__main__":
    main()
