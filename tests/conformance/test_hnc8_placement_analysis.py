# SPDX-License-Identifier: MIT
"""Synthetic and committed-metadata tests for frozen placement controls."""

from contextlib import redirect_stdout
import copy
import io
import inspect
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_placement_analysis as analysis  # noqa: E402


class PlacementAnalysisTests(unittest.TestCase):
    def test_default_clean_clone_does_not_read_external_or_oracle_data(self) -> None:
        output = io.StringIO()
        with patch.object(analysis, "_load_oracle", side_effect=AssertionError("oracle read")):
            with redirect_stdout(output):
                self.assertEqual(analysis.main(["--json"]), 0)
        report = json.loads(output.getvalue())
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(set(report["counts"].values()), {0})
        self.assertEqual(report["hypotheses"], {})

    def test_predictor_only_accepts_geometry_inputs_and_uses_full_ctm(self) -> None:
        self.assertEqual(list(inspect.signature(analysis.predict).parameters),
                         ["hypothesis", "media_box", "width", "height"])
        box = [-5.0, -7.0, 95.0, 193.0]
        expected = {
            "top-left": [24.0, 0.0, 0.0, -12.0, -5.0, 193.0],
            "center": [24.0, 0.0, 0.0, -12.0, 33.0, 99.0],
            "bottom-right": [24.0, 0.0, 0.0, -12.0, 71.0, 5.0],
        }
        for name, matrix in expected.items():
            with self.subTest(name=name):
                self.assertEqual(analysis.predict(name, box, 100, 50), matrix)
        with self.assertRaisesRegex(analysis.AnalysisError, "unrecognized"):
            analysis.predict("lookup-source-id", box, 100, 50)
        with self.assertRaisesRegex(analysis.AnalysisError, "dimensions"):
            analysis.predict("top-left", box, 0, 50)
        with self.assertRaisesRegex(analysis.AnalysisError, "MediaBox"):
            analysis.predict("top-left", [0, 0, 0, 100], 100, 50)

    def test_frozen_discovery_and_validation_counts_have_no_private_comparisons(self) -> None:
        expected = {"discovery": (20, 36), "validation": (9, 14)}
        for mode, (pages, draws) in expected.items():
            with self.subTest(mode=mode):
                output = io.StringIO()
                with redirect_stdout(output):
                    self.assertEqual(analysis.main(["--mode", mode, "--json"]), 0)
                report = json.loads(output.getvalue())
                self.assertEqual(report["status"], "UNKNOWN")
                self.assertEqual(report["placement_rule_status"],
                                 "UNKNOWN_GEOMETRY_ONLY_CONTROLS")
                self.assertEqual(report["counts"]["private_comparisons"], 0)
                self.assertEqual(report["counts"]["oracle_pages_compared"], pages)
                self.assertEqual(report["counts"]["oracle_additional_draws_compared"], draws)
                self.assertEqual(report["excluded_candidate_inputs"],
                                 ["source ID", "page number", "image hash", "observed CTM"])
                for hypothesis in analysis.HYPOTHESES:
                    record = report["hypotheses"][hypothesis]
                    self.assertEqual((record["attempted"], record["passing"], record["failing"]),
                                     (draws, 0, draws))
                    self.assertEqual(len(record["counterexamples"]), draws)
                    self.assertEqual(len(record["comparisons"]), draws)
                    self.assertTrue(all(len(row["absolute_errors_pt"]) == 6
                                        for row in record["comparisons"]))
        discovery = {number for case in analysis.PAGE_SPLITS["discovery"].values()
                     for number in case}
        validation = {number for case in analysis.PAGE_SPLITS["validation"].values()
                      for number in case}
        # The two variants may share page numbers; compare within each variant.
        self.assertEqual(discovery & validation, set())

    def test_all_six_ctm_components_and_0_001pt_tolerance_are_applied(self) -> None:
        cases = copy.deepcopy(analysis._load_oracle())
        for case_name, pages in analysis.PAGE_SPLITS["discovery"].items():
            for page_number in pages:
                page = cases[case_name]["pdf_pages"][page_number - 1]
                for draw in page["draws"][1:]:
                    draw["pdf_ctm"] = analysis.predict(
                        "top-left", page["media_box"], draw["width"], draw["height"])
                    draw["pdf_ctm"][1] = analysis.TOLERANCE_PT
        accepted = analysis.evaluate("discovery", cases)
        self.assertEqual(accepted["hypotheses"]["top-left"]["passing"], 36)
        first_page = analysis.PAGE_SPLITS["discovery"]["c8"][0]
        cases["c8"]["pdf_pages"][first_page - 1]["draws"][1]["pdf_ctm"][1] = 0.0011
        rejected = analysis.evaluate("discovery", cases)
        self.assertEqual(rejected["hypotheses"]["top-left"]["passing"], 35)
        counterexamples = rejected["hypotheses"]["top-left"]["counterexamples"]
        self.assertEqual(len(counterexamples), 1)
        self.assertEqual(counterexamples[0]["page_number"], 1)
        self.assertEqual(counterexamples[0]["draw_number"], 2)

    def test_selected_image_identity_and_observed_ctm_are_checked(self) -> None:
        cases = analysis._load_oracle()
        altered = copy.deepcopy(cases)
        altered["c8"]["pdf_pages"][0]["draws"][1]["raw_stream_sha256"] = "0" * 64
        with self.assertRaisesRegex(analysis.AnalysisError, "identity"):
            analysis.evaluate("discovery", altered)
        altered = copy.deepcopy(cases)
        altered["c8"]["pdf_pages"][0]["draws"][1]["pdf_ctm"][3] = float("nan")
        with self.assertRaisesRegex(analysis.AnalysisError, "CTM"):
            analysis.evaluate("discovery", altered)

    def test_modified_oracle_fails_hash_pin_without_private_comparison(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            wrong = Path(directory) / "oracle.json"
            wrong.write_text("{}", encoding="utf-8")
            with patch.object(analysis, "ORACLE", wrong):
                output = io.StringIO()
                with redirect_stdout(output):
                    self.assertEqual(analysis.main(["--mode", "discovery", "--json"]), 1)
            report = json.loads(output.getvalue())
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["counts"]["private_comparisons"], 0)
        self.assertEqual(report["counts"]["oracle_additional_draws_compared"], 0)
        self.assertIn("SHA-256", report["errors"][0])

    def test_oracle_parser_uses_the_exact_bytes_it_hashed(self) -> None:
        pinned = analysis.ORACLE.read_bytes()

        class RacingOracle:
            def __init__(self) -> None:
                self.opens = 0
                self.second_reads = 0

            def open(self, _mode: str) -> io.BytesIO:
                self.opens += 1
                return io.BytesIO(pinned)

            def read_text(self, **_kwargs: object) -> str:
                self.second_reads += 1
                return "{}"  # Simulate replacement between hash and parse.

        racing = RacingOracle()
        with patch.object(analysis, "ORACLE", racing):
            cases = analysis._load_oracle()
        self.assertEqual(set(cases), {"hn_a", "c8", "hn_b"})
        self.assertEqual(racing.opens, 1)
        self.assertEqual(racing.second_reads, 0)

    def test_unreadable_oracle_is_machine_readable_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing-oracle.json"
            with patch.object(analysis, "ORACLE", missing):
                output = io.StringIO()
                with redirect_stdout(output):
                    self.assertEqual(analysis.main(["--mode", "discovery", "--json"]), 1)
            report = json.loads(output.getvalue())
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["counts"]["private_comparisons"], 0)
        self.assertIn("unavailable", report["errors"][0])


if __name__ == "__main__":
    unittest.main()
