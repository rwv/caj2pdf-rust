# SPDX-License-Identifier: MIT
"""Tests for the opt-in conformance runner. Original MIT-licensed code."""

from contextlib import redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import conformance  # noqa: E402


def blob_oid(content: bytes) -> str:
    return hashlib.sha1(f"blob {len(content)}\0".encode("ascii") + content).hexdigest()


class CorpusRunnerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.corpus = self.root / "corpus"
        self.corpus.mkdir()
        self.content = b"independently authored sample\n"
        (self.corpus / "sample.caj").write_bytes(self.content)
        self.row = {
            "id": "sample.caj",
            "path": "sample.caj",
            "aliases": [],
            "size_bytes": len(self.content),
            "git_blob_oid": blob_oid(self.content),
            "sha256": hashlib.sha256(self.content).hexdigest(),
            "expected_outcome": "success",
            "page_count": 1,
            "outline_count": 0,
        }
        self.matrix = self.root / "matrix.json"
        self.write_matrix()

    def write_matrix(self) -> None:
        self.matrix.write_text(
            json.dumps({"schema_version": 1, "samples": [self.row]}), encoding="utf-8"
        )

    def test_unset_corpus_is_not_a_compatibility_pass(self) -> None:
        report = conformance.run(self.matrix, None, None, "missing-mutool")
        self.assertEqual(report["inventory"]["status"], "NOT_RUN")
        self.assertEqual(report["inventory"]["passed"], 0)
        self.assertEqual(report["inventory"]["not_run"], 1)
        self.assertEqual(report["pdf"]["status"], "NOT_RUN")
        self.assertEqual(report["pdf"]["passed"], 0)
        output = io.StringIO()
        with redirect_stdout(output):
            conformance.print_text(report)
        self.assertIn("NOT_RUN=1", output.getvalue())
        self.assertIn("CAJ2PDF_CORPUS_DIR is unset", output.getvalue())

    def test_valid_corpus_only_passes_inventory(self) -> None:
        report = conformance.run(self.matrix, self.corpus, None, "missing-mutool")
        self.assertEqual(report["inventory"]["status"], "PASS")
        self.assertEqual(report["inventory"]["passed"], 1)
        self.assertEqual(report["pdf"]["status"], "NOT_RUN")

    def test_requested_missing_directory_and_file_fail(self) -> None:
        missing = conformance.run(self.matrix, self.root / "absent", None, "mutool")
        self.assertEqual(missing["inventory"]["status"], "FAIL")
        self.assertEqual(
            missing["inventory"]["passed"]
            + missing["inventory"]["failed"]
            + missing["inventory"]["not_run"],
            1,
        )
        self.assertIn("corpus directory is missing", missing["inventory"]["reason"])
        (self.corpus / "sample.caj").unlink()
        report = conformance.run(self.matrix, self.corpus, None, "mutool")
        self.assertEqual(report["inventory"]["failed"], 1)
        self.assertIn("missing file", report["inventory"]["failures"][0]["reason"])

    def test_wrong_size_and_same_size_wrong_hash_fail(self) -> None:
        sample = self.corpus / "sample.caj"
        sample.write_bytes(self.content + b"x")
        report = conformance.run(self.matrix, self.corpus, None, "mutool")
        self.assertIn("size mismatch", report["inventory"]["failures"][0]["reason"])
        sample.write_bytes(b"X" + self.content[1:])
        report = conformance.run(self.matrix, self.corpus, None, "mutool")
        self.assertIn("Git blob hash mismatch", report["inventory"]["failures"][0]["reason"])

    def test_optional_sha256_is_checked(self) -> None:
        self.row["sha256"] = "0" * 64
        self.write_matrix()
        report = conformance.run(self.matrix, self.corpus, None, "mutool")
        self.assertIn("SHA-256 mismatch", report["inventory"]["failures"][0]["reason"])

    def test_symlink_escape_and_alias_do_not_duplicate_input(self) -> None:
        outside = self.root / "outside.caj"
        outside.write_bytes(self.content)
        (self.corpus / "sample.caj").unlink()
        (self.corpus / "sample.caj").symlink_to(outside)
        report = conformance.run(self.matrix, self.corpus, None, "mutool")
        self.assertIn("path escapes root", report["inventory"]["failures"][0]["reason"])
        (self.corpus / "sample.caj").unlink()
        (self.corpus / "sample.caj").write_bytes(self.content)
        (self.corpus / "alias.caj").symlink_to(self.corpus / "sample.caj")
        self.row["aliases"] = ["alias.caj"]
        self.write_matrix()
        report = conformance.run(self.matrix, self.corpus, None, "mutool")
        self.assertEqual(report["inventory"]["passed"], 1)
        self.assertEqual(report["inventory"]["failed"], 0)

    def test_unsafe_matrix_path_and_duplicate_canonical_file_fail(self) -> None:
        for bad in ("../outside.caj", "/tmp/outside.caj", "a//b.caj", "C:/outside.caj"):
            self.row["path"] = bad
            self.write_matrix()
            with self.subTest(path=bad), self.assertRaises(conformance.ConformanceError):
                conformance.load_matrix(self.matrix)
        self.row["path"] = "sample.caj"
        self.row["aliases"] = []
        duplicate = dict(self.row, id="duplicate", path="alias.caj")
        (self.corpus / "alias.caj").symlink_to(self.corpus / "sample.caj")
        self.matrix.write_text(
            json.dumps({"schema_version": 1, "samples": [self.row, duplicate]}),
            encoding="utf-8",
        )
        report = conformance.run(self.matrix, self.corpus, None, "mutool")
        self.assertEqual(report["inventory"]["failed"], 1)

    def test_pdf_request_requires_valid_inventory_and_tool(self) -> None:
        pdf_dir = self.root / "pdfs"
        pdf_dir.mkdir()
        report = conformance.run(self.matrix, None, pdf_dir, "mutool")
        self.assertEqual(report["pdf"]["status"], "FAIL")
        self.assertEqual(
            report["pdf"]["passed"] + report["pdf"]["failed"]
            + report["pdf"]["unsupported"] + report["pdf"]["not_run"],
            1,
        )
        self.assertIn("valid corpus inventory", report["pdf"]["reason"])
        report = conformance.run(self.matrix, self.corpus, pdf_dir, "missing-mutool")
        self.assertEqual(report["pdf"]["status"], "FAIL")
        self.assertIn("not found", report["pdf"]["reason"])

    def test_cli_text_and_json_keep_phases_separate(self) -> None:
        with patch.dict(os.environ, {}, clear=True), redirect_stdout(io.StringIO()) as output:
            code = conformance.main(["--matrix", str(self.matrix)])
        self.assertEqual(code, 0)
        self.assertIn("PDF output checks [NOT_RUN]: PASS=0", output.getvalue())
        self.assertIn("NOT_RUN=1", output.getvalue())
        with patch.dict(os.environ, {}, clear=True), redirect_stdout(io.StringIO()) as output:
            code = conformance.main(["--matrix", str(self.matrix), "--json"])
        self.assertEqual(code, 0)
        report = json.loads(output.getvalue())
        self.assertEqual(report["inventory"]["status"], "NOT_RUN")
        self.assertEqual(report["pdf"]["status"], "NOT_RUN")
        with patch.dict(os.environ, {}, clear=True), redirect_stdout(io.StringIO()) as output:
            code = conformance.main(
                ["--matrix", str(self.matrix), "--corpus-dir", str(self.root / "absent")]
            )
        self.assertEqual(code, 1)
        self.assertIn("Corpus inventory [FAIL]", output.getvalue())

    @unittest.skipUnless(shutil.which("mutool"), "optional mutool integration test")
    def test_pdf_directory_mapping_and_missing_output(self) -> None:
        self.row["page_count"] = 2
        self.row["outline_count"] = 2
        self.row["expected_pdf"] = {"page_dimensions_pt": [[200, 300], [400, 250]]}
        self.write_matrix()
        pdf_dir = self.root / "pdfs"
        pdf_dir.mkdir()
        missing = conformance.run(self.matrix, self.corpus, pdf_dir, "mutool")
        self.assertEqual(missing["inventory"]["status"], "PASS")
        self.assertEqual(missing["pdf"]["status"], "FAIL")
        self.assertEqual(missing["pdf"]["passed"], 0)
        shutil.copyfile(ROOT / "tests/fixtures/valid_nested_outline.pdf", pdf_dir / "sample.pdf")
        report = conformance.run(self.matrix, self.corpus, pdf_dir, "mutool")
        self.assertEqual(report["pdf"]["status"], "PASS", report["pdf"])
        self.assertEqual(report["pdf"]["passed"], 1)


