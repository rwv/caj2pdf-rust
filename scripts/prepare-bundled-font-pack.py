#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reproduce the pinned font pack from unchanged original files, without font tooling."""
import argparse
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('sources', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
manifest = json.loads((root / 'crates/caj2pdf-cli/font-pack.json').read_text())
resources = {item['offset']: item for item in manifest['resources']}
# Create a new output only; never overwrite a source or an existing pack.
with args.output.open('xb') as output:
    try:
        pack_hash = hashlib.sha256()
        for offset, item in sorted(resources.items()):
            if output.tell() != offset or Path(item['file']).name != item['file']:
                raise ValueError('invalid resource layout')
            digest, size = hashlib.sha256(), 0
            with (args.sources / item['file']).open('rb') as source:
                while chunk := source.read(65536):
                    size += len(chunk)
                    if size > item['size']:
                        raise ValueError(f"oversized font: {item['file']}")
                    digest.update(chunk)
                    pack_hash.update(chunk)
                    output.write(chunk)
            if size != item['size'] or digest.hexdigest() != item['sha256']:
                raise ValueError(f"unmeasured font: {item['file']}")
        if output.tell() != manifest['size'] or pack_hash.hexdigest() != manifest['sha256']:
            raise ValueError('pack differs from the pinned resource')
    except BaseException:
        output.close()
        args.output.unlink()
        raise
print(f"Verified {manifest['size']} bytes: {manifest['sha256']}")
