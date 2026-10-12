#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Test CID vertical-origin metrics using original external font programs.

Input must be a matching original axis-control PDF. This is research only;
it does not enable vertical writing in the converter or assert viewer parity.
"""

import argparse
from decimal import Decimal, localcontext
import hashlib
import json
from pathlib import Path
import subprocess

import fitz
import pikepdf

from wine_axis_rasters import axis_grid
from wine_type3_diagnostic import TEXT, character, fmt, font_hashes, sha


KINDS = ["h-clone", "v-zero", "v-x", "v-y", "v-split", "v-positive",
         "v-positive-range", "v-positive-wide", "dv-zero", "dv-base",
         "dv-half", "dv-y", "dv-positive", "r-zero", "r-x", "r-y", "r-split"]


def copy_dictionary(value):
    return pikepdf.Dictionary({str(key): item for key, item in value.items()})


def generate(source, output, family, reverse, kind):
    if output.exists():
        raise ValueError("Output already exists")
    _, glyphs = axis_grid(family, reverse)
    geometry_kind = 'v-' + kind[2:] if kind.startswith('r-') else kind
    records = []
    with pikepdf.open(source) as pdf, localcontext() as context:
        context.prec = 60
        if len(pdf.pages) != 1:
            raise ValueError("Expected a one-page original control")
        page = pdf.pages[0]
        box = list(page.MediaBox)
        original = page.Resources.Font
        # Retain original resources as well as the diagnostic clones so unused
        # embedded programs are not pruned during serialization.
        fonts = copy_dictionary(original)
        before = font_hashes(pdf)

        def replace(match):
            index = len(records)
            glyph = glyphs[index]
            a, d, x, y = [Decimal(value.decode()) for value in match.group(2, 3, 4, 5)]
            ox, oy = x, y
            if geometry_kind in ("v-x", "v-split"):
                ox = Decimal(glyph["x"] + 20) * 72 / 742
            if geometry_kind in ("v-y", "v-split"):
                oy = Decimal(str(box[3])) - Decimal(glyph["y"]) * 72 / 742
            default_metrics = kind.startswith('dv-')
            if default_metrics:
                width = Decimal(0 if kind == 'dv-zero' else 500 if kind == 'dv-half' else 1000)
                ox = x + a * width / 2000
                if kind == 'dv-y':
                    oy = Decimal(str(box[3])) - Decimal(glyph['y']) * 72 / 742
            vx, vy = (ox - x) * 1000 / a, (oy - y) * 1000 / d
            positive = kind in ('v-positive', 'v-positive-range', 'v-positive-wide', 'dv-positive')
            if kind.startswith('v-positive'):
                vx, vy = Decimal(125), Decimal(250)
            elif kind == 'dv-positive':
                vy = Decimal(250)
            expected_x, expected_y = x, y
            if positive:
                if not default_metrics:
                    expected_x -= a / 8
                expected_y -= d / 4
            if max(abs(ox - vx*a/1000 - expected_x), abs(oy - vy*d/1000 - expected_y)) >= Decimal("1e-25"):
                raise ValueError("Declared origin vector changes the expected transform")
            oldfont = original[match[1].decode()]
            if oldfont.Encoding != pikepdf.Name('/Identity-H') or len(oldfont.DescendantFonts) != 1:
                raise ValueError("Expected a horizontal identity CID font")
            parent = copy_dictionary(oldfont)
            descendant = copy_dictionary(oldfont.DescendantFonts[0])
            if descendant.Subtype != pikepdf.Name.CIDFontType2:
                raise ValueError("Expected an original TrueType CID font")
            cid = int(match[6], 16)
            if kind != "h-clone":
                parent.Encoding = pikepdf.Name('/Identity-V')
                descendant.DW2 = pikepdf.Array([0, 0])
                # Parsing the numeric tokens retains their precision; direct
                # construction from Decimal would round dictionary numbers.
                if default_metrics:
                    # DW2 derives x origin from half the default width. These
                    # clones deliberately omit per-CID widths; each Tj still
                    # paints one glyph with an explicit text matrix.
                    for key in ('/W', '/W2'):
                        if key in descendant:
                            del descendant[key]
                    descendant.DW = width
                    descendant.DW2 = pikepdf.Object.parse((f'[{fmt(vy)} 0]').encode())
                elif kind in ('v-positive-range', 'v-positive-wide') or kind.startswith('r-'):
                    first, last = (0, 65535) if kind == 'v-positive-wide' else (cid, cid)
                    descendant.W2 = pikepdf.Object.parse(
                        (f'[{first} {last} 0 {fmt(vx)} {fmt(vy)}]').encode())
                else:
                    descendant.W2 = pikepdf.Object.parse(
                        (f'[{cid} [0 {fmt(vx)} {fmt(vy)}]]').encode())
            parent.DescendantFonts = pikepdf.Array([pdf.make_indirect(descendant)])
            name = f'/Metric{index}'
            fonts[name] = pdf.make_indirect(parent)
            records.append(dict(index=index, code=glyph['code'], font=match[1].decode(),
                                resource=name, cid=cid, before=[fmt(v) for v in [a, d, x, y]],
                                text_matrix=[fmt(v) for v in [a, d, ox, oy]],
                                position_vector=[fmt(vx), fmt(vy)],
                                expected_origin=[fmt(expected_x), fmt(expected_y)]))
            return ('BT ' + name + ' 1 Tf\n'
                    + ' '.join([fmt(a), '0', '0', fmt(d), fmt(ox), fmt(oy)])
                    + ' Tm <' + match[6].decode() + '> Tj ET').encode()

        content, count = TEXT.subn(replace, page.Contents.read_bytes())
        if count != len(glyphs):
            raise ValueError("Input glyph count does not match the fixture")
        page.Resources.Font = fonts
        page.Contents = pikepdf.Stream(pdf, content)
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf, localcontext() as context:
        context.prec = 60
        if font_hashes(pdf) != before or list(pdf.pages[0].MediaBox) != box:
            raise ValueError("Embedded programs or page bounds changed")
        if pdf.pages[0].Contents.read_bytes() != content:
            raise ValueError("Serialized drawing operators changed")
        for record in records:
            parent = pdf.pages[0].Resources.Font[record['resource']]
            descendant = parent.DescendantFonts[0]
            if kind != 'h-clone':
                if kind.startswith('dv-'):
                    if '/W' in descendant or '/W2' in descendant or descendant.DW2[1] != 0:
                        raise ValueError('Default-metric control contains a per-CID override')
                    vx = Decimal(str(descendant.DW)) / 2
                    vy = Decimal(str(descendant.DW2[0]))
                elif kind in ('v-positive-range', 'v-positive-wide') or kind.startswith('r-'):
                    values = descendant.W2
                    if not values[0] <= record['cid'] <= values[1] or values[2] != 0:
                        raise ValueError('Vertical metric range misses the target CID')
                    vx, vy = [Decimal(str(values[i])) for i in (3, 4)]
                else:
                    values = descendant.W2[1]
                    if descendant.W2[0] != record['cid'] or values[0] != 0:
                        raise ValueError("Serialized vertical metric identifies the wrong CID")
                    vx, vy = [Decimal(str(values[i])) for i in (1, 2)]
                a, d, ox, oy = map(Decimal, record['text_matrix'])
                ex, ey = map(Decimal, record['expected_origin'])
                if max(abs(ox-vx*a/1000-ex), abs(oy-vy*d/1000-ey)) >= Decimal('1e-25'):
                    raise ValueError("Serialized vertical metric loses transform precision")
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    expected = ''.join(character(g['code']) for g in glyphs)
    with fitz.open(source) as baseline, fitz.open(output) as candidate:
        old = ''.join(baseline[0].get_text().split())
        new = ''.join(candidate[0].get_text().split())
        if old != expected or new != expected:
            raise ValueError('Semantic text changed: ' + repr((old, new, expected)))
    return dict(kind=kind, family=family, reverse=reverse, input_sha256=sha(source),
                output_sha256=sha(output), records=records, embedded_program_sha256=before,
                font_programs_unchanged=True, semantic_glyphs=len(glyphs),
                semantic_text_sha256=hashlib.sha256(expected.encode()).hexdigest(),
                transform_error_bound='1e-25 points', positive_control='positive' in kind)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--family', choices=['low', 'high'], required=True)
    parser.add_argument('--reverse', action='store_true')
    parser.add_argument('--kind', choices=KINDS, required=True)
    parser.add_argument('--receipt', type=Path)
    args = parser.parse_args()
    result = generate(args.source, args.output, args.family, args.reverse, args.kind)
    if args.receipt:
        args.receipt.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: result[key] for key in ['output_sha256', 'semantic_glyphs', 'positive_control']}))
