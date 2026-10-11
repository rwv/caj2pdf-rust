#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce independently authored multipage alphabet-origin controls.

Only standard-library source generation is required. Optional font preparation
accepts the five pinned, project-authored geometric programs from the preceding
study; it never reads vendor fonts. Use a new output directory.
"""
import argparse
import json
from pathlib import Path
import string
import struct

from wine_axis_rasters import document as hnb_document
from wine_cjk_aliases import document as c8_document
from wine_digit_origins import state_fonts
from wine_mode0_font_rounding import digest

CHARACTERS = ['中', *string.ascii_uppercase, *string.ascii_lowercase,
              *string.digits, '文']


def multipage(pages, width, height, variant):
    """Write original native records behind a bounded, explicit page index."""
    c8 = variant == 'C8'
    writer = c8_document if c8 else hnb_document
    header = bytearray(writer([], width, height)[:80 if c8 else 216])
    struct.pack_into('<I', header, 8 if c8 else 144, len(pages))
    offset = len(header) + (20 if c8 else 12) * len(pages)
    index, chunks = bytearray(), []
    for words in pages:
        words = [*words, 0x8004, 1]
        records = struct.pack('<' + 'H' * len(words), *words)
        if c8:
            index += struct.pack('<IIIII', offset, len(records), 0, 0,
                                 offset + len(records))
        else:
            index += struct.pack('<III', offset, len(records), 0)
        offset += len(records)
        chunks.append(records)
    return bytes(header + index) + b''.join(chunks)


def code_for(character):
    return 0xd6d0 if character == '中' else 0xcec4 if character == '文' else 0xa080 + ord(character)


def glyph_words(code, x, y, style, state, axis):
    words = [0x8001, 4274 + y, 0x8002, style, 0x801d, state,
             0x8067, 6, 0x80ce, 1]
    if axis is not None:
        words += [0x8070, axis, 0x8071, axis]
    return [*words, 4652 + x, code]


def highzoom(name):
    style, axis, width, height, x, y, state, reverse = {
        '35sq': (0x1084, None, 168, 150, 15, 35, 0, False),
        '42x35': (0x10a4, None, 210, 188, 15, 35, 0, False),
        '43sq': (0x1084, 43, 210, 188, 15, 35, 0, False),
        '35held': (0x1084, None, 210, 188, 26, 48, 4, True),
    }[name]
    pages, glyphs = [], []
    for page, character in enumerate(CHARACTERS[::-1] if reverse else CHARACTERS, 1):
        code = code_for(character)
        pages.append(glyph_words(code, x, y, style, state, axis))
        glyphs.append(dict(page=page, code=code, character=character, x=x, y=y,
                           cell_raw=[3, 3, width - 3, height - 3]))
    return multipage(pages, width, height, 'HN-B'), glyphs


def size_states(family):
    if family == 'sizes':
        width, height = 430, 430
        configs = [dict(style=0x1042 if n == 28 else 0x1084,
                        axis=None if n == 28 else n, state=0, width=n, height=n)
                   for n in [22, 28, 34, 36, 38, 40]]
    else:
        width, height = 560, 480
        configs = [dict(style=style, axis=None, state=state, width=w, height=h)
                   for style, w, h in [(0x1084, 35, 35), (0x10a4, 42, 35), (0x10a5, 42, 42)]
                   for state in [3, 28, 31]]
    pages, glyphs = [], []
    for page, character in enumerate(CHARACTERS[::-1] if family == 'states' else CHARACTERS, 1):
        code, words = code_for(character), []
        for index, config in enumerate(configs):
            row, column = divmod(index, 2 if family == 'sizes' else 3)
            x, y = ((15 + column * 210, 36 + row * 138) if family == 'sizes'
                    else (15 + column * 180, 40 + row * 150))
            cell = ([x - 5, y - 20, x + 190, y + 112] if family == 'sizes'
                    else [x - 5, y - 20, x + 170, y + 120])
            words += glyph_words(code, x, y, config['style'], config['state'], config['axis'])
            glyphs.append(dict(page=page, code=code, character=character,
                               x=x, y=y, cell_raw=cell, **config))
        pages.append(words)
    return multipage(pages, width, height, 'C8'), glyphs


def factor_controls(family):
    configs = {
        'small': [(4, 4, 0x1084, 4), (21, 21, 0x1000, None),
                  (24, 24, 0x1021, None), (31, 31, 0x1063, None)],
        'large': [(48, 48, 0x10c6, None), (56, 56, 0x10e7, None),
                  (63, 63, 0x1108, None), (72, 72, 0x1129, None)],
        'axes': [(84, 84, 0x094a, None), (84, 35, 0x1144, None),
                 (35, 84, 0x108a, None), (28, 72, 0x1049, None)],
    }[family]
    state, pages, glyphs = (4 if family == 'small' else 0), [], []
    for page, character in enumerate(CHARACTERS[::-1] if family == 'small' else CHARACTERS, 1):
        code, words = code_for(character), []
        for index, (width, height, style, axis) in enumerate(configs):
            row, column = divmod(index, 2)
            x, y = 15 + column * 330, 50 + row * 300
            words += glyph_words(code, x, y, style, state, axis)
            glyphs.append(dict(page=page, code=code, character=character, x=x, y=y,
                               width=width, height=height, style=style, axis=axis, state=state,
                               cell_raw=[x - 5, y - 40, x + 315, y + 230]))
        pages.append(words)
    return multipage(pages, 670, 620, 'C8'), glyphs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--geometric-fonts', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    files = []

    def emit(name, data, glyphs):
        path = args.output / name
        path.write_bytes(data)
        files.append(dict(name=name, sha256=digest(path), glyphs=glyphs))

    for name in ['35sq', '42x35', '43sq', '35held']:
        emit(f'latin-highzoom-{name}.caj', *highzoom(name))
    for family in ['sizes', 'states']:
        emit(f'latin-{family}-multipage-v4.caj', *size_states(family))
    for family in ['small', 'large', 'axes']:
        emit(f'latin-factor-{family}-v2.caj', *factor_controls(family))
    if args.geometric_fonts:
        state_fonts(args.geometric_fonts, args.output)
    (args.output / 'manifest.json').write_text(json.dumps(files, indent=2) + '\n')


if __name__ == '__main__':
    main()
