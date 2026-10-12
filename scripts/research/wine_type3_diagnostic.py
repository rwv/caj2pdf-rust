#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce Type 3 diagnostics for the original axis-study controls.

Requires pikepdf and PyMuPDF; paths also require fontTools. These experiments
are not a production correction. Input must match the selected axis fixture.
"""
import argparse
from decimal import Decimal, localcontext
from io import BytesIO
import shutil
import tempfile
import hashlib
import json
from pathlib import Path
import re
import subprocess

import fitz
import pikepdf

from wine_axis_rasters import axis_grid


TEXT = re.compile(rb'BT (/F\d+) 1 Tf\n([0-9.]+) 0 0 ([0-9.]+) ([\-0-9.]+) ([\-0-9.]+) Tm <([0-9A-Fa-f]{4})> Tj ET')


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def font_hashes(pdf):
    return sorted(hashlib.sha256(o.read_bytes()).hexdigest() for o in pdf.objects
                  if isinstance(o, pikepdf.Stream) and o.get('/Length1'))


def fmt(value):
    return format(value, '.30f').rstrip('0').rstrip('.') or '0'


def character(code):
    return chr(code - 0xa080) if 0xa0b0 <= code <= 0xa0fa else bytes([code >> 8, code & 255]).decode('gb18030')


def wrap(source, output, kind, glyphs):
    records = []
    with pikepdf.open(source) as pdf:
        before = font_hashes(pdf)
        page = pdf.pages[0]
        media_box = list(page.MediaBox)
        fonts = page.Resources.Font
        entries, differences, widths, cmap = pikepdf.Dictionary(), [1], [], []
        bounds = []

        def replacement(match):
            number = len(records) + 1
            assert number <= 255
            glyph = glyphs[number - 1]
            a, d, oldx, oldy = [Decimal(v.decode()) for v in match.group(2, 3, 4, 5)]
            if kind == 'direct':
                outerx, outery = oldx, oldy
            else:
                outerx = Decimal(glyph['x'] + 20) * 72 / 742
                outery = Decimal(str(page.MediaBox[3])) - Decimal(glyph['y']) * 72 / 742
            tx, ty = oldx - outerx, oldy - outery
            assert abs(Decimal(fmt(tx)) + Decimal(fmt(outerx)) - oldx) < Decimal('1e-25')
            assert abs(Decimal(fmt(ty)) + Decimal(fmt(outery)) - oldy) < Decimal('1e-25')
            box = [tx - 2 * a, ty - 2 * d, tx + 3 * a, ty + 3 * d]
            bounds.append(box)
            declaration = ('1 0 d0' if kind == 'split-d0' else '1 0 ' + ' '.join(fmt(v) for v in box) + ' d1')
            content = (declaration + '\nBT ' + match[1].decode() + ' 1 Tf\n'
                       + ' '.join([fmt(a), '0', '0', fmt(d), fmt(tx), fmt(ty)])
                       + ' Tm <' + match[6].decode() + '> Tj ET\n')
            name = pikepdf.Name(f'/g{number}')
            entries[str(name)] = pikepdf.Stream(pdf, content.encode())
            differences.append(name)
            widths.append(1)
            semantic = character(glyph['code'])
            cmap.append(f'<{number:02x}> <{semantic.encode("utf-16-be").hex()}>')
            records.append(dict(index=number, code=glyph['code'], inner_font=match[1].decode(),
                                inner_cid=match[6].decode(), before=[str(v) for v in [a, d, oldx, oldy]],
                                outer=[fmt(outerx), fmt(outery)], inner=[fmt(a), fmt(d), fmt(tx), fmt(ty)]))
            return ('BT /Type3Control 1 Tf\n1 0 0 1 ' + fmt(outerx) + ' ' + fmt(outery)
                    + f' Tm <{number:02x}> Tj ET').encode()

        with localcontext() as context:
            context.prec = 60
            data, count = TEXT.subn(replacement, page.Contents.read_bytes())
        assert count == len(glyphs)
        charmap = ('/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n'
                   '/CIDSystemInfo << /Registry (OriginalControl) /Ordering (Unicode) /Supplement 0 >> def\n'
                   '/CMapName /OriginalControl def\n/CMapType 2 def\n'
                   '1 begincodespacerange\n<00> <ff>\nendcodespacerange\n'
                   + str(count) + ' beginbfchar\n' + '\n'.join(cmap) + '\nendbfchar\n'
                   'endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n')
        bbox = [min(b[0] for b in bounds), min(b[1] for b in bounds),
                max(b[2] for b in bounds), max(b[3] for b in bounds)]
        font = pdf.make_indirect(pikepdf.Dictionary(
            Type=pikepdf.Name.Font, Subtype=pikepdf.Name.Type3, Name=pikepdf.Name.Type3Control,
            FontBBox=pikepdf.Array(bbox), FontMatrix=pikepdf.Array([1, 0, 0, 1, 0, 0]),
            CharProcs=entries, Encoding=pikepdf.Dictionary(Type=pikepdf.Name.Encoding,
                                                        Differences=pikepdf.Array(differences)),
            FirstChar=1, LastChar=count, Widths=pikepdf.Array(widths),
            Resources=pikepdf.Dictionary(Font=fonts), ToUnicode=pikepdf.Stream(pdf, charmap.encode())))
        page.Resources = pikepdf.Dictionary(Font=pikepdf.Dictionary(Type3Control=font))
        page.Contents = pikepdf.Stream(pdf, data)
        pdf.save(output)
    with pikepdf.open(output) as pdf:
        assert font_hashes(pdf) == before
        assert list(pdf.pages[0].MediaBox) == media_box
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    with fitz.open(source) as baseline, fitz.open(output) as candidate:
        expected = ''.join(character(g['code']) for g in glyphs)
        old_text = ''.join(baseline[0].get_text().split())
        new_text = ''.join(candidate[0].get_text().split())
        assert old_text == new_text == expected, (old_text, new_text, expected)
    return dict(records=records, embedded_program_hashes=before, semantic_text_sha256=hashlib.sha256(expected.encode()).hexdigest(),
                semantic_glyphs=count, font_programs_unchanged=True, exact_transform_error_bound='1e-25 points')


def outline_path(font, glyph_name):
    from fontTools.pens.basePen import BasePen

    class PDFPen(BasePen):
        def __init__(self):
            super().__init__(font.getGlyphSet())
            self.units = Decimal(font['head'].unitsPerEm)
            self.commands = []

        def point(self, point):
            return ' '.join(fmt(Decimal(str(v)) / self.units) for v in point)

        def _moveTo(self, point):
            self.commands.append(self.point(point) + ' m')

        def _lineTo(self, point):
            self.commands.append(self.point(point) + ' l')

        def _curveToOne(self, one, two, end):
            self.commands.append(' '.join(self.point(p) for p in [one, two, end]) + ' c')

        def _qCurveToOne(self, control, end):
            start = self._getCurrentPoint()
            one = tuple(Decimal(str(a)) + (Decimal(str(b)) - Decimal(str(a))) * 2 / 3 for a, b in zip(start, control))
            two = tuple(Decimal(str(a)) + (Decimal(str(b)) - Decimal(str(a))) * 2 / 3 for a, b in zip(end, control))
            self._curveToOne(one, two, end)

        def _closePath(self):
            self.commands.append('h')

    glyph = font['glyf'][glyph_name]
    if hasattr(glyph, 'program') and glyph.program.getBytecode():
        raise ValueError('control glyph has instructions; outline-only treatment would omit them')
    pen = PDFPen()
    font.getGlyphSet()[glyph_name].draw(pen)
    return '\n'.join(pen.commands) + '\nf\n'


def rewrite(source, output, records, shape, normalized):
    with pikepdf.open(source) as pdf, localcontext() as context:
        context.prec = 60
        before = font_hashes(pdf)
        page = pdf.pages[0]
        oldfont = page.Resources.Font.Type3Control
        resources = pikepdf.Dictionary()
        programs, transforms = {}, []
        for record in records:
            index = record['index']
            a, d, tx, ty = map(Decimal, record['inner'])
            declaration, body = oldfont.CharProcs[f'/g{index}'].read_bytes().split(b' d1\n', 1)
            declaration = declaration.decode() + ' d1\n'
            body = body.decode()
            if shape == 'rectangle':
                body = '0 0 ' + fmt(a) + ' ' + fmt(d) + ' re f\n'
            elif shape == 'paths':
                from fontTools.ttLib import TTFont

                descendant = oldfont.Resources.Font[record['inner_font']].DescendantFonts[0]
                mapping = descendant.CIDToGIDMap
                cid = int(record['inner_cid'], 16)
                gid = int.from_bytes(mapping.read_bytes()[2*cid:2*cid+2], 'big') if isinstance(mapping, pikepdf.Stream) else cid
                stream = descendant.FontDescriptor.FontFile2
                if stream.objgen not in programs:
                    programs[stream.objgen] = TTFont(BytesIO(stream.read_bytes()))
                font = programs[stream.objgen]
                path = outline_path(font, font.getGlyphOrder()[gid])
                body = ('q\n' + ' '.join([fmt(a), '0', '0', fmt(d), fmt(tx), fmt(ty)])
                        + ' cm\n' + path + 'Q\n')
            if normalized:
                sx, sy = ((a if normalized == 'axes' else d), d) if normalized != 'point' else (Decimal(1), Decimal(1))
                cx, cy = (1000 / sx, 1000 / sy) if normalized != 'point' else (Decimal(1), Decimal(1))
                width = 1000 if normalized != 'point' else 1
                box = [(tx - 2*a)*cx, (ty - 2*d)*cy, (tx + 3*a)*cx, (ty + 3*d)*cy]
                declaration = str(width) + ' 0 ' + ' '.join(fmt(v) for v in box) + ' d1\n'
                body = 'q\n' + fmt(cx) + ' 0 0 ' + fmt(cy) + ' 0 0 cm\n' + body + 'Q\n'
                copied = pikepdf.Dictionary({str(k): v for k, v in oldfont.items()})
                factor = Decimal('0.001') if normalized != 'point' else Decimal(1)
                copied.FontMatrix = pikepdf.Array([factor, 0, 0, factor, 0, 0])
                copied.FontBBox = pikepdf.Array(box)
                copied.CharProcs = pikepdf.Dictionary({f'/g{index}': pikepdf.Stream(pdf, (declaration + body).encode())})
                copied.Encoding = pikepdf.Dictionary(Type=pikepdf.Name.Encoding,
                                                     Differences=pikepdf.Array([index, pikepdf.Name(f'/g{index}')]))
                copied.FirstChar = copied.LastChar = index
                copied.Widths = pikepdf.Array([width])
                copied.Name = pikepdf.Name(f'/Control{index}')
                resources[f'/Control{index}'] = pdf.make_indirect(copied)
                transforms.append([fmt(sx), fmt(sy)])
            else:
                oldfont.CharProcs[f'/g{index}'] = pikepdf.Stream(pdf, (declaration + body).encode())
        if normalized:
            def replace(match):
                index = int(match[3], 16)
                xscale, yscale = transforms[index-1]
                return (f'BT /Control{index} 1 Tf\n{xscale} 0 0 {yscale} '
                        + match[1].decode() + ' ' + match[2].decode()
                        + ' Tm <' + match[3].decode() + '> Tj ET').encode()

            data, count = re.subn(rb'BT /Type3Control 1 Tf\n1 0 0 1 ([\-0-9.]+) ([\-0-9.]+) Tm <([0-9a-f]{2})> Tj ET',
                                 replace, page.Contents.read_bytes())
            if count != len(records):
                raise ValueError('unexpected text structure')
            page.Contents = pikepdf.Stream(pdf, data)
            page.Resources = pikepdf.Dictionary(Font=resources)
        pdf.save(output)
    with pikepdf.open(output) as pdf:
        if font_hashes(pdf) != before:
            raise ValueError('original programs changed')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--family', choices=['low', 'high'], required=True)
    parser.add_argument('--reverse', action='store_true')
    parser.add_argument('--origin', choices=['direct', 'split'], default='direct')
    parser.add_argument('--colored', action='store_true', help='use d0 instead of d1 (text only)')
    parser.add_argument('--shape', choices=['text', 'paths', 'rectangle'], default='text')
    parser.add_argument('--normalized', choices=['point', 'square', 'axes'])
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output must be new')
    if args.colored and (args.origin != 'split' or args.shape != 'text' or args.normalized):
        parser.error('the measured d0 treatment uses split origins, text and unnormalized coordinates')
    if args.shape == 'rectangle' and args.origin != 'direct':
        parser.error('the positive rectangle control uses direct origins')
    if args.normalized == 'point' and args.shape != 'rectangle':
        parser.error('point-sized individual fonts are used only for the rectangle control')
    _, glyphs = axis_grid(args.family, args.reverse)
    with tempfile.TemporaryDirectory(prefix='type3-control-', dir=args.output.parent) as folder:
        wrapped = Path(folder) / 'wrapped.pdf'
        metadata = wrap(args.source, wrapped, 'split-d0' if args.colored else args.origin, glyphs)
        if args.normalized or args.shape != 'text':
            rewrite(wrapped, args.output, metadata['records'], args.shape, args.normalized)
        else:
            shutil.copyfile(wrapped, args.output)
    subprocess.run(['qpdf', '--check', str(args.output)], capture_output=True, check=True)
    with fitz.open(args.source) as old, fitz.open(args.output) as new:
        if ''.join(old[0].get_text().split()) != ''.join(new[0].get_text().split()):
            raise ValueError('semantic sequence changed')
    print(json.dumps(dict(pdf_sha256=sha(args.output), semantic_glyphs=metadata['semantic_glyphs'],
                          original_programs_unchanged=True)))


if __name__ == '__main__':
    main()
