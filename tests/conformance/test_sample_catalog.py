# SPDX-License-Identifier: MIT
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import sample_catalog as catalog


class CatalogTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / 'a.caj'
        self.source.write_bytes(b'original test document')
        self.row = {'path': 'a.caj', 'aliases': [], 'detected_type': 'CAJ',
                    'sha256': hashlib.sha256(self.source.read_bytes()).hexdigest(),
                    'size_bytes': self.source.stat().st_size}
        self.path = self.root / 'catalog.json'

    def prepare(self, rows=None, selected=None):
        self.path.write_text(json.dumps({'samples': rows or [self.row]}))
        digest = hashlib.sha256(self.path.read_bytes()).hexdigest()
        with patch.object(catalog, 'CATALOG_SHA256', digest):
            return catalog.prepare(self.path, self.root, selected or set())

    def test_unknown_is_not_a_compatibility_pass(self):
        row = self.prepare()['samples'][0]
        self.assertEqual(row['expected_outcome'], 'unknown')
        self.assertEqual(row['python_reference']['convert_status'], 'not_run')

    def test_changed_input_fails(self):
        self.source.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'changed sample'):
            self.prepare()

    def test_missing_input_fails(self):
        self.source.unlink()
        with self.assertRaises((OSError, catalog.conformance.ConformanceError)):
            self.prepare()

    def test_deduplicates_content(self):
        self.assertEqual(len(self.prepare([self.row, self.row])['samples']), 1)

    def test_unknown_selection_fails(self):
        with self.assertRaisesRegex(ValueError, 'unknown selected'):
            self.prepare(selected={'missing.caj'})

    def test_wrong_catalog_pin_fails(self):
        self.path.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'pinned commit'):
            catalog.prepare(self.path, self.root, set())

    def test_historical_evidence_preserved(self):
        historical = dict(self.row, expected_outcome='error', python_reference={'convert_status': 'error'})
        with patch.object(catalog.conformance, 'load_matrix', return_value=[historical]):
            row = self.prepare()['samples'][0]
        self.assertEqual(row['expected_outcome'], 'error')
        self.assertEqual(row['python_reference'], {'convert_status': 'error'})
        self.assertNotIn('git_blob_oid', historical)


if __name__ == '__main__':
    unittest.main()
