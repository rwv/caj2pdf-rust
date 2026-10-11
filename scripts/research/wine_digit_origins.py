#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce original alphabet and digit-state placement controls.

Source generation uses the standard library only. Optional font variants need
fontTools and the five pinned, project-authored baseline geometric fonts from
the preceding axis study. No vendor font or implementation is an input.
"""
import argparse
import json
from pathlib import Path
import shutil
import string

from wine_axis_rasters import document as hnb_document
from wine_cjk_aliases import document as c8_document
from wine_mode0_font_rounding import digest

ROLES = {'cjk': 'HGSS_CNKI.TTF', 'latin': 'HGBZ_CNKI.ttf',
         'alternate-latin': 'HGHZ_CNKI.ttf', 'cjk-kai': 'HGKT_CNKI.TTF',
         'cjk-sans': 'HGHT_CNKI.ttf'}
PROFILES = [('35sq', 0x1084, None), ('42x35', 0x10a4, None), ('43sq', 0x1084, 43)]


def alphabet_grid(style, axis, state, batch, reverse):
    characters = string.ascii_uppercase + string.ascii_lowercase + string.digits
    codes = [0xd6d0] + [0xa080 + ord(c) for c in characters[batch * 31:(batch + 1) * 31]] + [0xcec4]
    words, glyphs = [], []
    for index, code in enumerate(codes[::-1] if reverse else codes):
        row, column = divmod(index, 6)
        x, y = 50 + column * 220 + 11 * reverse, 60 + row * 190 + 17 * reverse
        words += [0x8001, 4274 + y, 0x8002, style, 0x801d, state, 0x8067, 6, 0x80ce, 1]
        if axis:
            words += [0x8070, axis, 0x8071, axis]
        words += [4652 + x, code]
        character = chr(code - 0xa080) if code < 0xa100 else '中' if code == 0xd6d0 else '文'
        glyphs.append(dict(row=row, column=column, style=style, axis=axis, state=state,
                           code=code, character=character, x=x, y=y,
                           cell_raw=[x - 10, y - 40, x + 190, y + 140]))
    return hnb_document(words, 1360, 1200), glyphs


def state_grid(variant, reverse):
    states = [0, 3, 4, 28, 31] if variant == 'C8' else [3]
    configurations = [(state, style, axis) for state in states
                      for style, axis in [(0x1084, None), (0, 36)]]
    codes = [0xd6d0, *range(0xa0b0, 0xa0ba), 0xa0c1]
    words, glyphs = [], []
    for row, (state, style, axis) in enumerate(configurations[::-1] if reverse else configurations):
        for column, code in enumerate(codes[::-1] if reverse else codes):
            x, y = 80 + column * 210 + 9 * reverse, 80 + row * 210 + 13 * reverse
            words += [0x8001, 4274 + y, 0x8002, style, 0x801d, state, 0x8067, 6, 0x80ce, 1]
            if axis:
                words += [0x8070, axis, 0x8071, axis]
            words += [4652 + x, code]
            glyphs.append(dict(row=row, column=column, x=x, y=y, code=code,
                               state=state, style=style, axis=axis,
                               cell_raw=[x - 10, y - 40, x + 180, y + 170]))
    writer = c8_document if variant == 'C8' else hnb_document
    return writer(words, 2600, 2500 if variant == 'C8' else 650), glyphs


def state_fonts(source, output):
    from fontTools.ttLib import TTFont

    receipt = Path(__file__).resolve().parents[2] / 'docs/research/wine-axis-rasterization-20261011.json'
    records = json.loads(receipt.read_text())['plans']['latin-anchor2']['font_records']
    expected = {ROLES[f['role']]: f['font_sha256'] for f in records if f['profile'] == 'base'}
    if len(expected) != 5:
        raise ValueError('incomplete original-font baseline receipt')
    for name, checksum in expected.items():
        if digest(source / name) != checksum:
            raise ValueError(f'{name}: expected original geometric baseline')
    folder = output / 'fonts-digit-states'
    folder.mkdir()
    for name in expected:
        shutil.copyfile(source / name, folder / name)
    for family in ['HGBX_CNKI', 'HGB1_CNKI', 'HGB1X_CNKI']:
        with TTFont(source / 'HGBZ_CNKI.ttf', recalcTimestamp=False, recalcBBoxes=False) as font:
            for record in font['name'].names:
                if record.nameID in [1, 4, 6]:
                    record.string = family
            font.save(folder / (family + '.ttf'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--geometric-fonts', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    fixtures = []

    def emit(name, data, glyphs):
        path = args.output / name
        path.write_bytes(data)
        fixtures.append(dict(name=name, sha256=digest(path), glyphs=glyphs))

    for reverse in [False, True]:
        for profile, style, axis in (PROFILES[:1] if reverse else PROFILES):
            for state in ([0] if reverse else [0, 4]):
                for batch in [0, 1]:
                    name = f'latin-origins-{profile}-s{state}-b{batch}-r{int(reverse)}.caj'
                    emit(name, *alphabet_grid(style, axis, state, batch, reverse))
    for variant, reverse in [('C8', False), ('C8', True), ('HN-B', False)]:
        emit(f'digit-states-{variant}-{int(reverse)}.caj', *state_grid(variant, reverse))
    if args.geometric_fonts:
        state_fonts(args.geometric_fonts, args.output)
    files = {str(p.relative_to(args.output)): digest(p)
             for p in sorted(args.output.rglob('*')) if p.is_file()}
    (args.output / 'manifest.json').write_text(json.dumps(dict(fixtures=fixtures, files=files), indent=2) + '\n')


if __name__ == '__main__':
    main()
