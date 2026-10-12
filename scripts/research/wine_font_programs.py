#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Make external diagnostic PDFs with full fonts or isolated retained metadata.

Requires fontTools and pikepdf, plus qpdf and pdftotext. --fonts is a JSON
object mapping PDF resource names (e.g. /F0) to caller-owned TTF/TTC paths.
The input must use the converter's ordinary Unicode-CID TrueType resources;
this is not a general PDF font replacement tool. Research-only whole-file
processing does not change production ranged I/O. Output must be new.
"""
import argparse
import copy
import hashlib
import io
import json
from pathlib import Path
import subprocess

import pikepdf
from fontTools.ttLib import TTFont, newTable

VARIANTS = {'gasp': ['gasp'], 'full': None, 'cmap': ['cmap'],
            'os2': ['OS/2'], 'name': ['name'], 'post': ['post'],
            'metadata': ['cmap', 'OS/2', 'name', 'post', 'gasp']}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def text(path):
    return subprocess.run(['pdftotext', str(path), '-'], capture_output=True,
                          check=True).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--fonts', type=Path, required=True)
    parser.add_argument('--variant', choices=VARIANTS, action='append',
                        help='generate only this variant (repeatable; default: all)')
    args = parser.parse_args()
    mapping = json.loads(args.fonts.read_text())
    # Resolve relative resource paths against the map, not the working directory.
    paths = {key: args.fonts.parent / value for key, value in mapping.items()}
    fonts = {key: TTFont(path, fontNumber=0, recalcTimestamp=False)
             for key, path in paths.items()}
    programs = {}
    for key, font in fonts.items():
        if paths[key].suffix.lower() == '.ttc':
            buffer = io.BytesIO()
            font.save(buffer)
            programs[key] = buffer.getvalue()
        else:
            programs[key] = paths[key].read_bytes()
    args.output.mkdir(parents=True, exist_ok=False)
    original_text = text(args.input)
    results = []
    for variant in dict.fromkeys(args.variant or VARIANTS):
        tags = VARIANTS[variant]
        output = args.output / f'{variant}.pdf'
        details = []
        with pikepdf.open(args.input) as pdf:
            seen = set()
            for page in pdf.pages:
                for key, resource in page.Resources.Font.items():
                    if resource.objgen in seen:
                        continue
                    seen.add(resource.objgen)
                    descendant = resource.DescendantFonts[0]
                    descriptor = descendant.FontDescriptor
                    cids = descendant.CIDToGIDMap.read_bytes()
                    used = {i // 2: int.from_bytes(cids[i:i + 2], 'big')
                            for i in range(0, len(cids), 2)
                            if int.from_bytes(cids[i:i + 2], 'big')}
                    if not used:
                        continue
                    source = fonts[str(key)]
                    old = descriptor.FontFile2.read_bytes()
                    subset = TTFont(io.BytesIO(old), recalcTimestamp=False)
                    before = {tag: subset.reader[tag] for tag in subset.reader.keys()}
                    cmap = source.getBestCmap()
                    if variant == 'full':
                        remapped = bytearray(len(cids))
                        for cid in used:
                            gid = source.getGlyphID(cmap[cid])
                            remapped[cid * 2:cid * 2 + 2] = gid.to_bytes(2, 'big')
                        descendant.CIDToGIDMap = pdf.make_stream(bytes(remapped))
                        program = programs[str(key)]
                    else:
                        for tag in tags:
                            if tag == 'cmap':
                                gids = {source.getGlyphID(cmap[cid]): gid
                                        for cid, gid in used.items()}
                                gids[0] = 0
                                names = subset.getGlyphOrder()
                                new = newTable('cmap')
                                new.tableVersion = source['cmap'].tableVersion
                                new.tables = []
                                for table in source['cmap'].tables:
                                    # A lazy subtable can otherwise compile its old
                                    # raw bytes and ignore the remapped dictionary.
                                    table.ensureDecompiled()
                                    table = copy.deepcopy(table)
                                    table.cmap = {cp: names[gids[source.getGlyphID(glyph)]]
                                                  for cp, glyph in table.cmap.items()
                                                  if source.getGlyphID(glyph) in gids}
                                    new.tables.append(table)
                                subset['cmap'] = new
                            elif tag in source:
                                subset[tag] = copy.deepcopy(source[tag])
                        buffer = io.BytesIO()
                        subset.save(buffer)
                        program = buffer.getvalue()
                        changed = TTFont(io.BytesIO(program), recalcTimestamp=False)
                        # Preserve all old subset tables; head has a new checksum.
                        assert all(changed.reader[tag] == value
                                   for tag, value in before.items() if tag != 'head')
                    stream = pdf.make_stream(program)
                    stream.Length1 = len(program)
                    descriptor.FontFile2 = stream
                    details.append(dict(resource=str(key), used_cids=len(used),
                                        source_font_sha256=sha(paths[str(key)].read_bytes()),
                                        old_program_sha256=sha(old), new_program_sha256=sha(program),
                                        old_program_bytes=len(old), new_program_bytes=len(program)))
            pdf.save(output, deterministic_id=True)
        subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
        assert text(output) == original_text
        results.append(dict(variant=variant, input_sha256=sha(args.input.read_bytes()),
                            output_sha256=sha(output.read_bytes()), qpdf=True,
                            extracted_text_identical=True, fonts=details))
    (args.output / 'manifest.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
