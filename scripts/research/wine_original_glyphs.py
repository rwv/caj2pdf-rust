#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce original-font shape and mode-0 alias controls without font binaries."""

import argparse
import hashlib
import json
from pathlib import Path

from wine_cjk_aliases import document as c8_document
from wine_mode0_page_extents import document as hnb_document


def isolated(mode, reverse):
    """Six glyphs in six styles, independently positioned in fixed cells."""
    styles = [0x1084, 0x0884, 0x0CA4, 0x10A5, 0x10A4, 0x1000]
    codes = ([0xD6D0, 0xCEC4, 0xA3C1, 0xA3CD, 0xA3B1, 0xA980] if mode == 0
             else [0xD6D0, 0xCEC4, 0xA0C1, 0xA0CD, 0xA0B1, 0xA0E5])
    words, glyphs = [], []
    for i, style in enumerate(reversed(styles) if reverse else styles):
        for j, code in enumerate(reversed(codes) if reverse else codes):
            x = 220 + j * 470 + (13 if reverse else 0)
            y = 220 + i * 470 + (7 if reverse else 0)
            words += [0x8001, 4274 + y, 0x8002, style, 0x801D, 0,
                      0x8067, 6, 0x80CE, 1, 4652 + x, code]
            glyphs.append(dict(row=i, column=j, style=style, code=code, x=x, y=y,
                               cell_raw=[x - 100, y - 100, x + 300, y + 300]))
    document = hnb_document if mode == 0 else c8_document
    return document(words, 3300, 3200), glyphs


def mode_zero_aliases(reverse):
    """Ten digits, parentheses and two unchanged anchors in seven axis profiles."""
    styles = [(0x1084, None), (0x0884, None), (0x0CA4, None), (0x10A5, None),
              (0x10A4, None), (0x1000, None), (0x1084, 36)]
    codes = list(range(0xA3B0, 0xA3BA)) + [0xA3A8, 0xA3A9, 0xA3C1, 0xD6D0]
    words, glyphs = [], []
    for i, (style, axis) in enumerate(reversed(styles) if reverse else styles):
        for j, code in enumerate(reversed(codes) if reverse else codes):
            x = 200 + j * 240 + (13 if reverse else 0)
            y = 220 + i * 440 + (7 if reverse else 0)
            words += [0x8001, 4274 + y, 0x8002, style, 0x801D, 4 if reverse else 0,
                      0x8067, 6, 0x80CE, 1]
            if axis is not None:
                words += [0x8070, axis, 0x8071, axis]
            words += [4652 + x, code]
            glyphs.append(dict(row=i, column=j, style=style, axis=axis, code=code,
                               x=x, y=y, cell_raw=[x - 50, y - 80, x + 180, y + 250]))
    return hnb_document(words, 3700, 3500), glyphs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new output directory")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = []
    for mode in (0, 2):
        for reverse in (False, True):
            name = f"original-glyph-grid-mode{mode}-{int(reverse)}.caj"
            data, glyphs = isolated(mode, reverse)
            (args.output / name).write_bytes(data)
            manifest.append(dict(name=name, sha256=hashlib.sha256(data).hexdigest(),
                                 bytes=len(data), mode=mode, reverse=reverse, glyphs=glyphs))
    for reverse in (False, True):
        name = f"mode0-alias-grid-{int(reverse)}.caj"
        data, glyphs = mode_zero_aliases(reverse)
        (args.output / name).write_bytes(data)
        manifest.append(dict(name=name, sha256=hashlib.sha256(data).hexdigest(),
                             bytes=len(data), mode=0, reverse=reverse, glyphs=glyphs))
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
