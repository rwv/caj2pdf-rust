# SPDX-License-Identifier: MIT
"""Evidence-state and complete-trace tests for the optional #86 diagnostic."""

from __future__ import annotations

import contextlib
import hashlib
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
import jbig2_text_instance_diagnostic as diagnostic  # noqa: E402


def selected() -> list[tuple[tuple, str, int, int]]:
    entries = [(("pinned.caj", number, 1), diagnostic.text_oracle.STANDARD, 2, 1234)
               for number in range(1, diagnostic.EXPECTED)]
    entries.append((("anomaly.caj", 11, 1), diagnostic.text_oracle.ANOMALY, 1, 1234))
    return entries


def output_lines() -> list[str]:
    lines = [f"CASE\tpinned.caj\t{number}\t1\tCOMPLETE\t2\t1\t1\t1\t-\t22\tComplete\t{'a' * 64}"
             for number in range(1, diagnostic.EXPECTED)]
    lines.append("CASE\tanomaly.caj\t11\t1\tHEADER_REFUSED\t0\t0\t0\t0"
                 "\tmalformed_text_header\t1234\tHeader\t" + hashlib.sha256(b"").hexdigest())
    return lines


class TextInstanceDiagnosticTests(unittest.TestCase):
    def test_clean_clone_is_not_parity(self) -> None:
        output = StringIO()
        with mock.patch.dict(os.environ, {
            "CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": "",
        }), contextlib.redirect_stdout(output):
            self.assertEqual(diagnostic.main(["--json"]), 0)
        report = json.loads(output.getvalue())
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["diagnostic"]["attempted_cases"], 0)
        self.assertEqual(report["placement_compatibility"]["passed"], 0)
        self.assertEqual(report["pixel_compatibility"]["passed"], 0)

    def test_explicit_absent_or_changed_inputs_fail(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            root = Path(temporary)
            fixture = root / "table"
            fixture.write_bytes(b"wrong table")
            changed_manifest = root / "changed.json"
            changed_manifest.write_bytes(
                diagnostic.text_oracle.DEFAULT_MANIFEST.read_bytes().replace(
                    b'"normalized_pixel_sha256": "3',
                    b'"normalized_pixel_sha256": "4', 1))
            diagnostic.text_oracle.validate_manifest(json.loads(
                changed_manifest.read_text(encoding="utf-8")))
            variants = [
                ["--corpus-dir", str(root)],
                ["--table-fixture", str(fixture)],
                ["--corpus-dir", str(root), "--table-fixture", str(fixture)],
                ["--manifest", str(root / "absent.json")],
                ["--manifest", str(changed_manifest)],
            ]
            for arguments in variants:
                with self.subTest(arguments=arguments):
                    output = StringIO()
                    with mock.patch.dict(os.environ, {
                        "CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": "",
                    }), contextlib.redirect_stdout(output):
                        self.assertEqual(diagnostic.main([*arguments, "--json"]), 1)
                    report = json.loads(output.getvalue())
                    self.assertEqual(report["status"], "FAIL")
                    self.assertEqual(report["pixel_compatibility"]["passed"], 0)

    def test_only_complete_instances_count(self) -> None:
        entries = selected()
        lines = output_lines()
        report = diagnostic.parse_output("\n".join([*lines, "TOTAL\t546"]), entries)
        self.assertEqual(report["attempted_cases"], 546)
        self.assertEqual(report["standard_valid_attempted"], 545)
        self.assertEqual(report["complete_regions"], 545)
        self.assertEqual(report["strict_header_refusals"], 1)
        self.assertEqual(report["completed_instances"], 1090)
        self.assertEqual((report["ri_zero"], report["ri_one"]), (545, 545))
        self.assertEqual(report["first_typed_refusal"]["kind"], "malformed_text_header")
        self.assertEqual(report["first_typed_refusal"]["semantic_decision"], "Header")
        self.assertEqual(report["first_typed_refusal"]["source_byte_offset"], 1234)
        self.assertIsNone(report["first_standard_refusal"])
        self.assertEqual(len(report["region_traces"]), 546)

        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.parse_output("\n".join([*lines[:-1], "TOTAL\t546"]), entries)
        bad = lines.copy()
        bad[0] = bad[0].replace("\t2\t1\t1\t1\t", "\t2\t2\t1\t1\t")
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.parse_output("\n".join([*bad, "TOTAL\t546"]), entries)
        bad = lines.copy()
        bad[-1] = bad[-1].replace("HEADER_REFUSED", "COMPLETE")
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.parse_output("\n".join([*bad, "TOTAL\t546"]), entries)
        bad = lines.copy()
        bad[-1] = bad[-1].replace("\t1234\tHeader", "\t0\tHeader")
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.parse_output("\n".join([*bad, "TOTAL\t546"]), entries)

    def test_first_standard_refusal_is_distinct_from_strict_anomaly(self) -> None:
        lines = output_lines()
        lines[2] = lines[2].replace("COMPLETE\t2\t1\t1", "REFUSED\t1\t1\t0")
        lines[2] = lines[2].replace("\t-\t22\tComplete", "\tmarker_exhausted\t22\tSymbolId")
        report = diagnostic.parse_output("\n".join([*lines, "TOTAL\t546"]), selected())
        self.assertEqual(report["refused_regions"], 1)
        self.assertEqual(report["status"], "INCOMPLETE")
        self.assertEqual(report["completed_instances"], 1089)
        self.assertEqual(report["first_typed_refusal"]["coordinate"], ("pinned.caj", 3, 1))
        self.assertEqual(report["first_typed_refusal"]["kind"], "marker_exhausted")
        self.assertEqual(report["first_standard_refusal"]["kind"], "marker_exhausted")
        self.assertEqual(report["first_standard_refusal"]["semantic_decision"], "SymbolId")

        lines = output_lines()
        lines[1] = ("CASE\tpinned.caj\t2\t1\tREFUSED\t0\t0\t0\t0"
                    "\theader_malformed_flags\t1234\tHeader\t"
                    + hashlib.sha256(b"").hexdigest())
        report = diagnostic.parse_output("\n".join([*lines, "TOTAL\t546"]), selected())
        self.assertEqual(report["attempted_cases"], 546)
        self.assertEqual(report["first_standard_refusal"]["kind"], "header_malformed_flags")
        self.assertEqual(report["first_standard_refusal"]["source_byte_offset"], 1234)
        self.assertEqual(report["status"], "INCOMPLETE")

    def test_plan_rejects_missing_cases(self) -> None:
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.plan_for_cases([], {}, {}, {})

    def test_decoder_capture_enforces_live_output_bound(self) -> None:
        self.assertEqual(diagnostic.bounded_decode([
            sys.executable, "-c", "print('trace')",
        ]), "trace\n")
        with self.assertRaisesRegex(diagnostic.DiagnosticError, "exceeds 2 MiB"):
            diagnostic.bounded_decode([
                sys.executable, "-c", "import sys; sys.stdout.write('x' * 2097153)",
            ])


if __name__ == "__main__":
    unittest.main()
