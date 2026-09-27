# SPDX-License-Identifier: MIT
"""Original synthetic checks for the optional text-only black-box oracle."""

from __future__ import annotations

import hashlib
from io import StringIO
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import jbig2_oracle as full  # noqa: E402
import jbig2_text_oracle as text_oracle  # noqa: E402


def synthetic_case(path: Path, flags: int = 0x900E) -> dict:
    width, height = 9, 2
    dib = (struct.pack("<IiiHHI", 40, width, height, 1, 1, 0)
           + bytes(20) + b"\xff\xff\xff\0\0\0\0\0")
    region = struct.pack(">IIII", width, height, 0, 0) + b"\0"
    data_parts = (
        struct.pack(">IIII", width, height, 0, 0) + b"\x01\0\0",
        b"\x08\0\x02\xff" + struct.pack(">II", 1, 1),
        b"\x18\x02\x02\xff" + struct.pack(">II", 1, 0),
        region + flags.to_bytes(2, "big") + struct.pack(">I", 1) + b"\xff\xac",
        region + b"\x04\x02\xff\xff\xac",
    )
    kinds = (48, 0, 0, 6, 38)
    references = ((), (), (1,), (2,), ())
    record = bytearray(dib)
    segments = []
    for number, (kind, refs, data) in enumerate(zip(kinds, references, data_parts)):
        header = (number.to_bytes(4, "big") + bytes((kind, len(refs) << 5 | 1))
                  + bytes(refs) + b"\x01" + len(data).to_bytes(4, "big"))
        start = len(record)
        record.extend(header)
        record.extend(data)
        segments.append({
            "number": number, "type": kind, "page_association": 1,
            "refs": list(refs), "header_length": len(header),
            "data_offset": start + len(header), "data_length": len(data),
        })
    path.write_bytes(record)
    return {
        "id": "synthetic/sample.caj", "path": "synthetic/sample.caj",
        "variant": "HN", "source_sha256": full.sha256_file(path),
        "source_path": path, "offset": 0, "length": len(record),
        "width": width, "height": height, "page": 1, "image": 1,
        "segments": segments, "coordinate": ("synthetic/sample.caj", 1, 1),
    }


def synthetic_manifest(case: dict, flags: str = "0x900e") -> dict:
    spans = text_oracle.selected_spans(case)
    with case["source_path"].open("rb") as source:
        hashes = {span.number: full.sha256_span(source, span.offset, span.length)
                  for span in spans}
    digest = "a" * 64
    tool = {"version": "synthetic", "binary_sha256": digest}
    image = {
        "page": 1, "image": 1, "offset": 0, "length": case["length"],
        "width": 9, "height": 2, "text_header": {
            "flags": flags, "instances": 1,
            "classification": (text_oracle.ANOMALY if flags == "0xa40c"
                               else text_oracle.STANDARD),
        },
        "normalized_pixel_sha256": digest, "black_pixels": 0,
        "status": "PASS",
    }
    for span in spans:
        image[f"segment_{span.number}"] = {
            "number": span.number, "offset": span.offset,
            "length": span.length, "encoded_sha256": hashes[span.number],
        }
    return {
        "schema_version": text_oracle.SCHEMA_VERSION,
        "oracle_kind": text_oracle.ORACLE_KIND, "matrix_sha256": digest,
        "corpus_revision": text_oracle.CORPUS_REVISION,
        "normalization": text_oracle.NORMALIZATION,
        "backend_independence": "UNVERIFIED",
        "toolchains": {
            "tools": {name: tool.copy() for name in ("qpdf", "pdfimages", "mutool")},
            "backend_evidence": {
                "dynamic_libjbig2dec": {"mutool": None, "pdfimages": None},
                "implementation_independence": "UNVERIFIED",
                "meaning": "Synthetic tool agreement only.",
            },
        },
        "samples": [{"id": case["id"], "path": case["path"],
                     "variant": case["variant"], "source_sha256": case["source_sha256"],
                     "images": [image]}],
    }


class TextOracleTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.case = synthetic_case(self.root / "sample.caj")

    def test_selects_exactly_dib_and_first_four_complete_segments(self) -> None:
        case = self.case
        spans = text_oracle.selected_spans(case)
        self.assertEqual([span.number for span in spans], [0, 1, 2, 3])
        wrapper, hashes = text_oracle.selected_hashes(case, spans)
        record = self.root / "text-record.caj"
        size, digest = text_oracle.spool_record(case, spans, wrapper, hashes, record)
        original = case["source_path"].read_bytes()
        expected = original[:48] + b"".join(original[span.offset:span.offset + span.length]
                                           for span in spans)
        self.assertEqual(record.read_bytes(), expected)
        self.assertNotIn(original[case["segments"][4]["data_offset"]:], expected)
        self.assertEqual((size, digest), (len(expected), hashlib.sha256(expected).hexdigest()))
        pdf = self.root / "text.pdf"
        full.write_pdf({"source_path": record, "offset": 0, "length": size,
                        "width": 9, "height": 2}, pdf, digest)
        self.assertIn(expected[48:], pdf.read_bytes())

    def test_rejects_bad_raw_header_and_noncontiguous_metadata(self) -> None:
        case = self.case
        original = bytearray(case["source_path"].read_bytes())
        first = case["segments"][0]
        original[first["data_offset"] - first["header_length"]] = 1
        case["source_path"].write_bytes(original)
        with self.assertRaisesRegex(full.OracleError, "raw header differs"):
            text_oracle.selected_spans(case)
        case = synthetic_case(case["source_path"])
        case["segments"][2]["header_length"] += 1
        with self.assertRaisesRegex(full.OracleError, "not contiguous"):
            text_oracle.selected_spans(case)
        case = synthetic_case(case["source_path"])
        case["segments"][4]["data_length"] -= 1
        with self.assertRaisesRegex(full.OracleError, "raw header differs"):
            text_oracle.selected_spans(case)

    def test_source_mutation_after_hash_fails_before_any_renderer(self) -> None:
        case = self.case
        spans = text_oracle.selected_spans(case)
        wrapper, hashes = text_oracle.selected_hashes(case, spans)
        data = bytearray(case["source_path"].read_bytes())
        data[spans[2].offset + spans[2].length - 1] ^= 1
        case["source_path"].write_bytes(data)
        with patch.object(full, "write_pdf") as write_pdf:
            with self.assertRaisesRegex(full.OracleError, "changed between hash and spool"):
                text_oracle.decode_case(case, spans, wrapper, hashes, {})
            write_pdf.assert_not_called()

    def test_qpdf_warning_and_renderer_mismatch_fail(self) -> None:
        case = self.case
        spans = text_oracle.selected_spans(case)
        wrapper, hashes = text_oracle.selected_hashes(case, spans)
        tools = {name: Path(name) for name in ("qpdf", "pdfimages", "mutool")}
        calls = []

        def warning(argv: list[str], log: Path, _cap: int, _timeout: int = 60) -> None:
            calls.append(Path(argv[0]).name)
            log.write_bytes(b"warning: synthetic repair\n")

        with patch.object(text_oracle, "run_bounded_tool", side_effect=warning):
            with self.assertRaisesRegex(full.OracleError, "qpdf warned"):
                text_oracle.decode_case(case, spans, wrapper, hashes, tools)
        self.assertEqual(calls, ["qpdf"])

        def mismatch(argv: list[str], log: Path, _cap: int, _timeout: int = 60) -> None:
            log.write_bytes(b"")
            if argv[0] == "pdfimages":
                Path(argv[-1] + "-000.pbm").write_bytes(b"P4\n9 2\n\x80\x80\x00\x00")
            elif argv[0] == "mutool":
                Path(argv[argv.index("-o") + 1]).write_bytes(b"P4\n9 2\n\x40\x80\x00\x00")

        with patch.object(text_oracle, "run_bounded_tool", side_effect=mismatch):
            with self.assertRaisesRegex(full.OracleError, "pixel mismatch at row 0"):
                text_oracle.decode_case(case, spans, wrapper, hashes, tools)

    def test_truncated_pbm_and_resource_caps(self) -> None:
        left = self.root / "left.pbm"
        right = self.root / "right.pbm"
        left.write_bytes(b"P4\n9 2\n\x80")
        right.write_bytes(b"P4\n9 2\n\x80\x00\x00\x00")
        with self.assertRaisesRegex(full.OracleError, "row 0 is truncated"):
            full.compare_pbm(left, right, 9, 2)
        left.write_bytes(b"P5\n9 2\n\x80\x00\x00\x00")
        with self.assertRaisesRegex(full.OracleError, "magic or dimensions differ"):
            full.compare_pbm(left, right, 9, 2)
        with patch.object(text_oracle, "MAX_RECORD_BYTES", 48):
            with self.assertRaisesRegex(full.OracleError, "bounded image limit"):
                text_oracle.selected_spans(self.case)
        spans = text_oracle.selected_spans(self.case)
        wrapper, hashes = text_oracle.selected_hashes(self.case, spans)
        with patch.object(full, "write_pdf", return_value=text_oracle.MAX_PDF_BYTES + 1):
            with patch.object(text_oracle, "run_bounded_tool") as renderer:
                with self.assertRaisesRegex(full.OracleError, "PDF exceeds configured limit"):
                    text_oracle.decode_case(self.case, spans, wrapper, hashes, {})
                renderer.assert_not_called()
        for number in range(9):
            (self.root / f"entry-{number}").write_bytes(b"")
        with self.assertRaisesRegex(full.OracleError, "unexpected temporary entries"):
            text_oracle._bounded_files(self.root, 1, 1)

    def test_bounded_tool_rejects_failure_and_oversized_diagnostic(self) -> None:
        log = self.root / "tool.log"
        python = sys.executable
        with self.assertRaisesRegex(full.OracleError, "exited 7"):
            text_oracle.run_bounded_tool([python, "-c", "raise SystemExit(7)"], log, 8192)
        with self.assertRaisesRegex(full.OracleError, "diagnostic exceeds"):
            text_oracle.run_bounded_tool([python, "-c", "print('X' * 9000)"], log, 8192)
        with self.assertRaisesRegex(full.OracleError, "timed out"):
            text_oracle.run_bounded_tool([python, "-c", "import time; time.sleep(5)"],
                                         log, 8192, timeout=1)
        output = self.root / "oversized.bin"
        with self.assertRaisesRegex(full.OracleError, "exited"):
            text_oracle.run_bounded_tool([
                python, "-B", "-c",
                "import pathlib, sys; pathlib.Path(sys.argv[1]).write_bytes(b'X' * 4096)",
                str(output),
            ], log, 512)
        self.assertLessEqual(output.stat().st_size, 512)

    def test_manifest_schema_joins_and_anomaly_label(self) -> None:
        baseline = synthetic_manifest(self.case)
        text_oracle.validate_manifest(baseline, expected_images=1)
        changed = json.loads(json.dumps(baseline))
        changed["samples"][0]["images"][0]["segment_2"]["offset"] += 1
        with self.assertRaisesRegex(full.OracleError, "position differs"):
            text_oracle.validate_manifest(changed, expected_images=1)
        changed = json.loads(json.dumps(baseline))
        changed["samples"][0]["images"][0]["raw_pixel_bytes"] = "forbidden"
        with self.assertRaisesRegex(full.OracleError, "unknown fields"):
            text_oracle.validate_manifest(changed, expected_images=1)
        changed = json.loads(json.dumps(baseline))
        del changed["toolchains"]["backend_evidence"]["dynamic_libjbig2dec"]["mutool"]
        with self.assertRaisesRegex(full.OracleError, "missing fields"):
            text_oracle.validate_manifest(changed, expected_images=1)
        changed = json.loads(json.dumps(baseline))
        changed["samples"][0]["images"].append(changed["samples"][0]["images"][0].copy())
        with self.assertRaisesRegex(full.OracleError, "duplicate image coordinate"):
            text_oracle.validate_manifest(changed, expected_images=2)
        changed = json.loads(json.dumps(baseline))
        changed["samples"][0]["images"][0]["text_header"]["classification"] = text_oracle.ANOMALY
        with self.assertRaisesRegex(full.OracleError, "anomaly classification differs"):
            text_oracle.validate_manifest(changed, expected_images=1)
        anomaly_case = synthetic_case(self.root / "anomaly.caj", flags=0xA40C)
        anomaly = synthetic_manifest(anomaly_case, flags="0xa40c")
        text_oracle.validate_manifest(anomaly, expected_images=1)
        self.assertEqual(anomaly["samples"][0]["images"][0]["text_header"]["classification"],
                         text_oracle.ANOMALY)
        changed = json.loads(json.dumps(baseline))
        changed["samples"][0]["images"][0]["normalized_pixel_sha256"] = "b" * 64
        self.assertEqual(text_oracle.compare_manifest(changed, baseline),
                         ["synthetic/sample.caj: page 1 image 1"])

    def test_clean_clone_not_run_and_explicit_missing_input_fails(self) -> None:
        def call(args: list[str]) -> tuple[int, dict]:
            output = StringIO()
            with patch.dict(os.environ, {"CAJ2PDF_CORPUS_DIR": ""}), patch("sys.stdout", output):
                result = text_oracle.main(args + ["--json"])
            return result, json.loads(output.getvalue())

        exit_code, report = call([])
        self.assertEqual((exit_code, report["status"], report["tool_agreements"],
                          report["checked_images"]), (0, "NOT_RUN", 0, 0))
        exit_code, report = call(["--corpus-dir=" + str(self.root / "missing")])
        self.assertEqual((exit_code, report["status"]), (1, "FAIL"))
        self.assertEqual((report["completed"], report["failed"], report["skipped"]),
                         (0, 0, text_oracle.EXPECTED_IMAGES))
        exit_code, report = call(["--manifest=" + str(self.root / "missing.json"),
                                  "--write-manifest"])
        self.assertEqual((exit_code, report["status"]), (1, "FAIL"))

    def test_write_manifest_refuses_existing_pixel_drift(self) -> None:
        observed = json.loads(text_oracle.DEFAULT_MANIFEST.read_text(encoding="utf-8"))
        changed = json.loads(json.dumps(observed))
        changed["samples"][0]["images"][0]["normalized_pixel_sha256"] = "b" * 64
        text_oracle.validate_manifest(changed)
        destination = self.root / "changed-manifest.json"
        destination.write_text(json.dumps(changed), encoding="utf-8")
        before = destination.read_bytes()
        case_report = {
            "status": "PASS", "expected_images": text_oracle.EXPECTED_IMAGES,
            "checked_images": text_oracle.EXPECTED_IMAGES,
            "tool_agreements": text_oracle.EXPECTED_IMAGES,
            "completed": text_oracle.EXPECTED_IMAGES, "failed": 0, "skipped": 0,
            "source_hashes_before": 27, "source_hashes_after": 0,
        }
        output = StringIO()
        with (
            patch.dict(os.environ, {"CAJ2PDF_CORPUS_DIR": ""}),
            patch("sys.stdout", output),
            patch.object(full, "validate_sources", return_value=([None] * 27, {})),
            patch.object(full, "tool_path", return_value=Path("/bin/true")),
            patch.object(full, "run_directory_inventory", return_value={}),
            patch.object(full, "checked_cases", return_value=[]),
            patch.object(text_oracle, "preflight", return_value=[]),
            patch.object(text_oracle, "manifest_for_cases", return_value=(observed, case_report)),
        ):
            exit_code = text_oracle.main([
                "--corpus-dir", str(self.root), "--manifest", str(destination),
                "--write-manifest", "--json",
            ])
        report = json.loads(output.getvalue())
        self.assertEqual((exit_code, report["status"]), (1, "FAIL"))
        self.assertIn("manifest_differences", report)
        self.assertEqual(destination.read_bytes(), before)
        self.assertEqual((report["completed"], report["failed"], report["skipped"]),
                         (text_oracle.EXPECTED_IMAGES, 0, 0))

    def test_committed_manifest_covers_all_coordinates(self) -> None:
        manifest = json.loads(text_oracle.DEFAULT_MANIFEST.read_text(encoding="utf-8"))
        text_oracle.validate_manifest(manifest)
        self.assertEqual(sum(len(sample["images"]) for sample in manifest["samples"]), 546)
        kinds = [image["text_header"]["classification"]
                 for sample in manifest["samples"] for image in sample["images"]]
        self.assertEqual((kinds.count(text_oracle.STANDARD), kinds.count(text_oracle.ANOMALY)),
                         (545, 1))


if __name__ == "__main__":
    unittest.main()
