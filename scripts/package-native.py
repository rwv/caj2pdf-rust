#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Package an already tested native binary, with project and dependency notices."""
import argparse
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--target', required=True)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--out', type=Path, default=Path('dist'))
args = parser.parse_args()
metadata = json.loads(subprocess.check_output([
    'cargo', 'metadata', '--locked', '--format-version', '1', '--filter-platform', args.target,
]))
packages = {p['id']: p for p in metadata['packages']}
nodes = {n['id']: n for n in metadata['resolve']['nodes']}
cli = next(p for p in packages.values() if p['name'] == 'caj2pdf-cli')
version = cli['version']
assert version == json.loads(Path('js/package.json').read_text())['version']
visited = set()
pending = [cli['id']]
notices = []
while pending:
    key = pending.pop()
    if key in visited:
        continue
    visited.add(key)
    package = packages[key]
    pending.extend(d['pkg'] for d in nodes[key]['deps'] if any(k['kind'] != 'dev' for k in d['dep_kinds']))
    if package['source'] is None:
        continue
    root = Path(package['manifest_path']).parent
    candidates = sorted(root.glob('LICENSE*')) + sorted(root.glob('COPYING*'))
    chosen = None
    for candidate in candidates:
        if not candidate.is_file():
            continue
        text = candidate.read_text(errors='replace')
        if 'permission is hereby granted' in ' '.join(text.lower().split()):
            chosen = text
            if 'mit' in candidate.name.lower():
                break
    if chosen is None:
        raise RuntimeError(f"Missing reviewed MIT notice: {package['name']} {package['version']}")
    notices.append((package['name'], package['version'], chosen))
args.out.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory() as temporary:
    stage = Path(temporary)
    name = 'caj2pdf.exe' if 'windows' in args.target else 'caj2pdf'
    (stage / name).write_bytes(args.binary.read_bytes())
    (stage / name).chmod(0o755)
    for document in ['LICENSE', 'README.md']:
        (stage / document).write_bytes(Path(document).read_bytes())
    (stage / 'THIRD-PARTY-NOTICES.txt').write_text(''.join(
        f'{name} {version} — MIT grant\n{notice}\n\n' for name, version, notice in sorted(notices)
    ), encoding='utf-8')
    stem = f'caj2pdf-v{version}-{args.target}'
    if 'windows' in args.target:
        output = args.out / (stem + '.zip')
        with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED) as archive:
            for file in sorted(stage.iterdir()):
                archive.write(file, file.name)
    else:
        output = args.out / (stem + '.tar.gz')
        with tarfile.open(output, 'w:gz') as archive:
            for file in sorted(stage.iterdir()):
                archive.add(file, arcname=file.name)
    print(output)
