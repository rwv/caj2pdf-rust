# SPDX-License-Identifier: MIT
"""Original, clean-clone checks for the optional generic-only pixel oracle."""

from __future__ import annotations

import hashlib
from io import StringIO
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import jbig2_generic_oracle as generic  # noqa: E402
import jbig2_oracle as full  # noqa: E402


def synthetic_case(path: Path) -> dict:
    width, height = 9, 2
    dib = (
        struct.pack("<IiiHHI", 40, width, height, 1, 1, 0)
        + b"\0" * 20 + b"\xff\xff\xff\0\0\0\0\0"
    )
    page = struct.pack(">IIII", width, height, 0, 0) + b"\x01\0\0"
    symbol1 = b"\x08\0\x02\xff" + struct.pack(">II", 1, 1)
    symbol2 = b"\x18\x02\x02\xff" + struct.pack(">II", 1, 0)
    region = struct.pack(">IIII", width, height, 0, 0) + b"\0"
    text = region + b"\x90\x0e" + struct.pack(">I", 1)
    generic_data = region + b"\x04\x02\xff" + b"\xff\xac"
    data_parts = (page, symbol1, symbol2, text, generic_data)
    kinds = (48, 0, 0, 6, 38)
    references = ((), (), (1,), (2,), ())
    record = bytearray(dib)
    segments = []
    for number, (kind, refs, data) in enumerate(zip(kinds, references, data_parts)):
        retained = 1 | (2 if refs else 0)
        header = (
            number.to_bytes(4, "big") + bytes((kind, len(refs) << 5 | retained))
            + bytes(refs) + b"\x01" + len(data).to_bytes(4, "big")
        )
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


def synthetic_manifest(case: dict) -> dict:
    spans = generic.selected_spans(case)
    with case["source_path"].open("rb") as source:
        hashes = {span.number: full.sha256_span(source, span.offset, span.length) for span in spans}
    digest = "a" * 64
    tool = {"version": "synthetic", "binary_sha256": digest}
    return {
        "schema_version": generic.SCHEMA_VERSION,
        "oracle_kind": generic.ORACLE_KIND,
        "matrix_sha256": digest,
        "corpus_revision": generic.CORPUS_REVISION,
        "normalization": generic.NORMALIZATION,
        "backend_independence": "UNVERIFIED",
        "toolchains": {
            "tools": {name: tool.copy() for name in ("qpdf", "pdfimages", "mutool")},
            "backend_evidence": {
                "dynamic_libjbig2dec": {"mutool": None, "pdfimages": None},
                "implementation_independence": "UNVERIFIED",
                "meaning": "Synthetic tool agreement only.",
            },
        },
        "samples": [{
            "id": case["id"], "path": case["path"], "variant": case["variant"],
            "source_sha256": case["source_sha256"],
            "images": [{
                "page": 1, "image": 1, "offset": 0, "length": case["length"],
                "width": case["width"], "height": case["height"],
                "segment_0": {"number": 0, "offset": spans[0].offset,
                              "length": spans[0].length, "encoded_sha256": hashes[0]},
                "segment_4": {"number": 4, "offset": spans[1].offset,
                              "length": spans[1].length, "encoded_sha256": hashes[4]},
                "generic_profile": generic.checked_profile(case),
                "normalized_pixel_sha256": digest, "black_pixels": 0,
                "status": "PASS",
            }],
        }],
    }


class GenericOracleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.case = synthetic_case(self.root / "sample.caj")

    def test_selects_exactly_page_and_generic_segments_for_pdf(self) -> None:
        case = self.case
        spans = generic.selected_spans(case)
        self.assertEqual([span.number for span in spans], [0, 4])
        self.assertEqual(generic.checked_profile(case)["adaptive_pixel"], [2, -1])
        wrapper_sha, segment_hashes = generic.prehash_selected(case, spans)
        record = self.root / "generic-record.caj"
        size, digest = generic.spool_record(case, spans, wrapper_sha, segment_hashes, record)
        original = case["source_path"].read_bytes()
        expected = original[:48] + b"".join(
            original[span.offset:span.offset + span.length] for span in spans
        )
        self.assertEqual(record.read_bytes(), expected)
        self.assertEqual(size, len(expected))
        self.assertEqual(digest, hashlib.sha256(expected).hexdigest())
        pdf = self.root / "generic.pdf"
        full.write_pdf({"source_path": record, "offset": 0, "length": size,
                        "width": case["width"], "height": case["height"]}, pdf, digest)
        self.assertIn(expected[48:], pdf.read_bytes())

    def test_rejects_generic_flags_at_and_invalid_header_spans(self) -> None:
        case = self.case
        original = case["source_path"].read_bytes()
        generic_at = case["segments"][4]["data_offset"]
        for index, changed in ((17, 0x0C), (18, 0x01), (19, 0x00)):
            with self.subTest(field=index):
                data = bytearray(original)
                data[generic_at + index] = changed
                case["source_path"].write_bytes(data)
                with self.assertRaisesRegex(full.OracleError, "generic region profile differs"):
                    generic.checked_profile(case)
        case["source_path"].write_bytes(original)
        page = case["segments"][0]
        page["header_length"] += 1
        with self.assertRaisesRegex(full.OracleError, "escapes the type-3 image"):
            generic.selected_spans(case)
        page["header_length"] -= 1
        case["segments"][4]["data_length"] -= 1
        with self.assertRaisesRegex(full.OracleError, "does not end at the image boundary"):
            generic.selected_spans(case)

    def test_altered_source_sha_is_rejected_before_inventory(self) -> None:
        rows = []
        for number in range(27):
            path = self.root / f"source-{number}.caj"
            data = f"original synthetic source {number}".encode()
            path.write_bytes(data)
            rows.append({
                "id": path.name, "path": path.name, "variant": "HN",
                "size_bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
            })
        with patch.object(full.conformance, "load_matrix", return_value=rows):
            self.assertEqual(len(full.validate_sources(full.DEFAULT_MATRIX, self.root)[0]), 27)
            path = self.root / "source-26.caj"
            path.write_bytes(b"X" * rows[26]["size_bytes"])
            with self.assertRaisesRegex(full.OracleError, "source-26.caj: source SHA-256 differs"):
                full.validate_sources(full.DEFAULT_MATRIX, self.root)

    def test_mutation_after_prehash_stops_before_pdf_tools(self) -> None:
        case = self.case
        spans = generic.selected_spans(case)
        original_prehash = generic.prehash_selected

        def mutate_after_hash(inner_case: dict, inner_spans):
            result = original_prehash(inner_case, inner_spans)
            source = inner_case["source_path"]
            data = bytearray(source.read_bytes())
            data[inner_spans[1].offset + inner_spans[1].length - 1] ^= 1
            source.write_bytes(data)
            return result

        with patch.object(generic, "prehash_selected", side_effect=mutate_after_hash):
            with patch.object(full, "decode_case") as decode:
                with self.assertRaisesRegex(full.OracleError, "changed between hash and spool"):
                    generic.decode_generic_case(case, spans, {})
                decode.assert_not_called()

    def test_qpdf_warning_and_pixel_mismatch_fail_in_generic_path(self) -> None:
        case = self.case
        spans = generic.selected_spans(case)
        calls = []

        def warning(argv: list[str], log: Path, _timeout: int = 60) -> None:
            calls.append(Path(argv[0]).name)
            log.write_bytes(b"WARNING: repaired temporary PDF\n")

        tools = {name: Path(name) for name in ("qpdf", "pdfimages", "mutool")}
        with patch.object(full, "run_tool", side_effect=warning):
            with self.assertRaisesRegex(full.OracleError, "qpdf warned"):
                generic.decode_generic_case(case, spans, tools)
        self.assertEqual(calls, ["qpdf"])

        def mismatched_pixels(argv: list[str], log: Path, _timeout: int = 60) -> None:
            log.write_bytes(b"")
            if argv[0] == "pdfimages":
                Path(argv[-1] + "-000.pbm").write_bytes(b"P4\n9 2\n\x80\x80\x00\x00")
            elif argv[0] == "mutool":
                Path(argv[argv.index("-o") + 1]).write_bytes(b"P4\n9 2\n\x40\x80\x00\x00")

        with patch.object(full, "run_tool", side_effect=mismatched_pixels):
            with self.assertRaisesRegex(full.OracleError, "pixel mismatch at row 0"):
                generic.decode_generic_case(case, spans, tools)

    def test_padding_normalization_reuses_bounded_pbm_rows(self) -> None:
        left = self.root / "left.pbm"
        right = self.root / "right.pbm"
        left.write_bytes(b"P4\n9 1\n\x80\xff")
        right.write_bytes(b"P4\n9 1\n\x80\x80")
        digest, black = full.compare_pbm(left, right, 9, 1)
        self.assertEqual(digest, hashlib.sha256(b"\x80\x80").hexdigest())
        self.assertEqual(black, 2)

    def test_record_is_removed_after_external_tool_failure(self) -> None:
        case = self.case
        seen = []

        def fail_decode(temp_case: dict, _tools: dict, _sha: str):
            seen.append(temp_case["source_path"])
            self.assertTrue(seen[-1].exists())
            raise full.OracleError("synthetic tool failure")

        with patch.object(full, "decode_case", side_effect=fail_decode):
            with self.assertRaisesRegex(full.OracleError, "synthetic tool failure"):
                generic.decode_generic_case(case, generic.selected_spans(case), {})
        self.assertEqual(len(seen), 1)
        self.assertFalse(seen[0].exists())
        self.assertFalse(seen[0].parent.exists())

    def test_failure_report_locates_both_selected_segment_spans(self) -> None:
        case = self.case
        expected = generic.selected_spans(case)
        with (
            patch.object(full, "tool_metadata", return_value={}),
            patch.object(generic, "decode_generic_case", side_effect=generic.OracleError("synthetic mismatch")),
        ):
            _, report = generic.manifest_for_cases([case], {}, full.DEFAULT_MATRIX)
        self.assertEqual((report["status"], report["checked_images"], report["tool_agreements"]),
                         ("FAIL", 1, 0))
        failure, = report["failures"]
        self.assertEqual((failure["id"], failure["page"], failure["image"]),
                         (case["id"], 1, 1))
        self.assertEqual(failure["segments"], [
            {"number": span.number, "offset": span.offset, "length": span.length}
            for span in expected
        ])
        self.assertIn("synthetic mismatch", failure["error"])

    def test_strict_manifest_schema_duplicate_and_semantic_drift(self) -> None:
        baseline = synthetic_manifest(self.case)
        generic.validate_manifest(baseline, expected_images=1)
        paths = [
            (), ("toolchains",), ("toolchains", "tools"),
            ("toolchains", "tools", "qpdf"),
            ("toolchains", "backend_evidence"),
            ("toolchains", "backend_evidence", "dynamic_libjbig2dec"),
            ("samples", 0), ("samples", 0, "images", 0),
            ("samples", 0, "images", 0, "segment_0"),
            ("samples", 0, "images", 0, "segment_4"),
            ("samples", 0, "images", 0, "generic_profile"),
        ]
        for path in paths:
            with self.subTest(path=path):
                changed = json.loads(json.dumps(baseline))
                target = changed
                for part in path:
                    target = target[part]
                target["raw_pixel_bytes"] = "synthetic"
                with self.assertRaisesRegex(full.OracleError, "unknown fields: raw_pixel_bytes"):
                    generic.validate_manifest(changed, expected_images=1)
        changed = json.loads(json.dumps(baseline))
        changed["toolchains"]["tools"]["qpdf"]["binary_sha256"] = "b" * 64
        self.assertEqual(generic.compare_manifest(changed, baseline), [])
        changed["samples"][0]["images"][0]["normalized_pixel_sha256"] = "c" * 64
        self.assertEqual(generic.compare_manifest(changed, baseline),
                         ["synthetic/sample.caj: page 1 image 1"])
        baseline["samples"][0]["images"].append(baseline["samples"][0]["images"][0].copy())
        with self.assertRaisesRegex(full.OracleError, "duplicate image coordinate"):
            generic.validate_manifest(baseline, expected_images=2)

    def test_missing_corpus_and_tools_report_not_run(self) -> None:
        output = StringIO()
        missing_manifest = self.root / "absent.json"
        with patch("sys.stdout", output):
            self.assertEqual(generic.main(["--manifest", str(missing_manifest), "--json"]), 0)
        self.assertEqual(json.loads(output.getvalue())["status"], "NOT_RUN")
        self.assertEqual(json.loads(output.getvalue())["tool_agreements"], 0)
        output = StringIO()
        with (
            patch.object(full, "validate_sources", return_value=([{}] * 27, {})),
            patch.object(full, "tool_path", return_value=None),
            patch.object(full, "run_directory_inventory") as inventory,
            patch("sys.stdout", output),
        ):
            self.assertEqual(generic.main([
                "--manifest", str(missing_manifest), "--corpus-dir", str(self.root), "--json",
            ]), 0)
            inventory.assert_not_called()
        result = json.loads(output.getvalue())
        self.assertEqual((result["status"], result["tool_agreements"], result["source_hashes_checked"]),
                         ("NOT_RUN", 0, 27))

    def test_committed_manifest_covers_all_cases_and_spots(self) -> None:
        manifest = json.loads(generic.DEFAULT_MANIFEST.read_text(encoding="utf-8"))
        generic.validate_manifest(manifest)
        self.assertEqual(
            {sample["id"]: len(sample["images"]) for sample in manifest["samples"]},
            full.EXPECTED_TYPE3,
        )


if __name__ == "__main__":
    unittest.main()
