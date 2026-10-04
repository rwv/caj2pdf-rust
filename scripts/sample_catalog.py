#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify a pinned external sample catalog and reuse the current CLI runner."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import tempfile

import conformance
import current_formats

CATALOG_COMMIT = "a33905e19e8505ff922502b30a8e5c09477ff1b5"
CATALOG_SHA256 = "ef5544a90d392b692ff480dc8a31f9aa1b3faff28abfc408a5937eea7396d68a"


def prepare(catalog, corpus, selected):
    with catalog.open('rb') as source:
        raw = source.read(4 * 1024 * 1024 + 1)
    if hashlib.sha256(raw).hexdigest() != CATALOG_SHA256:
        raise ValueError(f'catalog differs from pinned commit {CATALOG_COMMIT}')
    data = json.loads(raw)
    historical = {r['sha256']: r for r in conformance.load_matrix(conformance.DEFAULT_MATRIX)}
    rows, seen, found = [], set(), set()
    for item in data['samples']:
        if selected and item['path'] not in selected:
            continue
        found.add(item['path'])
        sha = item['sha256']
        if sha in seen:
            continue
        seen.add(sha)
        source = conformance.contained_file(corpus, conformance.relative_path(item['path']))
        size = source.stat().st_size
        actual = hashlib.sha256()
        blob = hashlib.sha1(f'blob {size}\0'.encode())
        with source.open('rb') as stream:
            while chunk := stream.read(262144):
                actual.update(chunk)
                blob.update(chunk)
        if size != item['size_bytes'] or actual.hexdigest() != sha:
            raise ValueError(f"changed sample: {item['path']}")
        row = copy.deepcopy(historical.get(sha, {
            'expected_outcome': 'unknown',
            'python_reference': {'show_status': 'not_run', 'convert_status': 'not_run'},
            'page_count': None, 'outline_count': None,
        }))
        row.update(id=item['path'], path=item['path'], aliases=item['aliases'],
                   sha256=sha, size_bytes=size, git_blob_oid=blob.hexdigest(),
                   detected_type=item['detected_type'], variant=item['detected_type'])
        rows.append(row)
    if selected - found:
        raise ValueError('unknown selected paths: ' + ', '.join(sorted(selected - found)))
    if not rows:
        raise ValueError('no selected documents')
    return {'schema_version': 1, 'catalog_commit': CATALOG_COMMIT, 'samples': rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog', required=True, type=Path)
    parser.add_argument('--corpus-dir', required=True, type=Path)
    parser.add_argument('--sample', action='append', default=[], help='canonical path; repeat to select')
    parser.add_argument('--candidate', type=Path)
    parser.add_argument('--output-dir', type=Path)
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--export-matrix', type=Path, help='prepare only, for existing Node/browser workflows')
    args = parser.parse_args()
    try:
        matrix = prepare(args.catalog, args.corpus_dir.resolve(), set(args.sample))
        if args.export_matrix:
            if args.candidate or args.output_dir:
                raise ValueError('--export-matrix cannot be combined with a conversion run')
            with args.export_matrix.open('x', encoding='utf-8') as target:
                json.dump(matrix, target, indent=2, ensure_ascii=False)
            report = {'status': 'NOT_RUN', 'integrity': 'PASS', 'documents': len(matrix['samples']),
                      'catalog_commit': CATALOG_COMMIT, 'scope': 'identity only; conversion not run'}
        else:
            with tempfile.TemporaryDirectory(prefix='caj2pdf-catalog-') as temporary:
                path = Path(temporary) / 'matrix.json'
                path.write_text(json.dumps(matrix), encoding='utf-8')
                report = current_formats.run(path, args.corpus_dir.resolve(), args.candidate,
                                             args.output_dir, timeout=args.timeout)
                report['catalog_commit'] = CATALOG_COMMIT
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return int(report['status'] == 'FAIL')
    except (OSError, ValueError, conformance.ConformanceError) as error:
        print(json.dumps({'status': 'NOT_RUN', 'integrity': 'FAIL', 'error': str(error)}))
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
