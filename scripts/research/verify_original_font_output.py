#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify original-font PDF output against a subset baseline.

Usage: verify_original_font_output.py baseline.pdf candidate.pdf --fonts PACK
PACK contains manifest.json with fonts[].output paths and optional face indices.
Only external explicit inputs are read; this whole-file research check is not
part of the converter's bounded production path. Requires fontTools and pikepdf.
It compares every drawing operator, resolved glyph outline/metric and PDF width;
semantic text and Wine raster checks are separate obligations.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import pikepdf
from fontTools.ttLib import TTFont
from fontTools.pens.recordingPen import DecomposingRecordingPen

def width(font, cid):
    d = font.DescendantFonts[0]
    w = d.get('/W', [])
    i = 0
    while i < len(w):
        first = int(w[i])
        i += 1
        if isinstance(w[i], pikepdf.Array):
            v = w[i]
            i += 1
            if first <= cid < first + len(v):
                return v[cid - first]
        else:
            last = int(w[i])
            value = w[i + 1]
            i += 2
            if first <= cid <= last:
                return value
    return d.get('/DW', 1000)

def gid(font, cid):
    m = font.DescendantFonts[0].CIDToGIDMap
    if m == pikepdf.Name('/Identity'):
        return cid
    raw = m.read_bytes()
    assert (cid + 1) * 2 <= len(raw)
    return int.from_bytes(raw[cid * 2:cid * 2 + 2], 'big')

def program(font):
    return font.DescendantFonts[0].FontDescriptor.FontFile2.read_bytes()

def verify(baseline, candidate, pack):
    sources = {}
    for entry in json.loads((pack / 'manifest.json').read_text())['fonts']:
        raw = (pack / entry['output']).read_bytes()
        font = TTFont(io.BytesIO(raw), fontNumber=entry.get('face', 0), lazy=True, recalcTimestamp=False)
        name = font['name'].getDebugName(6)
        if name in sources:
            assert sources[name][0] == raw, 'conflicting source font identities'
        sources[name] = (raw, font)
    pairs = {}
    draws = 0
    with pikepdf.open(baseline) as a, pikepdf.open(candidate) as b:
        assert len(a.pages) == len(b.pages)
        for pa, pb in zip(a.pages, b.pages):
            for box in ['/MediaBox', '/CropBox', '/Rotate', '/UserUnit']:
                assert str(pa.get(box)) == str(pb.get(box)), (str(candidate), box)
            left = pikepdf.parse_content_stream(pa)
            right = pikepdf.parse_content_stream(pb)
            assert len(left) == len(right)
            selected = None
            for x, y in zip(left, right):
                assert x.operator == y.operator
                op = str(x.operator)
                if op == 'Tf':
                    selected = str(x.operands[0])
                if op != 'Tj':
                    assert pikepdf.unparse_content_stream([x]) == pikepdf.unparse_content_stream([y]), (str(candidate), op)
                    continue
                assert selected is not None
                lo, hi = (bytes(x.operands[0]), bytes(y.operands[0]))
                assert len(lo) == len(hi) == 2
                ci, cj = (int.from_bytes(lo, 'big'), int.from_bytes(hi, 'big'))
                fa, fb = (pa.Resources.Font[selected], pb.Resources.Font[selected])
                assert width(fa, ci) == width(fb, cj), (str(candidate), selected, ci, cj, 'pdf width')
                key = (fa.objgen, fb.objgen)
                if key not in pairs:
                    pairs[key] = {'fonts': (fa, fb), 'pairs': set(), 'resource': selected}
                pairs[key]['pairs'].add((gid(fa, ci), gid(fb, cj)))
                draws += 1
        fonts = []
        for entry in pairs.values():
            fa, fb = entry['fonts']
            ra, rb = (program(fa), program(fb))
            ta, tb = [TTFont(io.BytesIO(raw), recalcTimestamp=False) for raw in (ra, rb)]
            assert ta['head'].unitsPerEm == tb['head'].unitsPerEm
            source_raw, source = sources[tb['name'].getDebugName(6)]
            if source_raw[:4] != b'ttcf':
                assert rb == source_raw
            else:
                assert set(tb.reader.keys()) == set(source.reader.keys()) - {'DSIG'}
                for tag in tb.reader.keys():
                    v, w = (tb.reader[tag], source.reader[tag])
                    if tag == 'head':
                        v = v[:8] + v[12:]
                        w = w[:8] + w[12:]
                    assert v == w, (str(candidate), tag, 'original TTC table')
            ga, gb = (ta.getGlyphSet(), tb.getGlyphSet())
            oa, ob = (ta.getGlyphOrder(), tb.getGlyphOrder())
            for i, j in entry['pairs']:
                p, q = (DecomposingRecordingPen(ga), DecomposingRecordingPen(gb))
                ga[oa[i]].draw(p)
                gb[ob[j]].draw(q)
                assert p.value == q.value, (str(candidate), entry['resource'], i, j, 'outline')
                assert ta['hmtx'][oa[i]] == tb['hmtx'][ob[j]], (str(candidate), entry['resource'], i, j, 'metrics')
            fonts.append(dict(resource=entry['resource'], glyph_pairs=len(entry['pairs']), original_program_or_collection_tables=True, outline_and_metrics_equal=True))
        result = dict(baseline_sha256=hashlib.sha256(baseline.read_bytes()).hexdigest(), candidate_sha256=hashlib.sha256(candidate.read_bytes()).hexdigest(), pages=len(b.pages), draws=draws, fonts=fonts, all_non_glyph_content_and_page_boxes_identical=True, all_draw_widths_identical=True)
        return result

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('baseline', type=Path)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('--fonts', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.baseline, args.candidate, args.fonts), indent=2))
if __name__ == '__main__':
    main()
