# SPDX-License-Identifier: MIT
"""Public contract tests for the optional, metadata-only #107 oracle."""

from __future__ import annotations

import argparse
from copy import deepcopy
import json
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_layout_parity as layout  # noqa: E402


class LayoutParityTests(unittest.TestCase):
    def arguments(self, **overrides: object) -> argparse.Namespace:
        values = {"corpus_dir": None, "hn_a_pdf": None, "c8_pdf": None,
                  "hn_b_pdf": None, "oracle": layout.LAYOUT_ORACLE,
                  "candidate_output": None}
        values.update(overrides)
        return argparse.Namespace(**values)

    def oracle(self) -> dict:
        with layout.LAYOUT_ORACLE.open(encoding="utf-8") as source:
            return json.load(source)

    def test_clean_clone_is_not_run_with_zero_comparisons(self) -> None:
        report = layout.run(self.arguments())
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["source_pages_checked"], 0)
        self.assertEqual(report["output_pages_checked"], 0)
        self.assertEqual(report["draws_checked"], 0)
        self.assertEqual(report["existing_pages_matched"], 0)
        self.assertEqual(report["existing_draws_matched"], 0)
        self.assertEqual(report["malformed_checked"], 0)
        self.assertEqual(report["source_hashes_before"], {})
        self.assertEqual(report["pdf_hashes_before"], {})

    def test_partial_requested_input_fails_before_opening_a_document(self) -> None:
        with self.assertRaisesRegex(layout.LayoutParityError, "all three PDFs"):
            layout.run(self.arguments(corpus_dir=Path("/missing/corpus")))
        with self.assertRaisesRegex(layout.LayoutParityError, "all three PDFs"):
            layout.run(self.arguments(hn_a_pdf=Path("/missing/hn-a.pdf")))
        with self.assertRaisesRegex(layout.LayoutParityError, "candidate generation requires"):
            layout.run(self.arguments(candidate_output=Path("/tmp/new-layout-candidate.json")))

    def test_oracle_is_compact_metadata_without_private_bytes(self) -> None:
        oracle = self.oracle()
        self.assertLess(layout.LAYOUT_ORACLE.stat().st_size, 256 * 1024)
        self.assertEqual(set(oracle), {"schema_version", "matrix_sha256",
                                       "pdf_tool_hashes", "cases"})
        self.assertEqual([case["case"] for case in oracle["cases"]],
                         ["hn_a", "c8", "hn_b"])
        self.assertEqual(sum(len(case["pdf_pages"]) for case in oracle["cases"]), 77)
        self.assertEqual(sum(len(page["draws"]) for case in oracle["cases"]
                             for page in case["pdf_pages"]), 127)
        forbidden = {"raw_text", "text_bytes", "source_bytes", "pdf_bytes",
                     "bitmap", "pixels", "stream_bytes", "absolute_path",
                     "raw_10", "raw_12", "raw_16", "gap_start", "gap_length"}

        def visit(value: object) -> None:
            if isinstance(value, dict):
                self.assertTrue(forbidden.isdisjoint(value))
                for child in value.values():
                    visit(child)
            elif isinstance(value, list):
                for child in value:
                    visit(child)
            elif isinstance(value, str):
                self.assertNotIn("/tmp/", value)

        visit(oracle)

    def test_geometry_rejects_corrupted_first_box_and_extra_image_scale(self) -> None:
        cases = self.oracle()["cases"]
        measured = layout._geometry(cases)
        self.assertEqual(measured["fractional_additional_positions"], 50)
        self.assertEqual(measured["additional_placement_rule"], "UNKNOWN")
        bad_box = deepcopy(cases)
        bad_box[0]["pdf_pages"][0]["media_box"][2] += 1
        with self.assertRaisesRegex(layout.LayoutParityError, "300-ppi"):
            layout._geometry(bad_box)
        bad_draw = deepcopy(cases)
        bad_draw[1]["pdf_pages"][0]["draws"][1]["pdf_ctm"][0] += 1
        with self.assertRaisesRegex(layout.LayoutParityError, "300-ppi"):
            layout._geometry(bad_draw)

    def test_committed_matrix_and_case_order_are_pinned(self) -> None:
        rows, oracle = layout._baseline(layout.LAYOUT_ORACLE, candidate=False)
        self.assertEqual(len(rows), 27)
        self.assertIsNotNone(oracle)
        with tempfile.TemporaryDirectory() as name:
            changed = deepcopy(oracle)
            changed["cases"].reverse()
            path = Path(name) / "changed.json"
            path.write_text(json.dumps(changed), encoding="utf-8")
            with self.assertRaisesRegex(layout.LayoutParityError, "case order"):
                layout._baseline(path, candidate=False)

    def test_hashing_rejects_a_file_that_exceeds_its_bound(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "large.bin"
            path.write_bytes(b"abcd")
            with self.assertRaisesRegex(layout.LayoutParityError, "bound"):
                layout._sha256_file(path, 3)

    def test_candidate_output_never_replaces_inputs_or_existing_files(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            corpus = directory / "corpus"
            corpus.mkdir()
            source = corpus / "document.caj"
            source.write_bytes(b"original source")
            pdf = directory / "reference.pdf"
            pdf.write_bytes(b"original PDF")
            with self.assertRaisesRegex(layout.LayoutParityError, "outside the repository and corpus"):
                layout._write_candidate({}, source, corpus)
            with self.assertRaisesRegex(layout.LayoutParityError, "new file"):
                layout._write_candidate({}, pdf, corpus)
            self.assertEqual(source.read_bytes(), b"original source")
            self.assertEqual(pdf.read_bytes(), b"original PDF")
            candidate = directory / "candidate.json"
            layout._write_candidate({"schema_version": 1}, candidate, corpus)
            self.assertEqual(json.loads(candidate.read_text()), {"schema_version": 1})
            with self.assertRaisesRegex(layout.LayoutParityError, "new file"):
                layout._write_candidate({}, candidate, corpus)


if __name__ == "__main__":
    unittest.main()
