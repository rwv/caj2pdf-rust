#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Derive packaged Rust/JS resource constants from the CLI's font-pack.json."""
import argparse
import json
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--check', action='store_true', help='fail if checked-in constants differ')
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
source = 'crates/caj2pdf-cli/font-pack.json'
manifest = json.loads((root / source).read_text())
header = f'// SPDX-License-Identifier: MIT\n\n// Generated from {source}; do not edit.\n'
header += '// Regenerate with python3 scripts/generate-font-manifest.py.\n'

digest = bytes.fromhex(manifest['sha256'])
assert len(digest) == 32
resources = manifest['resources']
rust = header + f'\npub const PACK_SIZE: u64 = {manifest["size"]};\n'
rust += 'pub const PACK_SHA256: [u8; 32] = [' + ', '.join(map(str, digest)) + '];\n'
rust += f'pub const RESOURCES: [(u64, u64); {len(resources)}] = [\n'
for item in resources:
    rust += f'    ({item["offset"]}, {item["size"]}), // {item["role"]}\n'
rust += '];\n'
rust = subprocess.check_output(
    ['rustfmt', '--edition', '2024', '--emit', 'stdout'], input=rust.encode(), cwd=root,
).decode()
outputs = {
    'crates/caj2pdf-core/src/bundled_fonts/resources.rs': rust,
    'js/internal/font-pack.mjs': header + '\nexport const FONT_PACK = ' + json.dumps(manifest, indent=2) + ';\n',
}
stale = []
for name, content in outputs.items():
    path = root / name
    if args.check:
        if not path.is_file() or path.read_text() != content:
            stale.append(name)
    else:
        path.write_text(content)
if stale:
    raise SystemExit('Stale font constants: ' + ', '.join(stale) + '\nRun python3 scripts/generate-font-manifest.py')