class MatrixAndPdfTests(unittest.TestCase):
    def test_checked_in_matrix_has_56_unique_canonical_samples(self) -> None:
        samples = conformance.load_matrix(conformance.DEFAULT_MATRIX)
        self.assertEqual(len(samples), 56)
        self.assertEqual(len({row["path"] for row in samples}), 56)
        self.assertEqual(len({row["git_blob_oid"] for row in samples}), 56)
        self.assertEqual(sum(len(row["aliases"]) for row in samples), 51)

    def test_page_and_outline_parsers_include_destinations(self) -> None:
        pages = conformance.parse_pages(
            "input.pdf:\n<page pagenum=\"1\"><MediaBox l=\"0\" b=\"0\" "
            "r=\"200\" t=\"300\" /></page>\n"
        )
        self.assertEqual(pages, [(200.0, 300.0)])
        outlines = conformance.parse_outlines(
            '-\t"Part One"\t#page=1&view=Fit\n'
            '|\t\t"Chapter Two"\t#page=2&view=Fit\n'
        )
        self.assertEqual([(item["depth"], item["title"], item["page"]) for item in outlines],
                         [(0, "Part One", 1), (1, "Chapter Two", 2)])
        self.assertEqual(conformance.outline_sha256(outlines),
                         conformance.outline_sha256(list(outlines)))

    def test_renderer_version_mismatch_fails_before_running_tool(self) -> None:
        with self.assertRaisesRegex(conformance.ConformanceError, "version mismatch"):
            conformance.compare_pdf(
                "missing-mutool",
                Path("unused.pdf"),
                {"expected_pdf": {"mutool_version": "mutool version 1.25.1"}},
                "mutool version 1.24.0",
            )

    @unittest.skipUnless(shutil.which("mutool"), "optional mutool integration test")
    def test_synthetic_pdf_checks_pages_outlines_destinations_and_render(self) -> None:
        fixture = ROOT / "tests/fixtures/valid_nested_outline.pdf"
        mutool = shutil.which("mutool")
        assert mutool is not None
        version = conformance.mutool_run(mutool, "-v").strip()
        with tempfile.TemporaryDirectory() as directory:
            rendered = Path(directory) / "page.pam"
            subprocess.run(
                [mutool, "draw", "-q", "-L", "-B", "128", "-F", "pam", "-c", "rgb",
                 "-r", "72", "-o", str(rendered), str(fixture), "1"],
                check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            expected_render = hashlib.sha256(rendered.read_bytes()).hexdigest()
        row = {
            "id": "synthetic",
            "page_count": 2,
            "outline_count": 2,
            "expected_pdf": {
                "mutool_version": version,
                "page_dimensions_pt": [[200, 300], [400, 250]],
                "outlines": [
                    {"depth": 0, "title": "Part One", "page": 1},
                    {"depth": 1, "title": "Chapter Two", "page": 2},
                ],
                "rendered_pages": [{"page": 1, "sha256": expected_render}],
            },
        }
        result = conformance.compare_pdf(mutool, fixture, row, version)
        self.assertEqual(result["status"], "PASS", result)
        self.assertTrue(all(status == "PASS" for status in result["checks"].values()))
        self.assertEqual(
            conformance.compare_pdf(mutool, fixture, {"id": "no-expectations"}, version)["status"],
            "NOT_RUN",
        )
        row["expected_pdf"]["page_dimensions_pt"][0][0] = 200.005
        self.assertEqual(conformance.compare_pdf(mutool, fixture, row, version)["status"], "PASS")
        row["expected_pdf"]["page_dimensions_pt"][0][0] = 200.02
        result = conformance.compare_pdf(mutool, fixture, row, version)
        self.assertEqual(result["checks"]["page_dimensions"], "FAIL")
        row["expected_pdf"]["page_dimensions_pt"][0][0] = 200
        row["expected_pdf"]["outlines"][1]["page"] = 1
        result = conformance.compare_pdf(mutool, fixture, row, version)
        self.assertEqual(result["checks"]["outline_hierarchy_destinations"], "FAIL")


if __name__ == "__main__":
    unittest.main()
