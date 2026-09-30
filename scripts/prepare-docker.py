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
for arch, target in [('amd64', 'x86_64'), ('arm64', 'aarch64')]:
    path = args.archives / f'caj2pdf-v{version}-{target}-unknown-linux-musl.tar.gz'
    destination = Path('docker/artifacts') / arch
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(path) as archive:
        for name in ['caj2pdf', 'LICENSE', 'THIRD-PARTY-NOTICES.txt']:
            member = archive.getmember(name)
            if not member.isfile():
                raise ValueError(f'Expected regular file: {name}')
            (destination / name).write_bytes(archive.extractfile(member).read())
    (destination / 'caj2pdf').chmod(0o755)
