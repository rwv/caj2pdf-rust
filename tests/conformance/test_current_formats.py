# SPDX-License-Identifier: MIT
"""Current CLI inventory reports must not overclaim compatibility."""

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import current_formats as current


class CurrentFormatTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.corpus = self.root / "corpus"
        self.corpus.mkdir()
        source = b"original test input"
        (self.corpus / "sample.caj").write_bytes(source)
        self.row = {
            "id": "sample.caj", "path": "sample.caj", "aliases": [],
            "size_bytes": len(source),
            "git_blob_oid": hashlib.sha1(f"blob {len(source)}\0".encode() + source).hexdigest(),
            "sha256": hashlib.sha256(source).hexdigest(),
            "detected_type": "C8", "variant": "C8", "expected_outcome": "unknown",
            "page_count": 1, "outline_count": 0,
            "python_reference": {"show_status": "success", "convert_status": "skip"},
        }
        self.matrix = self.root / "matrix.json"
        self.matrix.write_text(json.dumps({"schema_version": 1, "samples": [self.row]}))
        self.candidate = self.root / "candidate"
        self.candidate.write_text("original test placeholder")
        self.output = self.root / "outputs"

    def test_absent_corpus_never_invokes_candidate(self):
        with patch.object(current.Commands, "run") as command:
            report = current.run(self.matrix, None, None, None)
        command.assert_not_called()
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertEqual(report["results"], [])

    def test_hash_mismatch_stops_before_writing_or_running(self):
        (self.corpus / "sample.caj").write_bytes(b"changed")
        with patch.object(current.Commands, "run") as command:
            report = current.run(self.matrix, self.corpus, self.candidate, self.output)
        command.assert_not_called()
        self.assertEqual(report["status"], "FAIL")
        self.assertFalse(self.output.exists())

    def test_existing_or_protected_output_is_not_touched(self):
        for output in (self.corpus / "generated", Path(current.__file__).parent / "generated", self.corpus):
            with self.assertRaises(ValueError):
                current.run(self.matrix, self.corpus, self.candidate, output)
        self.output.mkdir()
        sentinel = self.output / "keep"
        sentinel.write_text("keep")
        with self.assertRaises(FileExistsError):
            current.run(self.matrix, self.corpus, self.candidate, self.output)
        self.assertEqual(sentinel.read_text(), "keep")

    def test_unknown_metadata_does_not_invent_no_bookmarks_retry(self):
        replies = [self.reply()] * 4 + [self.reply(0, "not JSON"), self.reply(1, stderr="unsupported content")]
        with patch.object(current.Commands, "run", side_effect=replies) as command:
            report = current.run(self.matrix, self.corpus, self.candidate, self.output)
        self.assertEqual(command.call_count, 6)
        self.assertEqual(len(report["results"][0]["attempts"]), 1)
        self.assertIsNone(report["results"][0]["metadata"])

    @staticmethod
    def reply(code=0, stdout="", stderr=""):
        return {"exit_code": code, "stdout": stdout, "stderr": stderr,
                "seconds": 0.01, "stdout_truncated": False}

    def test_omission_is_explicit_and_warnings_are_not_fidelity_passes(self):
        def command(arguments, label):
            if label.startswith("version-"):
                return self.reply(stdout="test tool")
            if label.endswith("inspect"):
                return self.reply(stdout=json.dumps({"schema_version": 1, "variant": "C8", "page_count": 1}))
            if label.endswith("default"):
                return self.reply(1, stderr="unsupported outlines")
            if label.endswith("omit"):
                Path(arguments[3]).write_bytes(b"synthetic PDF placeholder")
                return self.reply()
            if label.endswith("qpdf"):
                return self.reply(3, stderr="warning")
            if label.endswith("pages"):
                return self.reply(stdout="2\n")
            if label.endswith("outlines"):
                (self.output / f"{label}.stdout").write_bytes(b"")
                return self.reply()
            self.fail(label)
        with patch.object(current.Commands, "run", side_effect=command), patch.object(
            current.current_format_order, "check", return_value={"status": "NOT_RUN"},
        ):
            report = current.run(self.matrix, self.corpus, self.candidate, self.output)
        default, omitted = report["results"][0]["attempts"]
        self.assertFalse(default["no_bookmarks"])
        self.assertEqual(default["conversion_status"], "UNSUPPORTED")
        self.assertTrue(omitted["no_bookmarks"])
        self.assertEqual(omitted["pdf_check"], "WARNING")
        self.assertEqual(omitted["page_count_check"], "FAIL")
        self.assertEqual(omitted["page_order_check"], "NOT_RUN")
        self.assertEqual(omitted["pixel_check"], "NOT_RUN")
        self.assertEqual(report["status"], "COMPLETE")

    def test_outline_identity_survives_page_order_failure(self):
        self.row.update(detected_type="HN", variant="HN")
        self.matrix.write_text(json.dumps({"schema_version": 1, "samples": [self.row]}))

        def command(arguments, label):
            if label.startswith("version-"):
                return self.reply(stdout="test tool")
            if label.endswith("inspect"):
                return self.reply(stdout=json.dumps({"schema_version": 1, "variant": "HN-A", "page_count": 1}))
            if label.endswith("default"):
                Path(arguments[3]).write_bytes(b"synthetic PDF placeholder")
                return self.reply()
            if label.endswith("pages"):
                return self.reply(stdout="1\n")
            if label.endswith("outlines"):
                (self.output / f"{label}.stdout").write_bytes(b"")
                return self.reply()
            return self.reply()
        empty = hashlib.sha256().hexdigest()
        with patch.object(current.Commands, "run", side_effect=command), patch.object(
            current.current_format_order, "check", side_effect=ValueError("order failed"),
        ), patch.object(current.current_format_order, "source_outline_hash", return_value=(0, empty)):
            attempt = current.run(self.matrix, self.corpus, self.candidate, self.output)["results"][0]["attempts"][0]
        self.assertEqual(attempt["page_order_check"], "FAIL")
        self.assertEqual(attempt["source_check_error"], "order failed")
        self.assertEqual(attempt["source_outline_check"], "PASS")

    def test_success_without_output_is_failure(self):
        with patch.object(current.Commands, "run", side_effect=[self.reply()] * 6):
            report = current.run(self.matrix, self.corpus, self.candidate, self.output)
        self.assertEqual(report["results"][0]["attempts"][0]["conversion_status"], "FAIL")

    def test_timeout_and_long_output_are_bounded(self):
        commands = current.Commands(self.root, timeout=0.1)
        result = commands.run([sys.executable, "-c", "import time; time.sleep(10)"], "timeout")
        self.assertEqual(result["exit_code"], "TIMEOUT")
        commands.timeout = 10
        result = commands.run([sys.executable, "-c", "print('x' * 100000)"], "long")
        self.assertTrue(result["stdout_truncated"])
        self.assertLessEqual(len(result["stdout"]), 65537)
        self.assertIsNone(current.metadata(result))

    def test_signal_failure_is_not_misreported_as_unsupported(self):
        self.assertEqual(current.conversion_status(-9, "unsupported outlines"), "FAIL")
        self.assertEqual(current.conversion_status("TIMEOUT", "unsupported outlines"), "FAIL")


if __name__ == "__main__":
    unittest.main()
