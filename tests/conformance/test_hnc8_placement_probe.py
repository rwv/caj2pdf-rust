# SPDX-License-Identifier: MIT
"""Synthetic JFIF placement-probe protocol tests; no private corpus is loaded."""

from contextlib import ExitStack, redirect_stdout
import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_placement_probe as placement  # noqa: E402


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def jpeg() -> bytes:
    sof0 = bytes.fromhex("ffc00011080002000303011100021101031101")
    sos = bytes.fromhex("ffda000c03010002110311003f00")
    return placement.JFIF_HEADER + sof0 + sos + b"\x11\x22" + b"\xff\xd9"


def probe(field: str = "units") -> placement.Probe:
    if field == "units":
        return placement.Probe("synthetic-units", "c8", 1, 2, 10,
                               "JFIF APP0 units", 23, 1, 23, 0, 1)
    return placement.Probe("synthetic-density", "c8", 1, 2, 10,
                           "JFIF APP0 Xdensity", 24, 2, 25, 1, 2)


def pdf_pages(*, target_sha: str = "b" * 64, target_x: float = 4.0,
              other_sha: str = "a" * 64) -> dict:
    return {
        "page_count": 1, "draw_count": 2,
        "pages": [{"page_number": 1, "media_box": [0.0, 0.0, 100.0, 200.0],
                   "draws": [
                       {"draw_number": 1, "object_id": 7, "generation": 0,
                        "xobject_name": "/Im0", "xobject_type": "Image",
                        "filter": "/FlateDecode", "color_space": "Indexed",
                        "bits_per_component": 1, "width": 100, "height": 200,
                        "pdf_ctm": [100.0, 0, 0, -200.0, 0, 200],
                        "raw_stream_sha256": other_sha, "raw_stream_length": 123},
                       {"draw_number": 2, "object_id": 8, "generation": 0,
                        "xobject_name": "/Im1", "xobject_type": "Image",
                        "filter": "/DCTDecode", "color_space": "DeviceRGB",
                        "bits_per_component": 8, "width": 3, "height": 2,
                        "pdf_ctm": [3.0, 0, 0, -2.0, target_x, 9.0],
                        "raw_stream_sha256": target_sha, "raw_stream_length": 55},
                   ]}],
    }


class PlacementProbeTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_clean_clone_has_not_run_and_zero_private_activity(self) -> None:
        report = placement.run()
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["placement_rule_status"], "UNKNOWN_NOT_TESTED")
        self.assertEqual(report["resources"]["max_ranged_request_bytes"], 0)
        self.assertEqual(report["resources"]["max_source_hash_read_request_bytes"], 0)
        self.assertEqual(report["probes"], [])
        self.assertEqual({value for key, value in report["counts"].items()
                          if key != "probes_planned"}, {0})
        output = io.StringIO()
        with redirect_stdout(output):
            code = placement.main(["--json"])
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(output.getvalue())["status"], "NOT_RUN")

    def test_partial_explicit_cli_inputs_fail(self) -> None:
        for args in (
            ["--reference-report", str(self.root / "missing.json")],
            ["--corpus-dir", str(self.root)],
            ["--qpdf", str(self.root / "missing")],
        ):
            with self.subTest(args=args):
                output = io.StringIO()
                with redirect_stdout(output):
                    code = placement.main([*args, "--json"])
                self.assertEqual(code, 1)
                report = json.loads(output.getvalue())
                self.assertEqual(report["status"], "FAIL")
                self.assertEqual(report["counts"]["probes_attempted"], 0)
                self.assertEqual(report["counts"]["skipped_probes"], len(placement.PROBES))

    def test_environment_record_preserves_all_pinned_executable_and_run_facts(self) -> None:
        environment = {
            "reference_revision": placement.reference.REFERENCE_REVISION,
            "reference_clean": True,
            "pypdf2": {"version": placement.reference.PYPDF2_VERSION,
                       "tree_sha256": placement.reference.PYPDF2_TREE_SHA256},
            "binaries": {name: {"sha256": value, "version": "synthetic"}
                         for name, value in placement.reference.PINNED_HASHES.items()},
            "native_compiler_record": placement.reference.NATIVE_COMPILER_RECORD,
            "environment": {"PYTHONHASHSEED": "0", "TZ": "UTC"},
            "invocation": ["python", "converter", "convert", "SOURCE", "-o", "OUTPUT"],
            "timeout_seconds": placement.reference.CONVERTER_TIMEOUT_SECONDS,
        }
        record = placement._environment_record(environment, "BEFORE_PASS")
        self.assertEqual(record, {"status": "BEFORE_PASS", **environment})
        self.assertEqual(record["binaries"]["libjbigdec"]["sha256"],
                         placement.reference.PINNED_HASHES["libjbigdec"])
        self.assertEqual(record["pypdf2"]["tree_sha256"],
                         placement.reference.PYPDF2_TREE_SHA256)
        with self.assertRaisesRegex(placement.ProbeError, "lacks required"):
            placement._environment_record({**environment, "binaries": {}}, "BEFORE_PASS")

    def test_layout_oracle_parser_uses_the_same_bytes_it_hashed(self) -> None:
        pinned = placement.ORACLE.read_bytes()

        class RacingOracle:
            def __init__(self) -> None:
                self.opens = 0
                self.second_reads = 0

            def open(self, _mode: str) -> io.BytesIO:
                self.opens += 1
                return io.BytesIO(pinned)

            def read_text(self, **_kwargs: object) -> str:
                self.second_reads += 1
                return "{}"  # Simulate replacement after the digest read.

        racing = RacingOracle()
        with patch.object(placement, "ORACLE", racing):
            cases = placement._load_oracle()
        self.assertEqual(set(cases), {"hn_a", "c8", "hn_b"})
        self.assertEqual(racing.opens, 1)
        self.assertEqual(racing.second_reads, 0)

    def test_mid_batch_failure_counts_launched_completed_failing_and_skipped(self) -> None:
        corpus = self.root / "corpus"
        corpus.mkdir()
        checkout = self.root / "external-checkout"
        checkout.mkdir()
        profiles = placement.reference.PROFILES
        rows = []
        baselines = {}
        oracle = {}
        for profile in profiles:
            source = corpus / profile.source_id
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_bytes(b"synthetic")
            rows.append({"id": profile.source_id, "path": profile.source_id,
                         "size_bytes": 9, "sha256": "a" * 64})
            oracle[profile.name] = {"pdf_pages": []}
            for index in (1, 2):
                pdf = self.root / f"{profile.name}-run{index}.pdf"
                pdf.write_bytes(b"synthetic PDF")
                baselines[f"{profile.name}-run{index}"] = pdf
        audited = [{"id": row["id"], "sha256": row["sha256"]} for row in rows]
        environment = {
            "reference_revision": placement.reference.REFERENCE_REVISION,
            "reference_clean": True, "pypdf2": {},
            "binaries": {name: {"sha256": value}
                         for name, value in placement.reference.PINNED_HASHES.items()},
            "native_compiler_record": {}, "environment": {},
            "invocation": [], "timeout_seconds": 180,
        }
        paths = {name: self.root / name for name in (
            "python", "pydeps", "libjbigdec", "git", "qpdf", "mutool",
            "pdfinfo", "pdfimages")}
        paths.update({"corpus": corpus, "reference_repo": checkout,
                      "artifact_root": self.root / "artifacts"})
        reference_report = self.root / "reference-report.json"
        reference_report.write_text("{}", encoding="utf-8")

        def metadata(pdf: Path, _paths: dict) -> dict:
            profile = next(profile for profile in profiles
                           if pdf.name.startswith(profile.name + "-"))
            return {"pdf_sha256": profile.expected_pdf_sha256,
                    "page_count": profile.expected_pages,
                    "draw_count": profile.expected_draws,
                    "pages": [],
                    "tools": {name: {"sha256": placement.reference.PINNED_HASHES[name]}
                              for name in ("qpdf", "mutool", "pdfimages")},
                    "resources": {"max_tool_output_bytes": 1, "max_child_rss_kib": 2}}

        first = {"name": "c8-units", "profile": "c8", "outcome": "TARGET_STREAM_ONLY_CHANGED",
                 "repeatable": True, "candidate_causal_effect": False,
                 "runs": [{"status": "PASS", "pdf_sha256": "0" * 64},
                          {"status": "PASS", "pdf_sha256": "0" * 64}]}
        with ExitStack() as stack:
            stack.enter_context(patch.object(placement.reference, "load_source_rows",
                                             return_value=(placement.reference.MATRIX_SHA256, rows)))
            stack.enter_context(patch.object(placement.reference, "audit_sources", return_value=audited))
            stack.enter_context(patch.object(placement.reference, "audit_environment",
                                             return_value=environment))
            stack.enter_context(patch.object(placement, "_load_oracle", return_value=oracle))
            stack.enter_context(patch.object(placement, "_read_reference_report",
                                             return_value=({"status": "PASS"}, baselines)))
            stack.enter_context(patch.object(placement, "_file_digest", return_value="b" * 64))
            stack.enter_context(patch.object(placement.reference, "pdf_metadata",
                                             side_effect=metadata))
            probe_call = stack.enter_context(patch.object(
                placement, "_run_probe",
                side_effect=placement.ProbeError("synthetic first-probe failure")))
            first_failed = placement.run(paths, reference_report)
            probe_call.side_effect = [first, placement.ProbeError("synthetic mid-batch failure")]
            second_failed = placement.run(paths, reference_report)
            rejected = {**first, "outcome": "CONVERSION_FAILED",
                        "runs": [{"status": "EXIT_NONZERO"}, {"status": "EXIT_NONZERO"}]}
            unsupported = {**first, "outcome": "PDF_METADATA_UNSUPPORTED"}
            other = {**first, "outcome": "OTHER_PDF_CHANGE"}
            probe_call.side_effect = [first, rejected, unsupported, other]
            fully_observed = placement.run(paths, reference_report)
            probe_call.side_effect = [rejected] * len(placement.PROBES)
            all_rejected = placement.run(paths, reference_report)
        self.assertEqual(probe_call.call_count, 11)
        self.assertEqual(first_failed["status"], "FAIL")
        self.assertEqual(first_failed["counts"]["probes_attempted"], 1)
        self.assertEqual(first_failed["counts"]["probes_completed"], 0)
        self.assertEqual(first_failed["counts"]["probe_runs"], 0)
        self.assertIn("not all converter launches", first_failed["count_semantics"])
        self.assertEqual(first_failed["counts"]["probes_passing"], 0)
        self.assertEqual(first_failed["counts"]["probes_failing"], 1)
        self.assertEqual(first_failed["counts"]["skipped_probes"], 3)
        self.assertEqual(first_failed["probes"][0]["outcome"], "PROBE_PROTOCOL_FAILED")
        self.assertEqual(second_failed["status"], "FAIL")
        self.assertEqual(second_failed["counts"]["probes_attempted"], 2)
        self.assertEqual(second_failed["counts"]["probes_completed"], 1)
        self.assertEqual(second_failed["counts"]["probes_passing"], 1)
        self.assertEqual(second_failed["counts"]["probes_failing"], 1)
        self.assertEqual(second_failed["counts"]["skipped_probes"], 2)
        self.assertEqual(fully_observed["status"], "PARTIAL")
        self.assertEqual(fully_observed["counts"]["probes_attempted"], 4)
        self.assertEqual(fully_observed["counts"]["probes_completed"], 4)
        self.assertEqual(fully_observed["counts"]["probes_passing"], 2)
        self.assertEqual(fully_observed["counts"]["probes_failing"], 2)
        self.assertEqual(fully_observed["counts"]["conversion_failed_probes"], 1)
        self.assertEqual(fully_observed["counts"]["unsupported_probes"], 1)
        self.assertEqual(fully_observed["counts"]["skipped_probes"], 0)
        self.assertEqual(fully_observed["source_audit"]["status"], "PASS")
        self.assertEqual(fully_observed["environment_audit"]["status"], "PASS")
        self.assertEqual(all_rejected["status"], "PARTIAL")
        self.assertEqual(all_rejected["counts"]["probes_completed"], 4)
        self.assertEqual(all_rejected["counts"]["probes_failing"], 4)
        self.assertEqual(all_rejected["counts"]["conversion_failed_probes"], 4)
        self.assertEqual(all_rejected["counts"]["unsupported_probes"], 0)

    def test_copy_changes_exactly_one_jfif_byte_and_keeps_source(self) -> None:
        source = self.root / "source.caj"
        content = b"X" * 10 + jpeg() + b"Y" * (placement.COPY_CHUNK + 11)
        source.write_bytes(content)
        for name in ("units", "density"):
            current = probe(name)
            copy_record = placement.copy_probe(source, self.root / name, current)
            mutated = copy_record["path"]
            self.assertEqual(copy_record["source_sha256"], digest(content))
            self.assertEqual(copy_record["changed_byte_positions"], [current.changed_offset])
            self.assertEqual(placement._different_positions(source, mutated), [current.changed_offset])
            self.assertNotEqual(copy_record["mutated_source_sha256"], digest(content))
            image = {"payload_offset": 10, "payload_length": len(jpeg()),
                     "width": 3, "height": 2}
            self.assertEqual(placement._jpeg_header(source, image, current, mutated=False)[:2], (3, 2))
            self.assertEqual(placement._jpeg_header(mutated, image, current, mutated=True)[:2], (3, 2))
        self.assertEqual(source.read_bytes(), content)
        with self.assertRaisesRegex(placement.ProbeError, "unexpected original value"):
            placement.copy_probe(source, self.root / "bad-value",
                                 placement.Probe("bad", "c8", 1, 2, 10,
                                                 "JFIF APP0 units", 23, 1, 23, 9, 1))

    def test_jpeg_structure_rejects_missing_sos_eoi_and_changed_dimensions(self) -> None:
        original = jpeg()
        source = self.root / "source.jpg"
        image = {"payload_offset": 0, "payload_length": len(original),
                 "width": 3, "height": 2}
        p = placement.Probe("synthetic", "c8", 1, 2, 0,
                            "JFIF APP0 units", 13, 1, 13, 0, 1)
        for content, message in (
            (original[:-2] + b"AB", "EOI"),
            (original.replace(b"\xff\xda", b"\xff\xc4"), "SOS"),
            (original.replace(bytes.fromhex("0800020003"), bytes.fromhex("0800020004")),
             "dimensions"),
        ):
            with self.subTest(message=message):
                source.write_bytes(content)
                with self.assertRaisesRegex(placement.ProbeError, message):
                    placement._jpeg_header(source, image, p, mutated=False)

    def test_diff_rejects_multiple_changes_or_length_mismatch(self) -> None:
        original = self.root / "original"
        mutated = self.root / "mutated"
        original.write_bytes(b"a" * (placement.COPY_CHUNK + 1))
        mutated.write_bytes(b"a" * placement.COPY_CHUNK + b"b")
        self.assertEqual(placement._different_positions(original, mutated),
                         [placement.COPY_CHUNK])
        mutated.write_bytes(b"b" + b"a" * (placement.COPY_CHUNK - 1) + b"b")
        with self.assertRaisesRegex(placement.ProbeError, "more than one"):
            placement._different_positions(original, mutated)
        mutated.write_bytes(b"a")
        with self.assertRaisesRegex(placement.ProbeError, "length"):
            placement._different_positions(original, mutated)

    def test_pdf_comparison_records_target_stream_only_and_translation(self) -> None:
        current = probe()
        baseline = pdf_pages()
        stream_only = placement.compare_pdf(baseline, pdf_pages(target_sha="c" * 64),
                                            current, "c" * 64)
        self.assertEqual(stream_only["outcome"], "TARGET_STREAM_ONLY_CHANGED")
        self.assertEqual(stream_only["changed_page_numbers"], [1])
        self.assertTrue(stream_only["other_draw_streams_unchanged"])
        self.assertFalse(stream_only["candidate_causal_effect"])
        moved = placement.compare_pdf(
            baseline, pdf_pages(target_sha="c" * 64, target_x=5.125), current, "c" * 64)
        self.assertEqual(moved["outcome"], "TARGET_TRANSLATION_CHANGED")
        self.assertTrue(moved["candidate_causal_effect"])
        self.assertEqual(moved["changed_ctms"][0]["after"][4], 5.125)
        wrong_stream = placement.compare_pdf(
            baseline, pdf_pages(target_sha="c" * 64, target_x=5.125,
                                other_sha="d" * 64), current, "c" * 64)
        self.assertEqual(wrong_stream["outcome"], "OTHER_PDF_CHANGE")
        self.assertFalse(wrong_stream["other_draw_streams_unchanged"])
        reencoded = placement.compare_pdf(
            baseline, pdf_pages(target_sha="c" * 64), current, "e" * 64)
        self.assertEqual(reencoded["outcome"], "OTHER_PDF_CHANGE")
        self.assertFalse(reencoded["target_stream_equals_mutated_source"])

    def test_pdf_comparison_rejects_order_dimensions_page_box_and_counts(self) -> None:
        current = probe()
        baseline = pdf_pages()
        reordered = copy.deepcopy(baseline)
        reordered["pages"][0]["draws"][1]["object_id"] = 99
        self.assertEqual(placement.compare_pdf(baseline, reordered, current, "b" * 64)["outcome"],
                         "OTHER_PDF_CHANGE")
        resized = copy.deepcopy(baseline)
        resized["pages"][0]["draws"][1]["width"] = 4
        self.assertEqual(placement.compare_pdf(baseline, resized, current, "b" * 64)["outcome"],
                         "OTHER_PDF_CHANGE")
        moved_box = copy.deepcopy(baseline)
        moved_box["pages"][0]["media_box"][2] = 101
        self.assertEqual(placement.compare_pdf(baseline, moved_box, current, "b" * 64)["outcome"],
                         "OTHER_PDF_CHANGE")
        fewer = copy.deepcopy(baseline)
        fewer["draw_count"] = 1
        self.assertEqual(placement.compare_pdf(baseline, fewer, current, "b" * 64)["outcome"],
                         "PAGE_OR_DRAW_COUNT_CHANGED")

    def test_report_and_baseline_pdfs_must_be_external_pinned_and_present(self) -> None:
        protected = self.root / "protected"
        protected.mkdir()
        inside = protected / "report.json"
        inside.write_text("{}", encoding="utf-8")
        with self.assertRaisesRegex(placement.ProbeError, "outside protected roots"):
            placement._external_file(inside, (protected,), "reference report")
        alias = self.root / "alias.json"
        alias.symlink_to(inside)
        with self.assertRaisesRegex(placement.ProbeError, "outside protected roots"):
            placement._external_file(alias, (protected,), "reference report")
        with self.assertRaises(FileNotFoundError):
            placement._external_file(self.root / "missing.json", (protected,), "reference report")

        profiles = tuple(placement.reference.Profile(
            name, f"{name}.caj", digest(name.encode()), 1, 1, 1)
            for name in ("hn_a", "c8", "hn_b"))
        rows = [{"id": p.source_id, "sha256": digest((p.name + "-source").encode())}
                for p in profiles]
        source_audit = [{"id": row["id"], "sha256": row["sha256"]} for row in rows]
        environment = {"reference_revision": placement.reference.REFERENCE_REVISION}
        generations = []
        for profile in profiles:
            pdfs = [self.root / f"{profile.name}-run{run}.pdf" for run in (1, 2)]
            for pdf in pdfs:
                pdf.write_bytes(profile.name.encode())
            generations.append({"profile": profile.name, "source_id": profile.source_id,
                                "source_sha256": next(row["sha256"] for row in rows
                                                      if row["id"] == profile.source_id),
                                "expected_pdf_sha256": profile.expected_pdf_sha256,
                                "page_count": 1, "draw_count": 1, "deterministic": True,
                                "runs": [{"status": "PASS", "timed_out": False,
                                          "pdf_sha256": profile.expected_pdf_sha256,
                                          "output_path": str(pdf)} for pdf in pdfs]})
        report = {"schema_version": 1, "protocol": "hnc8-layout-reference-v1",
                  "status": "PASS", "matrix_sha256": placement.reference.MATRIX_SHA256,
                  "source_audit": {"status": "PASS", "sources": source_audit},
                  "environment_audit": {"status": "PASS", **environment},
                  "generations": generations}
        external_report = self.root / "reference-report.json"
        external_report.write_text(json.dumps(report), encoding="utf-8")
        with patch.object(placement.reference, "PROFILES", profiles):
            _, pdfs = placement._read_reference_report(
                external_report, rows, environment, source_audit, (protected,))
            self.assertEqual(len(pdfs), 6)
            (self.root / "c8-run1.pdf").write_bytes(b"changed")
            with self.assertRaisesRegex(placement.ProbeError, "SHA-256"):
                placement._read_reference_report(
                    external_report, rows, environment, source_audit, (protected,))
            report["status"] = "PARTIAL"
            external_report.write_text(json.dumps(report), encoding="utf-8")
            with self.assertRaisesRegex(placement.ProbeError, "completed"):
                placement._read_reference_report(
                    external_report, rows, environment, source_audit, (protected,))

    def test_probe_rechecks_mutated_source_after_each_conversion_and_runs_twice(self) -> None:
        p = probe()
        source = self.root / "source.caj"
        source.write_bytes(b"X" * 10 + jpeg())
        image = {"image_number": 2, "record_type": 2, "payload_offset": 10,
                 "payload_length": len(jpeg()), "payload_sha256": "b" * 64,
                 "width": 3, "height": 2}
        page = {"page_number": 1, "images": [
            {"image_number": 1, "payload_sha256": "a" * 64}, image]}
        before = {"page": page, "image": image, "jpeg_width": 3,
                  "jpeg_height": 2, "sos_offset": 53}
        changed_page = copy.deepcopy(page)
        changed_page["images"][1]["payload_sha256"] = "c" * 64
        after = {"page": changed_page, "image": changed_page["images"][1],
                 "jpeg_width": 3, "jpeg_height": 2, "sos_offset": 53}
        profile = placement.reference.Profile("c8", "synthetic.caj", "d" * 64, 1, 2, 1)
        resources = placement._report()["resources"]
        baseline = pdf_pages()
        mutant = pdf_pages(target_sha="c" * 64)
        first_pdf = self.root / "converted.pdf"
        first_pdf.write_bytes(b"pdf")
        conversion = {"status": "PASS", "exit_code": 0, "timed_out": False,
                      "elapsed_milliseconds": 1, "pdf_sha256": digest(b"pdf"),
                      "pdf_size_bytes": 3, "output_path": str(first_pdf),
                      "max_observed_temporary_bytes": 10,
                      "max_observed_child_vmhwm_kib": 11,
                      "max_observed_session_bytes": 12}
        mutant["pdf_sha256"] = conversion["pdf_sha256"]
        mutant["tools"] = {name: {"sha256": placement.reference.PINNED_HASHES[name]}
                           for name in ("qpdf", "mutool", "pdfimages")}
        mutant["resources"] = {"max_tool_output_bytes": 13, "max_child_rss_kib": 14}
        with patch.object(placement, "_check_probe_source", side_effect=[(before, 17), (after, 17)]), \
                patch.object(placement.reference, "run_converter", return_value=conversion) as convert, \
                patch.object(placement.reference, "pdf_metadata", return_value=mutant):
            result = placement._run_probe(p, profile, source, {"source_pages": [page]},
                                          baseline, self.root, {}, digest(source.read_bytes()), resources)
        self.assertEqual(convert.call_count, 2)
        self.assertEqual(result["outcome"], "TARGET_STREAM_ONLY_CHANGED")
        self.assertTrue(result["repeatable"])
        self.assertEqual(len(result["runs"]), 2)
        self.assertEqual(resources["max_pdf_tool_output_bytes"], 13)

        wrong_pdf = copy.deepcopy(mutant)
        wrong_pdf["pdf_sha256"] = "f" * 64
        with patch.object(placement, "_check_probe_source", side_effect=[(before, 17), (after, 17)]), \
                patch.object(placement.reference, "run_converter", return_value=conversion), \
                patch.object(placement.reference, "pdf_metadata", return_value=wrong_pdf):
            with self.assertRaisesRegex(placement.ProbeError, "PDF changed"):
                placement._run_probe(p, profile, source, {"source_pages": [page]},
                                     baseline, self.root, {}, digest(source.read_bytes()), resources)

        wrong_tool = copy.deepcopy(mutant)
        wrong_tool["tools"]["qpdf"]["sha256"] = "f" * 64
        with patch.object(placement, "_check_probe_source", side_effect=[(before, 17), (after, 17)]), \
                patch.object(placement.reference, "run_converter", return_value=conversion), \
                patch.object(placement.reference, "pdf_metadata", return_value=wrong_tool):
            with self.assertRaisesRegex(placement.ProbeError, "pinned executable"):
                placement._run_probe(p, profile, source, {"source_pages": [page]},
                                     baseline, self.root, {}, digest(source.read_bytes()), resources)

        def corrupt_mutant(mutated: Path, *_args: object, **_kwargs: object) -> dict:
            mutated.write_bytes(b"corrupt")
            return conversion

        with patch.object(placement, "_check_probe_source", side_effect=[(before, 17), (after, 17)]), \
                patch.object(placement.reference, "run_converter", side_effect=corrupt_mutant):
            with self.assertRaisesRegex(placement.ProbeError, "changed during"):
                placement._run_probe(p, profile, source, {"source_pages": [page]},
                                     baseline, self.root, {}, digest(source.read_bytes()), resources)

    def test_converter_rejection_and_metadata_unsupported_are_not_causal(self) -> None:
        p = probe()
        source = self.root / "source.caj"
        source.write_bytes(b"X" * 10 + jpeg())
        image = {"image_number": 2, "payload_sha256": "b" * 64}
        page = {"images": [{"payload_sha256": "a" * 64}, image]}
        before = {"page": page, "image": image, "jpeg_width": 3,
                  "jpeg_height": 2, "sos_offset": 53}
        changed_page = copy.deepcopy(page)
        changed_page["images"][1]["payload_sha256"] = "c" * 64
        after = {"page": changed_page, "image": changed_page["images"][1],
                 "jpeg_width": 3, "jpeg_height": 2, "sos_offset": 53}
        profile = placement.reference.Profile("c8", "synthetic.caj", "d" * 64, 1, 2, 1)
        failed = {"status": "EXIT_NONZERO", "exit_code": 1, "timed_out": False,
                  "elapsed_milliseconds": 1, "max_observed_temporary_bytes": 0,
                  "max_observed_child_vmhwm_kib": 1, "max_observed_session_bytes": 1}
        with patch.object(placement, "_check_probe_source", side_effect=[(before, 17), (after, 17)]), \
                patch.object(placement.reference, "run_converter", return_value=failed) as convert:
            result = placement._run_probe(p, profile, source, {"source_pages": [page]},
                                          {}, self.root, {}, digest(source.read_bytes()),
                                          placement._report()["resources"])
        self.assertEqual(convert.call_count, 2)
        self.assertEqual(result["outcome"], "CONVERSION_FAILED")
        self.assertNotIn("candidate_causal_effect", result)

        successful = {**failed, "status": "PASS", "exit_code": 0,
                      "pdf_sha256": digest(b"synthetic PDF"),
                      "pdf_size_bytes": 13, "output_path": str(self.root / "synthetic.pdf")}
        with patch.object(placement, "_check_probe_source", side_effect=[(before, 17), (after, 17)]), \
                patch.object(placement.reference, "run_converter", return_value=successful) as convert, \
                patch.object(placement.reference, "pdf_metadata",
                             side_effect=placement.reference.ReferenceError("unsupported PDF")):
            result = placement._run_probe(p, profile, source, {"source_pages": [page]},
                                          {}, self.root, {}, digest(source.read_bytes()),
                                          placement._report()["resources"])
        self.assertEqual(convert.call_count, 2)
        self.assertEqual(result["outcome"], "PDF_METADATA_UNSUPPORTED")
        self.assertNotIn("candidate_causal_effect", result)

        with patch.object(placement, "_check_probe_source", side_effect=[(before, 17), (after, 17)]), \
                patch.object(placement.reference, "run_converter") as convert:
            with self.assertRaisesRegex(placement.ProbeError, "pinned source matrix"):
                placement._run_probe(p, profile, source, {"source_pages": [page]},
                                     {}, self.root, {}, "0" * 64,
                                     placement._report()["resources"])
        convert.assert_not_called()


if __name__ == "__main__":
    unittest.main()
