# SPDX-License-Identifier: MIT
"""Evidence-state tests for strict and opt-in #87/#88 pixel comparisons."""

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
import jbig2_text_region_parity as parity  # noqa: E402


def selected() -> list[dict]:
    cases = [{
        "coordinate": ("pinned.caj", number, 1),
        "classification": parity.oracle.STANDARD,
        "instances": 2, "flags_offset": 1234,
        "width": 8, "height": 2,
        "pixel_sha256": "a" * 64, "black_pixels": 1,
    } for number in range(1, parity.EXPECTED)]
    cases.append({
        "coordinate": ("anomaly.caj", 11, 1),
        "classification": parity.oracle.ANOMALY,
        "instances": 1, "flags_offset": 1234,
        "width": 8, "height": 2,
        "pixel_sha256": "b" * 64, "black_pixels": 1,
    })
    return cases


def output_lines() -> list[str]:
    lines = [
        f"CASE\tpinned.caj\t{number}\t1\tCOMPLETE\t2\t2\t2\t1\t{'a' * 64}"
        "\t2\t2\t4096\t-\t24\tComplete"
        for number in range(1, parity.EXPECTED)
    ]
    lines.append("CASE\tanomaly.caj\t11\t1\tHEADER_REFUSED\t0\t0\t0\t0\t"
                 + parity.EMPTY_SHA256 + "\t0\t0\t0\tmalformed_text_header\t1234\tHeader")
    return lines


def opt_in_output_lines() -> list[str]:
    lines = [line + "\t-" for line in output_lines()[:-1]]
    lines.append("CASE\tanomaly.caj\t11\t1\tCOMPLETE\t1\t2\t2\t1\t"
                 + "b" * 64 + "\t2\t2\t4096\t-\t0\tComplete\t"
                 + parity.ANOMALY_MARKER)
    return lines


