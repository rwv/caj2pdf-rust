# SPDX-License-Identifier: MIT
"""Synthetic checks of the predeclared text-component protocol."""

from contextlib import ExitStack
import copy
import hashlib
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_text_transplant as transplant  # noqa: E402
from hnc8_layout_source import FileInput, SourceExtractor  # noqa: E402


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def dib() -> bytes:
    return (struct.pack("<IiiHHI", 40, 1, 1, 1, 1, 0) + bytes(20) +
            bytes.fromhex("ffffff0000000000") + b"\x00")


def source_bytes() -> bytes:
    """Two independent C8 records with a valid first-descriptor boundary."""
    data = bytearray(254)
    data[:4] = bytes.fromhex("c8000000")
    data[8:12] = struct.pack("<i", 2)
    data[80:90] = struct.pack("<iih", 120, 8, 1)
    data[100:110] = struct.pack("<iih", 189, 4, 1)
    data[120:128] = b"P1-abcde"
    data[128:140] = struct.pack("<iii", 0, 140, 49)
    data[140:189] = dib()
    data[189:193] = b"P222"
    data[193:205] = struct.pack("<iii", 0, 205, 49)
    data[205:254] = dib()
    return bytes(data)


def synthetic_probe() -> transplant.Transplant:
    return transplant.Transplant("synthetic-text", "c8", 1, 2, 80,
                                 120, 8, 189, 4, 124, 128)


def pdf_pages(*, x: float = 3.0) -> dict:
    def draw(number: int, sha: str, left: float) -> dict:
        return {"draw_number": number, "object_id": number + 10,
                "generation": 0, "xobject_name": f"/Im{number}",
                "xobject_type": "Image", "filter": "/DCTDecode",
                "color_space": "DeviceRGB", "bits_per_component": 8,
                "width": 3, "height": 2, "raw_stream_sha256": sha,
                "raw_stream_length": 42, "pdf_ctm": [3.0, 0.0, 0.0, -2.0, left, 9.0]}
    return {"page_count": 2, "draw_count": 3,
            "pages": [
                {"page_number": 1, "media_box": [0.0, 0.0, 100.0, 200.0],
                 "draws": [draw(1, "a" * 64, 0.0), draw(2, "b" * 64, x)]},
                {"page_number": 2, "media_box": [0.0, 0.0, 100.0, 200.0],
                 "draws": [draw(1, "c" * 64, 0.0)]},
            ]}


class TextTransplantTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "synthetic.caj"
        self.source.write_bytes(source_bytes())

    def case(self) -> dict:
        with FileInput(self.source) as source:
            reader = SourceExtractor(source, "synthetic.caj")
            pages = [reader.read_page(1), reader.read_page(2)]
        return {"source_pages": pages}

    def test_clean_clone_has_zero_private_comparisons(self) -> None:
        report = transplant.run()
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["probes"], [])
        self.assertEqual({value for key, value in report["counts"].items()
                          if key != "probes_planned"}, {0})
        self.assertEqual(report["counts"]["probes_planned"], 2)
        self.assertEqual(report["resources"]["max_ranged_request_bytes"], 0)
        partial = transplant.run({}, None)
        self.assertEqual(partial["status"], "FAIL")
        self.assertEqual(partial["counts"]["skipped_probes"], 2)

    def test_plan_constants_preserve_descriptor_boundaries(self) -> None:
        self.assertEqual(transplant.TRANSPLANTS, (
            transplant.Transplant("c8-text", "c8", 1, 2, 80, 220, 14546,
                                  132124, 10690, 4076, 14766),
            transplant.Transplant("hn-a-text", "hn_a", 16, 22, 16664, 953320, 7501,
                                  1354683, 5344, 955477, 960821),
        ))
        for probe in transplant.TRANSPLANTS:
            transplant._check_plan(probe)
            self.assertEqual(probe.original_text_offset + probe.original_text_length,
                             probe.target_offset + probe.donor_length)
        invalid = transplant.Transplant("bad", "c8", 1, 2, 80, 120, 8,
                                        189, 4, 125, 128)
        with self.assertRaisesRegex(transplant.TransplantError, "descriptor boundary"):
            transplant._check_plan(invalid)

    def test_exact_bounded_transplant_preserves_images_and_other_page(self) -> None:
        probe = synthetic_probe()
        case = self.case()
        expected_hash = digest(source_bytes())
        donor_hash = case["source_pages"][1]["text_sha256"]
        before_request = transplant._check_source(
            self.source, "synthetic.caj", case, probe, mutated=False)
        record = transplant.copy_transplant(
            self.source, self.root / "copy", probe, expected_hash, donor_hash)
        mutant = Path(record["path"])
        after_request = transplant._check_source(
            mutant, "synthetic.caj", case, probe, mutated=True)
        self.assertLessEqual(max(before_request, after_request), transplant.COPY_CHUNK)
        self.assertEqual(record["source_sha256"], expected_hash)
        self.assertEqual(record["donor_text_sha256"], donor_hash)
        self.assertEqual(record["allowed_changed_spans"], [[80, 8], [124, 4]])
        changed = {position for start, length in record["changed_offset_runs"]
                   for position in range(start, start + length)}
        expected_changes = {position for position, (before, after) in enumerate(
            zip(source_bytes(), mutant.read_bytes())) if before != after}
        self.assertEqual(changed, expected_changes)
        self.assertEqual(len(changed), record["changed_byte_count"])
        self.assertEqual(record["changed_offset_runs"], sorted(record["changed_offset_runs"]))
        self.assertEqual(mutant.read_bytes()[124:128], b"P222")
        self.assertEqual(mutant.read_bytes()[128:], source_bytes()[128:])
        self.assertEqual(self.source.read_bytes(), source_bytes())
        with FileInput(mutant) as ranged:
            reader = SourceExtractor(ranged, "synthetic.caj")
            self.assertEqual(reader.read_page(2), case["source_pages"][1])
            self.assertEqual(reader.read_page(1)["images"], case["source_pages"][0]["images"])

    def test_source_checks_reject_unpinned_text_or_outside_edits(self) -> None:
        probe = synthetic_probe()
        case = self.case()
        source_hash = digest(source_bytes())
        donor_hash = case["source_pages"][1]["text_sha256"]
        with self.assertRaisesRegex(transplant.TransplantError, "donor text SHA-256"):
            transplant.copy_transplant(self.source, self.root / "wrong-donor", probe,
                                       source_hash, "0" * 64)
        record = transplant.copy_transplant(self.source, self.root / "good", probe,
                                            source_hash, donor_hash)
        mutant = Path(record["path"])
        with mutant.open("r+b") as stream:
            stream.seek(200)
            stream.write(b"X")
        with self.assertRaisesRegex(transplant.TransplantError, "outside the declared spans"):
            transplant._audit_diff(self.source, mutant, probe)
        self.source.write_bytes(source_bytes()[:80] + b"X" + source_bytes()[81:])
        with self.assertRaisesRegex(transplant.TransplantError, "matrix SHA-256"):
            transplant.copy_transplant(self.source, self.root / "wrong-source", probe,
                                       source_hash, donor_hash)

    def test_pdf_guard_allows_only_target_supplemental_translation(self) -> None:
        probe = synthetic_probe()
        baseline = pdf_pages()
        moved = transplant.compare_pdf(baseline, pdf_pages(x=5.125), probe, 2, 3)
        self.assertEqual(moved["outcome"], "TEXT_ROW_COMPONENT_DEPENDENCY")
        self.assertTrue(moved["component_placement_effect"])
        self.assertEqual(moved["changed_ctms"][0]["after"][4], 5.125)
        unchanged = transplant.compare_pdf(baseline, copy.deepcopy(baseline), probe, 2, 3)
        self.assertEqual(unchanged["outcome"], "NO_PLACEMENT_CHANGE")
        self.assertFalse(unchanged["component_placement_effect"])
        for change in ("stream", "object", "first", "other", "scale", "box", "count"):
            mutant = copy.deepcopy(baseline)
            if change == "stream":
                mutant["pages"][0]["draws"][1]["raw_stream_sha256"] = "d" * 64
            elif change == "object":
                mutant["pages"][0]["draws"][1]["object_id"] = 99
            elif change == "first":
                mutant["pages"][0]["draws"][0]["pdf_ctm"][4] = 1.0
            elif change == "other":
                mutant["pages"][1]["draws"][0]["pdf_ctm"][4] = 1.0
            elif change == "scale":
                mutant["pages"][0]["draws"][1]["pdf_ctm"][0] = 4.0
            elif change == "box":
                mutant["pages"][0]["media_box"][2] = 101.0
            else:
                mutant["draw_count"] = 2
            with self.subTest(change=change):
                result = transplant.compare_pdf(baseline, mutant, probe, 2, 3)
                self.assertEqual(result["outcome"], "UNSUPPORTED")
                self.assertTrue(result["guard_failures"])

    def test_probe_runs_twice_and_rechecks_mutated_source(self) -> None:
        probe = synthetic_probe()
        case = self.case()
        profile = transplant.reference.Profile("c8", "synthetic.caj", "f" * 64, 2, 3, 1)
        baseline = pdf_pages()
        resources = transplant._report()["resources"]
        session = self.root / "session"
        session.mkdir()
        pdf_sha = digest(b"synthetic PDF")
        calls = []

        def convert(mutant: Path, directory: Path, _paths: dict, *, label: str) -> dict:
            calls.append(label)
            output = directory / f"{label}.pdf"
            output.write_bytes(b"synthetic PDF")
            return {"status": "PASS", "exit_code": 0, "timed_out": False,
                    "elapsed_milliseconds": 1, "pdf_sha256": pdf_sha,
                    "pdf_size_bytes": output.stat().st_size,
                    "output_path": str(output), "max_observed_temporary_bytes": 10,
                    "max_observed_child_vmhwm_kib": 11, "max_observed_session_bytes": 12}

        def metadata(_pdf: Path, _paths: dict) -> dict:
            return {**copy.deepcopy(baseline), "pdf_sha256": pdf_sha,
                    "tools": {name: {"sha256": transplant.reference.PINNED_HASHES[name]}
                              for name in ("qpdf", "mutool", "pdfimages")},
                    "resources": {"max_tool_output_bytes": 13, "max_child_rss_kib": 14}}

        with patch.object(transplant.reference, "run_converter", side_effect=convert), \
             patch.object(transplant.reference, "pdf_metadata", side_effect=metadata):
            result = transplant._run_probe(
                probe, profile, self.source, case, baseline, session, {},
                digest(source_bytes()), resources)
        self.assertEqual(calls, ["synthetic-text-run1", "synthetic-text-run2"])
        self.assertEqual(result["outcome"], "NO_PLACEMENT_CHANGE")
        self.assertTrue(result["repeatable"])
        self.assertEqual(len(result["runs"]), 2)
        self.assertEqual(resources["max_pdf_tool_child_rss_kib"], 14)

        def tamper(mutant: Path, directory: Path, _paths: dict, *, label: str) -> dict:
            with mutant.open("r+b") as stream:
                stream.seek(150)
                stream.write(b"X")
            return convert(mutant, directory, _paths, label=label)

        with patch.object(transplant.reference, "run_converter", side_effect=tamper):
            with self.assertRaisesRegex(transplant.TransplantError, "changed during conversion"):
                transplant._run_probe(
                    probe, profile, self.source, case, baseline, session, {},
                    digest(source_bytes()), resources)

    def _mock_batch(self, stack: ExitStack):
        """Exercise the real run orchestration with only synthetic audit inputs."""
        number = getattr(self, "_batch_case_number", 0) + 1
        self._batch_case_number = number
        batch_root = self.root / f"batch-{number}"
        batch_root.mkdir()
        corpus = batch_root / "corpus"
        checkout = batch_root / "checkout"
        corpus.mkdir()
        checkout.mkdir()
        rows = []
        baseline_pdfs = {}
        for profile in transplant.reference.PROFILES:
            source = corpus / profile.source_id
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_bytes(b"synthetic")
            rows.append({"id": profile.source_id, "path": profile.source_id,
                         "size_bytes": 9, "sha256": digest(b"synthetic")})
            for index in (1, 2):
                pdf = batch_root / f"{profile.name}-run{index}.pdf"
                pdf.write_bytes(b"synthetic PDF")
                baseline_pdfs[f"{profile.name}-run{index}"] = pdf
        audited = [{"id": row["id"], "path": str(corpus / row["path"]),
                    "size_bytes": row["size_bytes"], "sha256": row["sha256"]}
                   for row in rows]
        environment = {
            "reference_revision": transplant.reference.REFERENCE_REVISION,
            "reference_clean": True, "pypdf2": {},
            "binaries": {name: {"sha256": sha}
                         for name, sha in transplant.reference.PINNED_HASHES.items()},
            "native_compiler_record": {}, "environment": {},
            "invocation": [], "timeout_seconds": 180,
        }
        paths = {name: batch_root / name for name in (
            "python", "pydeps", "libjbigdec", "git", "qpdf", "mutool",
            "pdfinfo", "pdfimages")}
        paths.update({"corpus": corpus, "reference_repo": checkout,
                      "artifact_root": batch_root / "artifacts"})
        report_path = batch_root / "reference-report.json"
        report_path.write_text("{}", encoding="utf-8")

        def metadata(pdf: Path, _paths: dict) -> dict:
            profile = next(profile for profile in transplant.reference.PROFILES
                           if pdf.name.startswith(profile.name + "-"))
            return {"pdf_sha256": profile.expected_pdf_sha256,
                    "page_count": profile.expected_pages,
                    "draw_count": profile.expected_draws,
                    "pages": [],
                    "tools": {name: {"sha256": transplant.reference.PINNED_HASHES[name]}
                              for name in ("qpdf", "mutool", "pdfimages")},
                    "resources": {"max_tool_output_bytes": 1, "max_child_rss_kib": 2}}

        stack.enter_context(patch.object(transplant.reference, "load_source_rows",
                                         return_value=(transplant.reference.MATRIX_SHA256, rows)))
        source_audit = stack.enter_context(patch.object(
            transplant.reference, "audit_sources", return_value=audited))
        stack.enter_context(patch.object(transplant.reference, "audit_environment",
                                         return_value=environment))
        stack.enter_context(patch.object(transplant.shared, "_load_oracle",
                                         return_value={profile.name: {"pdf_pages": []}
                                                       for profile in transplant.reference.PROFILES}))
        stack.enter_context(patch.object(transplant.shared, "_read_reference_report",
                                         return_value=({"status": "PASS"}, baseline_pdfs)))
        input_digest = stack.enter_context(patch.object(transplant.shared, "_file_digest",
                                                        return_value="b" * 64))
        stack.enter_context(patch.object(transplant.reference, "pdf_metadata",
                                         side_effect=metadata))
        probe_call = stack.enter_context(patch.object(transplant, "_run_probe"))
        return paths, report_path, probe_call, source_audit, input_digest

    def test_batch_counts_and_pre_post_audits_include_mid_probe_failures(self) -> None:
        normal = {"outcome": "NO_PLACEMENT_CHANGE", "repeatable": True,
                  "runs": [{"status": "PASS"}, {"status": "PASS"}]}
        dependency = {**normal, "outcome": "TEXT_ROW_COMPONENT_DEPENDENCY"}
        unsupported = {**normal, "outcome": "UNSUPPORTED"}
        rejected = {**normal, "outcome": "CONVERSION_FAILED"}

        with ExitStack() as stack:
            paths, reference_report, probe_call, source_audit, input_digest = self._mock_batch(stack)
            probe_call.side_effect = transplant.TransplantError("synthetic first failure")
            first_failed = transplant.run(paths, reference_report)
            probe_call.side_effect = [normal, transplant.TransplantError("synthetic second failure")]
            second_failed = transplant.run(paths, reference_report)
            probe_call.side_effect = [dependency, unsupported]
            partial = transplant.run(paths, reference_report)
            probe_call.side_effect = [normal, rejected]
            conversion_rejected = transplant.run(paths, reference_report)
            probe_call.side_effect = [normal, dependency]
            passed = transplant.run(paths, reference_report)

        self.assertEqual(first_failed["status"], "FAIL")
        self.assertEqual(first_failed["counts"]["probes_attempted"], 1)
        self.assertEqual(first_failed["counts"]["probes_completed"], 0)
        self.assertEqual(first_failed["counts"]["probe_runs"], 0)
        self.assertEqual(first_failed["counts"]["probes_failing"], 1)
        self.assertEqual(first_failed["counts"]["skipped_probes"], 1)
        self.assertEqual(second_failed["status"], "FAIL")
        self.assertEqual(second_failed["counts"]["probes_attempted"], 2)
        self.assertEqual(second_failed["counts"]["probes_completed"], 1)
        self.assertEqual(second_failed["counts"]["probe_runs"], 2)
        self.assertEqual(second_failed["counts"]["probes_passing"], 1)
        self.assertEqual(second_failed["counts"]["probes_failing"], 1)
        self.assertEqual(second_failed["counts"]["skipped_probes"], 0)
        self.assertIn("probe_runs counts those returned records only", passed["count_semantics"])
        self.assertEqual(partial["status"], "PARTIAL")
        self.assertEqual(partial["counts"]["probe_runs"], 4)
        self.assertEqual(partial["counts"]["placement_changed_probes"], 1)
        self.assertEqual(partial["counts"]["unsupported_probes"], 1)
        self.assertEqual(partial["counts"]["probes_passing"], 1)
        self.assertEqual(partial["counts"]["probes_failing"], 1)
        self.assertEqual(conversion_rejected["counts"]["conversion_failed_probes"], 1)
        self.assertEqual(conversion_rejected["counts"]["unsupported_probes"], 1)
        self.assertEqual(passed["status"], "PASS")
        self.assertEqual(passed["placement_rule_status"], "UNKNOWN_COMPONENT_DEPENDENCY_ONLY")
        self.assertEqual(passed["counts"]["probes_attempted"], 2)
        self.assertEqual(passed["counts"]["probes_completed"], 2)
        self.assertEqual(passed["counts"]["probe_runs"], 4)
        self.assertEqual(passed["counts"]["probes_passing"], 2)
        self.assertEqual(passed["counts"]["probes_failing"], 0)
        self.assertEqual(passed["counts"]["skipped_probes"], 0)
        self.assertEqual(passed["source_audit"]["status"], "PASS")
        self.assertEqual(passed["environment_audit"]["status"], "PASS")
        self.assertEqual(passed["input_audit"]["status"], "PASS")
        self.assertEqual(passed["counts"]["private_source_checks_before"], 3)
        self.assertEqual(passed["counts"]["private_source_checks_after"], 3)
        self.assertEqual(passed["counts"]["baseline_pdf_checks_before"], 6)
        self.assertEqual(passed["counts"]["baseline_pdf_checks_after"], 6)
        self.assertGreaterEqual(source_audit.call_count, 10)
        self.assertGreaterEqual(input_digest.call_count, 45)

    def test_batch_rejects_changed_post_run_source_audit(self) -> None:
        normal = {"outcome": "NO_PLACEMENT_CHANGE", "repeatable": True,
                  "runs": [{"status": "PASS"}, {"status": "PASS"}]}
        with ExitStack() as stack:
            paths, reference_report, probe_call, source_audit, _ = self._mock_batch(stack)
            source_audit.side_effect = [source_audit.return_value,
                                        [{"id": "changed", "sha256": "0" * 64}]]
            probe_call.side_effect = [normal, normal]
            report = transplant.run(paths, reference_report)
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["source_audit"]["status"], "FAIL")
        self.assertEqual(report["counts"]["private_source_checks_after"], 0)
        self.assertTrue(any("source matrix changed" in error for error in report["errors"]))

    def test_batch_rejects_changed_post_run_matrix_oracle_or_pdf(self) -> None:
        normal = {"outcome": "NO_PLACEMENT_CHANGE", "repeatable": True,
                  "runs": [{"status": "PASS"}, {"status": "PASS"}]}
        for changed_input in range(9):
            with self.subTest(changed_input=changed_input), ExitStack() as stack:
                paths, reference_report, probe_call, _, input_digest = self._mock_batch(stack)
                input_digest.side_effect = (["b" * 64] * 9 +
                                            ["c" * 64 if index == changed_input else "b" * 64
                                             for index in range(9)])
                probe_call.side_effect = [normal, normal]
                report = transplant.run(paths, reference_report)
                self.assertEqual(report["status"], "FAIL")
                self.assertEqual(report["input_audit"]["status"], "FAIL")
                self.assertEqual(report["counts"]["baseline_pdf_checks_after"], 0)
                self.assertTrue(any("input changed" in error for error in report["errors"]))

    def test_batch_requires_two_classified_run_records(self) -> None:
        with ExitStack() as stack:
            paths, reference_report, probe_call, _, _ = self._mock_batch(stack)
            probe_call.return_value = {"outcome": "NO_PLACEMENT_CHANGE",
                                       "repeatable": True, "runs": [{"status": "PASS"}]}
            report = transplant.run(paths, reference_report)
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["counts"]["probes_attempted"], 1)
        self.assertEqual(report["counts"]["probes_completed"], 0)
        self.assertEqual(report["counts"]["probe_runs"], 0)
        self.assertEqual(report["counts"]["probes_failing"], 1)
        self.assertEqual(report["counts"]["skipped_probes"], 1)


if __name__ == "__main__":
    unittest.main()
