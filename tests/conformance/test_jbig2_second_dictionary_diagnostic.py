# SPDX-License-Identifier: MIT
"""Evidence-state and trace-accounting tests for optional #2 diagnostics."""

from __future__ import annotations

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
import jbig2_second_dictionary_diagnostic as diagnostic  # noqa: E402


class SecondDictionaryDiagnosticTests(unittest.TestCase):
    def test_clean_clone_and_ci_empty_environment_are_not_compatibility(self) -> None:
        output = StringIO()
        with mock.patch.dict(os.environ, {
            "CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": "",
        }), contextlib.redirect_stdout(output):
            self.assertEqual(diagnostic.main(["--json"]), 0)
        result = json.loads(output.getvalue())
        self.assertEqual(result["status"], "NOT_RUN")
        self.assertEqual(result["diagnostic"]["attempted_cases"], 0)
        self.assertEqual(result["symbol_compatibility"]["status"], "NOT_RUN")
        self.assertEqual(result["symbol_compatibility"]["passed"], 0)

    def test_explicit_incomplete_inputs_fail(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            corpus = Path(temporary)
            for path, fixture in ((corpus, None), (None, corpus / "missing.table")):
                output = StringIO()
                args = ["--json"]
                if path is not None:
                    args.extend(("--corpus-dir", str(path)))
                if fixture is not None:
                    args.extend(("--table-fixture", str(fixture)))
                with mock.patch.dict(os.environ, {
                    "CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": "",
                }), contextlib.redirect_stdout(output):
                    self.assertEqual(diagnostic.main(args), 1)
                report = json.loads(output.getvalue())
                self.assertEqual(report["status"], "FAIL")
                self.assertEqual(report["symbol_compatibility"]["passed"], 0)

    def test_explicit_wrong_table_fails_before_corpus_read(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            fixture = Path(temporary) / "table"
            fixture.write_bytes(b"incorrect private table")
            report = diagnostic.initial_report()
            with self.assertRaises(diagnostic.parity.ParityError):
                diagnostic.run(Path(temporary), fixture, report=report)
            self.assertEqual(report["source_hashes_before"], 0)

    def test_only_complete_trace_lines_count(self) -> None:
        selected = [("pinned.caj", page, 1) for page in range(1, diagnostic.EXPECTED + 1)]
        lines = []
        for index, key in enumerate(selected):
            status = "REFUSED" if index == 2 else "COMPLETE"
            refusal = "iaai_multiple" if index == 2 else "-"
            lines.append(f"CASE\t{key[0]}\t{key[1]}\t{key[2]}\t{status}"
                         f"\t{int(index == 1)}\t0\t{int(index == 2)}\t{refusal}\t23")
        result = diagnostic.parse_output("\n".join(lines + ["TOTAL\t546"]), selected)
        self.assertEqual(result["attempted_cases"], 546)
        self.assertEqual(result["completed_cases"], 545)
        self.assertEqual(result["refused_cases"], 1)
        self.assertEqual((result["iaai_one"], result["iaai_multiple"]), (1, 1))
        self.assertEqual(result["first_typed_refusal"], {
            "coordinate": selected[2], "kind": "iaai_multiple",
            "source_byte_offset": 23,
        })
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.parse_output("\n".join(lines[:-1] + ["TOTAL\t546"]), selected)
        lines[2] = lines[2].replace("REFUSED", "COMPLETE")
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.parse_output("\n".join(lines + ["TOTAL\t546"]), selected)
        lines[2] = lines[2].replace("COMPLETE", "REFUSED").replace(
            "\t0\t0\t1\t", "\t0\t0\t0\t")
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.parse_output("\n".join(lines + ["TOTAL\t546"]), selected)

    def test_plan_rejects_unpinned_inventory(self) -> None:
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.plan_for_cases([], {"samples": []})


if __name__ == "__main__":
    unittest.main()
