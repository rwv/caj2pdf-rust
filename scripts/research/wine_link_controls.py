#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate original C8 link-appearance controls without external source data."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import xml.etree.ElementTree as ET
import zlib

from wine_rule_controls import ORIGIN, c8_document


def package(body, annotations):
    root = ET.Element('Package')
    notes = ET.SubElement(ET.SubElement(root, 'Note-Package'), 'NoteItems',
                          Author='Fixture', TextEncode1='Unicode',
                          ReadOnly='False', Version='1.0', Unit='PointsM',
                          Embedded='True')
    for entry in annotations:
        item = ET.SubElement(notes, 'Item', acp='936', Type='Link',
                             Page=str(entry['page']), Color=entry['color'],
                             LineStyle='4', ShowType='1')
        ET.SubElement(item, 'RC', **{key: str(value) for key, value in
                                     zip(('l', 't', 'r', 'b'), entry['rect'])})
        ET.SubElement(item, 'Item', Type='UrlLink',
                      Aim=f"https://example.invalid/link/{entry['id']}", ID='0')
    xml = ET.tostring(root, encoding='utf-8', xml_declaration=True)
    compressed = zlib.compress(xml)
    return (body + struct.pack('<II', len(xml), len(compressed)) + compressed
            + f'APPINFOSIGN {len(body)}'.encode('ascii'))


def generate(output):
    output.mkdir(exist_ok=False)
    annotations = []
    rectangles = [(2500, 2500, 15500, 4500), (3075, 6075, 17025, 8075),
                  (4100, 10950, 15850, 12450), (2550, 15800, 16550, 18800)]
    colors = ['00808080', '00808080', '00000000', '000000FF']
    for page in (1, 2):
        for index in (range(4) if page == 1 else reversed(range(4))):
            # The second page independently shifts all four rectangle edges.
            annotations.append(dict(id=10*page+index, page=page,
                                    rect=[v+(1000 if page == 2 else 0)
                                          for v in rectangles[index]],
                                    color=colors[index]))
    # Native fiducials stay identical when the declared page dimensions change.
    x, y = ORIGIN
    records = [0x8006, 0xa385, x+100, y+100, x+600, y+100,
               0x8006, 0xa385, x+100, y+200, x+100, y+600]
    files = []
    for size in [(2500, 3000), (3200, 3600)]:
        body = bytearray(c8_document([records, records]))
        struct.pack_into('<HH', body, 32, *size)
        for kind, items in [('links', annotations), ('empty', [])]:
            name = f'links-{size[0]}-{kind}.caj'
            data = package(bytes(body), items)
            (output/name).write_bytes(data)
            files.append(dict(name=name, sha256=hashlib.sha256(data).hexdigest(),
                              pages=2, raw_page_size=size, annotations=items,
                              native_prefix_sha256=hashlib.sha256(body).hexdigest()))
    manifest = dict(scope='Original rectangles, page assignments, edge shifts '
                    'and colors; fixed LineStyle 4 / ShowType 1. URLs use the '
                    'reserved .invalid domain and are never opened.', files=files)
    (output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(f"Created {len(generate(args.output)['files'])} original documents")
