# SPDX-License-Identifier: MIT
"""Tests for the optional Rust HN/C8 record comparison protocol."""

from __future__ import annotations

import contextlib
from io import StringIO
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import hnc8_container_compare as comparison  # noqa: E402


class Hnc8ContainerCompareTests(unittest.TestCase):
    def test_clean_clone_reports_zero_compatibility_passes(self) -> None:
        report = comparison.run(None)
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["type0"], {
            "status": "NOT_RUN", "expected": 1400, "passed": 0, "failed": 0,
            "missing": 0, "not_run": 1400,
        })
        self.assertEqual(report["baseline_invalid"], {
            "status": "NOT_RUN", "expected": 3, "observed": 0, "records": [],
        })
        self.assertEqual(report["source_hashes_before"], 0)

    def test_pinned_historical_details_are_distinct_from_rust_diagnostics(self) -> None:
        _, manifest = comparison.baseline()
        report = comparison.initial_report()
        comparison.record_baseline_invalid(report, manifest)
        self.assertEqual(report["baseline_invalid"]["status"], "PASS")
        self.assertEqual(report["baseline_invalid"]["observed"], 3)
        self.assertEqual(
            report["baseline_invalid"]["records"][0]["detail"],
            "page 2 image 1 is outside the source file",
        )
        self.assertEqual(report["expected_invalid"]["status"], "NOT_RUN")

        changed = json.loads(comparison.MANIFEST.read_text(encoding="utf-8"))
        changed["discovery_failures"][0]["detail"] = "page 2 image 1 has unsupported type"
        with tempfile.TemporaryDirectory() as directory:
            changed_path = Path(directory) / "changed-manifest.json"
            changed_path.write_text(json.dumps(changed), encoding="utf-8")
            with self.assertRaisesRegex(comparison.ComparisonError, "details differ"):
                comparison.baseline(manifest_path=changed_path)

    def test_explicitly_missing_corpus_fails_before_invoking_rust(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "not-present"
            output = StringIO()
            with contextlib.redirect_stdout(output), mock.patch.object(
                comparison, "rust_binary", side_effect=AssertionError("called Rust")
            ):
                self.assertEqual(comparison.main(["--corpus-dir", str(missing), "--json"]), 1)
            self.assertEqual(json.loads(output.getvalue())["status"], "FAIL")

    def test_rust_metadata_protocol_rejects_wrong_variant_and_bad_numbers(self) -> None:
        row = {"id": "sample", "variant": "HN"}
        valid = "I\tHN-A\t2\t1\t40\t0\t52\t12\nE\tHN-B\t3\t-\t70\timage_count\tnegative\n"
        images, errors = comparison.parse_rows(valid, row)
        self.assertEqual((images[0]["descriptor_offset"], images[0]["offset"]), (40, 52))
        self.assertEqual((errors[0]["page"], errors[0]["image"]), (3, None))
        for output in (
            valid.replace("HN-A", "C8", 1),
            valid.replace("\t52\t12", "\t-1\t12", 1),
            valid.replace("\t52\t12", "\t52\t0", 1),
            valid.replace("\t40\t0", "\t4_0\t0", 1),
            valid.replace("image_count", "", 1),
        ):
            with self.subTest(output=output), self.assertRaises(comparison.ComparisonError):
                comparison.parse_rows(output, row)

    def test_comparison_catches_missing_duplicate_and_changed_type0(self) -> None:
        key = comparison.ISSUE_100
        expected = {"samples": [{"id": "sample", "images": [
            {"page": 1, "image": 1, "offset": 52, "length": 12}
        ]}]}
        good = {"source_id": "sample", "page": 1, "image": 1,
                "descriptor_offset": 40, "record_type": 0, "offset": 52, "length": 12}
        other = {**good, "image": 2, "record_type": 1, "offset": 80}
        invalid = [
            {"source_id": key, "page": 2, "image": 1, "offset": 12886,
             "field": "image type", "error_kind": "unsupported"},
            {"source_id": key, "page": 3, "image": None, "offset": 264,
             "field": "image count", "error_kind": "malformed"},
            {"source_id": key, "page": 4, "image": None, "offset": 276,
             "field": "text span", "error_kind": "truncated"},
        ]
        with mock.patch.object(comparison, "EXPECTED_NONZERO", {1: 1}):
            report = comparison.initial_report()
            comparison.compare_records([good, other], invalid, expected, report)
            self.assertEqual((report["type0"]["passed"], report["type0"]["failed"],
                              report["type0"]["missing"]), (1, 0, 0))
            self.assertEqual(report["non_type0"]["counts"], {"1": 1})
            self.assertEqual(report["expected_invalid"]["status"], "PASS")
            report = comparison.initial_report()
            changed_location = [{**invalid[0], "offset": 12887}, *invalid[1:]]
            comparison.compare_records([good, other], changed_location, expected, report)
            self.assertEqual(report["expected_invalid"]["status"], "FAIL")
            for changed in ([good, good, other], [{**good, "offset": 53}, other], [other]):
                with self.subTest(changed=changed):
                    report = comparison.initial_report()
                    comparison.compare_records(changed, invalid, expected, report)
                    self.assertEqual(report["status"], "FAIL")
                    self.assertEqual(report["type0"]["status"], "FAIL")


if __name__ == "__main__":
    unittest.main()
