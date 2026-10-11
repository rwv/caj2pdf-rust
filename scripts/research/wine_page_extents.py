#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate seven original C8 page-boundary controls in a new directory.

Uses only the standard library and project-owned C8 framing. No external
document or font input is read. Render each at 75%, 100% and 150% with the
original rectangle query font described in the accompanying research note.
"""
import argparse
import hashlib
import json
from pathlib import Path

from wine_cjk_aliases import document


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    words = []
    for y in (4600, 5000, 5400, 5800, 6200):
        words += [0x8001, y, 0x8002, 0x1084, 0x80ce, 1]
        for x in (5000, 5500, 6000, 6500, 7000):
            words += [x, 0xa0c1]
    cases = [(f'page-extent-{width}.caj', width, 2800)
             for width in (4736, 5105, 5166, 5168)]
    cases += [(f'page-unit-holdout-{width}.caj', width, width)
              for width in (2973, 4038, 5150)]
    files = {}
    for name, width, height in cases:
        data = document(words, width, height)
        (args.output / name).write_bytes(data)
        files[name] = hashlib.sha256(data).hexdigest()
    (args.output / 'manifest.json').write_text(json.dumps(files, indent=2) + '\n')


if __name__ == '__main__':
    main()
