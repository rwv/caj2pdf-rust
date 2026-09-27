# SPDX-License-Identifier: MIT
"""Synthetic protocol and fault tests; no external converter/corpus is loaded."""

from contextlib import redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import hnc8_layout_reference as reference  # noqa: E402


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def geometry(extra_translation: float = 0, *, scale: float = 1, width: int = 20) -> dict:
    return {
        "page_count": 1,
        "pages": [
            {
                "media_box": [0, 0, 100, 200],
                "draws": [
                    {"draw_number": 1, "width": 100, "height": 200, "pdf_ctm": [100, 0, 0, 200, 0, 0]},
                    {
                        "draw_number": 2, "width": width, "height": 30,
                        "pdf_ctm": [20 * scale, 0, 0, 30, 4 + extra_translation, 9],
                    },
                ],
            }
        ],
    }


class ReferenceProtocolTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_clean_clone_reports_not_run_and_zero_comparisons(self) -> None:
        report = reference.run(None)
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(set(report["counts"].values()), {0})
        self.assertEqual(report["perturbations"]["attempted"], 0)
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(reference.main(["--json"]), 0)
        parsed = json.loads(output.getvalue())
        self.assertEqual(parsed["status"], "NOT_RUN")
        self.assertEqual(parsed["counts"]["existing_draw_comparisons"], 0)

    def test_explicit_partial_inputs_and_perturbation_without_inputs_fail(self) -> None:
        for options in (
            ["--corpus-dir", str(self.root)],
            ["--max-perturbations", "1"],
            ["--qpdf", str(self.root / "missing-qpdf")],
        ):
            with self.subTest(options=options):
                output = io.StringIO()
                with redirect_stdout(output):
                    self.assertEqual(reference.main([*options, "--json"]), 1)
                self.assertEqual(json.loads(output.getvalue())["status"], "FAIL")

    def test_audit_sources_rejects_missing_changed_and_escaping_paths(self) -> None:
        corpus = self.root / "corpus"
        corpus.mkdir()
        (corpus / "one.caj").write_bytes(b"known source")
        rows = [{"id": "one", "path": "one.caj", "size_bytes": 12, "sha256": digest(b"known source")}]
        self.assertEqual(reference.audit_sources(corpus, rows)[0]["sha256"], rows[0]["sha256"])
        (corpus / "one.caj").write_bytes(b"other source")
        with self.assertRaisesRegex(reference.ReferenceError, "changed"):
            reference.audit_sources(corpus, rows)
        (corpus / "one.caj").unlink()
        with self.assertRaisesRegex(reference.ReferenceError, "missing"):
            reference.audit_sources(corpus, rows)
        rows[0]["path"] = "../outside.caj"
        with self.assertRaisesRegex(reference.ReferenceError, "unsafe"):
            reference.audit_sources(corpus, rows)

    def test_single_field_copy_uses_bounded_chunks_and_verifies_only_one_byte(self) -> None:
        original = self.root / "original.caj"
        content = b"x" * (reference.READ_CHUNK + 3)
        original.write_bytes(content)
        field = reference.MutationField("c8", 1, "row_12", reference.READ_CHUNK - 2, 4, 1)
        mutation = reference.copy_with_one_field_mutated(original, self.root, field)
        self.assertEqual(mutation["changed_byte_positions"], [reference.READ_CHUNK - 1])
        self.assertEqual(mutation["mutated_field_byte_index"], 1)
        self.assertEqual(mutation["source_sha256"], digest(content))
        self.assertNotEqual(mutation["mutated_source_sha256"], mutation["source_sha256"])
        self.assertEqual(original.read_bytes(), content)
        with self.assertRaisesRegex(reference.ReferenceError, "outside"):
            reference.copy_with_one_field_mutated(
                original, self.root, reference.MutationField("c8", 1, "row_12", len(content) - 1, 4)
            )

    def test_geometry_filter_requires_extra_translation_with_stable_size_and_scale(self) -> None:
        before = geometry()
        outcome, delta, survives = reference.compare_geometry(before, geometry(1), 1)
        self.assertEqual(outcome, "EXTRA_IMAGE_TRANSLATION_CHANGED")
        self.assertTrue(survives)
        self.assertEqual(delta["before"]["draws"][1]["pdf_ctm"][4], 4)
        for after, expected in (
            (geometry(scale=2), "OTHER_GEOMETRY_CHANGE"),
            (geometry(width=21), "IMAGE_DIMENSIONS_CHANGED"),
            (geometry(), "NO_GEOMETRY_CHANGE"),
        ):
            with self.subTest(expected=expected):
                outcome, _, survives = reference.compare_geometry(before, after, 1)
                self.assertEqual(outcome, expected)
                self.assertFalse(survives)
        outcome, _, survives = reference.compare_geometry(before, {"page_count": 0, "pages": []}, 1)
        self.assertEqual(outcome, "PAGE_OR_DRAW_COUNT_CHANGED")
        self.assertFalse(survives)

    def test_mutation_plan_only_uses_unknown_page_row_fields(self) -> None:
        profiles = (
            reference.Profile("hn_a", "one.caj", "0" * 64, 1, 2, 1),
            reference.Profile("c8", "two.caj", "1" * 64, 1, 2, 1),
        )
        rows = [{"id": profile.source_id, "path": profile.source_id} for profile in profiles]
        page = {
            "page_number": 1,
            "row_offset": 80,
            "image_count": 2,
            "images": [{"gap_length": 0}, {"gap_length": 0}],
        }
        with patch.object(reference, "PROFILES", profiles), patch.object(
            reference, "_source_page", return_value=page
        ):
            fields, exclusions = reference.plan_mutations(rows, self.root)
        self.assertEqual([(field.name, field.absolute_offset, field.length) for field in fields[:3]],
                         [("row_10", 90, 2), ("row_12", 92, 4), ("row_16", 96, 4)])
        self.assertEqual(len(fields), 12)
        self.assertEqual(
            [(field.name, field.absolute_offset, field.byte_index) for field in fields[3:6]],
            [("row_10", 90, 1), ("row_12", 92, 2), ("row_16", 96, 2)],
        )
        self.assertEqual([item["nonzero_descriptor_gaps"] for item in exclusions], [0, 0])
        self.assertTrue(all("descriptor" not in field.name for field in fields))

    def test_hn_b_mapping_uses_source_payload_stream_hashes_not_page_count_alone(self) -> None:
        pages = [
            {"image_count": 1 if number in (1, 6) else 0,
             "images": [{"payload_sha256": str(number)}] if number in (1, 6) else [],
             "text_length": number}
            for number in range(1, 7)
        ]
        metadata = {
            "page_count": 2,
            "pages": [
                {"draws": [{"raw_stream_sha256": "1"}]},
                {"draws": [{"raw_stream_sha256": "6"}]},
            ],
        }
        with patch.object(reference, "_source_page", side_effect=pages):
            mapping = reference.hn_b_mapping(self.root / "unused", "hn_b", metadata)
        self.assertEqual(mapping["output_page_to_source_page"], [1, 6])
        self.assertTrue(mapping["positive_text_spans_on_image_free_rows"])
        metadata["pages"][1]["draws"][0]["raw_stream_sha256"] = "changed"
        with patch.object(reference, "_source_page", side_effect=pages):
            with self.assertRaisesRegex(reference.ReferenceError, "do not match"):
                reference.hn_b_mapping(self.root / "unused", "hn_b", metadata)
        metadata["pages"][1]["draws"][0]["raw_stream_sha256"] = "6"
        pages[2]["text_length"] = 0
        with patch.object(reference, "_source_page", side_effect=pages):
            with self.assertRaisesRegex(reference.ReferenceError, "positive text"):
                reference.hn_b_mapping(self.root / "unused", "hn_b", metadata)

    def test_requested_subset_is_partial_and_never_claims_full_probe(self) -> None:
        fields = [
            reference.MutationField("c8", 1, "row_10", 90, 2, 0),
            reference.MutationField("c8", 1, "row_10", 90, 2, 1),
        ]
        negative = {"candidate_survives": False, "outcome": "PDF_IDENTICAL"}
        with patch.object(reference, "plan_mutations", return_value=(fields, [])), patch.object(
            reference, "_profile_source", return_value=self.root / "unused"
        ), patch.object(reference, "run_mutation", return_value=negative):
            partial = reference.run_perturbations([], self.root, self.root, {}, {"c8": {}}, 1)
            complete = reference.run_perturbations([], self.root, self.root, {}, {"c8": {}}, 2)
        self.assertEqual((partial["status"], partial["tested_initial"], partial["planned_initial"]),
                         ("PARTIAL", 1, 2))
        self.assertEqual(partial["placement_rule_status"], "UNKNOWN_PARTIAL_PROBE")
        self.assertEqual((complete["status"], complete["tested_initial"], complete["planned_initial"]),
                         ("PASS", 2, 2))

    def test_generation_only_is_partial_even_when_pdf_counts_match(self) -> None:
        names = (
            "corpus", "reference_repo", "python", "pydeps", "libjbigdec",
            "artifact_root", "git", "qpdf", "mutool", "pdfinfo", "pdfimages",
        )
        paths = {name: self.root / name for name in names}
        rows = [{"id": "synthetic", "path": "synthetic.caj"}]
        audited = [{"id": "synthetic", "sha256": "0" * 64}]
        generations = [
            {"profile": "hn_a", "page_count": 68, "draw_count": 91, "runs": []},
            {"profile": "c8", "page_count": 7, "draw_count": 34, "runs": []},
            {"profile": "hn_b", "page_count": 2, "draw_count": 2, "runs": []},
        ]
        with patch.object(reference, "load_source_rows", return_value=("0" * 64, rows)), patch.object(
            reference, "audit_sources", return_value=audited
        ), patch.object(reference, "audit_environment", return_value={"pinned": True}), patch.object(
            reference, "run_profiles", return_value=(generations, {})
        ):
            report = reference.run(paths, max_perturbations=0)
        self.assertEqual(report["status"], "PARTIAL")
        self.assertEqual(report["perturbations"]["status"], "NOT_RUN")
        self.assertEqual(report["counts"]["existing_page_comparisons"], 75)
        self.assertEqual(report["counts"]["existing_draw_comparisons"], 125)

    def test_black_box_child_success_and_timeout_use_isolated_directory(self) -> None:
        checkout = self.root / "external"
        checkout.mkdir()
        lib = self.root / "libjbigdec.so"
        lib.write_bytes(b"stub library")
        source = self.root / "source.caj"
        source.write_bytes(b"source")
        pydeps = self.root / "pydeps"
        pydeps.mkdir()
        paths = {
            "reference_repo": checkout,
            "python": Path(sys.executable),
            "pydeps": pydeps,
            "libjbigdec": lib,
        }
        (checkout / "caj2pdf").write_text(
            "import pathlib, sys\n"
            "assert pathlib.Path('libjbigdec.so').is_symlink()\n"
            "pathlib.Path(sys.argv[-1]).write_bytes(b'%PDF-synthetic')\n",
            encoding="utf-8",
        )
        success = reference.run_converter(source, self.root, paths, label="success")
        self.assertEqual(success["status"], "PASS")
        self.assertEqual(success["pdf_sha256"], digest(b"%PDF-synthetic"))
        self.assertTrue((Path(success["cwd"]) / "libjbigdec.so").is_symlink())
        (checkout / "caj2pdf").write_text("import time; time.sleep(5)\n", encoding="utf-8")
        with patch.object(reference, "CONVERTER_TIMEOUT_SECONDS", 1):
            timed = reference.run_converter(source, self.root, paths, label="timeout")
        self.assertEqual(timed["status"], "TIMEOUT")
        self.assertTrue(timed["timed_out"])

    def test_tool_output_cap_stops_verbose_child_and_reaps_group(self) -> None:
        self.assertEqual(reference._output([sys.executable, "-c", "print('ok')"]), "ok")
        pid_file = self.root / "pid"
        noisy = (
            "import os,sys,time;"
            f"open({str(pid_file)!r},'w').write(str(os.getpid()));"
            "sys.stdout.write('x'*20000);sys.stdout.flush();time.sleep(5)"
        )
        start = time.monotonic()
        with self.assertRaisesRegex(reference.ReferenceError, "exceeded 4 KiB"):
            reference._output([sys.executable, "-c", noisy], timeout=2)
        self.assertLess(time.monotonic() - start, 2)
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)
        with self.assertRaisesRegex(reference.ReferenceError, "timed out"):
            reference._output([sys.executable, "-c", "import time; time.sleep(5)"], timeout=1)

    def test_mutated_copy_is_rechecked_after_black_box_child(self) -> None:
        source = self.root / "source.caj"
        source.write_bytes(b"x" * 64)
        profile = reference.Profile("c8", "synthetic", "0" * 64, 1, 2, 1)
        field = reference.MutationField("c8", 1, "row_10", 10, 2)

        def overwrite_mutated(path: Path, *_args: object, **_kwargs: object) -> dict:
            path.write_bytes(b"corrupted")
            return {"status": "PASS"}

        with patch.object(reference, "run_converter", side_effect=overwrite_mutated):
            with self.assertRaisesRegex(reference.ReferenceError, "changed during"):
                reference.run_mutation(profile, source, field, {}, self.root, {})
        self.assertEqual(source.read_bytes(), b"x" * 64)

    def test_sampling_failure_reaps_converter_process(self) -> None:
        checkout = self.root / "external"
        checkout.mkdir()
        pid_file = self.root / "child.pid"
        (checkout / "caj2pdf").write_text(
            f"import os,time,pathlib;pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid()));time.sleep(5)\n",
            encoding="utf-8",
        )
        pydeps = self.root / "pydeps"
        pydeps.mkdir()
        lib = self.root / "libjbigdec.so"
        lib.write_bytes(b"stub")
        source = self.root / "source.caj"
        source.write_bytes(b"source")
        paths = {
            "reference_repo": checkout, "python": Path(sys.executable),
            "pydeps": pydeps, "libjbigdec": lib,
        }

        def fail_after_start(_path: Path) -> int:
            deadline = time.monotonic() + 2
            while not pid_file.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            raise OSError("synthetic disk sampler failure")

        with patch.object(reference, "_tree_size", side_effect=fail_after_start):
            with self.assertRaisesRegex(OSError, "sampler failure"):
                reference.run_converter(source, self.root, paths, label="sampler-failure")
        self.assertTrue(pid_file.exists())
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)


if __name__ == "__main__":
    unittest.main()