class TextRegionParityTests(unittest.TestCase):
    def test_clean_clone_reports_not_run_without_claiming_parity(self) -> None:
        output = StringIO()
        with mock.patch.dict(os.environ, {
            "CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": "",
        }), contextlib.redirect_stdout(output):
            self.assertEqual(parity.main(["--json"]), 0)
        report = json.loads(output.getvalue())
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["compatibility"]["matching"], 0)
        self.assertEqual(report["compatibility"]["attempted"], 0)
        self.assertEqual(report["opt_in_anomaly"]["attempted"], 0)
        self.assertEqual(report["source_hashes_before"], 0)

        output = StringIO()
        with mock.patch.dict(os.environ, {
            "CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": "",
        }), contextlib.redirect_stdout(output):
            self.assertEqual(parity.main([
                "--text-header-policy", parity.HN_C8_POLICY, "--json",
            ]), 0)
        report = json.loads(output.getvalue())
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["text_header_policy"], parity.HN_C8_POLICY)
        self.assertEqual(report["opt_in_anomaly"]["attempted"], 0)

    def test_complete_comparison_counts_exact_matches_and_anomaly(self) -> None:
        report, strict, anomaly, resources = parity.parse_output(
            "\n".join([*output_lines(), "TOTAL\t546"]), selected())
        self.assertEqual(report["status"], "PASS")
        self.assertEqual((report["attempted"], report["standard_attempted"]), (546, 545))
        self.assertEqual((report["completed"], report["matching"],
                          report["failing"], report["skipped"]), (545, 545, 0, 0))
        self.assertEqual(strict, 1)
        self.assertEqual((anomaly["status"], anomaly["attempted"]), ("NOT_RUN", 0))
        self.assertEqual(len(report["cases"]), 546)
        self.assertEqual(resources["peak_resident_bytes"], 4096)
        self.assertEqual(resources["max_request_bytes"], 2)
        self.assertEqual(resources["max_temporary_bytes"], 2)
        self.assertTrue(resources["large_page"]["pixel_match"])

    def test_pixel_mismatch_and_typed_refusal_are_distinct_failures(self) -> None:
        lines = output_lines()
        lines[0] = lines[0].replace("\t" + "a" * 64 + "\t", "\t" + "c" * 64 + "\t")
        report, strict, _, _ = parity.parse_output("\n".join([*lines, "TOTAL\t546"]), selected())
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual((report["completed"], report["matching"], report["failing"]),
                         (545, 544, 1))
        self.assertEqual(strict, 1)
        self.assertEqual(report["first_pixel_mismatch"]["coordinate"], ("pinned.caj", 1, 1))
        self.assertIsNone(report["first_standard_refusal"])

        lines = output_lines()
        lines[1] = ("CASE\tpinned.caj\t2\t1\tREFUSED\t0\t0\t0\t0\t"
                    + parity.EMPTY_SHA256 + "\t0\t0\t4096\tmq_invalid_marker\t24\tTerminal")
        report, strict, _, _ = parity.parse_output("\n".join([*lines, "TOTAL\t546"]), selected())
        self.assertEqual((report["completed"], report["matching"], report["failing"]),
                         (544, 544, 1))
        self.assertEqual(report["first_standard_refusal"]["kind"], "mq_invalid_marker")
        self.assertEqual(report["first_typed_refusal"]["stage"], "Terminal")
        self.assertEqual(strict, 1)

    def test_malformed_or_incomplete_case_lines_fail(self) -> None:
        lines = output_lines()
        with self.assertRaises(parity.ParityError):
            parity.parse_output("\n".join([*lines[:-1], "TOTAL\t546"]), selected())
        bad = lines.copy()
        bad[0] = bad[0].replace("\t2\t2\t2\t1\t", "\t2\t2\t3\t1\t")
        with self.assertRaises(parity.ParityError):
            parity.parse_output("\n".join([*bad, "TOTAL\t546"]), selected())
        bad = lines.copy()
        bad[-1] = bad[-1].replace("\t1234\tHeader", "\t0\tHeader")
        with self.assertRaises(parity.ParityError):
            parity.parse_output("\n".join([*bad, "TOTAL\t546"]), selected())

    def test_explicit_missing_or_changed_inputs_fail(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            root = Path(temporary)
            fixture = root / "table"
            fixture.write_bytes(b"not the pinned table")
            changed = root / "changed.json"
            original = parity.oracle.DEFAULT_MANIFEST.read_bytes()
            altered = original.replace(
                b'"normalized_pixel_sha256": "3',
                b'"normalized_pixel_sha256": "4', 1)
            self.assertNotEqual(altered, original)
            changed.write_bytes(altered)
            variants = [
                ["--corpus-dir", str(root)],
                ["--table-fixture", str(fixture)],
                ["--corpus-dir", str(root), "--table-fixture", str(fixture)],
                ["--manifest", str(root / "missing.json")],
                ["--manifest", str(changed)],
            ]
            for arguments in variants:
                with self.subTest(arguments=arguments):
                    output = StringIO()
                    with mock.patch.dict(os.environ, {
                        "CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": "",
                    }), contextlib.redirect_stdout(output):
                        self.assertEqual(parity.main([*arguments, "--json"]), 1)
                    self.assertEqual(json.loads(output.getvalue())["status"], "FAIL")

            output = StringIO()
            with (mock.patch.dict(os.environ, {
                    "CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": "",
                  }),
                  mock.patch.object(parity.instances.parity, "check_table", return_value=fixture),
                  contextlib.redirect_stdout(output)):
                self.assertEqual(parity.main([
                    "--corpus-dir", str(root / "absent"),
                    "--table-fixture", str(fixture), "--json",
                ]), 1)
            report = json.loads(output.getvalue())
            self.assertEqual(report["status"], "FAIL")
            self.assertEqual(report["phase"], "source_precheck")

    def test_strict_anomaly_cannot_be_counted_as_pixel_match(self) -> None:
        lines = output_lines()
        lines[-1] = ("CASE\tanomaly.caj\t11\t1\tCOMPLETE\t1\t2\t2\t1\t"
                     + "b" * 64 + "\t2\t2\t4096\t-\t0\tComplete")
        with self.assertRaises(parity.ParityError):
            parity.parse_output("\n".join([*lines, "TOTAL\t546"]), selected())

    def test_opt_in_match_is_reported_separately_from_545_standard_cases(self) -> None:
        standard, strict, anomaly, resources = parity.parse_output(
            "\n".join([*opt_in_output_lines(), "TOTAL\t546"]), selected(),
            parity.HN_C8_POLICY)
        self.assertEqual(standard["status"], "PASS")
        self.assertEqual((standard["matching"], standard["completed"]), (545, 545))
        self.assertEqual(strict, 0)
        self.assertEqual((anomaly["status"], anomaly["attempted"],
                          anomaly["matching"], anomaly["failing"]), ("PASS", 1, 1, 0))
        self.assertEqual(anomaly["case"]["anomaly_marker"], parity.ANOMALY_MARKER)
        self.assertTrue(anomaly["case"]["pixel_match"])
        self.assertEqual(resources["max_temporary_bytes"], 2)

    def test_opt_in_anomaly_mismatch_and_refusal_fail_without_changing_standard_count(self) -> None:
        lines = opt_in_output_lines()
        lines[-1] = lines[-1].replace("\t" + "b" * 64 + "\t",
                                    "\t" + "c" * 64 + "\t")
        standard, _, anomaly, _ = parity.parse_output(
            "\n".join([*lines, "TOTAL\t546"]), selected(), parity.HN_C8_POLICY)
        self.assertEqual((standard["matching"], standard["status"]), (545, "FAIL"))
        self.assertEqual((anomaly["matching"], anomaly["failing"]), (0, 1))
        self.assertIsNotNone(anomaly["first_pixel_mismatch"])

        lines = opt_in_output_lines()
        lines[-1] = ("CASE\tanomaly.caj\t11\t1\tREFUSED\t0\t0\t0\t0\t"
                     + parity.EMPTY_SHA256 + "\t0\t0\t4096\tmq_invalid_marker\t1234\tTerminal\t"
                     + parity.ANOMALY_MARKER)
        standard, _, anomaly, _ = parity.parse_output(
            "\n".join([*lines, "TOTAL\t546"]), selected(), parity.HN_C8_POLICY)
        self.assertEqual((standard["matching"], standard["status"]), (545, "FAIL"))
        self.assertEqual(anomaly["first_typed_refusal"]["source_byte_offset"], 1234)
        self.assertEqual(anomaly["first_typed_refusal"]["stage"], "Terminal")

    def test_opt_in_protocol_requires_a_marker_and_explicit_policy(self) -> None:
        lines = opt_in_output_lines()
        lines[-1] = lines[-1].replace(parity.ANOMALY_MARKER, "-")
        with self.assertRaises(parity.ParityError):
            parity.parse_output("\n".join([*lines, "TOTAL\t546"]), selected(),
                                parity.HN_C8_POLICY)
        with self.assertRaises(parity.ParityError):
            parity.parse_output("\n".join([*output_lines(), "TOTAL\t546"]),
                                selected(), "unknown")

        with mock.patch.object(parity.instances, "bounded_decode",
                               return_value="\n".join([*opt_in_output_lines(), "TOTAL\t546"])) as decode:
            with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
                root = Path(temporary)
                parity.run_binary(root / "binary", root / "private-table", ["PLAN"],
                                  selected(), parity.HN_C8_POLICY)
            self.assertEqual(decode.call_args.args[0][-1], parity.HN_C8_POLICY)


if __name__ == "__main__":
    unittest.main()
