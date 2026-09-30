#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Extract only the known executable and notices from tested musl archives."""
import argparse
import json
from pathlib import Path
import tarfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('archives', type=Path)
args = parser.parse_args()
version = json.loads(Path('js/package.json').read_text())['version']
platforms = json.loads(Path('docs/container-platforms.json').read_text())
for platform, target in platforms.items():
    arch = platform.removeprefix('linux/').replace('/', '')
    path = args.archives / f'caj2pdf-v{version}-{target}.tar.gz'
    destination = Path('docker/artifacts') / arch
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(path) as archive:
        for name in ['caj2pdf', 'LICENSE', 'THIRD-PARTY-NOTICES.txt']:
            member = archive.getmember(name)
            if not member.isfile():
                raise ValueError(f'Expected regular file: {name}')
            (destination / name).write_bytes(archive.extractfile(member).read())
    (destination / 'caj2pdf').chmod(0o755)

# BuildKit may make these default CPU variants explicit in automatic ARGs.
for alias, original in [('amd64v1', 'amd64'), ('arm64v8', 'arm64')]:
    link = Path('docker/artifacts') / alias
    if not link.exists():
        link.symlink_to(original, target_is_directory=True)
