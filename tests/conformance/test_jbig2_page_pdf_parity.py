# SPDX-License-Identifier: MIT
"""Evidence-state controls for the optional selected type-3 PDF harness."""

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
import jbig2_page_pdf_parity as parity  # noqa: E402


def invoke(*arguments: str) -> tuple[int, dict]:
    output = StringIO()
    with (mock.patch.dict(os.environ, {
            "CAJ2PDF_CORPUS_DIR": "", "CAJ2PDF_T88_H2_FIXTURE_FILE": ""}),
          contextlib.redirect_stdout(output)):
        code = parity.main([*arguments, "--json"])
    return code, json.loads(output.getvalue())


def synthetic_case(key: tuple[str, int, int]) -> tuple[dict, dict]:
    case = {"coordinate": key, "page": key[1], "image": key[2],
            "variant": "HN", "offset": 100, "length": 1000}
    expected = {"width": 13, "height": 7, "flags_offset": 314,
                "pixel_sha256": "a" * 64, "black_pixels": 3}
    return case, expected


def complete_line(case: dict, *, marker: str = "-") -> str:
    return "\t".join(map(str, (
        "CASE", "COMPLETE", case["page"], case["image"], "HN-A",
        13, 7, 100, 1000, 12000, 270, 65536, 300000, 4321,
        marker, "-", 0,
    )))


class Type3PdfParityTests(unittest.TestCase):
    def test_clean_clone_reports_zero_even_with_explicit_opt_in(self) -> None:
        for policy in (parity.STRICT, parity.OPT_IN):
            with (self.subTest(policy=policy),
                  mock.patch.object(parity, "_tools", side_effect=AssertionError("tools opened")),
                  mock.patch.object(parity, "_build_binary", side_effect=AssertionError("built")),
                  mock.patch.object(parity.oracle, "validate_sources",
                                    side_effect=AssertionError("private source opened"))):
                code, report = invoke("--text-header-policy", policy)
            self.assertEqual((code, report["status"], report["matching"]),
                             (0, "NOT_RUN", 0))
            self.assertEqual((report["attempted"], report["completed"],
                              report["strict_anomaly_refusals"], report["failed"]),
                             (0, 0, 0, 0))
            self.assertEqual(report["source_hashes_before"], {})
            self.assertEqual(report["rendered"], {})

    def test_missing_or_altered_private_inputs_fail(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as folder:
            bad = Path(folder) / "bad-table"
            bad.write_bytes(b"no probability states")
            for arguments in (
                ("--corpus-dir", folder),
                ("--table-fixture", str(bad)),
                ("--corpus-dir", folder, "--table-fixture", str(bad)),
            ):
                with self.subTest(arguments=arguments):
                    code, report = invoke(*arguments)
                    self.assertEqual((code, report["status"], report["matching"]),
                                     (1, "FAIL", 0))
                    self.assertGreater(report["failed"], 0)

    def test_fixed_renderer_canaries_are_distinct_and_nonblank(self) -> None:
        manifest = parity.page.load_manifest()
        baseline = parity.page._oracle_images(manifest)
        keys = list(parity.RENDER_COORDINATES.values())
        self.assertEqual(len(set(keys)), 4)
        self.assertGreater(parity.RENDER_COORDINATES["multi"][2], 1)
        self.assertEqual(parity.RENDER_COORDINATES["anomaly"], parity.ANOMALY)
        for key in keys:
            self.assertIn(key, baseline)
            self.assertGreater(baseline[key]["black_pixels"], 0)

    def test_only_pinned_strict_anomaly_can_be_a_refusal(self) -> None:
        anomaly, expected = synthetic_case(parity.ANOMALY)
        line = "\t".join(map(str, (
            "CASE", "REFUSED", anomaly["page"], anomaly["image"], "-",
            0, 0, 0, 0, 100, 0, 64, 0, 2000, "-",
            "malformed_text_header", expected["flags_offset"],
        )))
        result = parity._parse_case(line, anomaly, expected, parity.STRICT)
        self.assertEqual(result["status"], "REFUSED")
        with self.assertRaises(parity.PdfParityError):
            parity._parse_case(line, anomaly, expected, parity.OPT_IN)
        ordinary, expected = synthetic_case(("synthetic/ordinary.caj", 2, 1))
        with self.assertRaises(parity.PdfParityError):
            parity._parse_case(line, ordinary, expected, parity.STRICT)

    def test_success_protocol_checks_selection_variant_and_anomaly_marker(self) -> None:
        ordinary, expected = synthetic_case(("synthetic/ordinary.caj", 2, 1))
        parsed = parity._parse_case(complete_line(ordinary), ordinary, expected, parity.STRICT)
        self.assertEqual((parsed["width"], parsed["height"], parsed["pdf_bytes"]),
                         (13, 7, 270))
        anomaly, expected = synthetic_case(parity.ANOMALY)
        with self.assertRaises(parity.PdfParityError):
            parity._parse_case(complete_line(anomaly), anomaly, expected, parity.OPT_IN)
        parsed = parity._parse_case(
            complete_line(anomaly, marker=parity.page.ANOMALY_MARKER),
            anomaly, expected, parity.OPT_IN)
        self.assertEqual(parsed["status"], "COMPLETE")

    def test_renderer_rejects_short_and_extra_row_streams(self) -> None:
        want = [b"\x80", b"\x80"]
        centre = [b"\x04" + b"\x00" * 9] * 20
        expected = {"width": 8, "height": 2, "black_pixels": 2}
        with tempfile.TemporaryDirectory(dir="/tmp") as folder:
            root = Path(folder)
            reference = root / "reference.pbm"

            def check(normal: list[bytes]) -> None:
                def rows(path: Path, _width: int, _height: int):
                    selected = (want if path == reference else
                                centre if path.name == "poppler.pbm" else normal)
                    return (row for row in selected)

                with (mock.patch.object(parity.oracle, "pbm_rows", side_effect=rows),
                      mock.patch.object(parity, "_tool_output", return_value=""),
                      mock.patch.object(parity, "_file_size", return_value=0)):
                    parity._render(root / "selected.pdf", reference, root,
                                   expected, {"pdftoppm": "pdftoppm", "mutool": "mutool"})

            check(want)
            for rows in (want[:1], [*want, b"\x00"]):
                with self.subTest(rows=len(rows)), self.assertRaises(parity.PdfParityError):
                    check(rows)

    def test_child_tmpdir_cleans_a_leaked_scratch_file(self) -> None:
        with tempfile.TemporaryDirectory(dir="/tmp") as parent:
            with tempfile.TemporaryDirectory(dir=parent) as child:
                leak = Path(child) / "leaked-scratch.bin"
                parity._tool_output(
                    [sys.executable, "-c",
                     "import os; from pathlib import Path; "
                     "Path(os.environ['TMPDIR'], 'leaked-scratch.bin').write_bytes(b'x')"],
                    "synthetic child scratch", child_env={**os.environ, "TMPDIR": child},
                )
                self.assertTrue(leak.is_file())
            self.assertFalse(leak.exists())

    def test_child_output_is_bounded_while_running(self) -> None:
        with self.assertRaisesRegex(parity.PdfParityError, "exceeds 32 KiB"):
            parity._tool_output(
                [sys.executable, "-c", "print('x' * 40000)"],
                "synthetic overlong child", timeout=2,
            )


if __name__ == "__main__":
    unittest.main()
