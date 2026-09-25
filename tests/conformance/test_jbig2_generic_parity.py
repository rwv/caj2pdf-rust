# SPDX-License-Identifier: MIT
"""Synthetic failure and clean-clone checks for optional Rust generic parity."""

from __future__ import annotations

from io import StringIO
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import jbig2_generic_parity as parity  # noqa: E402
from test_jbig2_generic_oracle import synthetic_case, synthetic_manifest  # noqa: E402


class GenericParityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.case = synthetic_case(self.root / "sample.caj")
        self.manifest = synthetic_manifest(self.case)

    def plan(self) -> tuple[list[str], list[dict]]:
        validate = parity.generic.validate_manifest
        with patch.object(parity, "EXPECTED", 1), patch.object(
            parity.generic, "validate_manifest",
            side_effect=lambda manifest: validate(manifest, expected_images=1),
        ):
            return parity.plan_for_cases([self.case], self.manifest)

    def test_clean_clone_reports_not_run_and_zero_matches(self) -> None:
        output = StringIO()
        with patch.dict(os.environ, {"CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": ""}), patch(
            "sys.stdout", output
        ):
            self.assertEqual(parity.main(["--json"]), 0)
        report = json.loads(output.getvalue())
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["rust_matches"], 0)
        self.assertEqual(report["tool_agreements"], 0)

    def test_explicit_missing_private_table_fails(self) -> None:
        output = StringIO()
        with patch("sys.stdout", output):
            self.assertEqual(parity.main([
                "--corpus-dir", str(self.root), "--table-fixture", str(self.root / "missing"),
                "--json",
            ]), 1)
        report = json.loads(output.getvalue())
        self.assertEqual((report["status"], report["rust_matches"]), ("FAIL", 0))
        self.assertEqual((report["phase"], report["rust_failures"], report["tool_agreements"]),
                         ("preflight", 0, 0))
        self.assertIn("table fixture", report["error"])

    def test_plan_pins_exact_segments_and_rejects_drift(self) -> None:
        lines, selected = self.plan()
        self.assertEqual(len(lines), 1)
        self.assertEqual(len(lines[0].split("\t")), 14)
        self.assertEqual(selected[0]["key"], self.case["coordinate"])
        self.assertEqual(selected[0]["generic_length"], self.manifest["samples"][0]["images"][0]["segment_4"]["length"])

        self.manifest["samples"][0]["images"][0]["segment_4"]["encoded_sha256"] = "b" * 64
        with self.assertRaisesRegex(parity.ParityError, "segment 4.*SHA differs"):
            self.plan()

        self.manifest = synthetic_manifest(self.case)
        self.case["segments"][4]["data_length"] -= 1
        with self.assertRaisesRegex(parity.generic.OracleError, "does not end at the image boundary"):
            self.plan()

    def test_plan_rejects_profile_and_source_identity_drift(self) -> None:
        self.manifest["samples"][0]["source_sha256"] = "b" * 64
        with self.assertRaisesRegex(parity.ParityError, "source SHA differs"):
            self.plan()
        self.manifest = synthetic_manifest(self.case)
        self.manifest["samples"][0]["images"][0]["generic_profile"]["adaptive_pixel"] = [1, -1]
        with self.assertRaisesRegex(parity.generic.OracleError, "profile differs"):
            self.plan()

    def test_decoder_digest_mismatch_fails_with_coordinate(self) -> None:
        _, selected = self.plan()
        case = self.case
        result = "\t".join(map(str, [
            "CASE", case["id"], 1, 1, 9, 2, "b" * 64, 0,
            2, 2, 18, 3, 10, 2, 2, 8, 1,
        ])) + "\nTOTAL\t1\t1\t1000\n"
        with patch.object(parity, "EXPECTED", 1):
            with self.assertRaisesRegex(parity.ParityError, "pixel SHA.*differs"):
                parity.parse_rust_output(result, selected)

    def test_oracle_not_run_or_warning_cannot_count_as_agreement(self) -> None:
        tools = {name: Path(name) for name in ("qpdf", "pdfimages", "mutool")}
        for report in (
            {"status": "NOT_RUN", "tool_agreements": 0},
            {"status": "FAIL", "tool_agreements": 0, "failures": ["qpdf warned"]},
        ):
            with self.subTest(report=report), patch.object(
                parity, "bounded_run",
                return_value=subprocess.CompletedProcess([], 0, json.dumps(report), ""),
            ):
                with self.assertRaisesRegex(parity.ParityError, "black-box oracle failed"):
                    parity.oracle_run(self.root, tools)

    def test_bounded_process_output_is_rejected_before_loading(self) -> None:
        temporary_directory = tempfile.TemporaryDirectory
        with patch.object(parity, "MAX_RUN_OUTPUT_BYTES", 32), patch.object(
            parity.tempfile, "TemporaryDirectory",
            side_effect=lambda **_kwargs: temporary_directory(dir=self.root, prefix="outputs-"),
        ):
            with self.assertRaisesRegex(parity.ParityError, "stdout exceeds 2 MiB"):
                parity.bounded_run([
                    sys.executable, "-c", "import sys; sys.stdout.write('x' * 33)"
                ], "synthetic child", 10)
        self.assertEqual(list(self.root.glob("outputs-*")), [])

    def test_private_plan_is_removed_after_rust_failure(self) -> None:
        seen = []

        def fail(command: list[str], *_args, **_kwargs):
            plan = Path(command[2])
            seen.append(plan)
            self.assertTrue(plan.exists())
            return subprocess.CompletedProcess(command, 1, "", "synthetic decoder failure")

        with patch.object(parity, "bounded_run", side_effect=fail):
            with self.assertRaisesRegex(parity.ParityError, "synthetic decoder failure"):
                parity.run_rust(["synthetic plan"], [], self.root / "table", self.root / "binary")
        self.assertEqual(len(seen), 1)
        self.assertFalse(seen[0].exists())
        self.assertFalse(seen[0].parent.exists())

    def test_pre_rust_oracle_failure_has_no_rust_failure_or_tool_agreement(self) -> None:
        rows = [{} for _ in range(27)]
        output = StringIO()
        with patch.object(parity.generic, "validate_manifest"), patch.object(
            parity, "check_table", return_value=self.root / "private.fixture"
        ), patch.object(parity.full, "validate_sources", return_value=(rows, {})), patch.object(
            parity.full, "tool_path", side_effect=Path
        ), patch.object(parity, "oracle_run", side_effect=parity.ParityError("qpdf warned")), patch.object(
            parity, "run_rust"
        ) as rust, patch("sys.stdout", output):
            self.assertEqual(parity.main([
                "--corpus-dir", str(self.root), "--table-fixture", str(self.root / "private.fixture"),
                "--json",
            ]), 1)
        report = json.loads(output.getvalue())
        self.assertEqual((report["phase"], report["rust_failures"], report["tool_agreements"]),
                         ("black_box_oracle", 0, 0))
        rust.assert_not_called()

    def test_source_hashes_are_postchecked_after_decoder_failure(self) -> None:
        rows = [{} for _ in range(27)]
        output = StringIO()
        with patch.object(parity.generic, "validate_manifest"), patch.object(
            parity, "check_table", return_value=self.root / "private.fixture"
        ), patch.object(parity.full, "validate_sources", return_value=(rows, {})) as sources, patch.object(
            parity.full, "tool_path", side_effect=Path
        ), patch.object(parity, "oracle_run", return_value={
            "tool_agreements": 546, "source_hashes_checked": 27,
        }), patch.object(parity.full, "run_directory_inventory", return_value={}), patch.object(
            parity.full, "checked_cases", return_value=[]
        ), patch.object(parity, "plan_for_cases", return_value=(["row"], [])), patch.object(
            parity, "rust_binary", return_value=self.root / "binary"
        ), patch.object(parity, "run_rust", side_effect=parity.ParityError("synthetic decoder mismatch")), patch(
            "sys.stdout", output
        ):
            self.assertEqual(parity.main([
                "--corpus-dir", str(self.root), "--table-fixture", str(self.root / "private.fixture"),
                "--json",
            ]), 1)
        report = json.loads(output.getvalue())
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual((report["phase"], report["rust_failures"], report["tool_agreements"]),
                         ("rust_decode", 1, 546))
        self.assertEqual((report["source_hashes_checked_before"], report["source_hashes_checked_after"]),
                         (27, 27))
        self.assertEqual(sources.call_count, 2)


if __name__ == "__main__":
    unittest.main()
