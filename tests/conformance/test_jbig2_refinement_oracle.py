# SPDX-License-Identifier: MIT
"""Evidence-state tests for the optional refinement-pixel oracle boundary."""

from __future__ import annotations

import hashlib
import contextlib
from io import StringIO
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import jbig2_refinement_oracle as oracle  # noqa: E402


class RefinementOracleStatusTests(unittest.TestCase):
    def test_ci_empty_environment_is_not_explicit_fixture(self) -> None:
        output = StringIO()
        with mock.patch.dict(os.environ, {
            "CAJ2PDF_CORPUS_DIR": "",
            "CAJ2PDF_T88_REFINEMENT_FIXTURE_FILE": "",
            "CAJ2PDF_T88_REFINEMENT_FIXTURE_SHA256": "",
        }), contextlib.redirect_stdout(output):
            self.assertEqual(oracle.main(["--json"]), 0)
        report = json.loads(output.getvalue())
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["refinement_compatibility"]["checked_cases"], 0)

    def test_clean_clone_is_not_pixel_compatibility(self) -> None:
        report = oracle.run(None, None, None)
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["refinement_compatibility"], {
            "status": "NOT_RUN",
            "checked_cases": 0,
            "passed": 0,
            "failed": 0,
            "reason": "no independent refinement-pixel oracle is configured",
        })
        self.assertEqual(report["corpus_metadata"]["checked_images"], 0)

    def test_explicit_missing_or_incomplete_fixture_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing.fixture"
            pinned = hashlib.sha256(b"original private bytes").hexdigest()
            for fixture, digest in [(missing, pinned), (missing, None), (None, pinned)]:
                with self.subTest(fixture=fixture, digest=digest):
                    report = oracle.run(None, fixture, digest)
                    self.assertEqual(report["status"], "FAIL")
                    self.assertEqual(report["refinement_compatibility"]["checked_cases"], 0)
                    self.assertEqual(report["refinement_compatibility"]["passed"], 0)

    def test_changed_fixture_fails_and_verified_fixture_still_is_not_parity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "private.fixture"
            fixture.write_bytes(b"original private bytes")
            pinned = hashlib.sha256(fixture.read_bytes()).hexdigest()
            verified = oracle.run(None, fixture, pinned)
            self.assertEqual(verified["status"], "NOT_RUN")
            self.assertEqual(verified["fixture"]["status"], "VERIFIED")
            self.assertEqual(verified["refinement_compatibility"]["checked_cases"], 0)
            fixture.write_bytes(b"changed  private bytes")
            changed = oracle.run(None, fixture, pinned)
            self.assertEqual(changed["status"], "FAIL")
            self.assertEqual(changed["fixture"]["status"], "FAIL")
            self.assertIn("SHA-256 differs", changed["reason"])

    def test_explicit_missing_corpus_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report = oracle.run(Path(directory) / "missing", None, None)
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["refinement_compatibility"]["checked_cases"], 0)


if __name__ == "__main__":
    unittest.main()
